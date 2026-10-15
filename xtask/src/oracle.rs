//! Oracle guard: the parity oracle (Reference Results, case inputs, tolerance, ported list) may
//! change only through `cargo xtask regen`, and every changed case must carry a recorded reason.
//!
//! [`check`] compares the guarded files on the base branch with the files on the PR head. The
//! caller adds the second half of the proof: re-running the frozen Python must reproduce the
//! committed Reference Results byte for byte.

use std::collections::BTreeMap;

/// Guarded files, path (repo-relative, `/`-separated) to text.
pub type Tree = BTreeMap<String, String>;

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Some guarded file differs between base and head.
    pub changed: bool,
    pub violations: Vec<String>,
}

use serde_json::Value;

pub const CASES_DIR: &str = "parity/cases/";
pub const REFERENCE_DIR: &str = "parity/reference/";
pub const TOLERANCE: &str = "parity/tolerance.json";
pub const PORTED: &str = "parity/ported.json";

/// ADR 0005: every tolerance key, with the value nothing may go above.
pub const TOLERANCE_CEILINGS: [(&str, f64); 5] = [
    ("rel", 1e-9),
    ("abs", 1e-9),
    ("search_objective_rel", 1e-9),
    ("polygon_area_rel", 1e-9),
    ("polygon_vertex_m", 1e-6),
];

/// Is this repo path one the oracle guard watches?
pub fn is_guarded(path: &str) -> bool {
    path.starts_with(CASES_DIR)
        || path.starts_with(REFERENCE_DIR)
        || path == TOLERANCE
        || path == PORTED
}

/// Compare the guarded files on the base branch with those on the head.
pub fn check(base: &Tree, head: &Tree) -> Report {
    let mut report = Report {
        changed: base != head,
        violations: Vec::new(),
    };
    if !report.changed {
        return report;
    }
    let v = &mut report.violations;
    check_tolerance(base.get(TOLERANCE), head.get(TOLERANCE), v);
    check_ported(base.get(PORTED), head.get(PORTED), v);

    let mut entry_files: Vec<&String> = base
        .keys()
        .chain(head.keys())
        .filter(|p| p.starts_with(CASES_DIR) || p.starts_with(REFERENCE_DIR))
        .collect();
    entry_files.sort();
    entry_files.dedup();
    let mut names: Vec<String> = entry_files
        .iter()
        .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
        .collect();
    names.sort();
    names.dedup();
    for name in names {
        check_entry_point(&name, base, head, v);
    }
    report
}

fn parse(path: &str, text: Option<&String>, v: &mut Vec<String>) -> Option<Value> {
    let text = text?;
    match serde_json::from_str(text) {
        Ok(value) => Some(value),
        Err(e) => {
            v.push(format!("{path}: not valid JSON ({e})"));
            None
        }
    }
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn check_tolerance(base: Option<&String>, head: Option<&String>, v: &mut Vec<String>) {
    let Some(head_val) = parse(TOLERANCE, head, v) else {
        v.push(format!("{TOLERANCE}: missing"));
        return;
    };
    let base_val = parse(TOLERANCE, base, v);
    for (key, spec) in TOLERANCE_CEILINGS {
        let Some(new) = head_val.get(key).and_then(Value::as_f64) else {
            v.push(format!("{TOLERANCE}: `{key}` must be a number"));
            continue;
        };
        let ceiling = base_val
            .as_ref()
            .and_then(|b| b.get(key))
            .and_then(Value::as_f64)
            .map_or(spec, |old| old.min(spec));
        // NaN fails too: it is not `<=` anything.
        if new.is_nan() || new > ceiling {
            v.push(format!(
                "{TOLERANCE}: `{key}` = {new:e} would loosen the parity tolerance (ceiling {ceiling:e}); never loosen it"
            ));
        }
    }
    if let Some(base_val) = base_val {
        let numbers = |t: &Value| {
            TOLERANCE_CEILINGS
                .iter()
                .map(|(k, _)| t.get(*k).cloned())
                .collect::<Vec<_>>()
        };
        if numbers(&base_val) != numbers(&head_val)
            && str_field(&base_val, "reason") == str_field(&head_val, "reason")
        {
            v.push(format!(
                "{TOLERANCE}: the tolerance changed but its `reason` did not; record why"
            ));
        }
    }
}

fn check_ported(base: Option<&String>, head: Option<&String>, v: &mut Vec<String>) {
    let list = |val: Option<Value>| -> Vec<String> {
        val.and_then(|x| x.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e.as_str().map(str::to_string))
            .collect()
    };
    let base_list = list(parse(PORTED, base, v));
    let head_list = list(parse(PORTED, head, v));
    for ep in base_list.iter().filter(|ep| !head_list.contains(ep)) {
        v.push(format!(
            "{PORTED}: entry point `{ep}` left the ported list; a ported entry point must stay green"
        ));
    }
}

/// id -> (input, reason), in file order, plus the retired ids with their reasons.
struct Cases {
    cases: Vec<(String, Value, String)>,
    retired: BTreeMap<String, String>,
}

fn read_cases(path: &str, text: Option<&String>, v: &mut Vec<String>) -> Option<Cases> {
    let doc = parse(path, text, v)?;
    let mut cases = Vec::new();
    for c in doc
        .get("cases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        cases.push((
            str_field(c, "id").to_string(),
            c.get("input").cloned().unwrap_or(Value::Null),
            str_field(c, "reason").to_string(),
        ));
    }
    let retired = doc
        .get("retired")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| {
            (
                str_field(r, "id").to_string(),
                str_field(r, "reason").to_string(),
            )
        })
        .collect();
    Some(Cases { cases, retired })
}

