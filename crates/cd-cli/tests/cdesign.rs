//! The `cdesign` design commands (spec 0002, ticket 60), on the synthetic river given as GeoJSON in
//! UTM: the chain `new -> import -> size`, `status`, `validate`, `schema`, `--json`, `-o`, stdin
//! and stdout, and the three exit codes.
//!
//! Label: property

use cd_core::geom::Geometry;
use cd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use cd_core::synthetic::{RiverParams, synthetic_river};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cdesign"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cdesign runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
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

/// A fresh directory for one test.
fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cdesign-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn line_json(pts: &[[f64; 2]]) -> String {
    format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// Write the synthetic river's banks and polygon as GeoJSON in UTM 30N; return the three paths.
fn river_files(dir: &Path) -> (String, String, String) {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let shift = |p: &[f64; 2]| [p[0] + 500_000.0, p[1] + 600_000.0];
    let pts = |g: &Geometry| match g {
        Geometry::LineString(l) => l.iter().map(shift).collect::<Vec<[f64; 2]>>(),
        other => panic!("a bank is a line, found {other:?}"),
    };
    let ring: Vec<[f64; 2]> = river.polygon.exterior.iter().map(shift).collect();
    let w = |name: &str, text: String| {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p.to_str().unwrap().to_string()
    };
    (
        w("left.geojson", line_json(&pts(&river.left_bank))),
        w("right.geojson", line_json(&pts(&river.right_bank))),
        w(
            "polygon.geojson",
            format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    )
}

/// `new`, `import` and `size`, in files in `dir`. Returns the document path.
fn build(dir: &Path) -> String {
    let (l, r, p) = river_files(dir);
    let doc = dir.join("river.cdesign").to_str().unwrap().to_string();
    assert_eq!(
        code(&run(
            &[
                "new",
                &doc,
                "--title",
                "Synthetic river",
                "--date",
                "2026-10-16"
            ],
            ""
        )),
        0
    );
    let o = run(
        &[
            "import",
            &doc,
            "--left-bank",
            &l,
            "--right-bank",
            &r,
            "--polygon",
            &p,
        ],
        "",
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    let o = run(
        &[
            "size",
            &doc,
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
        ],
        "",
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    doc
}

#[test]
fn the_chain_new_import_size_gives_a_document_that_validates_and_reports_status() {
    let dir = work_dir("chain");
    let doc = build(&dir);
    let v = run(&["validate", &doc, "--json"], "");
    assert_eq!(code(&v), 0);
    assert_eq!(
        serde_json::from_str::<Value>(&out(&v)).unwrap(),
        json!({"valid": true, "errors": []})
    );
    let s = run(&["status", &doc, "--json"], "");
    let status: Value = serde_json::from_str(&out(&s)).unwrap();
    assert_eq!(
        status["stages"],
        json!({"sizing": "fresh", "alignment": "not_run", "optimise": "not_run", "volumes": "not_run"})
    );
    let human = run(&["status", &doc], "");
    assert!(out(&human).contains("sizing: fresh") && out(&human).contains("alignment: not run"));
}

#[test]
fn the_size_command_gives_what_channel_shape_gives() {
    let dir = work_dir("size");
    let doc = build(&dir);
    let o = run(
        &[
            "size",
            &doc,
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
            "-o",
            "-",
            "--json",
        ],
        "",
    );
    assert_eq!(code(&o), 0);
    let direct = channel_shape(&RulesetRegistry::new(), "PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    // The document goes to stdout, so the JSON result goes to stderr.
    let result: Value = serde_json::from_str(err(&o).trim()).unwrap();
    assert_eq!(
        result["sizing"]["min_bend_radius_m"],
        json!(direct.min_bend_radius_m)
    );
    assert_eq!(
        result["sizing"]["bottom_width_straight_m"],
        json!(direct.bottom_width_straight_m)
    );
    let saved: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(saved["stages"]["sizing"]["results"], result["sizing"]);
}

#[test]
fn a_changed_vessel_shows_sizing_stale_and_nothing_reruns() {
    let dir = work_dir("stale");
    let doc = build(&dir);
    let text = std::fs::read_to_string(&doc).unwrap();
    // Change the vessel in the settings only, as an edit of the file would.
    let edited = text.replacen(
        r#""vessel_standard_id": "CEMT_IV",
        "draught_m""#,
        r#""vessel_standard_id": "CEMT_Va",
        "draught_m""#,
        1,
    );
    assert_ne!(edited, text, "the settings text was found");
    std::fs::write(&doc, &edited).unwrap();
    let s = run(&["status", &doc, "--json"], "");
    let status: Value = serde_json::from_str(&out(&s)).unwrap();
    assert_eq!(status["stages"]["sizing"], "stale");
    assert_eq!(status["stale"], json!(["sizing"]));
    assert_eq!(
        std::fs::read_to_string(&doc).unwrap(),
        edited,
        "status wrote nothing"
    );
    // The file still validates: stale is a state, not a fault.
    assert_eq!(code(&run(&["validate", &doc], "")), 0);
}

#[test]
fn schema_prints_the_published_schema_file() {
    let o = run(&["schema"], "");
    assert_eq!(code(&o), 0);
    let file = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/cdesign.schema.json"),
    )
    .unwrap();
    assert_eq!(out(&o), file);
}

#[test]
fn the_chain_runs_through_stdin_and_stdout() {
    let dir = work_dir("pipe");
    let (l, r, p) = river_files(&dir);
    let new = run(
        &["new", "-", "--title", "Piped", "--date", "2026-10-16"],
        "",
    );
    assert_eq!(code(&new), 0);
    let imp = run(
        &[
            "import",
            "-",
            "--left-bank",
            &l,
            "--right-bank",
            &r,
            "--polygon",
            &p,
            "-o",
            "-",
        ],
        &out(&new),
    );
    assert_eq!(code(&imp), 0, "{}", err(&imp));
    let size = run(
        &[
            "size",
            "-",
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "-o",
            "-",
        ],
        &out(&imp),
    );
    assert_eq!(code(&size), 0, "{}", err(&size));
    let st = run(&["status", "-", "--json"], &out(&size));
    assert_eq!(code(&st), 0);
    assert_eq!(
        serde_json::from_str::<Value>(&out(&st)).unwrap()["stages"]["sizing"],
        "fresh"
    );
}

#[test]
fn output_goes_where_dash_o_says_and_the_input_is_otherwise_rewritten_in_place() {
    let dir = work_dir("output");
    let (l, r, p) = river_files(&dir);
    let doc = dir.join("a.cdesign").to_str().unwrap().to_string();
    let copy = dir.join("b.cdesign").to_str().unwrap().to_string();
    run(&["new", &doc, "--title", "A", "--date", "2026-10-16"], "");
    let before = std::fs::read_to_string(&doc).unwrap();
    let o = run(
        &[
            "import",
            &doc,
            "--left-bank",
            &l,
            "--right-bank",
            &r,
            "--polygon",
            &p,
            "-o",
            &copy,
        ],
        "",
    );
    assert_eq!(code(&o), 0);
    assert_eq!(
        std::fs::read_to_string(&doc).unwrap(),
        before,
        "the input is left alone with -o"
    );
    assert_ne!(std::fs::read_to_string(&copy).unwrap(), before);
    run(
        &["size", &doc, "--ruleset", "PIANC-B", "--vessel", "CEMT_IV"],
        "",
    );
    assert_ne!(
        std::fs::read_to_string(&doc).unwrap(),
        before,
        "no -o: rewritten in place"
    );
}

#[test]
fn wgs84_input_is_reprojected_and_the_zone_is_reported() {
    let dir = work_dir("wgs84");
    let line = |pts: &str| format!(r#"{{"type":"LineString","coordinates":{pts}}}"#);
    std::fs::write(
        dir.join("l.geojson"),
        line("[[-2.10,5.20],[-2.09,5.21],[-2.08,5.22]]"),
    )
    .unwrap();
    std::fs::write(
        dir.join("r.geojson"),
        line("[[-2.10,5.201],[-2.09,5.211],[-2.08,5.221]]"),
    )
    .unwrap();
    let doc = dir.join("d.cdesign").to_str().unwrap().to_string();
    run(
        &["new", &doc, "--title", "Ankobra", "--date", "2026-10-16"],
        "",
    );
    let o = run(
        &[
            "import",
            &doc,
            "--left-bank",
            dir.join("l.geojson").to_str().unwrap(),
            "--right-bank",
            dir.join("r.geojson").to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    let res: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(res["project_epsg"], 32630);
    assert_eq!(res["inputs"][0]["source_crs"], "EPSG:4326");
}

#[test]
fn exit_code_0_is_success() {
    let dir = work_dir("exit0");
    let doc = dir.join("x.cdesign").to_str().unwrap().to_string();
    assert_eq!(code(&run(&["new", &doc, "--title", "x"], "")), 0);
    assert_eq!(code(&run(&["status", &doc], "")), 0);
}

#[test]
fn exit_code_1_is_a_failed_check() {
    let dir = work_dir("exit1");
    let doc = build(&dir);
    let text = std::fs::read_to_string(&doc).unwrap().replacen(
        "\"format_version\": 1",
        "\"format_version\": 2",
        1,
    );
    std::fs::write(&doc, text).unwrap();
    // `status` on an invalid document is an input error; `validate` reports the failed check.
    let v = run(&["validate", &doc], "");
    assert_eq!(code(&v), 1);
    assert!(out(&v).contains("format_version"), "{}", out(&v));
    let vj = run(&["validate", &doc, "--json"], "");
    assert_eq!(code(&vj), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&out(&vj)).unwrap()["valid"],
        false
    );
}

#[test]
fn exit_code_2_is_a_usage_or_input_error() {
    let dir = work_dir("exit2");
    let doc = build(&dir);
    // A usage error: an unknown command, a missing required option.
    assert_eq!(code(&run(&["frobnicate"], "")), 2);
    assert_eq!(code(&run(&["size", &doc], "")), 2);
    // An input error: a missing file, text that is not JSON, an unknown vessel, an unusable CRS.
    assert_eq!(
        code(&run(
            &["status", dir.join("nope.cdesign").to_str().unwrap()],
            ""
        )),
        2
    );
    assert_eq!(code(&run(&["status", "-"], "not json")), 2);
    let o = run(
        &["size", &doc, "--ruleset", "PIANC-B", "--vessel", "NOPE"],
        "",
    );
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("unknown vessel standard"), "{}", err(&o));
    let lambert = dir.join("lambert.geojson");
    std::fs::write(
        &lambert,
        r#"{"type":"LineString","crs":{"type":"name","properties":{"name":"EPSG:2154"}},"coordinates":[[1,2],[3,4]]}"#,
    )
    .unwrap();
    let o = run(
        &["import", &doc, "--centreline", lambert.to_str().unwrap()],
        "",
    );
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("EPSG:2154") && err(&o).contains("WGS84"),
        "{}",
        err(&o)
    );
}
