//! Appendix A of the screening report, the Rule Trace (spec 0007 E, spec 0014): rebuilt by
//! `Session::rule_trace`, as `owd trace` rebuilds it. The channel and reach rows in full; for the
//! report's option, every station row that does not pass, and the count of passing rows per Rule.

use crate::Context;
use crate::fmt::{md, text, val};
use owd_format::design::hash::hash_value;
use serde_json::{Value, json};

/// The station spacing of the trace, as `owd trace --option` reads it by default.
pub const STATION_M: f64 = 10.0;

fn row(r: &Value) -> Vec<String> {
    let limit = match (r["compare"].as_str(), r["limit"].as_f64()) {
        (Some(c), Some(_)) => format!("{c} {}", val(&r["limit"], 2, "")),
        _ => "—".into(),
    };
    vec![
        text(&r["rule"], "?").to_string(),
        text(&r["object"], "channel").to_string(),
        val(&r["value"], 2, "—"),
        limit,
        val(&r["margin"], 2, "—"),
        text(&r["outcome"], "?").to_string(),
        if r["governing"].as_bool() == Some(true) {
            "yes".into()
        } else {
            "no".into()
        },
        text(&r["status"], "?").to_string(),
    ]
}

const COLUMNS: [(&str, &str); 8] = [
    ("Rule", "left"),
    ("Object", "left"),
    ("Value", "right"),
    ("Limit", "right"),
    ("Margin", "right"),
    ("Outcome", "left"),
    ("Governing", "left"),
    ("Status", "left"),
];

fn rows_table(id: &str, caption: &str, rows: Vec<Vec<String>>) -> Value {
    json!({
        "type": "table", "id": id, "caption": caption,
        "columns": COLUMNS.iter().map(|(t, a)| json!({"title": t, "align": a})).collect::<Vec<_>>(),
        "rows": rows,
    })
}

fn prose(m: String) -> Value {
    json!({"type": "prose", "markdown": m})
}

/// The appendix, or `None` when the design has no sizing to trace.
pub fn appendix(ctx: &Context) -> Option<Value> {
    let channel = match ctx.session.rule_trace(None, STATION_M) {
        Ok(t) => t,
        Err(e) => {
            return Some(json!({
                "id": "rule-trace", "title": "Rule trace",
                "body": [prose(format!("The rule trace cannot be rebuilt: {}.", md(&e.to_string())))],
            }));
        }
    };
    let rows = |t: &Value| t["rows"].as_array().cloned().unwrap_or_default();
    let mut body = vec![prose(format!(
        "Every Rule of every selected Rule Set, at each object it applies to: its value, limit, \
         margin and outcome, whether it governs, and its status. {} The trace is rebuilt from the \
         design and its stored Rule Set hashes, as `owd trace` rebuilds it.",
        md(text(&channel["rules_notice"], ""))
    ))];
    body.push(json!({"type": "section", "id": "trace-channel", "title": "The channel and its reaches", "body": [
        prose(format!("SHA-256 of this trace: `{}`.", hash_value(&channel))),
        rows_table("trace-channel-rows", "The Rule Trace of the channel and its reaches",
            rows(&channel).iter().map(row).collect()),
    ]}));
    if let Some(id) = ctx.option.as_ref().and_then(|o| o["id"].as_str()) {
        let name = ctx.option.as_ref().map_or(id, |o| text(&o["name"], id));
        match ctx.session.rule_trace(Some(id), STATION_M) {
            Ok(t) => {
                let all = rows(&t);
                let not_pass: Vec<Vec<String>> = all
                    .iter()
                    .filter(|r| r["outcome"] != "pass")
                    .map(row)
                    .collect();
                let mut passes: Vec<(String, usize)> = Vec::new();
                for r in all.iter().filter(|r| r["outcome"] == "pass") {
                    let rule = text(&r["rule"], "?").to_string();
                    match passes.iter_mut().find(|(n, _)| *n == rule) {
                        Some(e) => e.1 += 1,
                        None => passes.push((rule, 1)),
                    }
                }
                let mut sec = vec![prose(format!(
                    "The station Rules at every {} m along **{}**: {} rows, of which {} do not pass. \
                     SHA-256 of this trace: `{}`. `owd trace --option {} --json` gives every row.",
                    STATION_M,
                    md(name),
                    all.len(),
                    not_pass.len(),
                    hash_value(&t),
                    md(id),
                ))];
                if !not_pass.is_empty() {
                    sec.push(rows_table(
                        "trace-option-rows",
                        "The station rows that do not pass",
                        not_pass,
                    ));
                }
                if !passes.is_empty() {
                    sec.push(json!({
                        "type": "table", "id": "trace-option-passes",
                        "caption": "The passing station rows, counted per Rule",
                        "columns": [{"title": "Rule", "align": "left"}, {"title": "Passing rows", "align": "right"}],
                        "rows": passes.iter().map(|(r, n)| vec![r.clone(), n.to_string()]).collect::<Vec<_>>(),
                    }));
                }
                body.push(json!({"type": "section", "id": "trace-option",
                    "title": format!("The stations of the option {name}"), "body": sec}));
            }
            Err(e) => body.push(prose(format!(
                "The station trace of the option cannot be rebuilt: {}.",
                md(&e.to_string())
            ))),
        }
    }
    Some(json!({"id": "rule-trace", "title": "Rule trace", "body": body}))
}
