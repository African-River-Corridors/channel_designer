//! The water level as a profile along the river centreline (ADR 0006), on the synthetic river.
//!
//! - Two design lines through the same place see the same water.
//! - Parity: with the test-only sampling of the frozen Python (the design line's own chainage) the
//!   quickstart's re-route and volumes equal spec 0001's results at the ADR 0005 tolerances.
//! - The model change: with the shipped projection, the quickstart differs from spec 0001 by a
//!   recorded amount, because water level belongs to the river, not to the design line.
//! - Pool levels from the optimise stage are the profile the volumes read; an old document with a
//!   start level and a slope converts to a two-point profile and its stages show as stale.
//!
//! Label: snapshot (spec 0001's reference); the rest are properties.

use cd_core::geom::{Pt, linear};
use cd_core::synthetic::{RiverParams, SyntheticRiver, synthetic_river};
use cd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use cd_format::geotiff::Dem;
use cd_format::parity::{Tolerance, compare};
use serde_json::{Value, json};
use std::path::Path;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn tolerance() -> Tolerance {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/tolerance.json");
    Tolerance::from_json(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn reference() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/examples/reference.json");
    serde_json::from_str::<Value>(&std::fs::read_to_string(p).unwrap()).unwrap()["quickstart"]
        .clone()
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

/// The quickstart river in a UTM 30N frame with the example's own origin, so no last bit moves.
fn quickstart(river: &SyntheticRiver, design_line_sampling: bool) -> Session {
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
    if design_line_sampling {
        s.sample_water_at_design_line_chainage();
    }
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    // The example's water level as a profile: the river's own level at each vertex of its centreline.
    let profile: Vec<[f64; 2]> = (0..river.centreline.len())
        .map(|i| {
            let c = linear::length(&river.centreline[..=i]);
            [c, river.water_level(c)]
        })
        .collect();
    s.set_water_profile(&profile).unwrap();
    s.set_dem("d.tif", b"d", &dem());
    s
}

fn run_quickstart(s: &mut Session, river: &SyntheticRiver) {
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| river.terrain(x, y))
            .collect()
    };
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.align(AlignMethod::Reroute, &Value::Null, Some(&terrain))
        .unwrap();
    s.run_volumes(&json!({"cut_model": "b", "maxw_m": 150.0}), Some(&terrain))
        .unwrap();
}

fn line_of(s: &Session, i: usize) -> Value {
    json!(s.document()["options"][i]["line"]["coordinates"])
}

#[test]
fn the_quickstart_matches_spec_0001_with_the_pythons_sampling_in_test_mode() {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let mut s = quickstart(&river, true);
    run_quickstart(&mut s, &river);
    let want = reference();
    compare(
        &want["lines"]["rolling circle"],
        &line_of(&s, 0),
        tolerance(),
    )
    .unwrap();
    compare(&want["lines"]["least-cost"], &line_of(&s, 1), tolerance()).unwrap();
    let rows = &s.document()["stages"]["volumes"]["results"]["options"];
    for (i, name) in ["rolling circle", "least-cost"].iter().enumerate() {
        let got = json!({
            "dredge_m3": rows[i]["dredge_m3"], "excav_m3": rows[i]["excav_m3"],
            "total_m3": rows[i]["total_m3"], "max_face_h_m": rows[i]["max_face_h_m"],
            "n_stations": rows[i]["stations"],
        });
        compare(&want["volumes"][name], &got, tolerance())
            .unwrap_or_else(|d| panic!("{name}: {}", d.join("; ")));
    }
}

#[test]
fn the_shipped_projection_differs_from_spec_0001_by_the_recorded_model_change() {
    // Water level belongs to the river, not the design line (ADR 0006). The frozen Python reads the
    // level at the design line's own chainage; the shipped code projects onto the river centreline.
    // So the quickstart differs from spec 0001 by a known amount. If a later change moves it, this
    // fails: either the model changed again, or something broke.
    let river = synthetic_river(RiverParams::default()).unwrap();
    let mut s = quickstart(&river, false);
    run_quickstart(&mut s, &river);
    let want = reference();
    let rows = &s.document()["stages"]["volumes"]["results"]["options"];
    let rel = |i: usize, name: &str, k: &str| {
        let (a, b) = (
            rows[i][k].as_f64().unwrap(),
            want["volumes"][name][k].as_f64().unwrap(),
        );
        ((a - b) / b).abs()
    };
    let rc = ("rolling circle", 0);
    let (dredge, dry) = (rel(rc.1, rc.0, "dredge_m3"), rel(rc.1, rc.0, "excav_m3"));
    assert!(
        (2.4e-4..=2.65e-4).contains(&dredge),
        "dredge differs by {dredge:e}"
    );
    assert!(
        (1.1e-4..=1.21e-4).contains(&dry),
        "dry cut differs by {dry:e}"
    );
    // The shape of the change: small, and not zero, for the re-route too.
    let route = rel(1, "least-cost", "dredge_m3");
    assert!(
        route > 1e-6 && route < 5e-3,
        "re-route dredge differs by {route:e}"
    );
}

