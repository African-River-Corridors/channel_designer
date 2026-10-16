//! The alignment stage on the synthetic river (given in UTM 30N, shifted into the zone): options
//! equal the core called directly, a failed R_min is recorded, an upstream stale stage is
//! refused, and two methods give two options that rerun independently.
//!
//! Label: property

use cd_core::alignment::rolling_circle::rolling_circle;
use cd_core::alignment::route::{self, GridParams, RouteParams};
use cd_core::geom::{Geometry, Polygon, Pt};
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_format::design::{AlignMethod, DesignError, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn design(centreline: &[Pt], vessel: &str) -> Session {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let ring: Vec<Pt> = river.polygon.exterior.iter().map(shift).collect();
    let polygon = format!(
        r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
        json!([ring])
    );
    let mut s = Session::new("River", "2026-10-16");
    s.import(&[
        (Role::Centreline, &line_text(centreline)),
        (Role::RiverPolygon, &polygon),
    ])
    .unwrap();
    s.set_sizing("PIANC-B", vessel, Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

fn river_line() -> Vec<Pt> {
    synthetic_river(RiverParams::default())
        .unwrap()
        .centreline
        .iter()
        .map(shift)
        .collect()
}

fn option<'a>(s: &'a Session, method: &str) -> &'a Value {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method)
        .unwrap()
}

fn coords(o: &Value) -> Vec<Pt> {
    o["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

#[test]
fn the_rolling_circle_option_equals_rolling_circle_called_directly() {
    let line = river_line();
    let mut s = design(&line, "CEMT_IV");
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let r_min = s.document()["stages"]["sizing"]["results"]["min_bend_radius_m"]
        .as_f64()
        .unwrap();
    let direct = rolling_circle(
        &Geometry::LineString(line),
        r_min,
        10.0,
        2,
        &["left".into(), "right".into()],
        None,
        None,
    )
    .unwrap();
    let o = option(&s, "rolling_circle");
    assert_eq!(coords(o), direct.centreline);
    assert_eq!(o["checks"]["radius_ok"], direct.radius_ok);
    assert_eq!(
        o["checks"]["measured_min_radius_m"],
        json!(direct.measured_min_radius_m)
    );
    assert_eq!(
        o["checks"]["radius_check_chord_m"],
        json!(direct.radius_check_chord_m)
    );
    assert_eq!(o["checks"]["r_min_m"], json!(r_min));
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    assert_eq!(Session::open(&s.to_text()).unwrap().to_text(), s.to_text());
}

#[test]
fn an_option_that_fails_r_min_carries_radius_ok_false_in_the_document() {
    // A zigzag the circle cannot straighten.
    let zig: Vec<Pt> = (0..=60)
        .map(|i| {
            [
                500_000.0 + 50.0 * f64::from(i),
                600_000.0 + if i % 2 == 0 { 8.0 } else { -8.0 },
            ]
        })
        .collect();
    let mut s = design(&zig, "CEMT_Va");
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let c = &option(&s, "rolling_circle")["checks"];
    assert_eq!(c["radius_ok"], false);
    assert!(c["measured_min_radius_m"].as_f64().unwrap() < 0.98 * c["r_min_m"].as_f64().unwrap());
    assert!(c["radius_check_chord_m"].as_f64().unwrap() > 0.0);
}

#[test]
fn align_with_a_stale_or_missing_sizing_is_refused_and_names_sizing() {
    let mut s = design(&river_line(), "CEMT_IV");
    s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
    assert_eq!(
        s.align(AlignMethod::RollingCircle, &Value::Null, None)
            .unwrap_err(),
        DesignError::StaleUpstream {
            stage: Stage::Alignment,
            run_first: Stage::Sizing
        }
    );
    let mut empty = Session::new("x", "2026-10-16");
    empty
        .import(&[(Role::Centreline, &line_text(&river_line()))])
        .unwrap();
    let e = empty
        .align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("sizing") && e.contains("not been run"), "{e}");
}

#[test]
fn two_methods_give_two_options_and_a_rerun_replaces_only_its_own() {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| river.terrain(x - SHIFT[0], y - SHIFT[1]))
            .collect()
    };
    let mut s = design(&river_line(), "CEMT_IV");
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let wl0 = river.water_level(0.0);
    s.set_water_fall(wl0, 0.0).unwrap();
    let reroute = json!({"corridor_sweep_m": [40.0, 80.0]});
    s.align(AlignMethod::Reroute, &reroute, Some(&terrain))
        .unwrap();
    assert_eq!(s.document()["options"].as_array().unwrap().len(), 2);

    // The re-route option equals optimise_route called directly, from the rolling-circle line.
    let rolled = coords(option(&s, "rolling_circle"));
    let shape = &s.document()["stages"]["sizing"]["results"];
    let (r_min, depth) = (
        shape["min_bend_radius_m"].as_f64().unwrap(),
        shape["design_depth_m"].as_f64().unwrap(),
    );
    let ring: Vec<Pt> = river.polygon.exterior.iter().map(shift).collect();
    let poly = [Polygon {
        exterior: ring,
        holes: vec![],
    }];
    let grid = route::build_cost_grid(
        &rolled,
        &terrain,
        &poly,
        &|_| wl0 - depth,
        &GridParams {
            bottom_width: shape["bottom_width_straight_m"].as_f64().unwrap(),
            depth,
            side_slope: 3.0,
            corridor_half: 80.0,
            station_step: 100.0,
            offset_step: 10.0,
            transect_step: 2.0,
            daylight_m: None,
        },
    )
    .unwrap();
    let direct = route::optimise_route(
        &grid,
        &RouteParams {
            r_min,
            wet_weight: 1.0,
            dry_weight: 1.0,
            corridor_sweep: vec![40.0, 80.0],
            check_chord_m: None,
        },
    )
    .unwrap();
    let o = option(&s, "reroute");
    assert_eq!(coords(o), direct.line);
    assert_eq!(o["checks"]["radius_ok"], direct.radius_ok);
    assert_eq!(
        o["checks"]["measured_min_radius_m"],
        json!(direct.measured_min_radius_m)
    );
    assert_eq!(
        o["checks"]["radius_check_chord_m"],
        json!(direct.radius_check_chord_m)
    );

    // Rerun the rolling circle with another step: its own option changes, the re-route's does not.
    let before = option(&s, "reroute").clone();
    s.align(AlignMethod::RollingCircle, &json!({"step_m": 20.0}), None)
        .unwrap();
    assert_eq!(s.document()["options"].as_array().unwrap().len(), 2);
    assert_eq!(option(&s, "rolling_circle")["settings"]["step_m"], 20.0);
    assert_eq!(*option(&s, "reroute"), before);
}

#[test]
fn the_reroute_says_what_it_needs() {
    let mut s = design(&river_line(), "CEMT_IV");
    let none = |_: &[f64], _: &[f64]| vec![];
    let e = s
        .align(AlignMethod::Reroute, &Value::Null, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("terrain"), "{e}");
    let e = s
        .align(AlignMethod::Reroute, &Value::Null, Some(&none))
        .unwrap_err()
        .to_string();
    assert!(e.contains("water level"), "{e}");
}

#[test]
fn a_stage_run_runs_the_rolling_circle_and_a_new_vessel_makes_it_stale_then_run_reruns_both() {
    let mut s = design(&river_line(), "CEMT_IV");
    s.run_stage(Stage::Alignment).unwrap();
    assert_eq!(
        s.document()["stages"]["alignment"]["results"]["methods"],
        json!(["rolling_circle"])
    );
    // A new vessel makes alignment stale; sizing first, then run reruns it.
    s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    assert_eq!(
        s.run_stale().unwrap(),
        vec![Stage::Sizing, Stage::Alignment]
    );
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
}
