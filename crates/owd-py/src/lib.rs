//! The Python extension `owd._core` (ticket 35): the Rust core, reached through the
//! same JSON seam the parity tests use. The Python package builds its public functions on this.
//!
//! `eval(entry_point, input_json, callbacks=None) -> json` runs one call. A refusal that the
//! frozen package raised as `ValueError` is a normal result (`{"error": {...}}`); a malformed call
//! raises `ValueError`.
//!
//! `callbacks` is a list of Python functions. A spec `{"kind": "callback", "id": n}` in the call
//! reads as a call to `callbacks[n]`. Each callback takes the Rust arguments as Python values and
//! answers a pair `(tag, value)`: `("f", float)`, `("a", list[float])`, `("s", str)` or
//! `("j", json_text)`. An exception in a callback does not unwind through Rust: the first one is
//! kept, the call finishes with NaN answers, and `eval` raises that exception.

use owd_format::host::{Host, Val, with_host};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};
use std::sync::{Arc, Mutex};

struct PyHost {
    callbacks: Vec<Py<PyAny>>,
    error: Mutex<Option<PyErr>>,
}

impl PyHost {
    fn run(&self, py: Python<'_>, id: u64, args: &[Val]) -> PyResult<Val> {
        let f = self
            .callbacks
            .get(id as usize)
            .ok_or_else(|| PyValueError::new_err(format!("no callback {id}")))?;
        let mut items: Vec<Py<PyAny>> = Vec::with_capacity(args.len());
        for a in args {
            items.push(match a {
                Val::F(x) => x.into_pyobject(py)?.into_any().unbind(),
                Val::A(v) => v.clone().into_pyobject(py)?.into_any().unbind(),
                Val::S(s) => s.into_pyobject(py)?.into_any().unbind(),
                Val::J(j) => j.to_string().into_pyobject(py)?.into_any().unbind(),
            });
        }
        let out = f.call1(py, PyTuple::new(py, items)?)?;
        let (tag, value): (String, Bound<'_, PyAny>) = out.extract(py)?;
        Ok(match tag.as_str() {
            "f" => Val::F(value.extract()?),
            "a" => Val::A(value.extract()?),
            "s" => Val::S(value.extract()?),
            "j" => {
                let text: String = value.extract()?;
                Val::J(
                    serde_json::from_str(&text)
                        .map_err(|e| PyValueError::new_err(e.to_string()))?,
                )
            }
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown callback tag {other}"
                )));
            }
        })
    }
}

impl Host for PyHost {
    fn call(&self, id: u64, args: &[Val]) -> Result<Val, String> {
        if self.error.lock().map(|e| e.is_some()).unwrap_or(true) {
            return Err("an earlier callback failed".into());
        }
        Python::attach(|py| match self.run(py, id, args) {
            Ok(v) => Ok(v),
            Err(e) => {
                let text = e.to_string();
                if let Ok(mut slot) = self.error.lock() {
                    slot.get_or_insert(e);
                }
                Err(text)
            }
        })
    }
}

/// Run one call through the Rust core. `input_json` is `{"fn": ..., "args": {...}}`.
#[pyfunction]
#[pyo3(signature = (entry_point, input_json, callbacks=None))]
fn eval(
    py: Python<'_>,
    entry_point: &str,
    input_json: &str,
    callbacks: Option<Vec<Py<PyAny>>>,
) -> PyResult<String> {
    let input: serde_json::Value = serde_json::from_str(input_json)
        .map_err(|e| PyValueError::new_err(format!("the call is not JSON: {e}")))?;
    let host = Arc::new(PyHost {
        callbacks: callbacks.unwrap_or_default(),
        error: Mutex::new(None),
    });
    let result = with_host(host.clone(), || owd_format::eval(entry_point, &input));
    if let Some(e) = host.error.lock().ok().and_then(|mut s| s.take()) {
        return Err(e);
    }
    let _ = py;
    let out = result.map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(out.to_string())
}

/// Draw a design (spec 0004): the files of every sheet, as `(name, bytes)` pairs. `design` is the text
/// of the `.owd` file, `dem` the bytes of the DEM file it names, `options_json` the draw options (see
/// `owd_format::drawing::DrawOptions::from_json`). The same call as `owd draw`, so the bytes are the same.
#[pyfunction]
#[pyo3(signature = (design, dem=None, options_json="{}"))]
fn draw<'py>(
    py: Python<'py>,
    design: &str,
    dem: Option<&[u8]>,
    options_json: &str,
) -> PyResult<Vec<(String, Bound<'py, PyBytes>)>> {
    let options: serde_json::Value = serde_json::from_str(options_json)
        .map_err(|e| PyValueError::new_err(format!("the options are not JSON: {e}")))?;
    let options = owd_format::drawing::DrawOptions::from_json(&options)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let files = owd_format::drawing::files_from_text(design, dem, &options)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(files
        .into_iter()
        .map(|(name, data)| (name, PyBytes::new(py, &data)))
        .collect())
}

/// The names of every entry point the core knows.
#[pyfunction]
fn entry_points() -> Vec<&'static str> {
    owd_format::ENTRY_POINTS.to_vec()
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(eval, m)?)?;
    m.add_function(wrap_pyfunction!(entry_points, m)?)?;
    m.add_function(wrap_pyfunction!(draw, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
