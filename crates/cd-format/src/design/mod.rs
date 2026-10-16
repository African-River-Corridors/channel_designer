//! The channel design document and the design session (spec 0002, ADR 0004).

pub mod alignment;
pub mod crs;
pub mod geojson;
pub mod geopackage;
pub mod hash;
pub mod schema;
pub mod session;
pub mod terrain;
pub mod volumes;

pub use alignment::{AlignMethod, Terrain};
pub use geojson::Role;
pub use session::{DesignError, ImportReport, Session, Stage, StageStatus, Status, date_from_unix};
pub use volumes::DemCheck;
