//! The full answer of `fits` (spec 0012, T9): with a bed model, a water level and a DEM, each class
//! is screened by the spec 0007 path along the river centreline, so its row has a verdict and
//! constraint stretches, depth among them.
//!
//! The river is straight, 3 km long and 400 m wide, with its water level at 10 m. The bed model gives
//! 4 m of depth everywhere but a shallow bar of 1.5 m from chainage 1200 to 1500, with linear ramps
//! from 1100 and to 1600. The land (the DEM) is at 12 m.
//!
//! Label: sourced - hand calculation. At the centreline the bed shape is 1, so the river's depth is
//! the table's: on the ramp up to the bar it is `4 - 2.5 (s - 1100) / 100`, which falls under a
//! class's design depth `d` (1.5 < d < 4) at `s = 1100 + 40 (4 - d)`, and on the ramp down it rises
//! back over `d` at `s = 1600 - 40 (4 - d)`. A class deeper than 4 m is short everywhere; one of
//! 1.5 m or less is short nowhere.

use owd_core::volumes::sections::DEFAULT_STATION_M;
use owd_format::design::{Role, Session, Stage};
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const LAND_M: f64 = 12.0;

fn river() -> Session {
    let mut s = Session::new("Shallow bar", "2026-10-21");
    let at = |x: f64, y: f64| [X0 + x, Y0 + y];
    let ring = json!([
        at(0.0, -200.0),
        at(3000.0, -200.0),
        at(3000.0, 200.0),
        at(0.0, 200.0),
        at(0.0, -200.0)
    ]);
    let polygon = format!(r#"{{"type":"Polygon",{UTM30},"coordinates":[{ring}]}}"#);
    let centre = format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!([at(0.0, 0.0), at(3000.0, 0.0)])
    );
    s.import(&[
        (Role::RiverPolygon, polygon.as_str()),
        (Role::Centreline, centre.as_str()),
    ])
    .unwrap();
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s
}

fn with_bar(mut s: Session) -> Session {
    s.set_bed_model(
        &[
            [0.0, 4.0],
            [1100.0, 4.0],
            [1200.0, 1.5],
            [1500.0, 1.5],
            [1600.0, 4.0],
            [3000.0, 4.0],
        ],
        Some("dry"),
        "2026-10-21",
    )
    .unwrap();
    s
}

fn land(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![LAND_M; xs.len()]
}

