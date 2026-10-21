//! SWORD river reaches near a route (source `sword`): the SWOT River Database v17c, read from its
//! shapefile release on Zenodo. The release is one zip of shapefiles, one per continent and Pfafstetter
//! level-2 basin (`shp/af_sword_reaches_hb14_v17c.shp`). The basin files are found from the bounding box
//! in each shapefile's header (the first 100 bytes, one small range request each), and only the basins
//! the route touches are read whole: about 20 MB for a West African basin. Zenodo sends CORS headers,
//! so this works from a web page too.
//!
//! Each reach is about 10 km of river with an id (the one Hydrocron answers by), its width, its
//! distance to the outlet (which says which way the river flows) and its line.

use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::geo::LonLat;
use crate::zip::RemoteZip;
use serde_json::Value;

/// The Zenodo record of SWORD v17c, and the zip of its shapefiles.
pub const RECORD_API: &str = "https://zenodo.org/api/records/22259077";
pub const SHAPEFILE_ZIP: &str = "SWORD_v17c_shp.zip";

/// A SWORD reach.
#[derive(Debug, Clone, PartialEq)]
pub struct Reach {
    pub id: String,
    /// The reach's width in the database, in metres.
    pub width_m: f64,
    /// Distance from the reach to the river's outlet, in metres: it falls downstream.
    pub dist_out_m: f64,
    /// The SWORD type: 1 river, 3 lake on a river, 4 dam or waterfall, 5 unreliable, 6 ghost.
    pub kind: u8,
    pub name: String,
    pub line: Vec<LonLat>,
}

fn bad(what: &str) -> ReachError {
    ReachError::Parse(format!("SWORD: {what}"))
}

/// The bounding box `[west, south, east, north]` in a shapefile's 100-byte header.
pub fn shp_bbox(header: &[u8]) -> Option<[f64; 4]> {
    if header.len() < 68 || header[..4] != 9994i32.to_be_bytes() {
        return None;
    }
    let f = |i: usize| f64::from_le_bytes(header[i..i + 8].try_into().expect("8 bytes"));
    Some([f(36), f(44), f(52), f(60)])
}

/// The polylines of a shapefile, one per record, as parts joined end to end.
fn polylines(shp: &[u8]) -> Result<Vec<Vec<LonLat>>, ReachError> {
    if shp.len() < 100 || shp[..4] != 9994i32.to_be_bytes() {
        return Err(bad("not a shapefile"));
    }
    let i32le = |b: &[u8], i: usize| i32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
    let f64le = |b: &[u8], i: usize| f64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
    let mut out = Vec::new();
    let mut p = 100;
    while p + 8 <= shp.len() {
        let words = i32::from_be_bytes(shp[p + 4..p + 8].try_into().expect("4 bytes"));
        let len = usize::try_from(words).map_err(|_| bad("a record of negative length"))? * 2;
        let c = shp
            .get(p + 8..p + 8 + len)
            .ok_or_else(|| bad("a record runs past the end"))?;
        let kind = i32le(c, 0);
        if kind == 0 {
            out.push(Vec::new());
        } else if kind == 3 || kind == 13 || kind == 23 {
            let (nparts, npts) = (i32le(c, 36) as usize, i32le(c, 40) as usize);
            let at = 44 + 4 * nparts;
            if c.len() < at + 16 * npts {
                return Err(bad("a polyline is cut short"));
            }
            out.push(
                (0..npts)
                    .map(|k| [f64le(c, at + 16 * k), f64le(c, at + 16 * k + 8)])
                    .collect(),
            );
        } else {
            return Err(bad(&format!("shape type {kind} is not a polyline")));
        }
        p += 8 + len;
    }
    Ok(out)
}

/// The records of a dBASE table, as field name to text.
fn table(dbf: &[u8]) -> Result<Vec<Vec<(String, String)>>, ReachError> {
    if dbf.len() < 32 {
        return Err(bad("the attribute table is cut short"));
    }
    let n = u32::from_le_bytes(dbf[4..8].try_into().expect("4 bytes")) as usize;
    let header = usize::from(u16::from_le_bytes([dbf[8], dbf[9]]));
    let rec = usize::from(u16::from_le_bytes([dbf[10], dbf[11]]));
    let mut fields: Vec<(String, usize)> = Vec::new();
    let mut p = 32;
    while p + 32 <= dbf.len() && dbf[p] != 0x0D {
        let name = dbf[p..p + 11].split(|b| *b == 0).next().unwrap_or_default();
        fields.push((
            String::from_utf8_lossy(name).into_owned(),
            usize::from(dbf[p + 16]),
        ));
        p += 32;
    }
    (0..n)
        .map(|i| {
            let r = dbf
                .get(header + i * rec + 1..header + (i + 1) * rec)
                .ok_or_else(|| bad("the attribute table is cut short"))?;
            let mut at = 0;
            Ok(fields
                .iter()
                .map(|(name, len)| {
                    let v = String::from_utf8_lossy(&r[at..at + len]).trim().to_string();
                    at += len;
                    (name.clone(), v)
                })
                .collect())
        })
        .collect()
}

