//! The `eval` seam the Python package calls (`owd._core`), and the `.owd` document (`.cdesign` before the rename, the same contents).

pub mod design;
mod eval;
mod eval_rules;
pub mod geotiff;
pub mod host;

pub use eval::{ENTRY_POINTS, EvalError, eval};
