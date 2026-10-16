//! The Python extension `channel_designer._core` (ticket 35): the Rust core, reached through the
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

use cd_format::host::{Host, Val, with_host};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
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
    let result = with_host(host.clone(), || cd_format::eval(entry_point, &input));
    if let Some(e) = host.error.lock().ok().and_then(|mut s| s.take()) {
        return Err(e);
    }
    let _ = py;
    let out = result.map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(out.to_string())
}

/// The names of every entry point the core knows.
#[pyfunction]
fn entry_points() -> Vec<&'static str> {
    cd_format::ENTRY_POINTS.to_vec()
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(eval, m)?)?;
    m.add_function(wrap_pyfunction!(entry_points, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