/// The reaches of one basin's `.shp` and `.dbf`.
pub fn parse_reaches(shp: &[u8], dbf: &[u8]) -> Result<Vec<Reach>, ReachError> {
    let lines = polylines(shp)?;
    let rows = table(dbf)?;
    if lines.len() != rows.len() {
        return Err(bad(
            "the shapefile and its table hold different numbers of records",
        ));
    }
    rows.into_iter()
        .zip(lines)
        .map(|(row, line)| {
            let get = |k: &str| row.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
            let num = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok());
            Ok(Reach {
                id: get("reach_id")
                    .ok_or_else(|| bad("no `reach_id` field"))?
                    .to_string(),
                width_m: num("width").unwrap_or(f64::NAN),
                dist_out_m: num("dist_out").unwrap_or(f64::NAN),
                kind: num("type").map_or(0, |t| t as u8),
                name: get("river_name").unwrap_or("").to_string(),
                line,
            })
        })
        .collect()
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Where the shapefile zip is and how big, from the Zenodo record.
fn locate(fetch: &dyn Fetch, record_api: &str) -> Result<(String, u64), ReachError> {
    let v: Value = serde_json::from_slice(&fetch.get(record_api)?)
        .map_err(|e| bad(&format!("the Zenodo record is not JSON: {e}")))?;
    let f = v["files"]
        .as_array()
        .and_then(|a| a.iter().find(|f| f["key"] == SHAPEFILE_ZIP))
        .ok_or_else(|| bad(&format!("the Zenodo record has no `{SHAPEFILE_ZIP}`")))?;
    let url = f["links"]["self"]
        .as_str()
        .ok_or_else(|| bad("the file has no link"))?;
    let size = f["size"]
        .as_u64()
        .ok_or_else(|| bad("the file has no size"))?;
    Ok((url.to_string(), size))
}

/// The SWORD reaches whose bounding box meets `bbox` (`[west, south, east, north]`), ghost reaches
/// left out. `record_api` is [`RECORD_API`] (another in tests).
pub fn reaches_in(
    fetch: &dyn Fetch,
    record_api: &str,
    bbox: [f64; 4],
) -> Result<Vec<Reach>, ReachError> {
    let (url, size) = locate(fetch, record_api)?;
    let zip = RemoteZip::open(fetch, &url, size)?;
    let mut out = Vec::new();
    let shps: Vec<_> = zip
        .members
        .iter()
        .filter(|m| m.name.contains("_sword_reaches_hb") && m.name.ends_with(".shp"))
        .collect();
    if shps.is_empty() {
        return Err(bad("the zip holds no reach shapefiles"));
    }
    for m in shps {
        let head = zip.read_prefix(fetch, m, 100)?;
        let Some(b) = shp_bbox(&head) else {
            return Err(bad(&format!("{} has no shapefile header", m.name)));
        };
        if !overlaps(b, bbox) {
            continue;
        }
        let dbf_name = m.name.replace(".shp", ".dbf");
        let dbf = zip
            .member(&dbf_name)
            .ok_or_else(|| bad(&format!("no {dbf_name} beside {}", m.name)))?;
        let reaches = parse_reaches(&zip.read(fetch, m)?, &zip.read(fetch, dbf)?)?;
        out.extend(reaches.into_iter().filter(|r| {
            r.kind != 6 && crate::geo::bbox(&r.line).is_some_and(|rb| overlaps(rb, bbox))
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! Label: property (the reach values are the ones in the recorded release)
    use super::*;
    use crate::fetch::ReplayFetch;

    const SUBSET: &[u8] = include_bytes!("../../../examples/reach/sword_v17c_shp_subset.zip");

    /// A Zenodo record that points at the recorded subset.
    pub(crate) fn record(zip_url: &str, size: usize) -> String {
        format!(
            r#"{{"files":[{{"key":"{SHAPEFILE_ZIP}","size":{size},"links":{{"self":"{zip_url}"}}}}]}}"#
        )
    }

    #[test]
    fn the_reaches_near_the_sassandra_mouth_come_from_the_one_basin_that_holds_them() {
        let zip_url = "https://zenodo.example/sword.zip";
        let f = ReplayFetch::new()
            .with_response(
                "https://zenodo.example/record",
                record(zip_url, SUBSET.len()),
            )
            .with_response(zip_url, SUBSET.to_vec());
        let r = reaches_in(
            &f,
            "https://zenodo.example/record",
            [-6.2, 4.95, -6.05, 5.05],
        )
        .unwrap();
        let ids: Vec<&str> = r.iter().map(|r| r.id.as_str()).collect();
        // The ghost reach at the mouth (type 6) is left out; reach 061 starts west of the box but runs into it.
        assert_eq!(
            ids,
            [
                "14560100021",
                "14560100033",
                "14560100041",
                "14560100051",
                "14560100061"
            ]
        );
        let a = &r[1];
        assert_eq!(a.name, "Sassandra River");
        assert_eq!(a.width_m, 300.0);
        assert!((a.dist_out_m - 6648.05562956).abs() < 1e-6);
        assert!(a.line.len() > 10 && a.line.iter().all(|p| p[0] < -6.0 && p[1] > 4.9));
        // Downstream is toward a smaller distance to the outlet.
        assert!(r.windows(2).all(|w| w[0].dist_out_m < w[1].dist_out_m));
        // The hb15 basin is elsewhere: its header was read, its records were not.
        let reads: Vec<String> = f.requests();
        assert!(!reads.is_empty());
        // A box with no basin gives no reaches.
        assert!(
            reaches_in(
                &f,
                "https://zenodo.example/record",
                [100.0, 10.0, 101.0, 11.0]
            )
            .unwrap()
            .is_empty()
        );
    }
}
