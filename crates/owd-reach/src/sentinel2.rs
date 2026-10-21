//! Sentinel-2 L2A imagery for the edge fit (source `sentinel2-l2a`): a dry-season, low-cloud scene found
//! in the Earth Search STAC, and a window of its bands read from the Cloud Optimized GeoTIFFs by range
//! requests (`tiff`), so a 110 km scene never sits whole in memory. Earth Search and the COG bucket
//! send CORS headers: this works from a web page.
//!
//! Bands: green B03, red B04 and near-infrared B08 (10 m), shortwave-infrared B11 and the scene
//! classification SCL (20 m, resampled to the 10 m grid). Reflectance is DN / 10000, less the 1000
//! offset of processing baseline 04.00 where Earth Search has not applied it already.

use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::overpass::urlencode;
use crate::tiff::TiledTiff;
use owd_format::design::crs::{Crs, Utm};
use serde_json::Value;

/// The Earth Search STAC endpoint.
pub const SEARCH_ENDPOINT: &str = "https://earth-search.aws.element84.com/v1/search";

/// One scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub id: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub cloud_pct: f64,
    pub epsg: Option<u32>,
    /// True when Earth Search has already taken the 1000 offset off the reflectance values.
    pub offset_applied: bool,
    /// `[west, south, east, north]` of the scene footprint in degrees, if the item gives one.
    pub bbox: Option<[f64; 4]>,
    pub green: String,
    pub red: String,
    pub nir: String,
    pub swir: String,
    pub scl: Option<String>,
}

/// The most cloud, in percent of the scene, for a scene to be used (provisional).
pub const DEFAULT_MAX_CLOUD_PCT: f64 = 20.0;

/// The months of the dry season at a latitude, until a better rule is calibrated (provisional): the
/// northern tropics have it from December to February, the southern from June to August.
pub fn default_dry_months(lat: f64) -> Vec<u32> {
    if lat >= 0.0 {
        vec![12, 1, 2]
    } else {
        vec![6, 7, 8]
    }
}

/// The search request for scenes over a box `[west, south, east, north]` between two dates
/// (`YYYY-MM-DD`).
pub fn search_url(endpoint: &str, bbox: [f64; 4], from: &str, to: &str, limit: u32) -> String {
    format!(
        "{endpoint}?collections=sentinel-2-l2a&bbox={},{},{},{}&datetime={}&limit={limit}",
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3],
        urlencode(&format!("{from}T00:00:00Z/{to}T23:59:59Z"))
    )
}

/// The scenes in a STAC answer. Items without the four bands are skipped.
pub fn parse_scenes(body: &[u8]) -> Result<Vec<Scene>, ReachError> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| ReachError::Parse(format!("the Earth Search answer is not JSON: {e}")))?;
    let items = v["features"]
        .as_array()
        .ok_or_else(|| ReachError::Parse("the Earth Search answer has no `features`".into()))?;
    let mut out = Vec::new();
    for f in items {
        let href = |k: &str| f["assets"][k]["href"].as_str().map(str::to_string);
        let (Some(green), Some(red), Some(nir), Some(swir)) =
            (href("green"), href("red"), href("nir"), href("swir16"))
        else {
            continue;
        };
        let p = &f["properties"];
        let bbox = f["bbox"].as_array().and_then(|b| {
            let g = |i: usize| b.get(i).and_then(Value::as_f64);
            Some([g(0)?, g(1)?, g(2)?, g(3)?])
        });
        out.push(Scene {
            id: f["id"].as_str().unwrap_or("").to_string(),
            date: p["datetime"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(10)
                .collect(),
            cloud_pct: p["eo:cloud_cover"].as_f64().unwrap_or(100.0),
            epsg: p["proj:epsg"].as_u64().map(|e| e as u32),
            offset_applied: p["earthsearch:boa_offset_applied"]
                .as_bool()
                .unwrap_or(false),
            bbox,
            green,
            red,
            nir,
            swir,
            scl: href("scl"),
        });
    }
    Ok(out)
}

