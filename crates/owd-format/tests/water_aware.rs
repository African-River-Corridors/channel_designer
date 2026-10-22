//! Spec 0005 in the channel design document (#100): the version 1 to 2 move, seed legs and fixed
//! pieces, and the water-aware rolling circle's checks in every export.
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::fixed_tail;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};

const SHIFT: Pt = [500_000.0, 600_000.0];
const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn pts(v: &Value) -> Vec<Pt> {
    v["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

/// A document from before spec 0005 (format version 1) is refused since spec 0007, which moves no
/// older file: the message says to rebuild the design.
#[test]
fn a_pre_0005_document_is_refused_with_a_message_to_rebuild() {
    let text = include_str!("data/pre-0005.owd");
    let old: Value = serde_json::from_str(text).unwrap();
    assert_eq!(old["format_version"], 1);
    let e = Session::open(text).unwrap_err().to_string();
    assert!(e.contains("format version 1"), "{e}");
    assert!(e.contains("Rebuild the design"), "{e}");
}

/// The fixed-tail fixture as a document: open water, two seed legs and a must-follow tail.
fn tail_design() -> Session {
    let f = fixed_tail();
    let ring: Vec<Pt> = f.water.exterior.iter().map(shift).collect();
    let polygon = format!(
        r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
        json!([ring])
    );
    let centre: Vec<Pt> = vec![shift(&[0.0, 0.0]), shift(&[3000.0, 100.0])];
    let line = format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!(centre)
    );
    let mut s = Session::new("Fixed tail", "2026-10-16");
    s.import(&[(Role::Centreline, &line), (Role::RiverPolygon, &polygon)])
        .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let mut doc: Value = serde_json::from_str(&s.to_text()).unwrap();
    let ls = |p: Vec<Pt>| json!({"type": "LineString", "coordinates": p});
    doc["inputs"]["seed_legs"] = json!([
        ls(vec![shift(&[0.0, 0.0]), shift(&[1000.0, 0.0])]),
        ls(vec![shift(&[1000.0, 0.0]), shift(&[2000.0, 0.0])]),
    ]);
    doc["inputs"]["fixed"] = json!({
        "start_pose": null,
        "end_pose": null,
        "must_follow": [ls(f.fixed[0].iter().map(shift).collect())],
    });
    Session::open(&serde_json::to_string(&doc).unwrap()).unwrap()
}

#[test]
fn seed_legs_and_fixed_pieces_give_the_joined_fitted_option() {
    let mut s = tail_design();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let o = &s.document()["options"][0];
    let line = pts(&o["line"]);
    let tail: Vec<Pt> = fixed_tail().fixed[0].iter().map(shift).collect();
    // Joined, fitted, and ending on the must-follow tail exactly.
    assert_eq!(&line[line.len() - 2..], &tail[..]);
    assert_eq!(line[0], shift(&[0.0, 0.0]));
    assert_eq!(o["checks"]["join_guard"]["ok"], true);
    assert_eq!(o["checks"]["fits_in_water"]["ok"], true, "{}", o["checks"]);
    assert_eq!(o["checks"]["radius_ok"], true, "{}", o["checks"]);
    assert_eq!(
        o["checks"]["fits_in_water"]["note"],
        "width only, no depth data"
    );
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    // Exports carry the note.
    let csv = s.export_csv(None);
    assert!(csv.contains("width only, no depth data"), "{csv}");
    let g: Value = serde_json::from_str(&s.export_geojson(false, None).unwrap()).unwrap();
    assert_eq!(
        g["features"][0]["properties"]["notes"],
        "width only, no depth data"
    );
}
