//! Reading GeoJSON inputs: one line or one polygon, in WGS84 or in WGS84 / UTM, reprojected to
//! the project's UTM zone.
//!
//! GeoJSON (RFC 7946) is WGS84 unless a legacy `crs` member says otherwise. A `crs` member naming
//! WGS84 / UTM is honoured; any other CRS is refused with a message that says what to convert to.

use super::crs::{Crs, Utm};
use serde_json::{Value, json};

/// Which input a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    LeftBank,
    RightBank,
    Centreline,
    RiverPolygon,
}

impl Role {
    /// The name used in the document and in messages.
    pub fn name(self) -> &'static str {
        match self {
            Role::LeftBank => "left_bank",
            Role::RightBank => "right_bank",
            Role::Centreline => "centreline",
            Role::RiverPolygon => "river_polygon",
        }
    }

    fn wants_polygon(self) -> bool {
        self == Role::RiverPolygon
    }

    /// Whether the line is a bank (its centroid chooses the UTM zone).
    pub fn is_bank(self) -> bool {
        matches!(self, Role::LeftBank | Role::RightBank)
    }
}

/// A geometry as read, still in its source CRS.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub crs: Crs,
    /// `"LineString"` or `"Polygon"`, as GeoJSON coordinates (x, y; any z is dropped).
    pub geometry: Value,
}

fn xy(p: &Value, what: &str) -> Result<[f64; 2], String> {
    let a = p
        .as_array()
        .filter(|a| a.len() >= 2)
        .ok_or_else(|| format!("{what}: a position needs at least x and y"))?;
    let n = |v: &Value| {
        v.as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| format!("{what}: a coordinate is not a finite number"))
    };
    Ok([n(&a[0])?, n(&a[1])?])
}

fn positions(v: &Value, what: &str) -> Result<Vec<[f64; 2]>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what}: coordinates must be a list of positions"))?
        .iter()
        .map(|p| xy(p, what))
        .collect()
}

fn crs_of(root: &Value) -> Result<Crs, String> {
    match root.get("crs") {
        None | Some(Value::Null) => Ok(Crs::Wgs84),
        Some(c) => {
            let name = c
                .get("properties")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .ok_or("the `crs` member has no `properties.name`")?;
            Crs::parse(name)
        }
    }
}

/// The geometries of a bare geometry, a Feature or a FeatureCollection.
fn geometries(root: &Value) -> Result<Vec<&Value>, String> {
    match root.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => {
            let feats = root
                .get("features")
                .and_then(Value::as_array)
                .ok_or("a FeatureCollection needs a `features` list")?;
            let mut out = Vec::new();
            for f in feats {
                out.extend(geometries(f)?);
            }
            Ok(out)
        }
        Some("Feature") => match root.get("geometry") {
            Some(g) if !g.is_null() => Ok(vec![g]),
            _ => Ok(vec![]),
        },
        Some(_) => Ok(vec![root]),
        None => Err("this is not GeoJSON: it has no `type`".into()),
    }
}

/// Read one input from GeoJSON text.
pub fn parse(role: Role, text: &str) -> Result<Parsed, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let crs = crs_of(&root)?;
    let geoms = geometries(&root)?;
    let want = if role.wants_polygon() {
        "Polygon"
    } else {
        "LineString"
    };
    if geoms.len() != 1 {
        return Err(format!(
            "expected exactly one {want} for {}, found {} geometries",
            role.name(),
            geoms.len()
        ));
    }
    let g = geoms[0];
    let kind = g.get("type").and_then(Value::as_str).unwrap_or("");
    let coords = g
        .get("coordinates")
        .ok_or_else(|| format!("{}: the geometry has no coordinates", role.name()))?;
    let what = role.name();
    let geometry =
        match (role.wants_polygon(), kind) {
            (false, "LineString") => {
                let pts = positions(coords, what)?;
                if pts.len() < 2 {
                    return Err(format!("{what}: a line needs at least two positions"));
                }
                json!({"type": "LineString", "coordinates": pts})
            }
            (false, "MultiLineString") => {
                let parts = coords.as_array().filter(|a| a.len() == 1).ok_or_else(|| {
                    format!("{what}: a MultiLineString must hold exactly one line")
                })?;
                let pts = positions(&parts[0], what)?;
                if pts.len() < 2 {
                    return Err(format!("{what}: a line needs at least two positions"));
                }
                json!({"type": "LineString", "coordinates": pts})
            }
            (true, "Polygon") => polygon(coords, what)?,
            (true, "MultiPolygon") => {
                let parts = coords.as_array().filter(|a| a.len() == 1).ok_or_else(|| {
                    format!("{what}: a MultiPolygon must hold exactly one polygon")
                })?;
                polygon(&parts[0], what)?
            }
            _ => {
                return Err(format!(
                    "{what}: expected a {want}, found a {}",
                    if kind.is_empty() {
                        "geometry without a type"
                    } else {
                        kind
                    }
                ));
            }
        };
    Ok(Parsed { crs, geometry })
}

