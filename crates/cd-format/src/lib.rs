//! The parity seam (`eval`) now; the `.cdesign` document and its validation in spec 0002.

pub mod design;
mod eval;
mod eval_rules;
pub mod parity;
pub mod search;

pub use eval::{ENTRY_POINTS, EvalError, eval};