/// A class's design depth under PIANC-B, from the sizing stage itself.
fn design_depth(class: &str) -> f64 {
    let mut s = river();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel(class, None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.document()["stages"]["sizing"]["results"]["design_depth_m"]
        .as_f64()
        .unwrap()
}

fn rows(s: &Session) -> Vec<Value> {
    s.document()["fits"]["results"]["rule_sets"][0]["classes"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn a_shallow_bar_gives_each_class_the_depth_stretch_a_hand_calculation_gives() {
    let mut s = with_bar(river());
    s.run_fits_with(None, Some(&land)).unwrap();
    let results = &s.document()["fits"]["results"];
    assert_eq!(results["mode"], "full");
    assert!(
        results["label"]
            .as_str()
            .unwrap()
            .contains("depth assumed, not measured")
    );
    let mut seen = (0, 0, 0);
    for row in rows(&s) {
        let class = row["vessel_standard_id"].as_str().unwrap();
        // Every row says the depth is assumed while the bed model is used.
        assert_eq!(row["depth_basis"], "depth assumed, not measured", "{class}");
        let d = design_depth(class);
        let depth: Vec<&Value> = row["constraint_stretches"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["kind"] == "depth")
            .collect();
        let tol = DEFAULT_STATION_M + 1e-6;
        if d <= 1.5 {
            assert!(depth.is_empty(), "{class}: d {d}, {depth:?}");
            seen.0 += 1;
        } else if d < 4.0 {
            assert_eq!(depth.len(), 1, "{class}: d {d}, {depth:?}");
            let (a, b) = (1100.0 + 40.0 * (4.0 - d), 1600.0 - 40.0 * (4.0 - d));
            let st = depth[0];
            let (s0, s1) = (
                st["start_chainage_m"].as_f64().unwrap(),
                st["end_chainage_m"].as_f64().unwrap(),
            );
            assert!(
                s0 >= a - 1e-6 && s0 <= a + tol,
                "{class}: d {d} starts {s0}, expected just after {a}"
            );
            assert!(
                s1 <= b + 1e-6 && s1 >= b - tol,
                "{class}: d {d} ends {s1}, expected just before {b}"
            );
            // At the bar, 1.5 m of water against the class's depth: the works are a dredge.
            assert!((st["river"].as_f64().unwrap() - 1.5).abs() < 1e-6, "{st}");
            assert!((st["required"].as_f64().unwrap() - d).abs() < 1e-9, "{st}");
            assert!(
                (st["deficit"].as_f64().unwrap() - (d - 1.5)).abs() < 1e-6,
                "{st}"
            );
            assert_eq!(st["works"], "dredge");
            assert!(st["dredge_m3"].as_f64().unwrap() > 0.0, "{st}");
            assert_eq!(row["verdict"], "navigable-with-works", "{class}");
            assert_eq!(row["fits"], false);
            seen.1 += 1;
        } else {
            assert_eq!(depth.len(), 1, "{class}: d {d}, {depth:?}");
            assert!(depth[0]["start_chainage_m"].as_f64().unwrap() < tol);
            assert!(depth[0]["end_chainage_m"].as_f64().unwrap() > 3000.0 - tol);
            seen.2 += 1;
        }
        // No other limit stops a class on this wide, straight river.
        assert_eq!(
            row["verdict"] == "navigable-as-is",
            row["constraint_stretches"].as_array().unwrap().is_empty(),
            "{class}"
        );
    }
    assert!(seen.1 > 0, "some class meets the bar: {seen:?}");
    // The largest class that fits as it is: the largest with no stretch.
    let largest = results["rule_sets"][0]["largest_fitting"]
        .as_str()
        .map(str::to_string);
    let expected = rows(&s)
        .iter()
        .rev()
        .find(|r| r["verdict"] == "navigable-as-is")
        .map(|r| r["vessel_standard_id"].as_str().unwrap().to_string());
    assert_eq!(largest, expected);
    assert_eq!(s.fits_status(), owd_format::design::StageStatus::Fresh);
    // The answer read the bed: a new depth makes it stale.
    s.set_bed_model(&[[0.0, 6.0]], None, "2026-10-21").unwrap();
    assert_eq!(s.fits_status(), owd_format::design::StageStatus::Stale);
}

#[test]
fn without_a_dem_or_without_a_bed_model_the_rows_are_the_plan_only_ones() {
    let mut plain = river();
    plain.run_fits(None).unwrap();
    let plan_only = plain.document()["fits"]["results"].clone();
    assert!(
        plan_only.get("mode").is_none(),
        "a plan-only answer is as T8 wrote it"
    );
    // A bed model but no DEM.
    let mut no_dem = with_bar(river());
    no_dem.run_fits_with(None, None).unwrap();
    assert_eq!(no_dem.document()["fits"]["results"], plan_only);
    // A DEM but no bed model.
    let mut no_bed = river();
    no_bed.run_fits_with(None, Some(&land)).unwrap();
    assert_eq!(no_bed.document()["fits"]["results"], plan_only);
    // And the plan-only answer still goes stale on the banks, not on the bed.
    let mut s = river();
    s.run_fits(None).unwrap();
    let mut s2 = with_bar(s.clone());
    assert_eq!(s2.fits_status(), owd_format::design::StageStatus::Fresh);
    s2.set_water_profile(&[[0.0, 11.0]]).unwrap();
    assert_eq!(s2.fits_status(), owd_format::design::StageStatus::Fresh);
}

/// The river of the shallow-bar test, but its centreline zigzags by 3 m every 40 m, as one drawn
/// from bank lines can: on the centreline the wiggle reads as tight bends; the water is straight.
fn wiggly() -> Session {
    let mut s = Session::new("Wiggly", "2026-10-22");
    let at = |x: f64, y: f64| [X0 + x, Y0 + y];
    let ring = json!([
        at(0.0, -200.0),
        at(3000.0, -200.0),
        at(3000.0, 200.0),
        at(0.0, 200.0),
        at(0.0, -200.0)
    ]);
    let polygon = format!(r#"{{"type":"Polygon",{UTM30},"coordinates":[{ring}]}}"#);
    let pts: Vec<[f64; 2]> = (0..=75)
        .map(|i| at(40.0 * f64::from(i), if i % 2 == 0 { 3.0 } else { -3.0 }))
        .collect();
    let centre = format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    );
    s.import(&[
        (Role::RiverPolygon, polygon.as_str()),
        (Role::Centreline, centre.as_str()),
    ])
    .unwrap();
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s.set_bed_model(&[[0.0, 6.0]], Some("dry"), "2026-10-22")
        .unwrap();
    s
}

fn radius_stretches(s: &Session) -> usize {
    rows(s)
        .iter()
        .map(|r| {
            r["constraint_stretches"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|x| x["kind"] == "radius")
                .count()
        })
        .sum()
}

#[test]
fn on_the_fairway_the_river_radius_is_the_water_s_not_the_wiggle_of_its_centreline() {
    use owd_format::design::fits::PlanLine;
    // Label: property. The default reads each class's fairway fit (ADR 0013): in the straight
    // water it has no tight bend.
    let mut fairway = wiggly();
    fairway.run_fits_with(None, Some(&land)).unwrap();
    assert_eq!(
        radius_stretches(&fairway),
        0,
        "{}",
        fairway.document()["fits"]["results"]
    );
    let r = &fairway.document()["fits"];
    assert_eq!(r["settings"]["plan_line"], "fairway");
    assert_eq!(r["results"]["plan_line"], "fairway");
    assert!(r["results"].get("plan_line_note").is_none());
    for c in rows(&fairway) {
        assert_eq!(c["plan_line"], "fairway", "{c}");
    }
    // Asked for: the centreline, whose 3 m zigzag is a run of tight bends. The answer says so.
    let mut centre = wiggly();
    centre
        .run_fits_on(None, Some(&land), PlanLine::Centreline)
        .unwrap();
    assert!(radius_stretches(&centre) > 0);
    let r = &centre.document()["fits"];
    assert_eq!(r["settings"]["plan_line"], "centreline");
    assert_eq!(r["results"]["plan_line"], "centreline");
    // Width and depth are read as before: the same stretches of those kinds either way.
    let others = |s: &Session| -> Vec<String> {
        rows(s)
            .iter()
            .flat_map(|r| r["constraint_stretches"].as_array().unwrap().clone())
            .filter(|x| x["kind"] != "radius")
            .map(|x| x.to_string())
            .collect()
    };
    assert_eq!(others(&centre), others(&fairway));
    assert!(PlanLine::parse("bank").is_err());
    assert_eq!(PlanLine::default(), PlanLine::Fairway);
}

#[test]
fn a_plan_only_answer_reads_the_centreline_and_says_why() {
    // Label: property. With no bed model the fairway cannot run: the answer falls back to the
    // centreline and says so, in the answer and in each row.
    let mut s = wiggly();
    s.clear_bed_model();
    s.run_fits_with(None, Some(&land)).unwrap();
    let r = &s.document()["fits"];
    assert_ne!(r["results"]["mode"], "full", "{}", r["results"]);
    assert_eq!(r["settings"]["plan_line"], "fairway");
    assert_eq!(r["results"]["plan_line"], "centreline");
    let note = r["results"]["plan_line_note"].as_str().unwrap();
    assert!(
        note.contains("fairway") && note.contains("centreline"),
        "{note}"
    );
    let radius = rows(&s)
        .iter()
        .flat_map(|c| c["stretches"].as_array().unwrap().clone())
        .filter(|x| x["kind"] == "radius")
        .count();
    assert!(radius > 0, "the centreline's zigzag");
}
