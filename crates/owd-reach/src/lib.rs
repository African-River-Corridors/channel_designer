//! Reach inputs made from open data (spec 0012, ADR 0010).
//!
//! Pure Rust: no file, clock or Python access, and network access only through [`Fetch`]. The CLI
//! and desktop give it a native HTTP client; the web gives it the browser's `fetch` in the session
//! Worker. The logic never knows which. It builds for native and for `wasm32-unknown-unknown`.

pub mod fetch;
pub mod provenance;
pub mod sources;

pub use fetch::{Fetch, FetchError, ReplayFetch};
pub use sources::{Source, registry, source};
