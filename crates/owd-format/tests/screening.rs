//! Spec 0007 T5: the screening outputs in the document (A, the verdict; B, the design basis; C, the
//! constraint stretches), their exports, and `rule_trace` (E), on the synthetic river in UTM 30N.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, SyntheticRiver, synthetic_river};
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    }
}

/// The synthetic river, sized for CEMT IV on PIANC-B at 2.5 m, aligned by the rolling circle, with
/// a DEM and a flat water level, and the volumes stage run.
fn screened(params: RiverParams) -> (Session, SyntheticRiver) {
    let r = synthetic_river(params).unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let centre: Vec<Pt> = r.centreline.iter().map(shift).collect();
    let mut s = Session::new("River", "2026-10-21");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.set_dem("dem/river.tif", b"pretend", &dem());
    s.set_water_fall(r.water_level(0.0), 0.0).unwrap();
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
            .collect()
    };
    s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
        .unwrap();
    (s, r)
}

fn screening(s: &Session) -> Vec<Value> {
    s.document()["stages"]["volumes"]["results"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["screening"].clone())
        .collect()
}

#[test]
fn every_option_has_a_verdict_and_its_stretches_and_the_sizing_has_its_design_basis() {
    let (s, _) = screened(RiverParams::default());
    let basis = s.document()["stages"]["sizing"]["results"]["design_basis"]
        .as_array()
        .unwrap()
        .clone();
    let quantities: Vec<&str> = basis
        .iter()
        .map(|b| b["quantity"].as_str().unwrap())
        .collect();
    assert_eq!(
        quantities,
        [
            "bottom-width",
            "min-bend-radius",
            "design-depth",
            "widening-at-rmin"
        ]
    );
    for b in &basis {
        assert_eq!(b["status"], "unverified");
        assert!(
            b["rule"].as_str().unwrap().starts_with("pianc-wg141/"),
            "{b}"
        );
        assert_eq!(b["rule_sets"], json!(["pianc-wg141@0.3.0"]));
    }
    let options = s.document()["options"].as_array().unwrap().clone();
    for (o, sc) in options.iter().zip(screening(&s)) {
        assert_eq!(sc["notice"], "Screening \u{2014} not for construction");
        assert!(sc["rules_notice"].as_str().unwrap().contains("unverified"));
        // The owd set's bed check is a warning, not a blocking Rule, so nothing blocking is left out.
        assert_eq!(
            sc["blocking_not_evaluated"],
            json!({"count": 0, "rules": []})
        );
        let stretches = sc["constraint_stretches"].as_array().unwrap();
        for x in stretches {
            let works = match x["kind"].as_str().unwrap() {
                "width" => "bank-cut",
                "depth" => "dredge",
                "radius" => "cut-through",
                "infeasible" => "none-possible",
                k => panic!("kind {k}"),
            };
            assert_eq!(x["works"], works);
            assert!(x["start_chainage_m"].as_f64() <= x["end_chainage_m"].as_f64());
            if x["kind"] != "infeasible" {
                assert!(x["deficit"].as_f64().unwrap() > 0.0, "{x}");
                assert!(x["governing_rule"].as_str().unwrap().contains('/'), "{x}");
            }
        }
        let infeasible = stretches.iter().any(|x| x["kind"] == "infeasible");
        let expected = if infeasible || o["checks"]["radius_ok"] == false {
            "not-navigable"
        } else if stretches.is_empty() {
            "navigable-as-is"
        } else {
            "navigable-with-works"
        };
        assert_eq!(sc["verdict"], expected, "{}", o["id"]);
    }
    // The synthetic river is 40 m wide; quality B needs 30.4 m plus the widening in bends, so some
    // bends are too narrow, and a 2 m bed is shallower than 1.3 x 2.5 m. The fixture finds work.
    let all: Vec<Value> = screening(&s)
        .iter()
        .flat_map(|sc| sc["constraint_stretches"].as_array().unwrap().clone())
        .collect();
    assert!(all.iter().any(|x| x["kind"] == "depth"), "{all:?}");
}

