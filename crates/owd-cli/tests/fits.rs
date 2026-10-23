//! `owd fits` (spec 0012, ticket T8): the plan-only answer, kept in the document, stale when the
//! banks change and never rerun by itself.
//!
//! Label: sourced - hand calculation (a 60 m throat, so a class is stopped by width exactly when
//! its straight bottom width is over 60 m)

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

/// A straight river, 3 km long, with a 60 m throat between two land teeth at x 900..1600.
fn files(dir: &Path, bank_shift: f64) -> (String, String) {
    let at = |p: [f64; 2]| [p[0] + 500_000.0, p[1] + bank_shift + 600_000.0];
    let h = 30.0;
    let ring: Vec<[f64; 2]> = [
        [-500.0, -1500.0],
        [900.0, -1500.0],
        [900.0, -h],
        [1600.0, -h],
        [1600.0, -1500.0],
        [3500.0, -1500.0],
        [3500.0, 1500.0],
        [1600.0, 1500.0],
        [1600.0, h],
        [900.0, h],
        [900.0, 1500.0],
        [-500.0, 1500.0],
        [-500.0, -1500.0],
    ]
    .into_iter()
    .map(at)
    .collect();
    let polygon = format!(
        r#"{{"type":"Polygon"{UTM30},"coordinates":[{}]}}"#,
        json!(ring)
    );
    let centre = format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!([at([0.0, 0.0]), at([3000.0, 0.0])])
    );
    let w = |name: &str, text: String| {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p.to_str().unwrap().to_string()
    };
    (w("polygon.geojson", polygon), w("centre.geojson", centre))
}

