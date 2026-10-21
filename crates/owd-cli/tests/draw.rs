//! `owd draw` (spec 0004, ticket 1) on the web example: the files it writes, `--document`, `--json`, the
//! refusals, and `owd draw run`, the runner of the standard's clause 17.3.
//!
//! Label: property

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd")
}

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

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-draw-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn draw_writes_five_files_per_sheet_and_the_documents_match_document_mode() {
    let dir = work_dir("files");
    let o = run(&[
        "draw",
        example().to_str().unwrap(),
        "--out",
        dir.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let res: Value = serde_json::from_str(&out(&o)).unwrap();
    let sheets = res["sheets"].as_array().unwrap();
    assert!(!sheets.is_empty());
    for s in sheets {
        let number = s["number"].as_str().unwrap();
        let stem = number.to_ascii_lowercase();
        for ext in ["drawing.json", "normalised.json", "svg", "pdf", "dxf"] {
            let f = dir.join(format!("{stem}.{ext}"));
            assert!(f.metadata().unwrap().len() > 0, "{}", f.display());
        }
        assert!(
            std::fs::read(dir.join(format!("{stem}.pdf")))
                .unwrap()
                .starts_with(b"%PDF")
        );
    }
    // `--document` prints the documents the files hold.
    let d = run(&["draw", example().to_str().unwrap(), "--document"]);
    assert_eq!(code(&d), 0, "{}", err(&d));
    let docs: Value = serde_json::from_str(&out(&d)).unwrap();
    for s in sheets {
        let number = s["number"].as_str().unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(
                dir.join(format!("{}.drawing.json", number.to_ascii_lowercase())),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(docs[number], written);
    }
}

#[test]
fn a_drawing_document_runs_through_the_drawing_crate() {
    let d = run(&["draw", example().to_str().unwrap(), "--document"]);
    let docs: Value = serde_json::from_str(&out(&d)).unwrap();
    let dir = work_dir("run");
    let (number, doc) = docs
        .as_object()
        .unwrap()
        .iter()
        .find(|(k, _)| k.contains("-XS-"))
        .unwrap();
    let path = dir.join(format!("{number}.json"));
    std::fs::write(&path, serde_json::to_string(doc).unwrap()).unwrap();
    let r = run(&["draw", "run", path.to_str().unwrap()]);
    assert_eq!(code(&r), 0, "{}", err(&r));
    let res: Value = serde_json::from_str(&out(&r)).unwrap();
    assert_eq!(res["result"], "ok");
    assert_eq!(res["normalised"]["sheet"]["size"], "A3");
}

#[test]
fn draw_needs_out_or_document_and_refuses_what_it_cannot_draw() {
    let ex = example();
    let ex = ex.to_str().unwrap();
    assert_eq!(code(&run(&["draw", ex])), 2);
    let o = run(&["draw", ex, "--document", "--option", "nope"]);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("no design line option `nope`"),
        "{}",
        err(&o)
    );
    let o = run(&["draw", ex, "--document", "--sheets", "zz"]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("unknown sheet kind `zz`"), "{}", err(&o));
    // A design with no results to draw from.
    let dir = work_dir("fresh");
    let doc = dir.join("new.owd");
    assert_eq!(
        code(&run(&[
            "new",
            doc.to_str().unwrap(),
            "--title",
            "T",
            "--date",
            "2026-10-21"
        ])),
        0
    );
    let o = run(&["draw", doc.to_str().unwrap(), "--document"]);
    assert_eq!(code(&o), 2);
}

#[test]
fn the_style_file_and_the_flags_set_the_sheets_and_a_bad_style_is_refused() {
    let ex = example();
    let ex = ex.to_str().unwrap();
    let notes = |args: &[&str]| -> String {
        let mut a = vec!["draw", ex, "--document"];
        a.extend_from_slice(args);
        let o = run(&a);
        assert_eq!(code(&o), 0, "{}", err(&o));
        let docs: Value = serde_json::from_str(&out(&o)).unwrap();
        let first = docs
            .as_object()
            .unwrap()
            .iter()
            .find(|(k, _)| k.contains("-XS-"))
            .unwrap()
            .1;
        first["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .find(|n| n.starts_with("SHEET STYLE"))
            .unwrap()
    };
    let base = notes(&[]);
    let finer = notes(&["--section-spacing", "250"]);
    assert_ne!(base, finer, "a flag changes the hash on the sheet");
    // The same single value in a file gives the same hash.
    let dir = work_dir("style");
    let file = dir.join("fine.style.json");
    std::fs::write(&file, r#"{"sections": {"spacing_m": 250}}"#).unwrap();
    assert_eq!(notes(&["--style", file.to_str().unwrap()]), finer);
    let bad = dir.join("bad.style.json");
    std::fs::write(&bad, r#"{"gallery": {"min_columns": 0}}"#).unwrap();
    let o = run(&["draw", ex, "--document", "--style", bad.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("/gallery/min_columns"), "{}", err(&o));
    let o = run(&["draw", ex, "--document", "--sheet-size", "A9"]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("/sheets/xs/size"), "{}", err(&o));
}

#[test]
fn sheets_picks_the_kinds_and_the_default_draws_both() {
    let ex = example();
    let ex = ex.to_str().unwrap();
    let kinds = |args: &[&str]| -> Vec<String> {
        let mut a = vec!["draw", ex, "--document"];
        a.extend_from_slice(args);
        let o = run(&a);
        assert_eq!(code(&o), 0, "{}", err(&o));
        let docs: Value = serde_json::from_str(&out(&o)).unwrap();
        docs.as_object()
            .unwrap()
            .keys()
            .map(|k| k.rsplit('-').nth(1).unwrap().to_string())
            .collect()
    };
    assert_eq!(kinds(&["--sheets", "ts"]), ["TS"]);
    assert!(kinds(&["--sheets", "xs"]).iter().all(|k| k == "XS"));
    let both = kinds(&[]);
    assert!(
        both.contains(&"XS".to_string()) && both.contains(&"TS".to_string()),
        "{both:?}"
    );
}

#[test]
fn imagery_offers_eox_cloudless_2016_only() {
    for name in [
        "eox-cloudless-2018",
        "eox-cloudless-2020",
        "eox-cloudless",
        "sentinel",
    ] {
        let o = run(&[
            "draw",
            example().to_str().unwrap(),
            "--sheets",
            "pl",
            "--imagery",
            name,
            "--document",
        ]);
        assert_eq!(code(&o), 2, "{name}: {}", err(&o));
        assert!(err(&o).contains("only eox-cloudless-2016"), "{}", err(&o));
        assert!(err(&o).contains("CC BY-NC-SA 4.0"), "{}", err(&o));
    }
}

#[test]
fn an_orthophoto_that_is_not_rgb_is_refused_with_its_reason() {
    // the example's DEM is a one-band GeoTIFF, not an orthophoto
    let dem = example().with_file_name("river-dem.tif");
    let o = run(&[
        "draw",
        example().to_str().unwrap(),
        "--sheets",
        "pl",
        "--orthophoto",
        dem.to_str().unwrap(),
        "--document",
    ]);
    assert_eq!(code(&o), 2, "{}", err(&o));
    assert!(err(&o).contains("1 band(s)"), "{}", err(&o));
}
