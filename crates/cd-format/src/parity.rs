//! Compare an `eval` output with its Reference Result (spec 0001, "What identical results means").
//!
//! - Floats match within the relative and absolute tolerance in `parity/tolerance.json`.
//! - Everything discrete matches exactly: integers, booleans, strings (including messages), null,
//!   list length and order, object keys.
//! - An integer never matches a float, so a Python `int` result stays an `int`.
//! - NaN and infinity are the tagged strings `"float:nan"`, `"float:inf"`, `"float:-inf"`, and so
//!   match exactly.
//! - A polygon (an object with `"geom": "Polygon"` or `"MultiPolygon"`) is compared normalised
//!   (ADR 0005): the same kind and number of polygons and holes, area within `polygon_area_rel`,
//!   and every vertex of each within `polygon_vertex_m` of the other's boundary. A ring may start at
//!   another vertex.
//!
//! Deleted with the Reference Results when the port is complete (ticket 37).

use cd_core::geom::{Polygon, polycmp};
use serde_json::Value;

/// Parity tolerances, read from `parity/tolerance.json` (ADR 0005). They live in that one file and
/// nowhere else; the oracle guard stops any change to them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Relative tolerance for deterministic floats.
    pub rel: f64,
    /// Absolute tolerance for deterministic floats.
    pub abs: f64,
    /// Relative tolerance on a search objective.
    pub search_objective_rel: f64,
    /// Relative tolerance on a polygon area.
    pub polygon_area_rel: f64,
    /// Largest distance, in metres, between matching polygon vertices.
    pub polygon_vertex_m: f64,
}

impl Tolerance {
    /// Parse `parity/tolerance.json`.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let get = |k: &str| {
            v.get(k)
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("tolerance.json: `{k}` must be a number"))
        };
        Ok(Self {
            rel: get("rel")?,
            abs: get("abs")?,
            search_objective_rel: get("search_objective_rel")?,
            polygon_area_rel: get("polygon_area_rel")?,
            polygon_vertex_m: get("polygon_vertex_m")?,
        })
    }

    /// Same rule as Python's `math.isclose(a, b, rel_tol=rel, abs_tol=abs)`.
    pub fn close(&self, a: f64, b: f64) -> bool {
        a == b || (a - b).abs() <= (self.rel * a.abs().max(b.abs())).max(self.abs)
    }
}

/// Compare `actual` (from `eval`) with `expected` (the Reference Result). On a mismatch, returns
/// every differing path, such as `$.ok.density: expected 450.0, got 451.0`.
pub fn compare(expected: &Value, actual: &Value, tol: Tolerance) -> Result<(), Vec<String>> {
    let mut diffs = Vec::new();
    walk("$", expected, actual, tol, &mut diffs);
    if diffs.is_empty() { Ok(()) } else { Err(diffs) }
}

fn walk(path: &str, expected: &Value, actual: &Value, tol: Tolerance, diffs: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Number(e), Value::Number(a)) => {
            let ok = match (e.is_f64(), a.is_f64()) {
                (true, true) => tol.close(
                    e.as_f64().unwrap_or(f64::NAN),
                    a.as_f64().unwrap_or(f64::NAN),
                ),
                (false, false) => e == a,
                _ => false,
            };
            if !ok {
                diffs.push(format!("{path}: expected {e}, got {a}"));
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                diffs.push(format!(
                    "{path}: expected {} items, got {}",
                    e.len(),
                    a.len()
                ));
            }
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                walk(&format!("{path}[{i}]"), ev, av, tol, diffs);
            }
        }
        (Value::Object(e), Value::Object(a)) if is_polygon(e) => {
            if let Err(why) = same_polygon(expected, actual, tol) {
                diffs.push(format!("{path}: {why}"));
            }
        }
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                match a.get(k) {
                    Some(av) => walk(&format!("{path}.{k}"), ev, av, tol, diffs),
                    None => diffs.push(format!("{path}.{k}: missing")),
                }
            }
            for k in a.keys().filter(|k| !e.contains_key(*k)) {
                diffs.push(format!("{path}.{k}: unexpected"));
            }
        }
        _ => {
            if expected != actual {
                diffs.push(format!("{path}: expected {expected}, got {actual}"));
            }
        }
    }
}

fn is_polygon(o: &serde_json::Map<String, Value>) -> bool {
    matches!(
        o.get("geom").and_then(Value::as_str),
        Some("Polygon" | "MultiPolygon")
    )
}

fn ring(v: &Value) -> Option<Vec<[f64; 2]>> {
    v.as_array()?
        .iter()
        .map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?]))
        .collect()
}

fn polygon(v: &Value) -> Option<Polygon> {
    let holes = match v.get("holes") {
        None => vec![],
        Some(h) => h.as_array()?.iter().map(ring).collect::<Option<_>>()?,
    };
    Some(Polygon {
        exterior: ring(v.get("exterior")?)?,
        holes,
    })
}

/// The polygons of a JSON (multi)polygon; an empty one has none.
fn polygons(v: &Value) -> Option<Vec<Polygon>> {
    let ps = match v.get("geom")?.as_str()? {
        "Polygon" => vec![polygon(v)?],
        _ => v
            .get("polygons")?
            .as_array()?
            .iter()
            .map(polygon)
            .collect::<Option<_>>()?,
    };
    Some(ps.into_iter().filter(|p| !p.exterior.is_empty()).collect())
}

