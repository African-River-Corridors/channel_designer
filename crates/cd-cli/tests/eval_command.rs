//! Seam 2: the `cdesign` binary's hidden `eval` command, on a few parity cases per ported entry
//! point. Proves the binary wires stdin -> `cd_format::eval` -> stdout. The full case set runs
//! in-process in `cd-format/tests/parity.rs`.

use cd_format::parity::{Tolerance, compare};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Cases per ported entry point to run through the binary.
const CASES_PER_ENTRY_POINT: usize = 3;

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("parity file")).expect("JSON")
}

fn cdesign(args: &[&str], stdin: &str) -> Output {
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

/// First, last and one error case (if any) of an entry point: (id, input, Reference Result).
fn sample(entry_point: &str) -> Vec<(String, Value, Value)> {
    let cases = read_json(&parity_dir().join(format!("cases/{entry_point}.json")));
    let refs = read_json(&parity_dir().join(format!("reference/{entry_point}.json")));
    let all: Vec<(String, Value, Value)> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(refs["results"].as_array().unwrap())
        .map(|(c, r)| {
            (
                c["id"].as_str().unwrap().into(),
                c["input"].clone(),
                r["output"].clone(),
            )
        })
        .collect();
    let mut picks: Vec<usize> = vec![0, all.len() - 1];
    if let Some(i) = all
        .iter()
        .position(|(_, _, out)| out.get("error").is_some())
    {
        picks.push(i);
    }
    picks.sort();
    picks.dedup();
    picks.truncate(CASES_PER_ENTRY_POINT);
    picks.into_iter().map(|i| all[i].clone()).collect()
}

#[test]
fn eval_command_matches_reference_results_for_sampled_cases() {
    let tol = Tolerance::from_json(
        &std::fs::read_to_string(parity_dir().join("tolerance.json")).unwrap(),
    )
    .unwrap();
    let ported = read_json(&parity_dir().join("ported.json"));
    let mut ran = 0;
    for ep in ported
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap())
    {
        for (id, input, expected) in sample(ep) {
            let out = cdesign(&["eval", ep], &input.to_string());
            assert!(
                out.status.success(),
                "{ep} {id}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let actual: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
            if let Err(diffs) = compare(&expected, &actual, tol) {
                panic!("{ep} {id}:\n{}", diffs.join("\n"));
            }
            ran += 1;
        }
    }
    assert!(ran > 0, "no case ran through the binary");
}

#[test]
fn eval_command_fails_loudly_on_an_unknown_entry_point() {
    let out = cdesign(&["eval", "no_such_entry_point"], r#"{"fn": "x"}"#);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown entry point `no_such_entry_point`")
    );
}

#[test]
fn eval_command_fails_loudly_on_input_that_is_not_json() {
    let out = cdesign(&["eval", "cut_model"], "not json");
    assert!(!out.status.success());
    assert!(!out.stderr.is_empty());
}

#[test]
fn eval_is_hidden_from_help() {
    let out = cdesign(&["--help"], "");
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("cdesign"), "{help}");
    assert!(!help.contains("eval"), "{help}");
}
