//! `owd check-banks` (spec 0013, ticket T1): clean banks pass and exit 0; a bank that dives to the
//! other side is reported and exits 1; the document is not changed.
//!
//! Label: property

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-check-banks-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A study with a straight channel 200 m wide; `dive` moves the left bank at x = 1000 to 1040.
fn study(name: &str, dive: f64) -> String {
    let dir = work_dir(name);
    let bank = |y: f64, d: f64| -> Vec<[f64; 2]> {
        (0..=300)
            .map(|i| {
                let x = i as f64 * 10.0;
                let off = if (1_000.0..=1_040.0).contains(&x) {
                    d
                } else {
                    0.0
                };
                [500_000.0 + x, 600_000.0 + y + off]
            })
            .collect()
    };
    let w = |f: &str, pts: Vec<[f64; 2]>| {
        let p = dir.join(f);
        std::fs::write(
            &p,
            format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(pts)
            ),
        )
        .unwrap();
        p.to_str().unwrap().to_string()
    };
    let (l, r) = (
        w("l.geojson", bank(100.0, dive)),
        w("r.geojson", bank(-100.0, 0.0)),
    );
    let doc = dir.join("t.owd").to_str().unwrap().to_string();
    assert!(
        run(&["new", &doc, "--title", "t", "--date", "2026-10-22"])
            .status
            .success()
    );
    let o = run(&["import", &doc, "--left-bank", &l, "--right-bank", &r]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    doc
}

#[test]
fn clean_banks_pass_and_a_bank_on_the_wrong_side_fails_with_exit_1() {
    let doc = study("clean", 0.0);
    let o = run(&["check-banks", &doc, "--json"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true);

    let doc = study("dive", -220.0);
    let before = std::fs::read(&doc).unwrap();
    let o = run(&["check-banks", &doc, "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], false);
    let checks: Vec<&str> = v["failed"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| {
            f["checks"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
        })
        .collect();
    assert!(checks.contains(&"side"), "{v}");
    assert!(v["crossing"].is_array(), "{v}");
    let human = run(&["check-banks", &doc]);
    assert!(String::from_utf8_lossy(&human.stdout).contains("side"));
    assert_eq!(
        std::fs::read(&doc).unwrap(),
        before,
        "check-banks changes nothing"
    );
}
