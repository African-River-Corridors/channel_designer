//! The channel design document and the design session (spec 0002, ADR 0004).

pub mod crs;
pub mod geojson;
pub mod hash;
pub mod schema;
pub mod session;

pub use geojson::Role;
pub use session::{DesignError, ImportReport, Session, Stage, StageStatus, Status};