/// The best scene: in a dry-season month, under the cloud limit, and covering the box if the items say
/// where they are; the least cloud wins, then the latest date.
pub fn pick_scene<'a>(
    scenes: &'a [Scene],
    months: &[u32],
    max_cloud_pct: f64,
    need: Option<[f64; 4]>,
) -> Option<&'a Scene> {
    scenes
        .iter()
        .filter(|s| {
            let month: Option<u32> = s.date.get(5..7).and_then(|m| m.parse().ok());
            month.is_some_and(|m| months.contains(&m))
                && s.cloud_pct <= max_cloud_pct
                && match (need, s.bbox) {
                    (Some(n), Some(b)) => {
                        b[0] <= n[0] && b[1] <= n[1] && b[2] >= n[2] && b[3] >= n[3]
                    }
                    _ => true,
                }
        })
        .min_by(|a, b| {
            a.cloud_pct
                .total_cmp(&b.cloud_pct)
                .then_with(|| b.date.cmp(&a.date))
        })
}

/// A window of a scene on the 10 m grid, in the scene's UTM zone.
#[derive(Debug, Clone)]
pub struct Imagery {
    pub utm: Utm,
    /// Easting of the west edge and northing of the north edge.
    pub x0: f64,
    pub y1: f64,
    /// Pixel size in metres (10).
    pub px: f64,
    pub cols: usize,
    pub rows: usize,
    /// Reflectance of green, red, near-infrared and shortwave-infrared, row-major.
    pub bands: [Vec<f32>; 4],
    /// False where there is no data or the scene classification says cloud, shadow or cirrus.
    pub valid: Vec<bool>,
}

impl Imagery {
    /// An image built from bands, for tests.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_bands(
        utm: Utm,
        x0: f64,
        y1: f64,
        px: f64,
        cols: usize,
        rows: usize,
        bands: [Vec<f32>; 4],
        valid: Vec<bool>,
    ) -> Imagery {
        Imagery {
            utm,
            x0,
            y1,
            px,
            cols,
            rows,
            bands,
            valid,
        }
    }
}

/// Scene classes that are not usable: cloud shadow (3), cloud medium and high probability (8, 9) and
/// thin cirrus (10), and no data (0).
fn scl_unusable(v: u8) -> bool {
    matches!(v, 0 | 3 | 8 | 9 | 10)
}

