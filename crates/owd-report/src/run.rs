//! The run report (spec 0014, report 4), for testers and developers: what asked for it, the
//! versions that made the result, each stage with its status, settings and dependency hash, each
//! input with its hash, and the hashes of the results (the document, the rule trace, the sheets).

use crate::fmt::text;
use crate::{Context, ORD_VERSION};
use owd_format::design::hash::{hash_value, sha256_hex};
use owd_format::design::session::ENGINE_VERSION;
use owd_format::design::{DemCheck, StageStatus};
use owd_format::drawing::{self, DrawOptions};
use serde_json::{Value, json};

/// The git tags the workspace pins (see `build.rs`).
pub const PIN_ORD: &str = env!("OWD_PIN_ORD");
pub const PIN_OTD: &str = env!("OWD_PIN_OTD");
pub const PIN_ARC_RULES: &str = env!("OWD_PIN_ARC_RULES");

fn section(id: &str, title: &str, body: Vec<Value>) -> Value {
    json!({"type": "section", "id": id, "title": title, "body": body})
}

fn prose(m: String) -> Value {
    json!({"type": "prose", "markdown": m})
}

/// A table; every cell that is a SHA-256 is printed in groups (see [`grouped_hash`]).
fn table(id: &str, caption: &str, columns: &[&str], rows: Vec<Vec<String>>) -> Value {
    let rows: Vec<Vec<String>> = rows
        .into_iter()
        .map(|r| r.iter().map(|c| grouped_hash(c)).collect())
        .collect();
    json!({
        "type": "table", "id": id, "caption": caption,
        "columns": columns.iter().map(|t| json!({"title": t, "align": "left"})).collect::<Vec<_>>(),
        "rows": rows,
    })
}

/// A one-line JSON of a value with a space after each `,` and `:` outside strings, so a table cell
/// can wrap it; or `—`.
fn compact(v: &Value) -> String {
    if v.is_null() {
        return "—".into();
    }
    let mut out = String::new();
    let (mut in_str, mut escaped) = (false, false);
    for c in short_hashes(v).to_string().chars() {
        out.push(c);
        if in_str {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_str = false,
                _ => {}
            }
        } else if c == '"' {
            in_str = true;
        } else if c == ',' || c == ':' {
            out.push(' ');
        }
    }
    out
}

