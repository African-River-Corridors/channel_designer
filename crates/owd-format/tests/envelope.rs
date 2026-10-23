//! `stages.envelope` (spec 0016 T2): the size sweep kept in the study.
//!
//! The river is straight, 3 km long, 400 m wide, with both banks: wide enough for every size.
//!
//! Label: property

use owd_core::envelope::Settings;
use owd_format::design::{Role, Session, StageStatus};
use serde_json::json;

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn line(y: f64) -> String {
    let at = |x: f64| [x + 500_000.0, y + 600_000.0];
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!([at(0.0), at(3000.0)])
    )
}

fn study() -> Session {
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let ring = json!([
        at(0.0, -200.0),
        at(3000.0, -200.0),
        at(3000.0, 200.0),
        at(0.0, 200.0),
        at(0.0, -200.0)
    ]);
    let polygon = format!(r#"{{"type":"Polygon",{UTM30},"coordinates":[{ring}]}}"#);
    let mut s = Session::new("Straight", "2026-10-23");
    let (l, r) = (line(200.0), line(-200.0));
    s.import(&[
        (Role::RiverPolygon, polygon.as_str()),
        (Role::LeftBank, l.as_str()),
        (Role::RightBank, r.as_str()),
    ])
    .unwrap();
    s
}

#[test]
fn the_sweep_is_kept_in_the_study_and_reopens_fresh() {
    let mut s = study();
    assert_eq!(s.envelope_status(), StageStatus::NotRun);
    s.run_envelope(Settings::default(), true).unwrap();
    let r = s.envelope_result().unwrap().clone();
    assert_eq!(r["largest"]["label"], "50x10");
    assert_eq!(r["plan_line"], "reference");
    assert_eq!(r["rule_set"], "PIANC-C");
    assert_eq!(r["draught_m"], 2.0);
    assert_eq!(r["notice"], "Screening \u{2014} not for construction");
    assert!(r["bank_caveat"].as_str().unwrap().contains("banks"));
    assert_eq!(r["sizes"].as_array().unwrap().len(), 5);
    let km = r["per_km"].as_array().unwrap();
    assert_eq!(km.len(), 3);
    assert!(
        km.iter()
            .all(|k| k["largest"]["label"] == "50x10" && k["from"].is_array())
    );
    assert_eq!(r["marks"].as_array().unwrap().len(), 12);
    let back = Session::open(&s.to_text()).unwrap();
    assert!(back.validate().is_empty(), "{:?}", back.validate());
    assert_eq!(back.envelope_status(), StageStatus::Fresh);
}

#[test]
fn the_sizes_run_at_once_give_the_answer_they_give_one_after_another() {
    let (mut a, mut b) = (study(), study());
    a.run_envelope(Settings::default(), true).unwrap();
    b.run_envelope(Settings::default(), false).unwrap();
    assert_eq!(a.envelope_result(), b.envelope_result());
}

#[test]
fn a_change_to_the_banks_makes_it_stale_and_nothing_reruns_it() {
    let mut s = study();
    s.run_envelope(Settings::default(), false).unwrap();
    s.import(&[(Role::LeftBank, line(190.0).as_str())]).unwrap();
    assert_eq!(s.envelope_status(), StageStatus::Stale);
    s.run_stale().unwrap();
    assert_eq!(s.envelope_status(), StageStatus::Stale);
}

#[test]
fn a_bed_model_is_reported_beside_the_answer() {
    let mut s = study();
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s.set_bed_model(&[[0.0, 3.0], [3000.0, 1.5]], Some("dry"), "2026-10-23")
        .unwrap();
    s.run_envelope(Settings::default(), false).unwrap();
    let b = &s.envelope_result().unwrap()["bed_model"];
    assert_eq!(b["shallowest_depth_m"], 1.5);
    assert_eq!(b["draught_m"], 2.0);
}