fn setup(name: &str) -> (PathBuf, String) {
    let dir = work_dir(name);
    let doc = dir.join("t.owd").to_str().unwrap().to_string();
    assert!(
        owd(&["new", &doc, "--title", "t", "--date", "2026-10-21"])
            .status
            .success()
    );
    let (poly, centre) = files(&dir, 0.0);
    let o = owd(&["import", &doc, "--polygon", &poly, "--centreline", &centre]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    (dir, doc)
}

#[test]
fn fits_answers_every_class_and_stores_the_answer_in_the_document() {
    let (_dir, doc) = setup("fits-run");
    let o = owd(&["fits", &doc, "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["label"], "width and radius only, no depth");
    let rs = &v["fits"]["rule_sets"][0];
    assert_eq!(rs["rule_set"], "PIANC-B");
    let classes = rs["classes"].as_array().unwrap();
    assert_eq!(classes.len(), 12);
    for c in classes {
        assert_eq!(c["label"], "width and radius only, no depth");
        // The chord the radius is read on is stated, a third of R_min (#368).
        let chord = c["radius_check_chord_m"].as_f64().unwrap();
        assert!((chord - c["r_min_m"].as_f64().unwrap() / 3.0).abs() < 1e-9);
        let wide = c["bottom_width_straight_m"].as_f64().unwrap() > 60.0;
        assert_eq!(c["fits"], !wide, "{}", c["vessel_standard_id"]);
        if wide {
            let s = &c["stretches"][0];
            assert_eq!(s["kind"], "width");
            assert!((s["start_chainage_m"].as_f64().unwrap() - 900.0).abs() < 0.05);
            assert!((s["end_chainage_m"].as_f64().unwrap() - 1600.0).abs() < 0.05);
        }
    }
    // The old `CEMT_VIa_motor_NL2011`, by its catalogue id (spec 0015 T0).
    assert_eq!(rs["largest_fitting"], "RWS-M12");
    // The document holds it, and the table reads the same.
    let text = std::fs::read_to_string(&doc).unwrap();
    let d: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(d["fits"]["results"], v["fits"]);
    assert!(owd(&["validate", &doc]).status.success());
    let t = out(&owd(&["fits", &doc]));
    assert!(t.contains("width and radius only, no depth"), "{t}");
    assert!(
        t.contains("Largest class that fits as it is: RWS-M12"),
        "{t}"
    );
    assert!(t.contains("width from 900 m to 1600 m"), "{t}");
    assert!(t.contains("chord m"), "{t}");
}

#[test]
fn all_checks_every_class_of_the_catalogue_and_is_kept_in_the_settings() {
    let (_dir, doc) = setup("fits-all");
    let o = owd(&["fits", &doc, "--all", "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    let classes = v["fits"]["rule_sets"][0]["classes"].as_array().unwrap();
    assert!(classes.len() > 12, "{}", classes.len());
    assert!(
        classes
            .iter()
            .any(|c| c["vessel_standard_id"] == "CEMT-VIb-motor")
    );
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(d["fits"]["settings"]["all_classes"], true);
    // Back to the default 12: the setting goes, so an `--all` answer reads as stale until rerun.
    let o = owd(&["fits", &doc, "--json"]);
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(
        v["fits"]["rule_sets"][0]["classes"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert!(d["fits"]["settings"]["all_classes"].is_null());
}

#[test]
fn a_change_to_the_banks_marks_the_answer_stale_and_nothing_reruns_it() {
    let (dir, doc) = setup("fits-stale");
    assert!(owd(&["fits", &doc]).status.success());
    let shown = |doc: &str| -> Value {
        serde_json::from_str(&out(&owd(&["fits", doc, "--show", "--json"]))).unwrap()
    };
    assert_eq!(shown(&doc)["status"], "fresh");
    let before = std::fs::read_to_string(&doc).unwrap();
    // Move the river 10 m north: the banks changed.
    let (poly, _) = files(&dir, 10.0);
    assert!(owd(&["import", &doc, "--polygon", &poly]).status.success());
    assert_eq!(shown(&doc)["status"], "stale");
    // `run` reruns stale stages, never the answer.
    assert!(owd(&["run", &doc]).status.success());
    assert_eq!(shown(&doc)["status"], "stale");
    let human = out(&owd(&["fits", &doc, "--show"]));
    assert!(human.contains("stale"), "{human}");
    // An explicit run replaces it.
    assert!(owd(&["fits", &doc]).status.success());
    assert_eq!(shown(&doc)["status"], "fresh");
    assert_ne!(std::fs::read_to_string(&doc).unwrap(), before);
}

#[test]
fn a_document_with_no_answer_and_a_bad_ruleset_are_refused() {
    let (_dir, doc) = setup("fits-refuse");
    let o = owd(&["fits", &doc, "--show"]);
    assert_eq!(o.status.code(), Some(2));
    let o = owd(&["fits", &doc, "--ruleset", "NOPE"]);
    assert_eq!(o.status.code(), Some(2));
    let o = owd(&[
        "fits",
        &doc,
        "--ruleset",
        "PIANC-A",
        "--ruleset",
        "PIANC-B",
        "--json",
    ]);
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["rule_sets"].as_array().unwrap().len(), 2);
    // The choices are kept for the next run.
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(
        d["fits"]["settings"]["rule_sets"],
        json!(["PIANC-A", "PIANC-B"])
    );
}

/// The study of `crates/owd-format/tests/fits_full.rs` (a straight, 400 m wide river with a shallow
/// bar of 1.5 m in a bed model of 4 m) and a flat DEM at 12 m, written to a folder.
fn full_study(name: &str) -> String {
    use owd_format::design::{Role, Session};
    use owd_format::geotiff::Dem;
    let dir = work_dir(name);
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let mut s = Session::new("t", "2026-10-21");
    let polygon = format!(
        r#"{{"type":"Polygon"{UTM30},"coordinates":[{}]}}"#,
        json!([
            at(0.0, -200.0),
            at(3000.0, -200.0),
            at(3000.0, 200.0),
            at(0.0, 200.0),
            at(0.0, -200.0)
        ])
    );
    let centre = format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!([at(0.0, 0.0), at(3000.0, 0.0)])
    );
    s.import(&[
        (Role::RiverPolygon, polygon.as_str()),
        (Role::Centreline, centre.as_str()),
    ])
    .unwrap();
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
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
    let (w, h) = (120, 40);
    let dem = Dem {
        width: w,
        height: h,
        data: vec![12.0; w * h],
        geotransform: [50.0, 0.0, 499_500.0, 0.0, -50.0, 601_000.0],
        nodata: None,
        epsg: Some(32630),
    };
    let bytes = owd_format::design::dem_source::write_geotiff(&dem).unwrap();
    std::fs::write(dir.join("dem.tif"), &bytes).unwrap();
    let back = owd_format::geotiff::read_bytes(&bytes).unwrap();
    s.set_dem("dem.tif", &bytes, &back);
    let doc = dir.join("t.owd");
    std::fs::write(&doc, s.to_text()).unwrap();
    doc.to_str().unwrap().to_string()
}

#[test]
fn with_a_bed_model_and_a_dem_the_answer_is_full_with_the_verdict_and_the_depth_stretches() {
    let doc = full_study("fits-full");
    let o = owd(&["fits", &doc, "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["mode"], "full");
    let classes = v["fits"]["rule_sets"][0]["classes"].as_array().unwrap();
    assert!(
        classes
            .iter()
            .all(|c| c["depth_basis"] == "depth assumed, not measured")
    );
    assert!(classes.iter().all(|c| c["verdict"].is_string()));
    assert!(
        classes.iter().any(|c| c["constraint_stretches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "depth" && s["works"] == "dredge")),
        "some class meets the bar"
    );
    assert!(owd(&["validate", &doc]).status.success());
    let t = out(&owd(&["fits", &doc, "--show"]));
    assert!(t.contains("depth assumed, not measured"), "{t}");
    assert!(t.contains("verdict: navigable-with-works"), "{t}");
    assert!(t.contains("works: dredge"), "{t}");
}

#[test]
fn the_answer_says_which_plan_line_it_read_the_radius_on() {
    // Plan only: the fairway needs the full answer, so the radius is read on the centreline.
    let (_dir, doc) = setup("fits-plan-line");
    let t = out(&owd(&["fits", &doc, "--plan-line", "fairway"]));
    assert!(t.contains("Plan line: centreline"), "{t}");
    assert!(t.contains("fairway"), "{t}");
    // The default, the reference line, needs the banks: this study has none.
    let t = out(&owd(&["fits", &doc]));
    assert!(t.contains("Plan line: centreline"), "{t}");
    assert!(t.contains("reference line could not be made"), "{t}");
    // Full: asked for, the fairway.
    let doc = full_study("fits-plan-line-full");
    let o = owd(&["fits", &doc, "--plan-line", "fairway", "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["plan_line"], "fairway");
    let t = out(&owd(&["fits", &doc, "--show"]));
    assert!(t.contains("Plan line: fairway"), "{t}");
    // Asked for: the centreline.
    let o = owd(&["fits", &doc, "--plan-line", "centreline", "--json"]);
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["plan_line"], "centreline");
    assert!(v["fits"].get("plan_line_note").is_none());
}

/// The route the README gives for a study with its own DEM and a fixed water level (#370): `owd new`,
/// then `owd import --dem … --water-level …` with the river, then `owd bed` and `owd fits`.
#[test]
fn a_local_dem_and_a_fixed_water_level_go_in_with_import() {
    use owd_format::geotiff::Dem;
    let dir = work_dir("fits-local-dem");
    let doc = dir.join("study.owd").to_str().unwrap().to_string();
    let ok = |o: Output| {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    };
    ok(owd(&[
        "new",
        &doc,
        "--title",
        "My river",
        "--date",
        "2026-10-23",
    ]));
    let (poly, centre) = files(&dir, 0.0);
    // A flat DEM at 12 m in the river's CRS (WGS84 / UTM 30N), 50 m cells over the whole river.
    let (w, h) = (100, 80);
    let dem = Dem {
        width: w,
        height: h,
        data: vec![12.0; w * h],
        geotransform: [50.0, 0.0, 499_000.0, 0.0, -50.0, 602_000.0],
        nodata: None,
        epsg: Some(32630),
    };
    let tif = dir.join("dem.tif");
    std::fs::write(
        &tif,
        owd_format::design::dem_source::write_geotiff(&dem).unwrap(),
    )
    .unwrap();
    ok(owd(&[
        "import",
        &doc,
        "--polygon",
        &poly,
        "--centreline",
        &centre,
        "--dem",
        tif.to_str().unwrap(),
        "--water-level",
        "10",
    ]));
    ok(owd(&["bed", &doc, "--depth", "3", "--season", "dry"]));
    let o = ok(owd(&["fits", &doc, "--json"]));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["mode"], "full", "{}", v["fits"]);
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(d["inputs"]["dem"]["path"], "dem.tif");
    ok(owd(&["validate", &doc]));
}

#[test]
fn with_both_banks_the_default_reads_the_reference_line_and_stores_it() {
    // Label: property (spec 0016 T1).
    let dir = work_dir("fits-reference");
    let doc = dir.join("t.owd").to_str().unwrap().to_string();
    assert!(
        owd(&["new", &doc, "--title", "t", "--date", "2026-10-22"])
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
    let (left, right) = (
        w("left.geojson", line(200.0)),
        w("right.geojson", line(-200.0)),
    );
    let o = owd(&[
        "import",
        &doc,
        "--polygon",
        &poly,
        "--left-bank",
        &left,
        "--right-bank",
        &right,
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = owd(&["fits", &doc, "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["plan_line"], "reference");
    assert!(v["fits"].get("plan_line_note").is_none());
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    let rl = &d["inputs"]["reference_line"]["line"]["coordinates"];
    assert!(rl.as_array().unwrap().len() > 2);
    assert!(owd(&["validate", &doc]).status.success());
    let t = out(&owd(&["fits", &doc, "--show"]));
    assert!(t.contains("Plan line: reference"), "{t}");
}

/// `fits` takes a Rule Set and its quality as `size` does (#362): `--rule-set pianc-wg141
/// --set-input se_quality=C` is the answer under PIANC-C.
#[test]
fn fits_names_a_rule_set_and_its_quality_as_size_does() {
    let (_dir, doc) = setup("fits-size-names");
    let o = owd(&[
        "fits",
        &doc,
        "--rule-set",
        "pianc-wg141",
        "--set-input",
        "se_quality=C",
        "--json",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["fits"]["rule_sets"][0]["rule_set"], "PIANC-C");
    assert_eq!(
        v["fits"]["rule_sets"][0]["selection"]["set_inputs"][0]["value"],
        "C"
    );
    // Two sets, each with its own input, and the old name too: three answers, in order.
    let o = owd(&[
        "fits",
        &doc,
        "--rule-set",
        "pianc-wg141",
        "--rule-set",
        "gb50139",
        "--set-input",
        "se_quality=A",
        "--set-input",
        "range_end=strict",
        "--ruleset",
        "PIANC-B",
        "--json",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    let names: Vec<&str> = v["fits"]["rule_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["rule_set"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["PIANC-B", "PIANC-A", "CN-conservative"]);
    // A set without its input, or an input the set does not take, is refused with the fix.
    let o = owd(&["fits", &doc, "--rule-set", "pianc-wg141"]);
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("se_quality"), "{err}");
    let o = owd(&[
        "fits",
        &doc,
        "--rule-set",
        "pianc-wg141",
        "--set-input",
        "se_quality=Z",
    ]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn draught_sets_every_class_s_draught_and_is_kept_in_the_settings() {
    // #364. The river is 4 m deep with a 1.5 m bar. At 1 m draught every class's design depth is
    // under 1.5 m, so no class meets the bar; at 3.5 m every class needs more than the 4 m.
    let doc = full_study("fits-draught");
    let rows = |args: &[&str]| -> (Value, Vec<Value>) {
        let o = owd(args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let v: Value = serde_json::from_str(&out(&o)).unwrap();
        let rows = v["fits"]["rule_sets"][0]["classes"]
            .as_array()
            .unwrap()
            .clone();
        (v, rows)
    };
    let depth = |c: &Value| {
        c["constraint_stretches"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["kind"] == "depth")
            .count()
    };
    let (v, shallow) = rows(&["fits", &doc, "--draught", "1", "--json"]);
    assert_eq!(v["fits"]["draught_m"], 1.0);
    assert!(shallow.iter().all(|c| depth(c) == 0), "{shallow:?}");
    let (_, deep) = rows(&["fits", &doc, "--draught", "3.5", "--json"]);
    assert!(deep.iter().all(|c| depth(c) > 0));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(d["fits"]["settings"]["draught_m"], 3.5);
    let t = out(&owd(&["fits", &doc, "--show"]));
    assert!(t.contains("Draught: 3.5 m for every class"), "{t}");
    // Without it, each class's own draught, and no key.
    let (v, _) = rows(&["fits", &doc, "--json"]);
    assert!(v["fits"].get("draught_m").is_none());
    let bad = owd(&["fits", &doc, "--draught", "-1"]);
    assert!(!bad.status.success());
}
