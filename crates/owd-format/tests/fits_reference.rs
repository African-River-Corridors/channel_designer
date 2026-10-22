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
    format!(r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#, json!(pts))
}

fn wiggle(i: usize, amp: f64) -> f64 {
    let t = i as f64;
    amp * (0.6 * (1.7 * t).sin() + 0.3 * (4.1 * t + 1.0).sin() + 0.1 * (9.3 * t).sin())
}

fn meander(with_banks: bool) -> Session {
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let k = 2.0 * std::f64::consts::PI / 1500.0;
    let (mut left, mut right, mut centre) = (vec![], vec![], vec![]);
    for i in 0..=600 {
        let x = 10.0 * i as f64;
        let (y, d) = (150.0 * (k * x).sin(), 150.0 * k * (k * x).cos());
        let l = (1.0 + d * d).sqrt();
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
    let polygon = format!(r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#, json!(ring));
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
    s.run_fits_on(Some(&choices), None, PlanLine::Centreline).unwrap();
    let on_centre = s.fits_result().unwrap().clone();
    s.run_fits_on(Some(&choices), None, PlanLine::Reference).unwrap();
    let on_ref = s.fits_result().unwrap().clone();
    assert_eq!(on_ref["plan_line"], "reference");
    let (a, b) = (radius_stretches(&on_ref), radius_stretches(&on_centre));
    assert!(a < b, "radius stretches: reference {a}, centreline {b}");
    // The answer is read along the reference line: its length is the line's.
    let rl = s.reference_line().unwrap();
    let len: f64 = rl
        .windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
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
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum();
    for c in r["rule_sets"][0]["classes"].as_array().unwrap() {
        assert!(c["verdict"].is_string(), "{}", c["vessel_standard_id"]);
        for st in c["constraint_stretches"].as_array().unwrap() {
            let end = st["end_chainage_m"].as_f64().unwrap();
            assert!(end <= len + 1.0, "a stretch ends at {end} m, past the line's {len} m");
            // The bed model gives 4 m everywhere in the water: every station of the walk is in it.
            if st["kind"] == "depth" {
                let d = st["river"].as_f64().unwrap();
                assert!(d > 3.9, "a station reads {d} m deep, not the bed model's 4 m");
            }
        }
    }
}
