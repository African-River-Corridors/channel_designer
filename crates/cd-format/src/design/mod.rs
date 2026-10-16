//! The channel design document and the design session (spec 0002, ADR 0004).

pub mod alignment;
pub mod crs;
pub mod export;
pub mod geojson;
pub mod geopackage;
pub mod hash;
pub mod optimise;
pub mod schema;
pub mod session;
pub mod terrain;
pub mod volumes;
pub mod water;

pub use alignment::{AlignMethod, Terrain};
pub use geojson::Role;
pub use session::{DesignError, ImportReport, Session, Stage, StageStatus, Status, date_from_unix};
pub use volumes::DemCheck;
pub use water::WaterSampling;
