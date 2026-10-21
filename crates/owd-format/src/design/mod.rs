//! The channel design document and the design session (spec 0002, ADR 0004).

pub mod alignment;
pub mod bank_edit;
pub mod bed;
pub mod crs;
pub mod dem_source;
pub mod export;
pub mod fits;
pub mod geojson;
pub mod geopackage;
pub mod hash;
pub mod hillshade;
pub mod map;
pub mod optimise;
pub mod provenance;
pub mod reach;
pub mod rule_sets;
pub mod schema;
pub mod sections;
pub mod session;
pub mod table;
pub mod terrain;
pub mod volumes;
pub mod water;

pub use alignment::{AlignMethod, Terrain};
pub use geojson::Role;
pub use session::{DesignError, ImportReport, Session, Stage, StageStatus, Status, date_from_unix};
pub use volumes::DemCheck;
pub use water::WaterSampling;
