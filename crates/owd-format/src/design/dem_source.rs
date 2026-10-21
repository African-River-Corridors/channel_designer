//! Where a DEM came from, for `owd fetch-dem` (spec 0002, T4): the Copernicus GLO-30 tile names a
//! reach needs, the tiles joined into one DEM, the licence notice, and the record in the document.
//! No network, no files: the caller (the CLI) fetches and writes.
//!
//! GLO-30 tiles are 1 degree by 1 degree and are named by their south-west corner. The grid is the
//! same for every tile in a latitude band and changes at 50, 60, 70, 75, 80 and 85 degrees, so tiles
//! are joined only when they share one grid. FABDEM is not offered: its licence is non-commercial.

use super::crs::{Crs, Utm};
use super::geojson::all_positions;
use super::session::{DesignError, Session};
use crate::geotiff::Dem;
use owd_core::geom::Pt;
use serde_json::{Value, json};
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

/// The name of the dataset, as recorded in the document.
pub const SOURCE_NAME: &str = "Copernicus GLO-30";

/// The licence notice the Copernicus DEM licence asks for. Written into the document and next to the
/// tiles.
pub const COPERNICUS_NOTICE: &str = "Produced using Copernicus WorldDEM-30 \u{a9} DLR e.V. 2010-2014 and \u{a9} Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved.";

/// The public open-data mirror of the tiles.
pub const DEFAULT_MIRROR: &str = "https://copernicus-dem-30m.s3.eu-central-1.amazonaws.com";

/// A 1 degree tile, by the integer longitude and latitude of its south-west corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TileId {
    pub lon: i32,
    pub lat: i32,
}

impl TileId {
    /// `Copernicus_DSM_COG_10_N05_00_W003_00_DEM`.
    pub fn name(self) -> String {
        format!(
            "Copernicus_DSM_COG_10_{}{:02}_00_{}{:03}_00_DEM",
            if self.lat < 0 { 'S' } else { 'N' },
            self.lat.abs(),
            if self.lon < 0 { 'W' } else { 'E' },
            self.lon.abs()
        )
    }

    /// The tile's URL on a mirror: `<mirror>/<name>/<name>.tif`.
    pub fn url(self, mirror: &str) -> String {
        format!(
            "{}/{}/{}.tif",
            mirror.trim_end_matches('/'),
            self.name(),
            self.name()
        )
    }
}

/// The tiles that cover a box in longitude and latitude (degrees): `(west, south, east, north)`.
pub fn tiles_for(bbox: (f64, f64, f64, f64)) -> Vec<TileId> {
    let (w, s, e, n) = bbox;
    let (lon0, lon1) = (w.floor() as i32, e.floor() as i32);
    let (lat0, lat1) = (s.floor() as i32, n.floor() as i32);
    let mut out = Vec::new();
    for lat in lat0..=lat1 {
        for lon in lon0..=lon1 {
            out.push(TileId { lon, lat });
        }
    }
    out
}

