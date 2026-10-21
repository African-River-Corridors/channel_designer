//! Spec 0007 T3: `owd rules` and `owd size` with Rule Sets.
//!
//! Label: property. No fixed number.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn json(o: &Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-rules-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn rules_lists_the_built_in_sets_shows_one_and_lists_the_measures() {
    let list = json(&run(&["rules", "--json"]));
    let ids: Vec<&str> = list["rule_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["pianc-wg141", "gb50139", "owd"]);
    let show = json(&run(&["rules", "show", "gb50139", "--json"]));
    assert_eq!(show["version"], "0.3.0");
    for r in show["rules"].as_array().unwrap() {
        assert_eq!(r["status"], "unverified", "{r}");
        assert_eq!(r["origin"], "national-practice", "{r}");
    }
    let text = String::from_utf8(run(&["rules", "show", "pianc-wg141"]).stdout).unwrap();
    assert!(
        text.contains("pianc-wg141/Table5.7/bottom-width-b"),
        "{text}"
    );
    let m = json(&run(&["rules", "--measures", "--json"]));
    assert!(
        m["measures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "channel.bottom-width")
    );
    assert!(!run(&["rules", "show", "nope"]).status.success());
}

#[test]
fn size_takes_rule_sets_set_inputs_and_a_set_file_loaded_at_run_time() {
    let dir = work_dir("size");
    let doc = dir.join("d.owd");
    let doc = doc.to_str().unwrap();
    assert!(
        run(&["new", doc, "--title", "T", "--date", "2026-10-21"])
            .status
            .success()
    );
    let two = json(&run(&[
        "size",
        doc,
        "--rule-set",
        "pianc-wg141",
        "--rule-set",
        "gb50139",
        "--set-input",
        "se_quality=B",
        "--set-input",
        "range_end=strict",
        "--vessel",
        "CEMT_IV",
        "--json",
    ]));
    assert_eq!(
        two["sizing"]["ruleset_id"],
        "pianc-wg141@0.3.0 + gb50139@0.3.0 (se_quality B, range_end strict)"
    );
    // A set of my own, loaded at run time: PIANC quality B with a wider bottom.
    let mine = owd_core::rules::engine::BUILT_IN[0]
        .1
        .replace("\"k\": 3.2,", "\"k\": 3.6,")
        .replace("\"version\": \"0.3.0\"", "\"version\": \"0.9.0\"");
    let file = dir.join("mine.json");
    std::fs::write(&file, mine).unwrap();
    let f = file.to_str().unwrap();
    let o = json(&run(&[
        "size",
        doc,
        "--rule-set-file",
        f,
        "--rule-set",
        "pianc-wg141@0.9.0",
        "--set-input",
        "se_quality=B",
        "--json",
    ]));
    assert_eq!(
        o["sizing"]["bottom_width_straight_m"],
        3.6 * o["sizing"]["beam_m"].as_f64().unwrap()
    );
    // `run` without the file cannot size it; with it, nothing is stale.
    assert!(!run(&["run", doc]).status.success());
    assert_eq!(
        json(&run(&["run", doc, "--rule-set-file", f, "--json"]))["ran"],
        serde_json::json!([])
    );
    // A file with a built-in id and version but other content is refused.
    let clash = owd_core::rules::engine::BUILT_IN[0]
        .1
        .replace("\"k\": 3.2,", "\"k\": 3.6,");
    let bad = dir.join("clash.json");
    std::fs::write(&bad, clash).unwrap();
    let o = run(&["rules", "--rule-set-file", bad.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("pianc-wg141@0.3.0"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn waterway_marks_lists_and_removes_reaches_and_the_claim_sizes_the_canal() {
    let dir = work_dir("waterway");
    let doc = dir.join("d.owd");
    let doc = doc.to_str().unwrap();
    assert!(
        run(&["new", doc, "--title", "T", "--date", "2026-10-21"])
            .status
            .success()
    );
    let o = json(&run(&[
        "waterway",
        "add",
        doc,
        "--from",
        "0",
        "--waterway",
        "canal",
        "--claim-reduced-radius",
        "--json",
    ]));
    assert_eq!(o["waterway_reaches"][0]["reduced_radius_claim"], "yes");
    let sized = json(&run(&[
        "size",
        doc,
        "--rule-set",
        "gb50139",
        "--set-input",
        "range_end=strict",
        "--vessel",
        "CN_III_motor",
        "--json",
    ]));
    let loa = sized["sizing"]["loa_m"].as_f64().unwrap();
    assert_eq!(sized["sizing"]["min_bend_radius_m"], 3.0 * loa);
    let list = json(&run(&["waterway", "list", doc, "--json"]));
    assert_eq!(list["waterway_reaches"].as_array().unwrap().len(), 1);
    // A claim off a canal is refused.
    assert!(
        !run(&[
            "waterway",
            "add",
            doc,
            "--from",
            "5000",
            "--waterway",
            "free-flowing-river",
            "--claim-reduced-radius"
        ])
        .status
        .success()
    );
    let o = json(&run(&["waterway", "remove", doc, "--from", "0", "--json"]));
    assert_eq!(o["waterway_reaches"], serde_json::json!([]));
}

/// `owd trace` on the web example (every stage fresh): the same rows on every run, as JSON and as a
/// table; and `owd export --stretches` writes the stretches with the screening notice.
#[test]
fn trace_and_the_stretch_exports_run_on_the_example() {
    let dir = work_dir("trace");
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    for f in ["river-example.owd", "river-dem.tif"] {
        std::fs::copy(web.join(f), dir.join(f)).unwrap();
    }
    let doc = dir.join("river-example.owd");
    let doc = doc.to_str().unwrap();
    let a = json(&run(&["trace", doc, "--json"]));
    let b = json(&run(&["trace", doc, "--json"]));
    assert_eq!(a, b);
    assert!(!a["rows"].as_array().unwrap().is_empty());
    let table = String::from_utf8(run(&["trace", doc]).stdout).unwrap();
    assert!(table.starts_with("# Screening"), "{table}");
    let o = json(&run(&["trace", doc, "--option", "rolling-circle", "--json"]));
    assert!(o["rows"].as_array().unwrap().len() > a["rows"].as_array().unwrap().len());
    let csv = run(&["export", doc, "--csv", "--stretches"]);
    assert!(csv.status.success());
    let csv = String::from_utf8(csv.stdout).unwrap();
    assert!(csv.starts_with("# Screening"), "{csv}");
    assert!(csv.contains("option_id,kind,start_chainage_m"));
    let gj = json(&run(&["export", doc, "--geojson", "--stretches"]));
    assert_eq!(gj["type"], "FeatureCollection");
}
