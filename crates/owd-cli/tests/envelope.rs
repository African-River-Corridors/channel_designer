//! `owd envelope` (spec 0016 T2): the largest vessel the river allows, as JSON, CSV and a PNG
//! chart, kept in the study.
//!
//! The river is straight, 3 km long and 400 m wide, with both banks: every size of the default
//! series passes.
//!
//! Label: property

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn owd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn study(name: &str) -> (PathBuf, String) {
    let dir = work_dir(name);
    let doc = dir.join("t.owd").to_str().unwrap().to_string();
    assert!(
        owd(&["new", &doc, "--title", "t", "--date", "2026-10-23"])
            .status
            .success()
    );
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let w = |name: &str, text: String| {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p.to_str().unwrap().to_string()
    };
    let line = |y: f64| {
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!([at(0.0, y), at(3000.0, y)])
        )
    };
    let ring = json!([
        at(0.0, -200.0),
        at(3000.0, -200.0),
        at(3000.0, 200.0),
        at(0.0, 200.0),
        at(0.0, -200.0)
    ]);
    let poly = w(
        "polygon.geojson",
        format!(r#"{{"type":"Polygon"{UTM30},"coordinates":[{ring}]}}"#),
    );
    let (l, r) = (
        w("left.geojson", line(200.0)),
        w("right.geojson", line(-200.0)),
    );
    let o = owd(&[
        "import",
        &doc,
        "--polygon",
        &poly,
        "--left-bank",
        &l,
        "--right-bank",
        &r,
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    (dir, doc)
}

#[test]
fn the_sweep_answers_in_json_and_is_kept_in_the_study() {
    let (_dir, doc) = study("envelope-json");
    let o = owd(&["envelope", &doc, "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    let e = &v["envelope"];
    assert_eq!(e["largest"]["label"], "50x10");
    assert_eq!(e["rule_set"], "PIANC-C");
    assert_eq!(e["draught_m"], 2.0);
    assert_eq!(e["notice"], "Screening \u{2014} not for construction");
    assert_eq!(v["status"], "fresh");
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(d["stages"]["envelope"]["results"], *e);
    assert!(owd(&["validate", &doc]).status.success());
    // The table says the same, with the notices.
    let t = out(&owd(&["envelope", &doc, "--show"]));
    assert!(
        t.contains("Largest size that passes the whole reach: 50 x 10 m"),
        "{t}"
    );
    assert!(t.contains("Screening \u{2014} not for construction"), "{t}");
    assert!(t.contains("PIANC-C"), "{t}");
}

#[test]
fn the_settings_are_read_and_kept() {
    let (_dir, doc) = study("envelope-settings");
    let o = owd(&[
        "envelope",
        &doc,
        "--sizes",
        "10x2,30x6",
        "--draught",
        "1.5",
        "--rule-set",
        "PIANC-B",
        "--json",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["envelope"]["sizes"].as_array().unwrap().len(), 2);
    assert_eq!(v["envelope"]["draught_m"], 1.5);
    assert_eq!(v["envelope"]["rule_set"], "PIANC-B");
    let bad = owd(&["envelope", &doc, "--sizes", "30by6"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("LENGTHxBEAM"));
}

#[test]
fn csv_and_png_carry_the_strip_and_the_notices() {
    let (dir, doc) = study("envelope-csv-png");
    let o = owd(&["envelope", &doc, "--csv"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let csv = out(&o);
    let lines: Vec<&str> = csv.lines().collect();
    assert!(
        lines[0].contains("Screening \u{2014} not for construction"),
        "{csv}"
    );
    assert!(
        csv.contains("PIANC-C") && csv.contains("draught 2 m") && csv.contains("banks"),
        "{csv}"
    );
    let header = lines.iter().find(|l| !l.starts_with('#')).unwrap();
    assert_eq!(*header, "from_m,to_m,largest,10x2,20x4,30x6,40x8,50x10");
    let rows: Vec<&&str> = lines
        .iter()
        .filter(|l| !l.starts_with('#'))
        .skip(1)
        .collect();
    assert_eq!(rows.len(), 3);
    assert!(
        rows[0].starts_with("0,1000,50x10,pass,pass,pass,pass,pass"),
        "{}",
        rows[0]
    );
    let png = dir.join("envelope.png");
    let o = owd(&["envelope", &doc, "--show", "--png", png.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
}

#[test]
fn a_change_to_the_banks_shows_the_sweep_stale() {
    let (dir, doc) = study("envelope-stale");
    assert!(owd(&["envelope", &doc]).status.success());
    let moved = dir.join("left2.geojson");
    let at = |x: f64| [x + 500_000.0, 190.0 + 600_000.0];
    std::fs::write(
        &moved,
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!([at(0.0), at(3000.0)])
        ),
    )
    .unwrap();
    assert!(
        owd(&["import", &doc, "--left-bank", moved.to_str().unwrap()])
            .status
            .success()
    );
    let t = out(&owd(&["envelope", &doc, "--show"]));
    assert!(t.contains("stale"), "{t}");
}
