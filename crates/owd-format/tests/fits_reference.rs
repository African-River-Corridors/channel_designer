//! `fits` on the reference line (spec 0016 T1): the plan line that belongs to no vessel. The
//! answer reads width and radius (and, when full, depth and the reaches) on the reference line.
//!
//! The river meanders (y = 150 sin(2πx/1500), 6 km), 120 m wide, its banks wiggling by a few
//! metres. Its centreline is drawn jumpy on purpose: the mid-points of the banks, each moved up to
//! 6 m sideways, as a station-by-station centreline is.
//!
//! Label: property

use owd_format::design::fits::PlanLine;
use owd_format::design::{Role, Session, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn line(pts: &[[f64; 2]]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn wiggle(i: usize, amp: f64) -> f64 {
    let t = i as f64;
    amp * (0.6 * libm::sin(1.7 * t) + 0.3 * libm::sin(4.1 * t + 1.0) + 0.1 * libm::sin(9.3 * t))
}

fn meander(with_banks: bool) -> Session {
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let k = 2.0 * std::f64::consts::PI / 1500.0;
    let (mut left, mut right, mut centre) = (vec![], vec![], vec![]);
    for i in 0..=600 {
        let x = 10.0 * i as f64;
        let (y, d) = (150.0 * libm::sin(k * x), 150.0 * k * libm::cos(k * x));
        let l = libm::hypot(1.0, d);
        let n = [-d / l, 1.0 / l];
        let (hl, hr) = (60.0 + wiggle(i, 4.0), 60.0 + wiggle(i + 7919, 4.0));
        left.push(at(x + hl * n[0], y + hl * n[1]));
        right.push(at(x - hr * n[0], y - hr * n[1]));
        let j = wiggle(i + 104_729, 6.0) + (hl - hr) / 2.0;
        centre.push(at(x + j * n[0], y + j * n[1]));
    }
    let mut ring = right.clone();
    ring.extend(left.iter().rev());
    ring.push(ring[0]);
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!(ring)
    );
    let mut s = Session::new("Meander", "2026-10-22");
    let (l, r, c) = (line(&left), line(&right), line(&centre));
    let mut roles = vec![
        (Role::RiverPolygon, polygon.as_str()),
        (Role::Centreline, c.as_str()),
    ];
    if with_banks {
        roles.push((Role::LeftBank, l.as_str()));
        roles.push((Role::RightBank, r.as_str()));
    }
    s.import(&roles).unwrap();
    s
}

fn radius_stretches(results: &Value) -> usize {
    results["rule_sets"][0]["classes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["stretches"].as_array().unwrap())
        .filter(|s| s["kind"] == "radius")
        .count()
}

#[test]
fn the_reference_line_reads_fewer_radius_stretches_than_a_jumpy_centreline() {
    let choices = ["PIANC-B".to_string()];
    let mut s = meander(true);
    s.run_fits_on(Some(&choices), None, PlanLine::Centreline)
        .unwrap();
    let on_centre = s.fits_result().unwrap().clone();
    s.run_fits_on(Some(&choices), None, PlanLine::Reference)
        .unwrap();
    let on_ref = s.fits_result().unwrap().clone();
    assert_eq!(on_ref["plan_line"], "reference");
    let (a, b) = (radius_stretches(&on_ref), radius_stretches(&on_centre));
    assert!(a < b, "radius stretches: reference {a}, centreline {b}");
    // The answer is read along the reference line: its length is the line's.
    let rl = s.reference_line().unwrap();
    let len: f64 = rl
        .windows(2)
        .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
        .sum();
    assert!((on_ref["length_m"].as_f64().unwrap() - len).abs() < 1.0);
    // Stored with the answer, and both fresh when the study is opened again.
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.reference_line_status(), StageStatus::Fresh);
    assert_eq!(back.fits_status(), StageStatus::Fresh);
}

#[test]
fn a_study_without_banks_reads_the_centreline_and_says_why() {
    let mut s = meander(false);
    s.run_fits_on(None, None, PlanLine::Reference).unwrap();
    let r = s.fits_result().unwrap();
    assert_eq!(r["plan_line"], "centreline");
    let note = r["plan_line_note"].as_str().unwrap();
    assert!(note.contains("reference line"), "{note}");
    assert!(note.contains("bank"), "{note}");
}

#[test]
fn the_plan_line_is_parsed_by_name() {
    assert_eq!(PlanLine::parse("reference").unwrap(), PlanLine::Reference);
    assert_eq!(PlanLine::Reference.name(), "reference");
    let e = PlanLine::parse("mid").unwrap_err().to_string();
    assert!(e.contains("reference"), "{e}");
}

#[test]
fn the_full_answer_walks_the_reference_line() {
    let mut s = meander(true);
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s.set_bed_model(&[[0.0, 4.0], [7000.0, 4.0]], Some("dry"), "2026-10-22")
        .unwrap();
    // The land at 12 m everywhere.
    let terrain = |xs: &[f64], _ys: &[f64]| vec![12.0; xs.len()];
    let choices = ["PIANC-B".to_string()];
    s.run_fits_on(Some(&choices), Some(&terrain), PlanLine::Reference)
        .unwrap();
    let r = s.fits_result().unwrap();
    assert_eq!(r["mode"], "full");
    assert_eq!(r["plan_line"], "reference");
    let len: f64 = s
        .reference_line()
        .unwrap()
        .windows(2)
        .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
        .sum();
    for c in r["rule_sets"][0]["classes"].as_array().unwrap() {
        assert!(c["verdict"].is_string(), "{}", c["vessel_standard_id"]);
        for st in c["constraint_stretches"].as_array().unwrap() {
            let end = st["end_chainage_m"].as_f64().unwrap();
            assert!(
                end <= len + 1.0,
                "a stretch ends at {end} m, past the line's {len} m"
            );
            // The bed model gives 4 m everywhere in the water: every station of the walk is in it.
            if st["kind"] == "depth" {
                let d = st["river"].as_f64().unwrap();
                assert!(
                    d > 3.9,
                    "a station reads {d} m deep, not the bed model's 4 m"
                );
            }
        }
    }
}

#[test]
fn the_default_plan_line_is_the_reference_line() {
    assert_eq!(PlanLine::default(), PlanLine::Reference);
    let mut s = meander(true);
    s.run_fits(None).unwrap();
    assert_eq!(s.fits_result().unwrap()["plan_line"], "reference");
    assert_eq!(s.document()["fits"]["settings"]["plan_line"], "reference");
}

/// A straight river 200 m wide with straight banks and a centreline that zigzags 30 m either side
/// of the middle every 60 m; water at 10 m, an assumed bed of 3 m.
fn zigzag_study() -> Session {
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let left: Vec<[f64; 2]> = (0..=60).map(|i| at(50.0 * i as f64, 100.0)).collect();
    let right: Vec<[f64; 2]> = (0..=60).map(|i| at(50.0 * i as f64, -100.0)).collect();
    let centre: Vec<[f64; 2]> = (0..=50)
        .map(|i| at(60.0 * i as f64, if i % 2 == 0 { 30.0 } else { -30.0 }))
        .collect();
    let mut ring = right.clone();
    ring.extend(left.iter().rev());
    ring.push(ring[0]);
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!(ring)
    );
    let mut s = Session::new("Zigzag", "2026-10-23");
    let (l, r, c) = (line(&left), line(&right), line(&centre));
    s.import(&[
        (Role::RiverPolygon, polygon.as_str()),
        (Role::Centreline, c.as_str()),
        (Role::LeftBank, l.as_str()),
        (Role::RightBank, r.as_str()),
    ])
    .unwrap();
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s.set_bed_model(&[[0.0, 3.0], [4000.0, 3.0]], Some("dry"), "2026-10-23")
        .unwrap();
    s
}

