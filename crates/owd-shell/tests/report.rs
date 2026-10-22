//! The app's Report button (spec 0014, ticket 6) through the shell: each report has the bytes
//! `owd report` writes for the same design, option, kind, format and document control.
//!
//! Label: property

use owd_shell::report::ReportAsk;
use owd_shell::state::AppState;
use serde_json::json;
use std::path::{Path, PathBuf};

const DATE: &str = "2026-10-22";

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd")
}

fn opened() -> AppState {
    let text = std::fs::read_to_string(example()).unwrap();
    let dem = std::fs::read(example().with_file_name("river-dem.tif")).unwrap();
    AppState::open(&example(), &text, Some(&dem)).expect("the example opens with its DEM")
}

fn ask(kind: &str, format: &str) -> ReportAsk {
    ReportAsk::from_json(
        &json!({"kind": kind, "format": format, "revision": "B", "status": "for-checking",
                "prepared": "Engineer", "date": DATE}),
        "test (shell)",
    )
    .unwrap()
}

#[test]
fn the_reports_are_byte_identical_to_owd_report() {
    let state = opened();
    let files = state
        .report_job(Some("rolling-circle"), ask("all", "all"))
        .files()
        .unwrap();
    assert_eq!(files.len(), 12);
    let dir = std::env::temp_dir().join(format!("owd-report-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // `owd report FILE --kind all --format all ...`, through the command itself
    owd_cli::report::report(owd_cli::report::ReportFlags {
        doc: example().to_string_lossy().into_owned(),
        kind: "all".into(),
        format: "all".into(),
        option: Some("rolling-circle".into()),
        out: Some(dir.to_string_lossy().into_owned()),
        revision: Some("B".into()),
        date: Some(DATE.into()),
        status: Some("for-checking".into()),
        prepared: Some("Engineer".into()),
        checked: None,
        approved: None,
        organisation: None,
        json: false,
    })
    .map_err(|f| f.message)
    .unwrap();
    for (name, bytes) in &files {
        let cli = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        if name.contains("-rr.") {
            // The run report names what asked for it: the app, or the command line. All else is equal.
            assert!(cli != *bytes, "{name}: the request line should differ");
        } else {
            assert!(cli == *bytes, "{name} differs from owd report's");
        }
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), files.len());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_run_report_is_the_report_crates() {
    let state = opened();
    let files = state
        .report_job(Some("rolling-circle"), ask("run", "html"))
        .files()
        .unwrap();
    let text = std::fs::read_to_string(example()).unwrap();
    let dem = std::fs::read(example().with_file_name("river-dem.tif")).unwrap();
    let mut control = owd_report::Control::draft(DATE);
    control.revision = "B".into();
    control.status = "for-checking".into();
    control.prepared = Some("Engineer".into());
    let req = owd_report::Request {
        kind: owd_report::Kind::Run,
        option: Some("rolling-circle".into()),
        control,
        requested_by: Some("test (shell)".into()),
    };
    let want = owd_report::build(&text, Some(&dem), &req)
        .unwrap()
        .render(owd_report::Format::Html)
        .unwrap()
        .bytes;
    assert_eq!(files.len(), 1);
    assert!(files[0].1 == want);
}

#[test]
fn a_bad_ask_is_refused() {
    let state = opened();
    let e = state
        .report_job(None, ask("nonsense", "pdf"))
        .files()
        .unwrap_err();
    assert!(e.contains("nonsense"), "{e}");
    assert!(ReportAsk::from_json(&json!({"kind": "screening"}), "x").is_err());
}