#[test]
fn two_design_lines_through_the_same_place_see_the_same_water() {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let mut s = quickstart(&river, false);
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    // Line A is the river itself. Line B starts 1 km on and is shifted 15 m sideways.
    let a = river.centreline.clone();
    let b: Vec<Pt> = river
        .centreline
        .iter()
        .filter(|p| p[0] >= 1000.0)
        .map(|p| [p[0], p[1] + 15.0])
        .collect();
    // The river's point at x = 3000, as a chainage on each line.
    let on = |line: &[Pt]| {
        let c = linear::project(
            line,
            [
                3000.0,
                linear::interpolate(&a, linear::project(&a, [3000.0, 0.0])).unwrap()[1],
            ],
        );
        s.water_levels_along(line, &[c]).unwrap()[0]
    };
    let (la, lb) = (on(&a), on(&b));
    assert!((la - lb).abs() < 1e-3, "A sees {la} m, B sees {lb} m");
    // Chainage along each line differs, so a profile on each line's own chainage could not agree.
    let (ca, cb) = (
        linear::project(&a, [3000.0, 0.0]),
        linear::project(&b, [3000.0, 0.0]),
    );
    assert!((ca - cb).abs() > 500.0, "{ca} {cb}");
    assert!((la - river.water_level(linear::project(&a, [3000.0, 0.0]))).abs() < 1e-3);
}

#[test]
fn a_profile_steps_at_a_weir_and_holds_flat_beyond_its_ends() {
    use cd_format::design::water::level_at;
    let p = [[0.0, 1.0], [100.0, 1.0], [100.0, 2.0], [300.0, 4.0]];
    assert_eq!(level_at(&p, -50.0), 1.0);
    assert_eq!(level_at(&p, 50.0), 1.0);
    assert_eq!(level_at(&p, 100.0), 2.0, "at a step the later level holds");
    assert_eq!(level_at(&p, 200.0), 3.0);
    assert_eq!(level_at(&p, 900.0), 4.0);
    assert_eq!(
        level_at(&[[5.0, 7.0]], 123.0),
        7.0,
        "one point is a constant"
    );
}

#[test]
fn an_old_document_with_a_start_level_and_slope_converts_and_its_stages_are_stale() {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let mut s = quickstart(&river, false);
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| river.terrain(x, y))
            .collect()
    };
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    // The document as the first #61-#63 builds wrote it: no profile; the level in the stage settings.
    let mut old = s.document().clone();
    old["inputs"].as_object_mut().unwrap().remove("water_level");
    old["stages"]["volumes"]["settings"]["water_level_start_m"] = json!(3.0);
    old["stages"]["volumes"]["settings"]["water_surface_slope"] = json!(0.001);
    let opened = Session::open(&serde_json::to_string(&old).unwrap()).unwrap();
    let len = linear::length(&river.centreline);
    let got = opened.input_water_profile().unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], [0.0, 3.0]);
    assert!((got[1][0] - len).abs() < 1e-9 && (got[1][1] - (3.0 - 0.001 * len)).abs() < 1e-9);
    let settings = &opened.document()["stages"]["volumes"]["settings"];
    assert!(
        settings.get("water_level_start_m").is_none()
            && settings.get("water_surface_slope").is_none()
    );
    // Its results came from the old model: stale, never silently fresh.
    assert_eq!(opened.stage_status(Stage::Volumes), StageStatus::Stale);
    // A document that already has a profile is left as it is.
    assert_eq!(
        Session::open(&s.to_text()).unwrap().input_water_profile(),
        s.input_water_profile()
    );
}
