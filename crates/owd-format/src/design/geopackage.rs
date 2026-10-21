//! Reading vectors from a GeoPackage (OGC 12-128r19): bank lines and the river polygon.
//!
//! The file is read from memory through SQLite compiled into the binary (`rusqlite` `bundled`),
//! so no system SQLite is needed at run time. A feature layer is turned into GeoJSON text with
//! its CRS named, and then goes through the same import as a GeoJSON file, so the same river from
//! either format gives the same document.

use rusqlite::{Connection, MAIN_DB, OptionalExtension};
use serde_json::{Value, json};
use std::io::Cursor;

/// The leading bytes of every SQLite 3 database file.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Whether `bytes` look like a SQLite database (so a GeoPackage).
pub fn is_geopackage(bytes: &[u8]) -> bool {
    bytes.starts_with(SQLITE_MAGIC)
}

/// One feature layer of a GeoPackage.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub table: String,
    pub geometry_column: String,
    pub geometry_type: String,
    pub srs_id: i64,
}

fn open(bytes: &[u8]) -> Result<Connection, String> {
    if !is_geopackage(bytes) {
        return Err("the file is not a GeoPackage (it is not a SQLite database)".into());
    }
    let mut conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.deserialize_read_exact(MAIN_DB, Cursor::new(bytes), bytes.len(), true)
        .map_err(|e| format!("cannot read the GeoPackage: {e}"))?;
    Ok(conn)
}

/// The feature layers of a GeoPackage, in table-name order.
pub fn layers(bytes: &[u8]) -> Result<Vec<Layer>, String> {
    layers_of(&open(bytes)?)
}

fn layers_of(conn: &Connection) -> Result<Vec<Layer>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.table_name, g.column_name, g.geometry_type_name, g.srs_id \
             FROM gpkg_contents c JOIN gpkg_geometry_columns g ON g.table_name = c.table_name \
             WHERE c.data_type = 'features' ORDER BY c.table_name",
        )
        .map_err(|_| "the file is not a GeoPackage: it has no feature tables".to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Layer {
                table: r.get(0)?,
                geometry_column: r.get(1)?,
                geometry_type: r.get::<_, String>(2)?.to_ascii_uppercase(),
                srs_id: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// The CRS of a layer as an `EPSG:n` name: the GeoPackage's spatial reference system row.
fn crs_name(conn: &Connection, srs_id: i64) -> Result<String, String> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT organization, organization_coordsys_id FROM gpkg_spatial_ref_sys WHERE srs_id = ?1",
            [srs_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match row {
        Some((org, code)) if org.eq_ignore_ascii_case("EPSG") => Ok(format!("EPSG:{code}")),
        Some((org, code)) => Err(format!(
            "the layer's coordinate system is {org}:{code}, not an EPSG code. Convert the data to \
             WGS84 (EPSG:4326) or to WGS84 / UTM (EPSG:326xx for the northern hemisphere, \
             EPSG:327xx for the southern)."
        )),
        None => Err(format!(
            "the layer names spatial reference system {srs_id}, which the file does not define"
        )),
    }
}

// --- GeoPackage binary geometry: header + WKB ------------------------------------------------

struct Wkb<'a> {
    b: &'a [u8],
    i: usize,
}

impl Wkb<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len());
        let end = end.ok_or("the geometry blob is cut short")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }

    fn u32(&mut self, le: bool) -> Result<u32, String> {
        let s: [u8; 4] = self.take(4)?.try_into().map_err(|_| "short")?;
        Ok(if le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    }

    fn f64(&mut self, le: bool) -> Result<f64, String> {
        let s: [u8; 8] = self.take(8)?.try_into().map_err(|_| "short")?;
        Ok(if le {
            f64::from_le_bytes(s)
        } else {
            f64::from_be_bytes(s)
        })
    }

    /// One geometry: its GeoJSON form.
    fn geometry(&mut self) -> Result<Value, String> {
        let le = match self.take(1)?[0] {
            1 => true,
            0 => false,
            other => return Err(format!("the geometry has a bad byte-order mark ({other})")),
        };
        let raw = self.u32(le)?;
        // EWKB marks Z, M and an SRID with the high bits; ISO WKB adds 1000 (Z), 2000 (M), 3000 (ZM).
        let ewkb = raw & 0xE000_0000 != 0;
        let iso = if ewkb { 0 } else { raw / 1000 };
        let has_z = (ewkb && raw & 0x8000_0000 != 0) || iso == 1 || iso == 3;
        let has_m = (ewkb && raw & 0x4000_0000 != 0) || iso == 2 || iso == 3;
        let has_srid = ewkb && raw & 0x2000_0000 != 0;
        let kind = (raw & 0x0FFF_FFFF) % 1000;
        if has_srid {
            self.u32(le)?;
        }
        let dims = 2 + usize::from(has_z) + usize::from(has_m);
        match kind {
            2 => Ok(json!({"type": "LineString", "coordinates": self.points(le, dims)?})),
            3 => Ok(json!({"type": "Polygon", "coordinates": self.rings(le, dims)?})),
            5 => {
                let n = self.u32(le)?;
                let mut lines = Vec::new();
                for _ in 0..n {
                    let g = self.geometry()?;
                    lines.push(g["coordinates"].clone());
                }
                Ok(json!({"type": "MultiLineString", "coordinates": lines}))
            }
            6 => {
                let n = self.u32(le)?;
                let mut polys = Vec::new();
                for _ in 0..n {
                    let g = self.geometry()?;
                    polys.push(g["coordinates"].clone());
                }
                Ok(json!({"type": "MultiPolygon", "coordinates": polys}))
            }
            1 | 4 => {
                Err("the layer holds points: bank lines and the river polygon are needed".into())
            }
            other => Err(format!("geometry type {other} is not supported")),
        }
    }

    fn points(&mut self, le: bool, dims: usize) -> Result<Vec<[f64; 2]>, String> {
        let n = self.u32(le)? as usize;
        let mut out = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            let (x, y) = (self.f64(le)?, self.f64(le)?);
            for _ in 2..dims {
                self.f64(le)?;
            }
            out.push([x, y]);
        }
        Ok(out)
    }

    fn rings(&mut self, le: bool, dims: usize) -> Result<Vec<Vec<[f64; 2]>>, String> {
        let n = self.u32(le)?;
        (0..n).map(|_| self.points(le, dims)).collect()
    }
}