fn polygon(coords: &Value, what: &str) -> Result<Value, String> {
    let rings = coords
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| format!("{what}: a polygon needs a ring"))?;
    let mut out = Vec::new();
    for r in rings {
        let pts = positions(r, what)?;
        if pts.len() < 4 || pts.first() != pts.last() {
            return Err(format!(
                "{what}: a polygon ring must be closed, with at least four positions"
            ));
        }
        out.push(pts);
    }
    Ok(json!({"type": "Polygon", "coordinates": out}))
}

/// Every position of a parsed line or polygon, flattened.
pub fn all_positions(geometry: &Value) -> Vec<[f64; 2]> {
    fn walk(v: &Value, out: &mut Vec<[f64; 2]>) {
        if let Some(a) = v.as_array() {
            if a.len() >= 2 && a[0].is_number() {
                if let (Some(x), Some(y)) = (a[0].as_f64(), a[1].as_f64()) {
                    out.push([x, y]);
                }
            } else {
                a.iter().for_each(|c| walk(c, out));
            }
        }
    }
    let mut out = Vec::new();
    if let Some(c) = geometry.get("coordinates") {
        walk(c, &mut out);
    }
    out
}

/// Map every position of a parsed geometry through `f`.
pub fn map_positions(
    geometry: &Value,
    f: &dyn Fn([f64; 2]) -> Result<[f64; 2], String>,
) -> Result<Value, String> {
    fn walk(v: &Value, f: &dyn Fn([f64; 2]) -> Result<[f64; 2], String>) -> Result<Value, String> {
        let a = v.as_array().ok_or("coordinates must be arrays")?;
        if a.len() >= 2 && a[0].is_number() {
            let p = f([
                a[0].as_f64().unwrap_or(f64::NAN),
                a[1].as_f64().unwrap_or(f64::NAN),
            ])?;
            Ok(json!([p[0], p[1]]))
        } else {
            Ok(Value::Array(
                a.iter().map(|c| walk(c, f)).collect::<Result<_, _>>()?,
            ))
        }
    }
    let mut out = geometry.clone();
    if let Some(c) = geometry.get("coordinates") {
        out["coordinates"] = walk(c, f)?;
    }
    Ok(out)
}

/// Reproject a parsed input to `target`.
pub fn to_project_crs(parsed: &Parsed, target: Utm) -> Result<Value, String> {
    match parsed.crs {
        Crs::Utm(u) if u == target => Ok(parsed.geometry.clone()),
        Crs::Utm(u) => Err(format!(
            "the input is in {} (EPSG:{}) but the channel design uses {} (EPSG:{}). Convert the \
             input to the channel design's zone, or to WGS84 (EPSG:4326).",
            u.name(),
            u.epsg(),
            target.name(),
            target.epsg()
        )),
        Crs::Wgs84 => map_positions(&parsed.geometry, &|p| target.forward(p[0], p[1])),
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn a_feature_collection_with_one_line_reads_as_that_line() {
        let text = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},
            "geometry":{"type":"LineString","coordinates":[[1,2,9],[3,4,9]]}}]}"#;
        let p = parse(Role::LeftBank, text).unwrap();
        assert_eq!(p.crs, Crs::Wgs84);
        assert_eq!(p.geometry["coordinates"], json!([[1.0, 2.0], [3.0, 4.0]]));
    }

    #[test]
    fn a_declared_utm_crs_is_kept_and_another_crs_is_refused() {
        let utm = r#"{"type":"LineString","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}},
            "coordinates":[[500000,0],[500100,0]]}"#;
        assert_eq!(parse(Role::Centreline, utm).unwrap().crs.epsg(), 32630);
        let lambert = utm.replace("32630", "2154");
        let e = parse(Role::Centreline, &lambert).unwrap_err();
        assert!(e.contains("EPSG:2154") && e.contains("WGS84"), "{e}");
    }

    #[test]
    fn the_wrong_kind_of_geometry_and_open_rings_are_refused() {
        let point = r#"{"type":"Point","coordinates":[1,2]}"#;
        assert!(
            parse(Role::LeftBank, point)
                .unwrap_err()
                .contains("expected a LineString")
        );
        let open = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}"#;
        assert!(
            parse(Role::RiverPolygon, open)
                .unwrap_err()
                .contains("closed")
        );
        assert!(
            parse(Role::LeftBank, "{")
                .unwrap_err()
                .contains("not valid JSON")
        );
    }
}
