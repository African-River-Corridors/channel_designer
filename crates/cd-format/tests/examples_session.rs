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

#[test]
fn optimise_pools_through_the_session_matches_the_frozen_python_to_the_adr_0005_tolerances() {
    // The example's straight design line is the river line here, so projecting onto the river
    // centreline is the example's own reading of the pool levels. The example fixes the half-width at
    // 15 m (no bend widening): the two `half_width_m` settings say so.
    let want = reference()["optimise_pools"].clone();
    let river = synthetic_river(RiverParams {
        length_m: 3000.0,
        wavelength_m: 2400.0,
        amplitude_m: 120.0,
        hill_at: [1500.0, -60.0],
        hill_height_m: 10.0,
        hill_radius_m: 80.0,
        ..RiverParams::default()
    })
    .unwrap();
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| river.terrain(x, y))
            .collect()
    };
    let mut s = Session::new("Pools", "2026-10-16");
    s.import(&[
        (
            Role::Centreline,
            &format!(r#"{{"type":"LineString"{UTM30},"coordinates":[[0.0,0.0],[3000.0,0.0]]}}"#),
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
    s.set_dem(
        "d.tif",
        b"d",
        &cd_format::geotiff::Dem {
            width: 2,
            height: 2,
            data: vec![0.0; 4],
            geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
            nodata: None,
            epsg: Some(32630),
        },
    );
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s.run_optimise(
        &json!({
            "candidate_sites_m": [500.0, 1000.0, 1500.0, 2000.0, 2500.0],
            "lifts_m": [0.5, 1.0], "terminal_min_level_m": 0.5, "min_spacing_m": 1000.0,
            "dry_weight": 1.5, "structure_fixed": 1.0e5, "structure_per_lift": 2.0e5,
            "cut_model": "b", "maxw_m": 120.0, "half_width_m": 15.0,
            "rake_half_m": 80.0, "rake_step_m": 5.0, "ds_step_m": 10.0,
            "pitch_m": 20.0, "n_heading": 24, "radius_multiples": [1.0, 2.0, 4.0],
            "corridor_half_m": 180.0, "cell_m": 5.0,
        }),
        Some(&terrain),
    )
    .unwrap();
    s.run_volumes(
        &json!({"cut_model": "b", "maxw_m": 120.0, "half_width_m": 15.0}),
        Some(&terrain),
    )
    .unwrap();

    let res = &s.document()["stages"]["optimise"]["results"];
    let weirs: Vec<Value> = res["cascade"]["weirs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| json!({"chainage_m": w["chainage_m"], "pool_level_m": w["pool_level_m"]}))
        .collect();
    compare(&want["weirs"], &json!(weirs), tolerance()).unwrap();
    compare(&want["pools"], &res["pools"], tolerance()).unwrap();
    assert_eq!(
        s.document()["options"][1]["details"]["n_replaced"],
        want["n_replaced"]
    );
    let line: Vec<Value> = pts(&s.document()["options"][1]["line"])
        .iter()
        .map(|p| json!(p))
        .collect();
    compare(&want["lines"]["optimised"], &json!(line), tolerance()).unwrap();
    let rows = &s.document()["stages"]["volumes"]["results"]["options"];
    for (i, name) in ["straight", "optimised"].iter().enumerate() {
        let got = json!({
            "dredge_m3": rows[i]["dredge_m3"], "excav_m3": rows[i]["excav_m3"],
            "total_m3": rows[i]["total_m3"], "max_face_h_m": rows[i]["max_face_h_m"],
            "n_stations": rows[i]["stations"],
        });
        compare(&want["volumes"][name], &got, tolerance())
            .unwrap_or_else(|d| panic!("{name}: {}", d.join("; ")));
    }
}
