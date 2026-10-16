//! The Python extension `channel_designer._core` (ticket 35): the Rust core, reached through the
//! same JSON seam the parity tests use. The Python package builds its public functions on this.
//!
//! `eval(entry_point, input_json) -> output_json` runs one call. A Python `ValueError` of the
//! frozen package is a normal result (`{"error": {...}}`); a malformed call raises `ValueError`.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Run one call through the Rust core. `input_json` is `{"fn": ..., "args": {...}}`.
#[pyfunction]
fn eval(entry_point: &str, input_json: &str) -> PyResult<String> {
    let input: serde_json::Value = serde_json::from_str(input_json)
        .map_err(|e| PyValueError::new_err(format!("the call is not JSON: {e}")))?;
    let out = cd_format::eval(entry_point, &input).map_err(|e| PyValueError::new_err(e.to_string()))?;
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