/// Read the window of a scene that covers `[min_x, min_y, max_x, max_y]` metres in the scene's UTM zone.
pub fn read_imagery(
    fetch: &dyn Fetch,
    scene: &Scene,
    min: [f64; 2],
    max: [f64; 2],
) -> Result<Imagery, ReachError> {
    let g = TiledTiff::open(fetch, &scene.green)?;
    let epsg = g
        .epsg
        .or(scene.epsg)
        .ok_or_else(|| ReachError::Parse(format!("{}: the scene names no CRS", scene.id)))?;
    let utm = match Crs::from_epsg(epsg) {
        Ok(Crs::Utm(u)) => u,
        _ => {
            return Err(ReachError::Parse(format!(
                "{}: EPSG:{epsg} is not a UTM zone",
                scene.id
            )));
        }
    };
    let px = g.pixel[0];
    if (px - 10.0).abs() > 1e-6 || (g.pixel[1] - 10.0).abs() > 1e-6 {
        return Err(ReachError::Parse(format!(
            "{}: the green band is not on a 10 m grid",
            scene.id
        )));
    }
    // Whole 20 m cells, so the 20 m bands line up with the 10 m grid.
    let (x0, y1) = (
        (min[0] / 20.0).floor() * 20.0,
        (max[1] / 20.0).ceil() * 20.0,
    );
    let cols = (((max[0] - x0) / px).ceil() as usize + 1).next_multiple_of(2);
    let rows = (((y1 - min[1]) / px).ceil() as usize + 1).next_multiple_of(2);
    let col0 = ((x0 - g.origin[0]) / px).round() as i64;
    let row0 = ((g.origin[1] - y1) / px).round() as i64;
    let read10 = |url: &str| -> Result<Vec<u16>, ReachError> {
        let t = TiledTiff::open(fetch, url)?;
        if (t.origin[0] - g.origin[0]).abs() > 1e-6 || (t.origin[1] - g.origin[1]).abs() > 1e-6 {
            return Err(ReachError::Parse(format!(
                "{url}: the band is not on the scene's grid"
            )));
        }
        t.read_window16(fetch, col0, row0, cols, rows)
    };
    let green = g.read_window16(fetch, col0, row0, cols, rows)?;
    let red = read10(&scene.red)?;
    let nir = read10(&scene.nir)?;
    // 20 m bands: read the cells under the window, repeat each to 2 x 2 pixels.
    let (c20, r20) = (col0.div_euclid(2), row0.div_euclid(2));
    let (w20, h20) = (cols / 2 + 1, rows / 2 + 1);
    let up = |tiff: &TiledTiff, raw: Vec<u16>| -> Vec<u16> {
        let _ = tiff;
        let mut out = vec![0u16; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let (rr, cc) = (
                    ((row0 + r as i64).div_euclid(2) - r20) as usize,
                    ((col0 + c as i64).div_euclid(2) - c20) as usize,
                );
                out[r * cols + c] = raw[rr * w20 + cc];
            }
        }
        out
    };
    let swir_t = TiledTiff::open(fetch, &scene.swir)?;
    let swir = up(&swir_t, swir_t.read_window16(fetch, c20, r20, w20, h20)?);
    let scl = match &scene.scl {
        Some(url) => {
            let t = TiledTiff::open(fetch, url)?;
            let raw = t.read_window(fetch, c20, r20, w20, h20)?;
            let mut out = vec![0u8; cols * rows];
            for r in 0..rows {
                for c in 0..cols {
                    let (rr, cc) = (
                        ((row0 + r as i64).div_euclid(2) - r20) as usize,
                        ((col0 + c as i64).div_euclid(2) - c20) as usize,
                    );
                    out[r * cols + c] = raw[rr * w20 + cc];
                }
            }
            Some(out)
        }
        None => None,
    };
    let offset = if scene.offset_applied { 0.0 } else { 1000.0 };
    let refl = |dn: &[u16]| -> Vec<f32> {
        dn.iter()
            .map(|v| ((f32::from(*v) - offset as f32) / 10_000.0).max(0.0))
            .collect()
    };
    let valid: Vec<bool> = (0..cols * rows)
        .map(|i| {
            green[i] != 0
                && red[i] != 0
                && nir[i] != 0
                && swir[i] != 0
                && scl.as_ref().is_none_or(|s| !scl_unusable(s[i]))
        })
        .collect();
    Ok(Imagery {
        utm,
        x0,
        y1,
        px,
        cols,
        rows,
        bands: [refl(&green), refl(&red), refl(&nir), refl(&swir)],
        valid,
    })
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    const STAC: &str = include_str!("../../../examples/reach/stac_search_sassandra.json");

    #[test]
    fn the_stac_answer_gives_scenes_and_the_pick_is_dry_season_low_cloud() {
        let s = parse_scenes(STAC.as_bytes()).unwrap();
        assert_eq!(s.len(), 4);
        let a = s
            .iter()
            .find(|s| s.id == "S2B_29NRF_20231223_0_L2A")
            .unwrap();
        assert_eq!(a.date, "2023-12-23");
        assert!(a.offset_applied && a.epsg == Some(32629));
        assert!(
            a.green.ends_with("/B03.tif")
                && a.swir.ends_with("/B11.tif")
                && a.scl.as_ref().unwrap().ends_with("/SCL.tif")
        );
        // Least cloud in December to February under 30 %: the 23 December scene (0.009 %) over the 18
        // December one (0.94 %); a limit between them still keeps it.
        let best = pick_scene(&s, &default_dry_months(5.0), 30.0, None).unwrap();
        assert_eq!(best.id, "S2B_29NRF_20231223_0_L2A");
        assert_eq!(
            pick_scene(&s, &default_dry_months(5.0), 0.5, None)
                .unwrap()
                .id,
            "S2B_29NRF_20231223_0_L2A"
        );
        // Under a stricter limit nothing qualifies; out of season nothing qualifies.
        assert!(pick_scene(&s, &default_dry_months(5.0), 0.001, None).is_none());
        assert!(pick_scene(&s, &[6, 7, 8], 100.0, None).is_none());
        assert_eq!(default_dry_months(-12.0), vec![6, 7, 8]);
    }

    #[test]
    fn the_search_url_names_the_box_and_the_dates() {
        let u = search_url(
            SEARCH_ENDPOINT,
            [-6.15, 4.93, -6.05, 5.0],
            "2024-12-01",
            "2025-03-01",
            30,
        );
        assert!(u.starts_with("https://earth-search.aws.element84.com/v1/search?collections=sentinel-2-l2a&bbox=-6.15,4.93,-6.05,5&datetime="), "{u}");
        assert!(
            u.contains("2024-12-01T00%3A00%3A00Z%2F2025-03-01T23%3A59%3A59Z")
                && u.ends_with("&limit=30"),
            "{u}"
        );
    }
}