/// Join tiles that sit side by side on one grid into one DEM, with no resampling. The tiles must be
/// the same size and pixel size and fill a rectangle. A single tile is returned as it is.
pub fn mosaic(tiles: &[Dem]) -> Result<Dem, String> {
    let first = tiles.first().ok_or("no tiles to join")?;
    if tiles.len() == 1 {
        return Ok(first.clone());
    }
    let [a, _, _, _, e, _] = first.geotransform;
    for t in tiles {
        if t.width != first.width
            || t.height != first.height
            || t.geotransform[0] != a
            || t.geotransform[4] != e
        {
            return Err("the reach crosses a latitude where the GLO-30 grid changes (50, 60, 70, 75, 80 or 85 degrees), so its tiles do not share one grid and cannot be joined without resampling. Fetch the two parts as two channel designs, or give a DEM of your own".into());
        }
    }
    let left = tiles
        .iter()
        .map(|t| t.geotransform[2])
        .fold(f64::INFINITY, f64::min);
    let top = tiles
        .iter()
        .map(|t| t.geotransform[5])
        .fold(f64::NEG_INFINITY, f64::max);
    let place = |t: &Dem| -> (usize, usize) {
        (
            ((t.geotransform[2] - left) / a).round() as usize,
            ((t.geotransform[5] - top) / e).round() as usize,
        )
    };
    let (mut w, mut h) = (0, 0);
    for t in tiles {
        let (c, r) = place(t);
        w = w.max(c + t.width);
        h = h.max(r + t.height);
    }
    if w * h != tiles.len() * first.width * first.height {
        return Err("the tiles do not fill a rectangle without gaps or overlaps".into());
    }
    let mut data = vec![f64::NAN; w * h];
    let mut filled = vec![false; w * h];
    for t in tiles {
        let (c0, r0) = place(t);
        for r in 0..t.height {
            for c in 0..t.width {
                let i = (r0 + r) * w + c0 + c;
                if filled[i] {
                    return Err("two tiles cover the same pixels".into());
                }
                filled[i] = true;
                data[i] = t.data[r * t.width + c];
            }
        }
    }
    Ok(Dem {
        width: w,
        height: h,
        data,
        geotransform: [a, 0.0, left, 0.0, e, top],
        nodata: first.nodata,
        epsg: first.epsg,
    })
}

/// A DEM as GeoTIFF bytes: one float32 band, north up, `PixelIsArea`, with its EPSG code. Nodata
/// pixels are written as the DEM's nodata value, or NaN when it has none.
pub fn write_geotiff(dem: &Dem) -> Result<Vec<u8>, String> {
    let epsg = dem.epsg.ok_or("the DEM has no EPSG code to write")?;
    let epsg = u16::try_from(epsg).map_err(|_| "the EPSG code does not fit a GeoTIFF key")?;
    let [a, _, c, _, e, f] = dem.geotransform;
    let fill = dem.nodata.unwrap_or(f64::NAN);
    let data: Vec<f32> = (0..dem.height)
        .flat_map(|r| (0..dem.width).map(move |col| (r, col)))
        .map(|(r, col)| {
            let v = dem.value(r, col);
            (if v.is_nan() { fill } else { v }) as f32
        })
        .collect();
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).map_err(|e| e.to_string())?;
        let mut img = enc
            .new_image::<colortype::Gray32Float>(dem.width as u32, dem.height as u32)
            .map_err(|e| e.to_string())?;
        let geo = if epsg == 4326 { 2 } else { 1 };
        let epsg_key = if epsg == 4326 { 2048 } else { 3072 };
        let keys: [u16; 16] = [
            1, 1, 0, 3, 1024, 0, 1, geo, 1025, 0, 1, 1, epsg_key, 0, 1, epsg,
        ];
        let enc_ = img.encoder();
        enc_.write_tag(Tag::Unknown(33550), &[a, -e, 0.0][..])
            .map_err(|x| x.to_string())?;
        enc_.write_tag(Tag::Unknown(33922), &[0.0, 0.0, 0.0, c, f, 0.0][..])
            .map_err(|x| x.to_string())?;
        enc_.write_tag(Tag::Unknown(34735), &keys[..])
            .map_err(|x| x.to_string())?;
        if let Some(nd) = dem.nodata {
            enc_.write_tag(Tag::Unknown(42113), format!("{nd}").as_str())
                .map_err(|x| x.to_string())?;
        }
        img.write_data(&data).map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

/// What the DEM source records about one tile.
#[derive(Debug, Clone, PartialEq)]
pub struct TileRecord {
    pub name: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    pub url: String,
}

