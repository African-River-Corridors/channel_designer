//! The calculation core of channel_designer. Each module mirrors one Python module of the frozen
//! reference (`parity/frozen_python`). No I/O.

pub mod alignment;
pub mod cut_model;
pub mod geom;
pub(crate) mod numpy;
pub mod pyrepr;
pub mod rules;
pub mod synthetic;

/// Python's `min(a, b)`: `b` only if `b < a`. Differs from `f64::min` when a value is NaN.
pub(crate) fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Python's `max(a, b)`: `b` only if `b > a`. Differs from `f64::max` when a value is NaN.
pub(crate) fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}