/// Decode one GeoPackage geometry blob (a header, then WKB) into GeoJSON. `None` for an empty
/// geometry.
fn decode(blob: &[u8]) -> Result<Option<Value>, String> {
    if blob.len() < 8 || &blob[..2] != b"GP" {
        return Err("a geometry is not in GeoPackage binary form (no `GP` header)".into());
    }
    let flags = blob[3];
    let envelope = match (flags >> 1) & 0b111 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        other => {
            return Err(format!(
                "the geometry header has a bad envelope code ({other})"
            ));
        }
    };
    if flags & 0b0001_0000 != 0 {
        return Ok(None); // the empty-geometry flag
    }
    let start = 8 + envelope;
    if blob.len() <= start {
        return Err("the geometry blob is cut short".into());
    }
    Wkb { b: blob, i: start }.geometry().map(Some)
}

/// Read one feature layer as GeoJSON text (a FeatureCollection naming the layer's CRS), ready for
/// the same import as a GeoJSON file.
///
/// `layer` picks the table. With none given the file must hold exactly one feature layer; if it
/// holds several, the error lists them.
pub fn to_geojson(bytes: &[u8], layer: Option<&str>) -> Result<String, String> {
    let conn = open(bytes)?;
    let all = layers_of(&conn)?;
    let chosen = match layer {
        Some(name) => all.iter().find(|l| l.table == name).ok_or_else(|| {
            format!(
                "the GeoPackage has no feature layer `{name}`. Its layers: {}",
                layer_list(&all)
            )
        })?,
        None => match all.as_slice() {
            [one] => one,
            [] => return Err("the GeoPackage has no feature layers".into()),
            many => {
                return Err(format!(
                    "the GeoPackage has {} feature layers: {}. Name one with FILE#LAYER.",
                    many.len(),
                    layer_list(many)
                ));
            }
        },
    };
    let crs = crs_name(&conn, chosen.srs_id)?;
    let sql = format!(
        "SELECT \"{}\" FROM \"{}\"",
        chosen.geometry_column.replace('"', "\"\""),
        chosen.table.replace('"', "\"\"")
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let blobs = stmt
        .query_map([], |r| r.get::<_, Option<Vec<u8>>>(0))
        .map_err(|e| e.to_string())?;
    let mut features = Vec::new();
    for blob in blobs {
        let Some(blob) = blob.map_err(|e| e.to_string())? else {
            continue;
        };
        if let Some(g) = decode(&blob).map_err(|e| format!("layer `{}`: {e}", chosen.table))? {
            features.push(json!({"type": "Feature", "properties": {}, "geometry": g}));
        }
    }
    Ok(json!({
        "type": "FeatureCollection",
        "crs": {"type": "name", "properties": {"name": crs}},
        "features": features,
    })
    .to_string())
}

fn layer_list(layers: &[Layer]) -> String {
    layers
        .iter()
        .map(|l| format!("{} ({})", l.table, l.geometry_type.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(", ")
}