impl Session {
    /// The extent of the inputs in longitude and latitude, widened by `margin_m` metres:
    /// `(west, south, east, north)`. The river polygon, else the centreline, else the banks.
    pub fn extent_wgs84(&self, margin_m: f64) -> Result<(f64, f64, f64, f64), DesignError> {
        let utm: Option<Utm> = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        let inp = &self.doc["inputs"];
        let mut pts: Vec<Pt> = Vec::new();
        if utm.is_some() {
            for key in ["river_polygon", "centreline", "left_bank", "right_bank"] {
                pts = all_positions(&inp[key]);
                if !pts.is_empty() {
                    break;
                }
            }
        }
        let Some(utm) = utm.filter(|_| !pts.is_empty()) else {
            return self.reach_extent_wgs84(margin_m);
        };
        let (mut w, mut s, mut e, mut n) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for p in pts {
            // The margin in metres, in the project CRS, before converting: a box around each point.
            for (dx, dy) in [
                (-margin_m, -margin_m),
                (margin_m, margin_m),
                (-margin_m, margin_m),
                (margin_m, -margin_m),
            ] {
                let [lon, lat] = utm
                    .inverse(p[0] + dx, p[1] + dy)
                    .map_err(DesignError::Input)?;
                w = w.min(lon);
                e = e.max(lon);
                s = s.min(lat);
                n = n.max(lat);
            }
        }
        Ok((w, s, e, n))
    }

    /// The extent of the reach the study was started from (spec 0012), for a document that has no
    /// banks yet: the box round its start, via points, end and trace, widened by `margin_m`. The
    /// route itself is not stored as geometry, so a river that wanders far from the straight box needs a
    /// larger margin (or the banks, which give the true extent).
    fn reach_extent_wgs84(&self, margin_m: f64) -> Result<(f64, f64, f64, f64), DesignError> {
        let no_extent =
            || DesignError::Input("import the river first: there is no extent to fetch for".into());
        let reach = self.reach().ok_or_else(no_extent)?;
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let mut take = |v: &Value| {
            if let (Some(x), Some(y)) = (v[0].as_f64(), v[1].as_f64()) {
                pts.push([x, y]);
            }
        };
        take(&reach["start"]);
        take(&reach["end"]);
        for v in reach["via"].as_array().into_iter().flatten() {
            take(v);
        }
        for v in reach["trace"].as_array().into_iter().flatten() {
            take(v);
        }
        for part in reach["route"].as_array().into_iter().flatten() {
            for v in part["line"].as_array().into_iter().flatten() {
                take(v);
            }
        }
        if pts.is_empty() {
            return Err(no_extent());
        }
        let mid_lat = pts.iter().map(|p| p[1]).sum::<f64>() / pts.len() as f64;
        let dlat = margin_m / 111_320.0;
        let dlon = margin_m / (111_320.0 * libm::cos(mid_lat.to_radians()).max(0.05));
        let (mut w, mut s, mut e, mut n) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for p in pts {
            w = w.min(p[0] - dlon);
            e = e.max(p[0] + dlon);
            s = s.min(p[1] - dlat);
            n = n.max(p[1] + dlat);
        }
        Ok((w, s, e, n))
    }

    /// Record where the DEM came from: the dataset, its tiles with their hashes, and the licence
    /// notice. Call after [`Session::set_dem`].
    pub fn set_dem_source(&mut self, tiles: &[TileRecord]) -> Result<(), DesignError> {
        let dem = self.doc["inputs"]["dem"]
            .as_object_mut()
            .ok_or_else(|| DesignError::Input("the document has no DEM to describe".into()))?;
        dem.insert(
            "source".into(),
            json!({
                "name": SOURCE_NAME,
                "licence": COPERNICUS_NOTICE,
                "tiles": tiles.iter().map(|t| json!({
                    "name": t.name, "file": t.file, "sha256": t.sha256,
                    "bytes": t.bytes, "url": t.url,
                })).collect::<Vec<Value>>(),
            }),
        );
        Ok(())
    }

    /// The tiles the DEM source recorded, if the DEM was fetched.
    pub fn dem_source_tiles(&self) -> Vec<TileRecord> {
        self.doc["inputs"]["dem"]["source"]["tiles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| TileRecord {
                        name: t["name"].as_str().unwrap_or("").into(),
                        file: t["file"].as_str().unwrap_or("").into(),
                        sha256: t["sha256"].as_str().unwrap_or("").into(),
                        bytes: t["bytes"].as_u64().unwrap_or(0),
                        url: t["url"].as_str().unwrap_or("").into(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}
