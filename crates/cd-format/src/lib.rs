//! The `eval` seam the Python package calls (`channel_designer._core`), and the `.cdesign` document.

pub mod design;
mod eval;
mod eval_rules;
pub mod geotiff;
pub mod host;

pub use eval::{ENTRY_POINTS, EvalError, eval};
