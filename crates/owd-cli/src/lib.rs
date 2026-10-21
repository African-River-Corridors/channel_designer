//! The commands of `cdesign` as a library, so the desktop app can run the same ones (for example
//! `fetch-dem`) instead of keeping a second copy. The binary in `main.rs` only parses arguments.

pub mod commands;
pub mod fetch;
