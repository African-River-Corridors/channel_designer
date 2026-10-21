//! `owd fetch-dem`: the only network code in the tool (spec 0002, T4). It lives here, never in
//! the core or the design session. The session plans (tile names, joining, the licence record); this
//! module fetches and writes files.
//!
//! A mirror is an `http(s)` URL (the public Copernicus open-data bucket by default) or, for tests and
//! offline work, a folder laid out the same way: `<mirror>/<tile>/<tile>.tif`.

use owd_format::design::dem_source::{TileId, TileRecord};
use std::path::Path;

/// Something that returns the bytes of a tile.
pub trait Fetcher {
    fn get(&self, tile: TileId, mirror: &str) -> Result<Vec<u8>, String>;
}

/// The real fetcher: HTTPS for a URL mirror, a file read for a folder mirror.
pub struct Mirror;

/// Tiles are about 50 MB; refuse anything absurdly larger.
const MAX_TILE_BYTES: u64 = 400_000_000;

impl Fetcher for Mirror {
    fn get(&self, tile: TileId, mirror: &str) -> Result<Vec<u8>, String> {
        let url = tile.url(mirror);
        if mirror.starts_with("http://") || mirror.starts_with("https://") {
            let mut resp = ureq::get(&url)
                .call()
                .map_err(|e| format!("cannot fetch {url}: {e}"))?;
            resp.body_mut()
                .with_config()
                .limit(MAX_TILE_BYTES)
                .read_to_vec()
                .map_err(|e| format!("cannot read {url}: {e}"))
        } else {
            let path = Path::new(mirror)
                .join(tile.name())
                .join(format!("{}.tif", tile.name()));
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
        }
    }
}

/// A tile record for bytes just fetched or found.
pub fn record(tile: TileId, mirror: &str, file: &str, bytes: &[u8]) -> TileRecord {
    TileRecord {
        name: tile.name(),
        file: file.to_string(),
        sha256: owd_format::design::hash::sha256_hex(bytes),
        bytes: bytes.len() as u64,
        url: tile.url(mirror),
    }
}