/// `v` with every string that is a SHA-256 cut to its first 16 hex digits and "…". The settings
/// name Rule Set hashes, which the Versions table prints in full.
fn short_hashes(v: &Value) -> Value {
    match v {
        Value::String(h) if is_sha256(h) => Value::String(format!("{}…", &h[..16])),
        Value::Array(a) => Value::Array(a.iter().map(short_hashes).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), short_hashes(x)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn is_sha256(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A SHA-256 in four groups of 16 hex digits, so a table cell can wrap it. Any other text as it is.
pub fn grouped_hash(h: &str) -> String {
    if is_sha256(h) {
        (0..4)
            .map(|i| &h[i * 16..i * 16 + 16])
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        h.to_string()
    }
}

pub fn report(ctx: &Context, requested_by: Option<&str>) -> (Vec<Value>, Vec<Value>) {
    let d = ctx.doc;
    let mut body = vec![
        json!({"type": "section", "id": "data", "title": "Data and limits",
        "numbered": false, "body": [crate::limits::data_box(ctx)]}),
    ];

    body.push(section(
        "request",
        "Request",
        vec![prose(format!(
            "Asked for by: `{}`.",
            requested_by.unwrap_or("not recorded").replace('`', "'")
        ))],
    ));

    // Versions.
    let mut versions = vec![
        vec!["Open Waterway Designer".into(), ENGINE_VERSION.into()],
        vec![
            "Open Report Designer (ord)".into(),
            format!("{ORD_VERSION} (tag {PIN_ORD})"),
        ],
        vec![
            "Open Technical Drawer (otd)".into(),
            format!("tag {PIN_OTD}"),
        ],
        vec![
            "Rules engine (arc-rules)".into(),
            format!("tag {PIN_ARC_RULES}"),
        ],
        vec![
            "Design file format_version".into(),
            compact(&d["format_version"]),
        ],
        vec![
            "Design file engine_version".into(),
            text(&d["engine_version"], "not recorded").to_string(),
        ],
    ];
    for r in d["stages"]["sizing"]["settings"]["rule_sets"]
        .as_array()
        .into_iter()
        .flatten()
    {
        versions.push(vec![
            format!("Rule Set {}", text(&r["id"], "?")),
            format!(
                "{}, SHA-256 {}",
                text(&r["version"], "?"),
                grouped_hash(text(&r["hash"], "?"))
            ),
        ]);
    }
    body.push(section(
        "versions",
        "Versions",
        vec![table(
            "versions-table",
            "The versions that made this result",
            &["Part", "Version"],
            versions,
        )],
    ));

    // Stages.
    let status = ctx.session.status_with_dem(ctx.dem);
    let mut stages: Vec<Vec<String>> = status
        .stages
        .iter()
        .map(|(s, x)| {
            let st = &d["stages"][s.name()];
            vec![
                s.name().to_string(),
                x.name().to_string(),
                text(&st["engine_version"], "—").to_string(),
                grouped_hash(text(&st["depends_on"], "—")),
                compact(&st["settings"]),
            ]
        })
        .collect();
    if d["fits"].is_object() {
        stages.push(vec![
            "fits".into(),
            match ctx.session.fits_status() {
                StageStatus::Fresh => "fresh",
                StageStatus::Stale => "stale",
                StageStatus::NotRun => "not run",
            }
            .into(),
            text(&d["fits"]["engine_version"], "—").to_string(),
            grouped_hash(text(&d["fits"]["depends_on"], "—")),
            compact(&d["fits"]["settings"]),
        ]);
    }
    body.push(section(
        "stages",
        "Stages",
        vec![table(
            "stages-table",
            "Each stage: status, the engine that ran it, its dependency hash and its settings",
            &[
                "Stage",
                "Status",
                "Engine",
                "Depends on (SHA-256)",
                "Settings",
            ],
            stages,
        )],
    ));

    // Inputs.
    let mut inputs = Vec::new();
    if let Some(map) = d["inputs"].as_object() {
        for (name, v) in map {
            if v.is_null() {
                continue;
            }
            let source = ctx
                .session
                .provenance(name)
                .and_then(|p| p["source"].as_str())
                .unwrap_or("the user's file");
            inputs.push(vec![
                name.clone(),
                source.to_string(),
                grouped_hash(&hash_value(v)),
            ]);
        }
    }
    let mut body_inputs = vec![
        prose(
            "The SHA-256 of each input is of its canonical JSON in the design file (sorted keys), \
             as the stages hash it."
                .into(),
        ),
        table(
            "inputs-table",
            "The inputs and their hashes",
            &["Input", "Source", "SHA-256"],
            inputs,
        ),
    ];
    if let Some(stored) = d["inputs"]["dem"]["sha256"].as_str() {
        let given = ctx.dem.map(sha256_hex);
        let verdict = match ctx.session.dem_check(ctx.dem) {
            DemCheck::Matches => "they match",
            DemCheck::Changed => "they differ: the DEM changed since the volumes ran",
            DemCheck::Missing => "no DEM was given",
            DemCheck::NoDem => "the design has no DEM",
        };
        body_inputs.push(table(
            "dem-table",
            "The DEM file",
            &["Hash", "SHA-256"],
            vec![
                vec!["Recorded in the design".into(), stored.into()],
                vec![
                    "Of the bytes given".into(),
                    given.unwrap_or_else(|| "none given".into()),
                ],
                vec!["Result".into(), verdict.into()],
            ],
        ));
    }
    body.push(section("inputs", "Inputs", body_inputs));

    // Results.
    let mut results = vec![vec![
        "The design document".into(),
        grouped_hash(&hash_value(d)),
    ]];
    let push = |results: &mut Vec<Vec<String>>, what: String, h: String| {
        results.push(vec![what, grouped_hash(&h)])
    };
    match ctx.session.rule_trace(None, crate::trace::STATION_M) {
        Ok(t) => results.push(vec![
            "Rule trace, channel and reaches".into(),
            hash_value(&t),
        ]),
        Err(e) => results.push(vec![
            "Rule trace, channel and reaches".into(),
            format!("not rebuilt: {e}"),
        ]),
    }
    let option = ctx.option.as_ref().and_then(|o| o["id"].as_str());
    if let Some(id) = option {
        match ctx.session.rule_trace(Some(id), crate::trace::STATION_M) {
            Ok(t) => push(
                &mut results,
                format!("Rule trace, option {id}"),
                hash_value(&t),
            ),
            Err(e) => results.push(vec![
                format!("Rule trace, option {id}"),
                format!("not rebuilt: {e}"),
            ]),
        }
    }
    let opts = DrawOptions {
        option: option.map(String::from),
        ..DrawOptions::default()
    };
    match drawing::sheets_from_text(&ctx.session.to_text(), ctx.dem, &opts) {
        Ok(sheets) => {
            for s in &sheets {
                match drawing::render(s) {
                    Ok(o) => push(
                        &mut results,
                        format!("Sheet {}", o.number),
                        sha256_hex(o.normalised.as_bytes()),
                    ),
                    Err(e) => results.push(vec![
                        format!("Sheet {}", s.number),
                        format!("not drawn: {e}"),
                    ]),
                }
            }
        }
        Err(e) => results.push(vec!["Sheets".into(), format!("not drawn: {e}")]),
    }
    body.push(section(
        "results",
        "Hashes of the results",
        vec![
            prose(
                "Each SHA-256 is printed in four groups of 16 hex digits. The document hash is of \
                 its canonical JSON. Each trace hash is of the trace `owd \
                 trace --json` prints. Each sheet hash is of the sheet's normalised content, the \
                 file `owd draw` writes as `<number>.normalised.json`."
                    .into(),
            ),
            table(
                "results-table",
                "The hashes of the results",
                &["Result", "SHA-256"],
                results,
            ),
        ],
    ));
    (body, Vec::new())
}
