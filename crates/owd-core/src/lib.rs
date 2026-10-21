//! The calculation core of owd. Each module mirrors one Python module of the frozen
//! reference (the Python package at the tag `py-reference`). No I/O.

pub mod alignment;
/// Python-style number formatting, for the text of messages and printed results.
pub use ovcd_core::pyfmt;

pub mod bed;
pub mod cut_model;
pub mod geom;
pub(crate) mod numpy;
pub mod optimise;
pub mod pyrepr;
pub mod rules;
pub mod synthetic;
pub mod volumes;

/// Python's `min(a, b)`: `b` only if `b < a`. Differs from `f64::min` when a value is NaN.
pub(crate) fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Python's `max(a, b)`: `b` only if `b > a`. Differs from `f64::max` when a value is NaN.
pub(crate) fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// Python's `sum()` of floats, starting from the int 0. CPython 3.12 sums floats with Neumaier
/// compensation, so a plain left-to-right fold differs in the last bit. The reference is printed
/// by 3.12.
pub(crate) fn py_sum(xs: impl IntoIterator<Item = f64>) -> f64 {
    let mut sum = 0.0_f64;
    let mut comp = 0.0_f64;
    for x in xs {
        if !x.is_finite() {
            sum += x;
            continue;
        }
        let t = sum + x;
        if sum.abs() >= x.abs() {
            comp += (sum - t) + x;
        } else {
            comp += (x - t) + sum;
        }
        sum = t;
    }
    if comp != 0.0 && comp.is_finite() {
        sum += comp;
    }
    sum
}
