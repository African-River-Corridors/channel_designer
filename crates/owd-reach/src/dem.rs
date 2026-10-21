//! The Copernicus GLO-30 DEM for a reach (source `cop-dem-glo30`): which 1 degree tiles a bounding box
//! needs, as names and plain download links, and one DEM made from tiles fetched or dropped in.
//!
//! The bucket sends no CORS header, so on the web the page lists the links ([`download_list`]), the
//! user downloads the tiles and drops them back in ([`crate::DroppedFiles`]); native fetches them.
//! A tile the source does not hold (much of the sea) is reported by name, never skipped.

use crate::error::ReachError;
use crate::fetch::{Fetch, FetchError};
use owd_format::design::dem_source::{DEFAULT_MIRROR, TileId, mosaic, tiles_for};
use owd_format::geotiff::{Dem, read_bytes};

/// A 1 degree tile of the DEM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemTile(pub TileId);

impl DemTile {
    /// `Copernicus_DSM_COG_10_N05_00_W003_00_DEM`.
    pub fn name(self) -> String {
        self.0.name()
    }

    /// The file name a downloaded tile keeps: `<name>.tif`.
    pub fn file_name(self) -> String {
        format!("{}.tif", self.0.name())
    }

    /// The download link on the public bucket.
    pub fn url(self) -> String {
        self.0.url(DEFAULT_MIRROR)
    }
}

/// Widen a box `[west, south, east, north]` by `margin_m` metres on every side.
pub fn widen(bbox: [f64; 4], margin_m: f64) -> [f64; 4] {
    let mid_lat = (bbox[1] + bbox[3]) / 2.0;
    let dlat = margin_m / 111_320.0;
    let dlon = margin_m / (111_320.0 * mid_lat.to_radians().cos().max(0.05));
    [
        bbox[0] - dlon,
        bbox[1] - dlat,
        bbox[2] + dlon,
        bbox[3] + dlat,
    ]
}

/// The tiles that cover a box `[west, south, east, north]` widened by `margin_m`, south-west first.
pub fn tiles_for_bbox(bbox: [f64; 4], margin_m: f64) -> Vec<DemTile> {
    let b = widen(bbox, margin_m);
    tiles_for((b[0], b[1], b[2], b[3]))
        .into_iter()
        .map(DemTile)
        .collect()
}

/// The links a web user downloads: `(file name, URL)` for each tile.
pub fn download_list(bbox: [f64; 4], margin_m: f64) -> Vec<(String, String)> {
    tiles_for_bbox(bbox, margin_m)
        .into_iter()
        .map(|t| (t.file_name(), t.url()))
        .collect()
}

/// What fetching a set of tiles gave.
#[derive(Debug, Default)]
pub struct Fetched {
    pub found: Vec<(DemTile, Vec<u8>)>,
    /// Tiles the source does not hold.
    pub withheld: Vec<DemTile>,
}

/// Fetch each tile. A tile the source does not hold goes in `withheld`; any other failure is an error.
pub fn fetch_tiles(fetch: &dyn Fetch, tiles: &[DemTile]) -> Result<Fetched, ReachError> {
    let mut out = Fetched::default();
    for t in tiles {
        match fetch.get(&t.url()) {
            Ok(b) => out.found.push((*t, b)),
            Err(FetchError::NotFound(_)) => out.withheld.push(*t),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// One DEM from the tiles of a box: fetched (native) or dropped in (web). Tiles on one grid are
/// joined with no resampling. A withheld tile is an error that names it.
pub fn dem_from_tiles(fetch: &dyn Fetch, tiles: &[DemTile]) -> Result<Dem, ReachError> {
    let f = fetch_tiles(fetch, tiles)?;
    if !f.withheld.is_empty() {
        return Err(ReachError::Input(withheld_message(&f.withheld)));
    }
    let dems = f
        .found
        .iter()
        .map(|(t, b)| read_bytes(b).map_err(|e| ReachError::Parse(format!("{}: {e}", t.name()))))
        .collect::<Result<Vec<_>, _>>()?;
    mosaic(&dems).map_err(ReachError::Input)
}

/// The sentence that names the tiles the source does not hold.
pub fn withheld_message(withheld: &[DemTile]) -> String {
    let names: Vec<String> = withheld.iter().map(|t| t.name()).collect();
    format!(
        "the source has no DEM tile for {}: Copernicus does not publish tiles over much of the sea. \
         Give a DEM of your own, or ask for a box that stays on land",
        names.join(", ")
    )
}
