//! Seam 1, the parity contract: every case of every ported entry point, run in-process through
//! `cd_format::eval`, must match its Reference Result from the frozen Python.
//!
//! Needs no Python: it reads the committed files in `parity/` only.

use cd_format::parity::Tolerance;
use cd_format::search::{Judged, judge};
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

/// A case of one entry point: id, input, Reference Result, and the `search` record a search case
/// carries (its near-tie flag).
struct Case {
    id: String,
    input: Value,
    expected: Value,
    search: Option<Value>,
}

/// The cases of one entry point, in file order.
fn cases(entry_point: &str) -> Vec<Case> {
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
            Case {
                id: c["id"].as_str().expect("case id").to_string(),
                input: c["input"].clone(),
                expected: r["output"].clone(),
                search: c.get("search").cloned(),
            }
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
    let mut near_ties = Vec::new();
    let mut checked = 0;
    for ep in ported() {
        for Case {
            id,
            input,
            expected,
            search,
        } in cases(&ep)
        {
            checked += 1;
            match cd_format::eval(&ep, &input) {
                Ok(actual) => match judge(&ep, &input, search.as_ref(), &expected, &actual, tol) {
                    Ok(Judged::NearTie) => near_ties.push(format!("{ep} {id}")),
                    Ok(Judged::Exact) => {}
                    Err(diffs) => {
                        failures.push(format!("{ep} {id}:\n    {}", diffs.join("\n    ")))
                    }
                },
                Err(e) => failures.push(format!("{ep} {id}: {e}")),
            }
        }
    }
    // A near tie is reported as one, never counted as a failure (ADR 0005).
    eprintln!(
        "search class: {} near-tie case(s) judged on objective and constraints{}{}",
        near_ties.len(),
        if near_ties.is_empty() { "" } else { ": " },
        near_ties.join(", ")
    );
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
        for case in cases(&ep) {
            let id = &case.id;
            assert_eq!(
                cd_format::eval(&ep, &case.input),
                Err(cd_format::EvalError::NotPorted(ep.clone())),
                "{ep} {id}: the dispatch handles it, so add `{ep}` to parity/ported.json"
            );
        }
    }
}

/// Issues #9 and #10 (ticket 28): on every rolling-circle case, near tie or not, `radius_ok`, the
/// measured minimum radius and the chord match the Reference Result. This test does not use the
/// near-tie rule, so a tie can never excuse a wrong radius.
#[test]
fn radius_ok_measured_radius_and_chord_match_on_every_rolling_circle_case() {
    let tol = tolerance();
    let mut checked = 0;
    for case in cases("rolling_circle") {
        let Some(want) = case.expected["ok"].get("result") else {
            continue;
        };
        let got = cd_format::eval("rolling_circle", &case.input).expect("eval");
        for field in ["radius_ok", "measured_min_radius_m", "radius_check_chord_m"] {
            let diffs = cd_format::parity::compare(&want[field], &got["ok"]["result"][field], tol);
            assert!(diffs.is_ok(), "{} {field}: {diffs:?}", case.id);
        }
        checked += 1;
    }
    assert!(
        checked >= 10,
        "only {checked} rolling_circle results were checked"
    );
}