/// ADR 0005's normalised polygon check.
fn same_polygon(expected: &Value, actual: &Value, tol: Tolerance) -> Result<(), String> {
    if expected.get("geom") != actual.get("geom") {
        return Err(format!(
            "expected a {}, got {}",
            expected["geom"],
            actual.get("geom").unwrap_or(&Value::Null)
        ));
    }
    let e = polygons(expected).ok_or("expected polygon is malformed")?;
    let a = polygons(actual).ok_or("actual polygon is malformed")?;
    polycmp::same_shape(&e, &a, tol.polygon_area_rel, tol.polygon_vertex_m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOL: Tolerance = Tolerance {
        rel: 1e-9,
        abs: 1e-9,
        search_objective_rel: 1e-9,
        polygon_area_rel: 1e-9,
        polygon_vertex_m: 1e-6,
    };

    fn ok(e: Value, a: Value) -> bool {
        compare(&e, &a, TOL).is_ok()
    }

    #[test]
    fn floats_match_within_relative_tolerance() {
        assert!(ok(json!(450.0), json!(450.0 * (1.0 + 0.9e-9))));
        assert!(!ok(json!(450.0), json!(450.0 * (1.0 + 1.1e-9))));
    }

    #[test]
    fn floats_near_zero_match_within_absolute_tolerance() {
        assert!(ok(json!(0.0), json!(0.9e-9)));
        assert!(!ok(json!(0.0), json!(1.1e-9)));
    }

    #[test]
    fn an_integer_never_matches_a_float() {
        assert!(!ok(json!(3), json!(3.0)));
        assert!(!ok(json!(3.0), json!(3)));
        assert!(!ok(json!(3), json!(4)));
        assert!(ok(json!(3), json!(3)));
    }

    #[test]
    fn text_and_booleans_match_exactly() {
        assert!(!ok(
            json!("Unknown cargo type 'coal'"),
            json!("Unknown cargo type 'coal' ")
        ));
        assert!(!ok(json!(true), json!(false)));
        assert!(!ok(json!(null), json!(0)));
    }

    #[test]
    fn tagged_non_finite_floats_match_only_themselves() {
        assert!(ok(json!("float:nan"), json!("float:nan")));
        assert!(!ok(json!("float:inf"), json!("float:-inf")));
        assert!(!ok(json!("float:nan"), json!(0.0)));
    }

    #[test]
    fn lists_match_in_order_and_length() {
        assert!(!ok(json!([1, 2]), json!([2, 1])));
        assert!(!ok(json!([1, 2]), json!([1, 2, 3])));
    }

    #[test]
    fn objects_need_the_same_keys_and_report_the_path() {
        let e = json!({"ok": {"density": 450.0, "phase": "liquid_gas"}});
        let diffs = compare(&e, &json!({"ok": {"density": 451.0, "extra": 1}}), TOL).unwrap_err();
        assert!(
            diffs.contains(&"$.ok.density: expected 450.0, got 451.0".to_string()),
            "{diffs:?}"
        );
        assert!(
            diffs.contains(&"$.ok.phase: missing".to_string()),
            "{diffs:?}"
        );
        assert!(
            diffs.contains(&"$.ok.extra: unexpected".to_string()),
            "{diffs:?}"
        );
    }

    #[test]
    fn tolerance_parses_from_its_file() {
        let text = r#"{"rel": 1e-09, "abs": 2e-09, "search_objective_rel": 1e-09,
            "polygon_area_rel": 1e-09, "polygon_vertex_m": 1e-06, "reason": "x"}"#;
        let t = Tolerance::from_json(text).unwrap();
        assert_eq!(
            t,
            Tolerance {
                rel: 1e-9,
                abs: 2e-9,
                search_objective_rel: 1e-9,
                polygon_area_rel: 1e-9,
                polygon_vertex_m: 1e-6,
            }
        );
        assert!(Tolerance::from_json(r#"{"rel": 1e-09}"#).is_err());
    }

    fn square(dx: f64, start: usize) -> Value {
        let mut pts = vec![
            [dx, 0.0],
            [100.0 + dx, 0.0],
            [100.0 + dx, 100.0],
            [dx, 100.0],
        ];
        pts.rotate_left(start);
        pts.push(pts[0]);
        json!({"geom": "Polygon", "exterior": pts, "holes": []})
    }

    #[test]
    fn a_polygon_moved_by_2e_6_m_fails_the_normalised_check() {
        let diffs = compare(&square(0.0, 0), &square(2e-6, 0), TOL).unwrap_err();
        assert!(diffs[0].contains("from the other shape"), "{diffs:?}");
    }

    #[test]
    fn a_ring_that_starts_at_another_vertex_passes_the_normalised_check() {
        assert!(ok(square(0.0, 0), square(0.0, 2)));
        // The same rings as plain lists of points are compared strictly, and differ.
        assert!(!ok(
            square(0.0, 0)["exterior"].clone(),
            square(0.0, 2)["exterior"].clone()
        ));
    }

    #[test]
    fn a_polygon_is_not_a_multipolygon() {
        let multi = json!({"geom": "MultiPolygon", "polygons": [square(0.0, 0)]});
        assert!(!ok(square(0.0, 0), multi));
    }
}
