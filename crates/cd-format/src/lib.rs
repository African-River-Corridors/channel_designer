//! The parity seam (`eval`) now; the `.cdesign` document and its validation in spec 0002.

mod eval;
pub mod parity;

pub use eval::{ENTRY_POINTS, EvalError, eval};
