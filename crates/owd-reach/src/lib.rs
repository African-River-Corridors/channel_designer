//! Reach inputs made from open data (spec 0012, ADR 0010).
//!
//! Pure Rust: no file, clock or Python access, and network access only through [`Fetch`]. The CLI
//! and desktop give it a native HTTP client; the web gives it the browser's `fetch` in the session
//! Worker. The logic never knows which. It builds for native and for `wasm32-unknown-unknown`.

pub mod bankcheck;
pub mod banks;
pub mod dem;
pub mod edgefit;
pub mod error;
pub mod fetch;
pub mod geo;
pub mod jrc;
pub mod level;
pub mod overlay;
pub mod overpass;
pub mod provenance;
pub mod reach;
pub mod route;
pub mod sentinel2;
pub mod sources;
pub mod sword;
pub mod tiff;
pub mod water;
pub mod zip;

pub use error::ReachError;
pub use fetch::{DroppedFiles, Fetch, FetchError, ReplayFetch};
pub use sources::{Source, registry, source};
