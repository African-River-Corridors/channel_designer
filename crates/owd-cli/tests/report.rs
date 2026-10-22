//! `owd report` (spec 0014, ticket 1): the screening report of the web example in three formats,
//! named after its document number; document control refused by ORD fails the command; a bad kind
//! or format is refused.
//!
//! Label: property

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

/// A copy of the web example and its DEM in a fresh folder.
fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-report-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    for f in ["river-example.owd", "river-dem.tif"] {
        std::fs::copy(ex.join(f), d.join(f)).unwrap();
    }
    d
}

#[test]
fn writes_the_screening_report_in_three_formats() {
    let d = work_dir("all");
    let doc = d.join("river-example.owd");
    let out = d.join("reports");
    let o = run(&[
        "report",
        doc.to_str().unwrap(),
        "--kind",
        "screening",
        "--format",
        "all",
        "--date",
        "2026-10-22",
        "-o",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let r = &v["reports"][0];
    let number = r["number"].as_str().unwrap();
    assert!(
        number.starts_with("OWD-") && number.ends_with("-SR"),
        "{number}"
    );
    let stem = number.to_lowercase();
    for ext in ["pdf", "html", "docx"] {
        let p = out.join(format!("{stem}.{ext}"));
        assert!(p.is_file(), "{} missing", p.display());
        assert!(std::fs::metadata(&p).unwrap().len() > 0);
    }
    assert!(
        std::fs::read(out.join(format!("{stem}.pdf")))
            .unwrap()
            .starts_with(b"%PDF")
    );
    assert!(
        std::fs::read(out.join(format!("{stem}.docx")))
            .unwrap()
            .starts_with(b"PK")
    );
}

#[test]
fn a_status_without_its_sign_offs_fails() {
    let d = work_dir("status");
    let doc = d.join("river-example.owd");
    let o = run(&[
        "report",
        doc.to_str().unwrap(),
        "--status",
        "for-approval",
        "--prepared",
        "Engineer",
        "--date",
        "2026-10-22",
        "-o",
        d.to_str().unwrap(),
    ]);
    assert_ne!(o.status.code(), Some(0));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("ORD004"), "{err}");
}

#[test]
fn a_bad_kind_or_format_is_refused() {
    let d = work_dir("bad");
    let doc = d.join("river-example.owd");
    for args in [["--kind", "nonsense"], ["--format", "rtf"]] {
        let mut all = vec!["report", doc.to_str().unwrap()];
        all.extend(args);
        let o = run(&all);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains(args[1]),
            "{args:?}"
        );
    }
}

#[test]
fn kind_all_and_format_all_write_twelve_files_for_both_examples() {
    let d = work_dir("kinds");
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    std::fs::copy(
        ex.join("synthetic-river.owd"),
        d.join("synthetic-river.owd"),
    )
    .unwrap();
    for (doc, out) in [
        ("river-example.owd", "web"),
        ("synthetic-river.owd", "synthetic"),
    ] {
        let doc = d.join(doc);
        let out = d.join(out);
        let o = run(&[
            "report",
            doc.to_str().unwrap(),
            "--kind",
            "all",
            "--format",
            "all",
            "--date",
            "2026-10-22",
            "-o",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        let kinds: Vec<&str> = v["reports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["screening", "quantities", "sections", "run"]);
        let mut files = 0;
        for r in v["reports"].as_array().unwrap() {
            let stem = r["number"].as_str().unwrap().to_lowercase();
            for ext in ["pdf", "html", "docx"] {
                assert!(out.join(format!("{stem}.{ext}")).is_file(), "{stem}.{ext}");
                files += 1;
            }
        }
        assert_eq!(files, 12);
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 12);
    }
}
