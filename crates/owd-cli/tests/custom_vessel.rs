//! `owd size --custom-vessel` (spec 0015 T1): the vessel is saved into the study, later commands
//! read it from there, and every output labels it `Custom: <name>, L × B × T`.
//!
//! Label: property (the web example, sized for a custom vessel with CEMT-IV's figures).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LABEL: &str = "Custom: My barge, 85 × 9.5 × 2.5 m";

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

/// A copy of the web example and its DEM in a fresh folder; the design's path.
fn work_dir(name: &str) -> (PathBuf, String) {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-custom-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    for f in ["river-example.owd", "river-dem.tif"] {
        std::fs::copy(ex.join(f), d.join(f)).unwrap();
    }
    let doc = d.join("river-example.owd").to_str().unwrap().to_string();
    (d, doc)
}

fn size_custom(doc: &str, extra: &[&str]) -> Output {
    let mut a = vec![
        "size",
        doc,
        "--custom-vessel",
        "My barge",
        "--loa",
        "85",
        "--beam",
        "9.5",
        "--draught",
        "2.5",
        "--formation",
        "motor",
    ];
    a.extend_from_slice(extra);
    run(&a)
}

fn read_doc(doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

#[test]
fn size_saves_the_custom_vessel_and_says_its_label() {
    let (_, doc) = work_dir("size");
    let o = size_custom(&doc, &["--air-draught", "7", "--deadweight", "1350"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains(LABEL), "{}", out(&o));
    let o = size_custom(&doc, &["--json"]);
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["sizing"]["vessel_label"], LABEL);
    assert!(v["sizing"]["vessel_standard_id"].is_null());
    let d = read_doc(&doc);
    let st = &d["stages"]["sizing"]["settings"];
    assert!(st["vessel_standard_id"].is_null());
    assert_eq!(st["custom_vessel"]["name"], "My barge");
    assert_eq!(d["format_version"], "0.4");
    // `owd size` with no vessel flag sizes the custom vessel the study has.
    let o = run(&[
        "size",
        &doc,
        "--rule-set",
        "pianc-wg141",
        "--set-input",
        "se_quality=B",
    ]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains(LABEL), "{}", out(&o));
    // `--vessel ID` replaces it with a class.
    let o = run(&["size", &doc, "--vessel", "CEMT-IV"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let st = &read_doc(&doc)["stages"]["sizing"]["settings"];
    assert_eq!(st["vessel_standard_id"], "CEMT-IV");
    assert!(st.get("custom_vessel").is_none());
}

#[test]
fn a_bad_custom_vessel_is_refused_by_field_and_a_missing_flag_is_named() {
    let (_, doc) = work_dir("refused");
    let before = std::fs::read_to_string(&doc).unwrap();
    let o = run(&[
        "size",
        &doc,
        "--custom-vessel",
        "Wide",
        "--loa",
        "20",
        "--beam",
        "30",
        "--draught",
        "2",
        "--formation",
        "motor",
    ]);
    assert_eq!(code(&o), 2, "{}", err(&o));
    assert!(err(&o).contains("beam_m"), "{}", err(&o));
    let o = run(&[
        "size",
        &doc,
        "--custom-vessel",
        "X",
        "--loa",
        "80",
        "--beam",
        "9",
        "--draught",
        "2",
    ]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("--formation"), "{}", err(&o));
    let o = run(&["size", &doc, "--custom-vessel", "X", "--vessel", "CEMT-IV"]);
    assert_eq!(code(&o), 2, "--custom-vessel and --vessel together");
    let o = run(&["size", &doc, "--loa", "80"]);
    assert_eq!(code(&o), 2, "--loa without --custom-vessel");
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), before);
}

#[test]
fn an_unusual_custom_vessel_sizes_with_a_warning() {
    let (_, doc) = work_dir("warned");
    let o = run(&[
        "size",
        &doc,
        "--custom-vessel",
        "Wide barge",
        "--loa",
        "60",
        "--beam",
        "29",
        "--draught",
        "2.5",
        "--formation",
        "motor",
    ]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("warning: the custom vessel's length-to-beam ratio"),
        "{}",
        out(&o)
    );
}

#[test]
fn every_output_labels_the_custom_vessel() {
    let (d, doc) = work_dir("labels");
    assert_eq!(code(&size_custom(&doc, &[])), 0);
    let o = run(&["run", &doc]);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));

    // fits: one more row, after the classes.
    let o = run(&["fits", &doc, "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    let rows = v["fits"]["rule_sets"][0]["classes"]
        .as_array()
        .or(v["results"]["rule_sets"][0]["classes"].as_array())
        .unwrap_or_else(|| panic!("{v}"))
        .clone();
    assert_eq!(rows.len(), 13, "the 12 classes and the custom vessel");
    let last = rows.last().unwrap();
    assert_eq!(last["custom"], true);
    assert_eq!(last["vessel_label"], LABEL);
    let o = run(&["fits", &doc]);
    assert!(out(&o).contains(LABEL), "{}", out(&o));

    // The rule trace.
    let o = run(&["trace", &doc]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains(&format!("# vessel: {LABEL}")),
        "{}",
        out(&o)
    );

    // The typical section.
    let o = run(&["draw", &doc, "--document", "--sheets", "ts"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("CUSTOM: MY BARGE, 85 X 9.5 X 2.5 M"),
        "the typical section names the vessel (sheet text is upper-case ASCII)"
    );

    // The exports.
    let o = run(&["export", &doc, "--csv"]);
    assert!(
        out(&o).contains(&format!("# Vessel: {LABEL}: user-entered dimensions")),
        "{}",
        out(&o)
    );
    let o = run(&["export", &doc, "--geojson"]);
    assert!(out(&o).contains(LABEL), "{}", out(&o));

    // The screening report: the design basis and the Data and limits box.
    let rep = d.join("reports");
    let o = run(&[
        "report",
        &doc,
        "--format",
        "html",
        "--date",
        "2026-10-22",
        "-o",
        rep.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let html: String = std::fs::read_dir(&rep)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    assert!(
        html.contains("Custom: My barge"),
        "the report names the vessel"
    );
    assert!(html.contains("user-entered dimensions, not a catalogue class"));
}
