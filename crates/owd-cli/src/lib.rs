//! The commands of `owd` as a library, so the desktop app can run the same ones (for example
//! `fetch-dem`) instead of keeping a second copy. The argument parser is in `cli`; the `owd` binary and the `owd` alias both call it.

pub mod cli;
pub mod commands;
pub mod fetch;