#[test]
fn the_assumed_bed_is_deepest_on_the_reference_line_not_on_a_jumpy_centreline() {
    // #398: on the zigzag centreline's parabola, the middle of the river reads as little as
    // 1 - (30/100)^2 = 0.91 of 3 m, under the 2.6 m of CN-III-MOTOR; on the reference line, which
    // runs down the middle, it reads the full 3 m.
    let terrain = |xs: &[f64], _ys: &[f64]| vec![12.0; xs.len()];
    let choices = ["PIANC-B".to_string()];
    let mut s = zigzag_study();
    s.run_fits_on(Some(&choices), Some(&terrain), PlanLine::Reference)
        .unwrap();
    let r = s.fits_result().unwrap();
    let row = r["rule_sets"][0]["classes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["vessel_standard_id"] == "CN-III-MOTOR")
        .unwrap();
    let depth: Vec<&Value> = row["constraint_stretches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|st| st["kind"] == "depth")
        .collect();
    assert!(depth.is_empty(), "{depth:?}");
}

#[test]
fn with_a_bed_model_a_new_reference_line_makes_the_volumes_stale() {
    use owd_format::design::Stage;
    let mut s = zigzag_study();
    let before = s.document().clone();
    let h0 = s.stage_hash(Stage::Volumes);
    s.run_reference_line().unwrap();
    assert_ne!(s.stage_hash(Stage::Volumes), h0);
    // Without a bed model, the reference line does not touch the volumes.
    let mut t = Session::open(&serde_json::to_string(&before).unwrap()).unwrap();
    t.clear_bed_model();
    let h1 = t.stage_hash(Stage::Volumes);
    t.run_reference_line().unwrap();
    assert_eq!(t.stage_hash(Stage::Volumes), h1);
}
