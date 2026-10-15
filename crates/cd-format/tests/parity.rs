//! Seam 1, the parity contract: every case of every ported entry point, run in-process through
//! `cd_format::eval`, must match its Reference Result from the frozen Python.
//!
//! Needs no Python: it reads the committed files in `parity/` only.

use cd_format::parity::{Tolerance, compare};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ported() -> Vec<String> {
    let v = read_json(&parity_dir().join("ported.json"));
    v.as_array()
        .expect("ported.json is a list")
        .iter()
        .map(|e| {
            e.as_str()
                .expect("entry point names are strings")
                .to_string()
        })
        .collect()
}

fn tolerance() -> Tolerance {
    let text =
        std::fs::read_to_string(parity_dir().join("tolerance.json")).expect("tolerance.json");
    Tolerance::from_json(&text).expect("tolerance.json parses")
}

/// (case id, case input, Reference Result) for one entry point, in file order.
fn cases(entry_point: &str) -> Vec<(String, Value, Value)> {
    let cases = read_json(
        &parity_dir()
            .join("cases")
            .join(format!("{entry_point}.json")),
    );
    let reference = read_json(
        &parity_dir()
            .join("reference")
            .join(format!("{entry_point}.json")),
    );
    let cases = cases["cases"].as_array().expect("cases list");
    let results = reference["results"].as_array().expect("results list");
    assert_eq!(
        cases.len(),
        results.len(),
        "{entry_point}: run `cargo xtask regen`"
    );
    cases
        .iter()
        .zip(results)
        .map(|(c, r)| {
            assert_eq!(c["id"], r["id"], "{entry_point}: run `cargo xtask regen`");
            (
                c["id"].as_str().expect("case id").to_string(),
                c["input"].clone(),
                r["output"].clone(),
            )
        })
        .collect()
}

fn all_entry_points() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(parity_dir().join("cases"))
        .expect("parity/cases exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn every_case_of_every_ported_entry_point_matches_its_reference_result() {
    let tol = tolerance();
    let mut failures = Vec::new();
    let mut checked = 0;
    for ep in ported() {
        for (id, input, expected) in cases(&ep) {
            checked += 1;
            match cd_format::eval(&ep, &input) {
                Ok(actual) => {
                    if let Err(diffs) = compare(&expected, &actual, tol) {
                        failures.push(format!("{ep} {id}:\n    {}", diffs.join("\n    ")));
                    }
                }
                Err(e) => failures.push(format!("{ep} {id}: {e}")),
            }
        }
    }
    assert!(
        checked > 0,
        "no parity cases ran; the harness must never pass vacuously"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} parity case(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_ported_entry_point_has_cases() {
    let ported = ported();
    assert!(!ported.is_empty(), "ported.json lists no entry point");
    for ep in ported {
        assert!(
            !cases(&ep).is_empty(),
            "{ep} is ported but has no parity cases"
        );
    }
}

#[test]
fn entry_points_not_yet_ported_say_so_and_are_not_half_ported() {
    let ported = ported();
    for ep in all_entry_points()
        .into_iter()
        .filter(|ep| !ported.contains(ep))
    {
        for (id, input, _) in cases(&ep) {
            assert_eq!(
                cd_format::eval(&ep, &input),
                Err(cd_format::EvalError::NotPorted(ep.clone())),
                "{ep} {id}: the dispatch handles it, so add `{ep}` to parity/ported.json"
            );
        }
    }
}
