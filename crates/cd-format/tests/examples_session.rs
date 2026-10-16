//! The worked examples through the design session (the water-level parity of the quickstart's re-route
//! and volumes is in `water_profile.rs`), against what the frozen Python printed
//! (`parity/examples/reference.json`). The river is given in a UTM 30N frame with its own origin at
//! (0, 0), so the numbers are the examples' own and no last bit moves.
//!
//! Label: snapshot (the Python reference). Never hand-edit it.

use cd_core::geom::Pt;
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_format::design::{AlignMethod, Role, Session, Stage};
use cd_format::parity::{Tolerance, compare};
use serde_json::{Value, json};
use std::path::Path;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn reference() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/examples/reference.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn tolerance() -> Tolerance {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/tolerance.json");
    Tolerance::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn pts(v: &Value) -> Vec<Pt> {
    v["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

#[test]
fn the_quickstart_through_the_session_gives_the_examples_shape_and_rolling_circle() {
    let want = reference()["quickstart"].clone();
    let params = RiverParams::default();
    let river = synthetic_river(params).unwrap();
    let mut s = Session::new("Quickstart", "2026-10-16");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(river.centreline)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([river.polygon.exterior])
            ),
        ),
    ])
    .unwrap();
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let shape = s.document()["stages"]["sizing"]["results"].clone();
    let got = json!({
        "bottom_width_straight_m": shape["bottom_width_straight_m"],
        "min_bend_radius_m": shape["min_bend_radius_m"],
        "design_depth_m": shape["design_depth_m"],
    });
    compare(&want["shape"], &got, tolerance()).unwrap();

    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let o = &s.document()["options"][0];
    assert_eq!(o["details"]["arcs"], want["n_arcs"]);
    let line: Vec<Value> = pts(&o["line"]).iter().map(|p| json!(p)).collect();
    compare(&want["lines"]["rolling circle"], &json!(line), tolerance()).unwrap();
}