/// id -> output, in file order.
fn read_reference(path: &str, text: Option<&String>, v: &mut Vec<String>) -> Vec<(String, Value)> {
    let Some(doc) = parse(path, text, v) else {
        return Vec::new();
    };
    doc.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| {
            (
                str_field(r, "id").to_string(),
                r.get("output").cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn check_entry_point(name: &str, base: &Tree, head: &Tree, v: &mut Vec<String>) {
    let cases_path = format!("{CASES_DIR}{name}");
    let ref_path = format!("{REFERENCE_DIR}{name}");
    let base_cases = read_cases(&cases_path, base.get(&cases_path), v);
    let Some(head_cases) = read_cases(&cases_path, head.get(&cases_path), v) else {
        if base_cases.is_some() || head.contains_key(&ref_path) {
            v.push(format!(
                "{cases_path}: missing; parity cases may not be deleted"
            ));
        }
        return;
    };
    let base_ref: BTreeMap<String, Value> = read_reference(&ref_path, base.get(&ref_path), v)
        .into_iter()
        .collect();
    let head_ref = read_reference(&ref_path, head.get(&ref_path), v);

    let case_ids: Vec<&String> = head_cases.cases.iter().map(|c| &c.0).collect();
    let ref_ids: Vec<&String> = head_ref.iter().map(|r| &r.0).collect();
    if case_ids != ref_ids {
        v.push(format!(
            "{ref_path}: out of step with {cases_path}; run `cargo xtask regen`"
        ));
    }
    let head_ref: BTreeMap<String, Value> = head_ref.into_iter().collect();

    let mut seen = std::collections::BTreeSet::new();
    let base_map: BTreeMap<&String, (&Value, &String)> = base_cases
        .iter()
        .flat_map(|c| c.cases.iter().map(|(id, i, r)| (id, (i, r))))
        .collect();
    for (id, input, reason) in &head_cases.cases {
        if id.is_empty() {
            v.push(format!("{cases_path}: a case has no `id`"));
            continue;
        }
        if !seen.insert(id) {
            v.push(format!("{cases_path}: duplicate case id `{id}`"));
        }
        match base_map.get(id) {
            None => {
                if reason.trim().is_empty() {
                    v.push(format!(
                        "{cases_path}: new case `{id}` has no recorded reason"
                    ));
                }
            }
            Some((old_input, old_reason)) => {
                let input_changed = *old_input != input;
                let output_changed = base_ref.get(id) != head_ref.get(id);
                if (input_changed || output_changed)
                    && (reason.trim().is_empty() || reason == *old_reason)
                {
                    let what = if input_changed {
                        "input"
                    } else {
                        "Reference Result"
                    };
                    v.push(format!(
                        "{cases_path}: case `{id}` changed its {what} without a new recorded reason"
                    ));
                }
            }
        }
    }
    for id in base_map.keys() {
        if !seen.contains(*id) {
            match head_cases.retired.get(*id) {
                Some(r) if !r.trim().is_empty() => {}
                _ => v.push(format!(
                    "{cases_path}: case `{id}` was removed without a `retired` entry and reason"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LNG_OK: &str = r#"{"ok": {"name": "LNG", "density": 450.0}}"#;

    fn cases(entry: &str, body: &str, retired: &str) -> String {
        format!(r#"{{"entry_point": "{entry}", "cases": [{body}], "retired": [{retired}]}}"#)
    }

    fn case(id: &str, arg: &str, reason: &str) -> String {
        format!(
            r#"{{"id": "{id}", "tier": 1, "reason": "{reason}", "input": {{"fn": "get_cargo", "args": {{"cargo_type": "{arg}"}}}}}}"#
        )
    }

    fn reference(entry: &str, results: &[(&str, &str)]) -> String {
        let body: Vec<String> = results
            .iter()
            .map(|(id, out)| format!(r#"{{"id": "{id}", "output": {out}}}"#))
            .collect();
        format!(
            r#"{{"entry_point": "{entry}", "results": [{}]}}"#,
            body.join(",")
        )
    }

    fn tolerance(rel: &str, abs: &str, reason: &str) -> String {
        format!(
            r#"{{"rel": {rel}, "abs": {abs}, "search_objective_rel": 1e-9, "polygon_area_rel": 1e-9, "polygon_vertex_m": 1e-6, "reason": "{reason}"}}"#
        )
    }

    /// A consistent oracle with one cargo case.
    fn tree() -> Tree {
        let mut t = Tree::new();
        t.insert(
            "parity/cases/cargo.json".into(),
            cases("cargo", &case("get_cargo/LNG", "LNG", "initial"), ""),
        );
        t.insert(
            "parity/reference/cargo.json".into(),
            reference("cargo", &[("get_cargo/LNG", LNG_OK)]),
        );
        t.insert(
            "parity/tolerance.json".into(),
            tolerance("1e-9", "1e-9", "spec"),
        );
        t.insert("parity/ported.json".into(), r#"["cargo"]"#.into());
        t
    }

    fn assert_violation(report: &Report, needle: &str) {
        assert!(
            report.violations.iter().any(|v| v.contains(needle)),
            "expected a violation mentioning {needle:?}, got {:?}",
            report.violations
        );
    }

    #[test]
    fn unchanged_oracle_passes_and_reports_no_change() {
        assert_eq!(check(&tree(), &tree()), Report::default());
    }

    #[test]
    fn reformatting_a_file_is_not_a_change_to_any_case() {
        let mut head = tree();
        let pretty = head["parity/cases/cargo.json"].replace(", ", ",\n  ");
        head.insert("parity/cases/cargo.json".into(), pretty);
        let report = check(&tree(), &head);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }

    #[test]
    fn first_introduction_of_an_oracle_with_reasons_passes() {
        let report = check(&Tree::new(), &tree());
        assert!(report.changed);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }

    #[test]
    fn hand_edited_reference_result_without_a_new_reason_fails() {
        let mut head = tree();
        head.insert(
            "parity/reference/cargo.json".into(),
            reference(
                "cargo",
                &[(
                    "get_cargo/LNG",
                    r#"{"ok": {"name": "LNG", "density": 451.0}}"#,
                )],
            ),
        );
        let report = check(&tree(), &head);
        assert_violation(&report, "get_cargo/LNG");
    }

    #[test]
    fn changed_case_input_needs_a_new_reason() {
        let mut head = tree();
        head.insert(
            "parity/cases/cargo.json".into(),
            cases("cargo", &case("get_cargo/LNG", "LPG_mix", "initial"), ""),
        );
        assert_violation(&check(&tree(), &head), "get_cargo/LNG");

        head.insert(
            "parity/cases/cargo.json".into(),
            cases(
                "cargo",
                &case("get_cargo/LNG", "LPG_mix", "nudged off a knife edge"),
                "",
            ),
        );
        let report = check(&tree(), &head);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }

    #[test]
    fn new_case_with_empty_reason_fails() {
        let mut head = tree();
        let two = format!(
            "{},{}",
            case("get_cargo/LNG", "LNG", "initial"),
            case("get_cargo/bauxite", "bauxite", "")
        );
        head.insert("parity/cases/cargo.json".into(), cases("cargo", &two, ""));
        head.insert(
            "parity/reference/cargo.json".into(),
            reference(
                "cargo",
                &[("get_cargo/LNG", LNG_OK), ("get_cargo/bauxite", LNG_OK)],
            ),
        );
        assert_violation(&check(&tree(), &head), "get_cargo/bauxite");
    }

    #[test]
    fn removed_case_must_be_retired_with_a_reason() {
        let empty_ref = reference("cargo", &[]);
        let mut head = tree();
        head.insert("parity/cases/cargo.json".into(), cases("cargo", "", ""));
        head.insert("parity/reference/cargo.json".into(), empty_ref.clone());
        assert_violation(&check(&tree(), &head), "get_cargo/LNG");

        head.insert(
            "parity/cases/cargo.json".into(),
            cases(
                "cargo",
                "",
                r#"{"id": "get_cargo/LNG", "reason": "duplicate of a sweep case"}"#,
            ),
        );
        let report = check(&tree(), &head);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }

    #[test]
    fn loosened_tolerance_always_fails() {
        let mut head = tree();
        head.insert(
            "parity/tolerance.json".into(),
            tolerance("1e-6", "1e-9", "a new reason"),
        );
        assert_violation(&check(&tree(), &head), "loosen");
    }

    #[test]
    fn tolerance_above_the_spec_value_fails_even_on_first_introduction() {
        let mut head = tree();
        head.insert(
            "parity/tolerance.json".into(),
            tolerance("1e-9", "1e-8", "spec"),
        );
        assert_violation(&check(&Tree::new(), &head), "loosen");
    }

    #[test]
    fn every_tolerance_key_is_guarded_not_only_rel_and_abs() {
        for key in [
            "search_objective_rel",
            "polygon_area_rel",
            "polygon_vertex_m",
        ] {
            let mut head = tree();
            let loose = head["parity/tolerance.json"]
                .replace(&format!(r#""{key}": 1e-9"#), &format!(r#""{key}": 1e-3"#));
            let loose = loose.replace(&format!(r#""{key}": 1e-6"#), &format!(r#""{key}": 1e-3"#));
            head.insert("parity/tolerance.json".into(), loose);
            assert_violation(&check(&tree(), &head), key);
        }
    }

    #[test]
    fn a_missing_tolerance_key_fails() {
        let mut head = tree();
        head.insert(
            "parity/tolerance.json".into(),
            r#"{"rel": 1e-9, "abs": 1e-9, "reason": "spec"}"#.into(),
        );
        assert_violation(&check(&tree(), &head), "polygon_vertex_m");
    }

    #[test]
    fn tightened_tolerance_needs_a_new_reason() {
        let mut head = tree();
        head.insert(
            "parity/tolerance.json".into(),
            tolerance("1e-10", "1e-9", "spec"),
        );
        assert_violation(&check(&tree(), &head), "tolerance");

        head.insert(
            "parity/tolerance.json".into(),
            tolerance("1e-10", "1e-9", "tightened after the port"),
        );
        let report = check(&tree(), &head);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }

    #[test]
    fn entry_point_leaving_the_ported_list_fails() {
        let mut head = tree();
        head.insert("parity/ported.json".into(), "[]".into());
        assert_violation(&check(&tree(), &head), "cargo");
    }

    #[test]
    fn reference_out_of_step_with_cases_fails() {
        let mut head = tree();
        head.insert(
            "parity/reference/cargo.json".into(),
            reference("cargo", &[("get_cargo/other", LNG_OK)]),
        );
        assert_violation(&check(&tree(), &head), "regen");
    }

    #[test]
    fn deleting_a_cases_file_fails() {
        let mut head = tree();
        head.remove("parity/cases/cargo.json");
        head.remove("parity/reference/cargo.json");
        assert_violation(&check(&tree(), &head), "cargo");
    }

    #[test]
    fn duplicate_case_ids_fail() {
        let mut head = tree();
        let two = format!(
            "{},{}",
            case("get_cargo/LNG", "LNG", "initial"),
            case("get_cargo/LNG", "LNG", "initial")
        );
        head.insert("parity/cases/cargo.json".into(), cases("cargo", &two, ""));
        head.insert(
            "parity/reference/cargo.json".into(),
            reference(
                "cargo",
                &[("get_cargo/LNG", LNG_OK), ("get_cargo/LNG", LNG_OK)],
            ),
        );
        assert_violation(&check(&tree(), &head), "duplicate");
    }
}