#[test]
fn the_trace_is_the_same_on_every_run_and_is_refused_when_a_stored_hash_differs() {
    let (s, _) = screened(RiverParams::default());
    let id = s.document()["options"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = s.rule_trace(Some(&id), 50.0).unwrap();
    let b = Session::open(&s.to_text())
        .unwrap()
        .rule_trace(Some(&id), 50.0)
        .unwrap();
    assert_eq!(a, b);
    let rows = a["rows"].as_array().unwrap();
    // Every Rule of every selected set appears; the station Rules once per station.
    for set in ["pianc-wg141/", "owd/"] {
        assert!(
            rows.iter()
                .any(|r| r["rule"].as_str().unwrap_or("").starts_with(set))
        );
    }
    assert!(rows.iter().filter(|r| r["governing"] == true).count() >= 4);
    assert!(
        rows.iter()
            .all(|r| r["status"] == "unverified" || r["rule"].is_null())
    );
    let none = s.rule_trace(None, 50.0).unwrap();
    assert!(none["rows"].as_array().unwrap().len() < rows.len());
    // A stored hash that is not the engine's: refused, never re-evaluated.
    let mut doc: Value = serde_json::from_str(&s.to_text()).unwrap();
    doc["stages"]["sizing"]["settings"]["rule_sets"][0]["hash"] = json!("0".repeat(64));
    let tampered = Session::open(&doc.to_string()).unwrap();
    let e = tampered.rule_trace(None, 50.0).unwrap_err().to_string();
    assert!(e.contains("content hash"), "{e}");
}

#[test]
fn the_stretches_export_as_csv_and_geojson_lines_with_the_notice_basis_and_verdicts() {
    let (s, _) = screened(RiverParams::default());
    let n: usize = screening(&s)
        .iter()
        .map(|sc| sc["constraint_stretches"].as_array().unwrap().len())
        .sum();
    let csv = s.export_stretches_csv(None);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "# Screening \u{2014} not for construction");
    assert!(lines[1].contains("unverified"));
    assert!(csv.contains("# design basis: bottom-width = "));
    assert!(csv.contains("# verdict: "));
    let header = lines
        .iter()
        .position(|l| l.starts_with("option_id,kind,"))
        .unwrap();
    assert_eq!(lines.len() - header - 1, n);
    let gj: Value =
        serde_json::from_str(&s.export_stretches_geojson(false, None).unwrap()).unwrap();
    assert_eq!(gj["screening"], "Screening \u{2014} not for construction");
    let features = gj["features"].as_array().unwrap();
    assert_eq!(features.len(), n);
    for f in features {
        assert_eq!(f["geometry"]["type"], "LineString");
        assert!(f["geometry"]["coordinates"].as_array().unwrap().len() >= 2);
        assert_eq!(f["properties"]["screening"], gj["screening"]);
    }
    let wgs: Value =
        serde_json::from_str(&s.export_stretches_geojson(true, None).unwrap()).unwrap();
    if let Some(f) = wgs["features"].as_array().unwrap().first() {
        let lon = f["geometry"]["coordinates"][0][0].as_f64().unwrap();
        assert!(lon.abs() < 4.0, "{lon}");
    }
}

#[test]
fn a_straight_option_has_no_radius_stretch_and_says_which_line_it_read() {
    // #369: the radius of a design line option is read on the option's own line. A straight line
    // past the meanders' crests (240 m to one side of the river's axis, where the river bends at
    // about 220 m, under the 255 m R_min) has no bend, whatever the river's bends are.
    let (s, r) = screened(RiverParams::default());
    let mut doc = s.document().clone();
    let first = shift(&r.centreline[0]);
    let last = shift(&r.centreline[r.centreline.len() - 1]);
    let straight: Vec<Pt> = (0..=100)
        .map(|i| {
            let t = i as f64 / 100.0;
            [
                first[0] + t * (last[0] - first[0]),
                first[1] + 240.0 + t * (last[1] - first[1]),
            ]
        })
        .collect();
    doc["options"][0]["line"]["coordinates"] = json!(straight);
    let mut s = Session::open(&doc.to_string()).unwrap();
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
            .collect()
    };
    s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
        .unwrap();
    let sc = &screening(&s)[0];
    let radius: Vec<&Value> = sc["constraint_stretches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["kind"] == "radius")
        .collect();
    assert!(radius.is_empty(), "{radius:?}");
    assert_eq!(sc["radius_read_on"], "the design line option");
}
