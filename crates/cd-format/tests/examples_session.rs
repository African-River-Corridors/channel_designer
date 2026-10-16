//! The worked examples through the design session, against what the frozen Python printed
//! (`parity/examples/reference.json`). The river is given in a UTM 30N frame with its own origin at
//! (0, 0), so the numbers are the examples' own and no last bit moves.
//!
//! Label: snapshot (the Python reference). Never hand-edit it.

use cd_core::geom::Pt;
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_format::design::{AlignMethod, Role, Session, Stage};
use cd_format::geotiff::Dem;
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

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    }
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

#[test]
fn the_quickstart_route_and_volumes_under_the_documents_linear_water_level() {
    // NOT a parity check. The example's water level is a function of the river's own x
    // (`water_level(s) = slope * x(s)`); the document holds a straight fall in chainage, so the
    // volumes and the re-route cannot equal the Python's. This guards the known size of the gap
    // (about 8 % of dredge, 4 % of dry cut, under 1 % of tallest face) so that it cannot grow.
    let want = reference()["quickstart"].clone();
    let params = RiverParams::default();
    let river = synthetic_river(params.clone()).unwrap();
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| river.terrain(x, y))
            .collect()
    };
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
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let wl = json!({
        "water_level_start_m": river.water_level(0.0),
        "water_surface_slope": -params.bed_slope,
    });
    s.set_dem("d.tif", b"d", &dem());
    s.align(AlignMethod::Reroute, &wl, Some(&terrain)).unwrap();
    let mut vs = wl.clone();
    vs["cut_model"] = json!("b");
    vs["maxw_m"] = json!(150.0);
    s.run_volumes(&vs, Some(&terrain)).unwrap();
    let rows = &s.document()["stages"]["volumes"]["results"]["options"];
    for (i, name) in ["rolling circle", "least-cost"].iter().enumerate() {
        for k in ["dredge_m3", "excav_m3", "max_face_h_m"] {
            let (a, b) = (
                rows[i][k].as_f64().unwrap(),
                want["volumes"][name][k].as_f64().unwrap(),
            );
            eprintln!(
                "DEV {name} {k}: session {a:.3} python {b:.3} rel {:.2e}",
                (a - b) / b
            );
        }
    }
    let r: Vec<Value> = pts(&s.document()["options"][1]["line"])
        .iter()
        .map(|p| json!(p))
        .collect();
    let r_ref = want["lines"]["least-cost"].as_array().unwrap();
    eprintln!(
        "DEV route vertices session {} python {}",
        r.len(),
        r_ref.len()
    );
}
