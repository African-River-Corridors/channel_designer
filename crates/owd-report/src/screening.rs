//! The screening report (spec 0014, report 1): the reach, data sources and their limits, what fits,
//! the constraint stretches, the design basis, and the works and quantities. Every number is read
//! from the document: the fits answer, the sizing stage's design basis, and the volumes stage's
//! screening of each design line option (spec 0007).

use crate::Context;
use crate::fmt::{km, m3, md, num, text, val};
use crate::{figures, limits, trace};
use owd_format::design::alignment::NO_FEASIBLE_LINE;
use owd_format::design::export::{failed_checks, no_feasible_line};
use serde_json::{Value, json};

/// The most stretches the screening report lists; the rest are in the quantity schedule.
pub const STRETCH_CAP: usize = 200;

/// The body and the appendices of the report; the figures' bytes go into `assets`.
pub fn body(ctx: &Context, assets: &mut Vec<(String, Vec<u8>)>) -> (Vec<Value>, Vec<Value>) {
    let body = vec![
        reach(ctx, assets),
        sources(ctx),
        what_fits(ctx),
        stretches(ctx),
        design_basis(ctx),
        quantities(ctx),
        section(
            "long-section-section",
            "Long section",
            vec![figures::long_section(ctx, assets)],
        ),
    ];
    (body, trace::appendix(ctx).into_iter().collect())
}

fn section(id: &str, title: &str, body: Vec<Value>) -> Value {
    json!({"type": "section", "id": id, "title": title, "body": body})
}

fn prose(markdown: impl Into<String>) -> Value {
    json!({"type": "prose", "markdown": markdown.into()})
}

/// An ORD table. `columns` is `(title, align, unit)`.
fn table(
    id: &str,
    caption: &str,
    columns: &[(&str, &str, Option<&str>)],
    rows: Vec<Vec<String>>,
) -> Value {
    let cols: Vec<Value> = columns
        .iter()
        .map(|(t, a, u)| {
            let mut c = json!({"title": t, "align": a});
            if let Some(u) = u {
                c["unit"] = json!(u);
            }
            c
        })
        .collect();
    json!({"type": "table", "id": id, "caption": caption, "columns": cols, "rows": rows})
}

fn reach(ctx: &Context, assets: &mut Vec<(String, Vec<u8>)>) -> Value {
    let d = ctx.doc;
    let mut text_ = format!(
        "This report screens **{}** for navigation. It is a screening: it ranks and sizes, and it is \
         not a design.",
        md(&ctx.title)
    );
    if let Some(l) = d["fits"]["results"]["length_m"].as_f64() {
        text_.push_str(&format!(" The river centreline is {} km long.", km(l)));
    }
    if let Some(o) = &ctx.option {
        text_.push_str(&format!(
            " The design line option this report reads is **{}**.",
            md(text(&o["name"], text(&o["id"], "")))
        ));
    }
    section(
        "reach",
        "The reach",
        vec![
            limits::data_box(ctx),
            prose(text_),
            figures::key_map(ctx, assets),
        ],
    )
}

const INPUTS: [(&str, &str); 8] = [
    ("route", "Route"),
    ("left_bank", "Left bank"),
    ("right_bank", "Right bank"),
    ("centreline", "River centreline"),
    ("river_polygon", "River polygon"),
    ("dem", "DEM"),
    ("water_level", "Water level"),
    ("bed", "Depth (bed model)"),
];

fn sources(ctx: &Context) -> Value {
    let d = ctx.doc;
    let mut rows = Vec::new();
    for (key, name) in INPUTS {
        let present = d["inputs"][key].is_object()
            || (key == "bed" && ctx.session.bed_model_value().is_some())
            || (key == "route" && d["reach"].is_object());
        if !present {
            continue;
        }
        match ctx.session.provenance(key) {
            Some(p) => rows.push(vec![
                name.to_string(),
                text(&p["source"], limits::NOT_RECORDED).to_string(),
                text(&p["date"], limits::NOT_RECORDED).to_string(),
                text(&p["mark"], limits::NOT_RECORDED).to_string(),
                if p["provisional"].as_bool() == Some(true) {
                    "yes".into()
                } else {
                    "no".into()
                },
                text(&p["notice"], "").to_string(),
            ]),
            None => rows.push(vec![
                name.to_string(),
                "the user's file".into(),
                limits::NOT_RECORDED.into(),
                "user".into(),
                "no".into(),
                String::new(),
            ]),
        }
    }
    let mut body = vec![prose(
        "Each input the result rests on, where it came from, and its licence notice. A provisional \
         default is not yet calibrated against a surveyed reach.",
    )];
    if rows.is_empty() {
        body.push(prose("The design records no inputs."));
    } else {
        body.push(table(
            "inputs",
            "The inputs and their sources",
            &[
                ("Input", "left", None),
                ("Source", "left", None),
                ("Date", "left", None),
                ("Made by", "left", None),
                ("Provisional", "left", None),
                ("Notice", "left", None),
            ],
            rows,
        ));
    }
    body.push(prose(stage_limits(ctx)));
    section("sources", "Data sources and their limits", body)
}

/// The limits the stages record, as a bullet list.
fn stage_limits(ctx: &Context) -> String {
    let d = ctx.doc;
    let mut items = Vec::new();
    if let Some(list) = d["low_confidence_width"]["stretches"].as_array() {
        let mut kinds: Vec<(String, usize)> = Vec::new();
        for s in list {
            let k = text(&s["kind"], "?").to_string();
            match kinds.iter_mut().find(|(n, _)| *n == k) {
                Some(e) => e.1 += 1,
                None => kinds.push((k, 1)),
            }
        }
        for (k, n) in kinds {
            items.push(format!("{n} low-confidence width stretches of kind `{k}`."));
        }
    }
    if let Some(n) = ctx.session.depth_note() {
        items.push(format!("The depth is {}.", md(n)));
    }
    for o in d["stages"]["volumes"]["results"]["options"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let singles = o["screening"]["constraint_stretches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["kind"] == "depth" && s["start_chainage_m"] == s["end_chainage_m"])
            .count();
        if singles > 0 {
            items.push(format!(
                "Option {}: {singles} depth stretches on a single station.",
                md(text(&o["name"], text(&o["id"], "?")))
            ));
        }
    }
    if items.is_empty() {
        "The stages record no further limits.".into()
    } else {
        format!(
            "The limits the stages record:\n\n{}",
            items
                .iter()
                .map(|i| format!("- {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

/// Stretch counts by kind: `width / depth / radius` (and `infeasible` when there are any).
fn kind_counts(list: &[Value]) -> String {
    let n = |k: &str| list.iter().filter(|s| s["kind"] == k).count();
    let mut s = format!("{} / {} / {}", n("width"), n("depth"), n("radius"));
    let inf = n("infeasible");
    if inf > 0 {
        s.push_str(&format!(" (+{inf} infeasible)"));
    }
    s
}

fn what_fits(ctx: &Context) -> Value {
    let d = ctx.doc;
    let mut body = Vec::new();
    let fits = &d["fits"]["results"];
    match fits["rule_sets"].as_array() {
        Some(sets) => {
            let full = fits["mode"] == "full";
            body.push(prose(format!(
                "The verdict for each vessel class, from the fits answer ({}). Stretches are \
                 counted by kind: width / depth / radius.",
                md(text(&fits["label"], "no label"))
            )));
            let mut rows = Vec::new();
            for rs in sets {
                for c in rs["classes"].as_array().into_iter().flatten() {
                    let list = c["constraint_stretches"]
                        .as_array()
                        .or(c["stretches"].as_array())
                        .cloned()
                        .unwrap_or_default();
                    let verdict = if full {
                        text(&c["verdict"], "none").to_string()
                    } else if c["fits"].as_bool() == Some(true) {
                        "fits by width and radius".into()
                    } else {
                        "does not fit by width and radius".into()
                    };
                    rows.push(vec![
                        text(&rs["rule_set"], "?").to_string(),
                        text(&c["vessel_standard_id"], "?").to_string(),
                        verdict,
                        kind_counts(&list),
                    ]);
                }
            }
            body.push(table(
                "fits",
                "The verdict for each vessel class",
                &[
                    ("Rule set", "left", None),
                    ("Class", "left", None),
                    ("Verdict", "left", None),
                    ("Stretches", "right", None),
                ],
                rows,
            ));
            let largest: Vec<String> = sets
                .iter()
                .map(|rs| {
                    format!(
                        "- {}: {}",
                        md(text(&rs["rule_set"], "?")),
                        match &rs["largest_fitting"] {
                            Value::String(s) => md(s),
                            Value::Null => "no class fits as it is".into(),
                            other => md(&other.to_string()),
                        }
                    )
                })
                .collect();
            body.push(prose(format!(
                "The largest class that fits as it is:\n\n{}",
                largest.join("\n")
            )));
        }
        None => body.push(prose("The design has no fits answer (`owd fits`).")),
    }
    let rows: Vec<Vec<String>> = d["stages"]["volumes"]["results"]["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["screening"].is_object())
        .map(|o| {
            let s = &o["screening"];
            let b = &s["blocking_not_evaluated"];
            let names: Vec<&str> = b["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let blocking = match b["count"].as_u64().unwrap_or(0) {
                0 => "0".to_string(),
                n => format!("{n}: {}", names.join(", ")),
            };
            let list = s["constraint_stretches"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            vec![
                text(&o["name"], text(&o["id"], "?")).to_string(),
                text(&s["verdict"], "none").to_string(),
                blocking,
                kind_counts(&list),
            ]
        })
        .collect();
    if rows.is_empty() {
        body.push(prose(
            "The design has no screened design line option (`owd volumes`).",
        ));
    } else {
        body.push(table(
            "option-verdicts",
            "The verdict for each design line option",
            &[
                ("Option", "left", None),
                ("Verdict", "left", None),
                ("Blocking Rules not evaluated", "left", None),
                ("Stretches", "right", None),
            ],
            rows,
        ));
    }
    section("what-fits", "What fits", body)
}

fn stretch_list(ctx: &Context) -> Option<Vec<Value>> {
    ctx.volumes_row()?["screening"]["constraint_stretches"]
        .as_array()
        .cloned()
}

fn stretches(ctx: &Context) -> Value {
    let Some(list) = stretch_list(ctx) else {
        return section(
            "stretches",
            "Constraint stretches",
            vec![prose(
                "The design has no screened design line option, so it has no constraint stretches.",
            )],
        );
    };
    let mut body = Vec::new();
    let mut summary = Vec::new();
    for kind in ["width", "depth", "radius", "infeasible"] {
        let of: Vec<&Value> = list.iter().filter(|s| s["kind"] == kind).collect();
        if of.is_empty() {
            continue;
        }
        let length: f64 = of
            .iter()
            .map(|s| {
                s["end_chainage_m"].as_f64().unwrap_or(0.0)
                    - s["start_chainage_m"].as_f64().unwrap_or(0.0)
            })
            .sum();
        let worst = of
            .iter()
            .filter_map(|s| s["deficit"].as_f64())
            .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))));
        let mut works: Vec<String> = Vec::new();
        for s in &of {
            let w = match &s["works"] {
                Value::String(w) => w.clone(),
                _ => "none possible".into(),
            };
            if !works.contains(&w) {
                works.push(w);
            }
        }
        summary.push(vec![
            kind.to_string(),
            of.len().to_string(),
            km(length),
            worst.map_or("—".into(), |w| num(w, 2)),
            works.join(", "),
        ]);
    }
    if summary.is_empty() {
        body.push(prose(
            "The chosen design line option has no constraint stretch: the river as found meets every \
             governing limit along it.",
        ));
        return section("stretches", "Constraint stretches", body);
    }
    body.push(prose(
        "Where the river as found falls short of a governing limit along the chosen design line \
         option, by kind.",
    ));
    body.push(table(
        "stretch-summary",
        "Constraint stretches by kind",
        &[
            ("Kind", "left", None),
            ("Count", "right", None),
            ("Length", "right", Some("km")),
            ("Largest deficit", "right", None),
            ("Works", "left", None),
        ],
        summary,
    ));
    let mut shown: Vec<&Value> = list.iter().collect();
    let left_out = shown.len().saturating_sub(STRETCH_CAP);
    if left_out > 0 {
        shown.sort_by(|a, b| {
            let d = |s: &Value| s["deficit"].as_f64().unwrap_or(f64::INFINITY);
            d(b).total_cmp(&d(a))
        });
        shown.truncate(STRETCH_CAP);
    }
    shown.sort_by(|a, b| {
        let c = |s: &Value| s["start_chainage_m"].as_f64().unwrap_or(0.0);
        c(a).total_cmp(&c(b))
    });
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|s| {
            vec![
                text(&s["kind"], "?").to_string(),
                km(s["start_chainage_m"].as_f64().unwrap_or(0.0)),
                km(s["end_chainage_m"].as_f64().unwrap_or(0.0)),
                val(&s["required"], 2, "—"),
                val(&s["river"], 2, "—"),
                val(&s["deficit"], 2, "—"),
                match &s["works"] {
                    Value::String(w) => w.clone(),
                    _ => "none possible".into(),
                },
                m3(&s["dredge_m3"]),
                m3(&s["dry_cut_m3"]),
            ]
        })
        .collect();
    let mut t = table(
        "stretch-list",
        "The constraint stretches of the chosen option",
        &[
            ("Kind", "left", None),
            ("From", "right", Some("km")),
            ("To", "right", Some("km")),
            ("Required", "right", None),
            ("Found", "right", None),
            ("Deficit", "right", None),
            ("Works", "left", None),
            ("Dredge", "right", Some("m³")),
            ("Dry cut", "right", Some("m³")),
        ],
        rows,
    );
    let mut notes = String::from(
        "Required, found and deficit are in metres (width and depth) or metres of radius.",
    );
    if left_out > 0 {
        notes.push_str(&format!(
            " The table holds the {STRETCH_CAP} stretches with the largest deficit and leaves out \
             {left_out}; the quantity schedule and `owd export --stretches` list all {}.",
            list.len()
        ));
    }
    t["notes"] = json!(notes);
    body.push(t);
    section("stretches", "Constraint stretches", body)
}

fn design_basis(ctx: &Context) -> Value {
    let r = &ctx.doc["stages"]["sizing"]["results"];
    let Some(list) = r["design_basis"].as_array() else {
        return section(
            "design-basis",
            "Design basis",
            vec![prose(
                "The sizing stage has not run, so there is no design basis.",
            )],
        );
    };
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|b| {
            vec![
                text(&b["quantity"], "?").to_string(),
                text(&b["waterway"], "all").to_string(),
                format!("{} {}", val(&b["value"], 2, "—"), text(&b["unit"], "")),
                text(&b["rule"], "?").to_string(),
                b["rule_sets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                text(&b["citation"], "").to_string(),
                text(&b["origin"], "").to_string(),
                text(&b["status"], "").to_string(),
            ]
        })
        .collect();
    let mut body = vec![prose(format!(
        "The vessel is **{}** (length {} m, beam {} m, draught {} m). Each dimension names the Rule \
         that governs it.",
        md(text(&r["vessel_standard_id"], "not recorded")),
        val(&r["loa_m"], 2, "—"),
        val(&r["beam_m"], 2, "—"),
        val(&r["draught_m"], 2, "—"),
    ))];
    body.push(table(
        "design-basis-table",
        "The design basis",
        &[
            ("Quantity", "left", None),
            ("Waterway", "left", None),
            ("Value", "right", None),
            ("Governing Rule", "left", None),
            ("Rule Sets", "left", None),
            ("Citation", "left", None),
            ("Origin", "left", None),
            ("Status", "left", None),
        ],
        rows,
    ));
    section("design-basis", "Design basis", body)
}

fn quantities(ctx: &Context) -> Value {
    let d = ctx.doc;
    let rows: Vec<Vec<String>> = d["options"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|o| {
            let v = d["stages"]["volumes"]["results"]["options"]
                .as_array()
                .and_then(|a| a.iter().find(|r| r["id"] == o["id"]))
                .cloned()
                .unwrap_or(Value::Null);
            let failed = failed_checks(o);
            // Issue #366: an option with no feasible line is not a design and has no quantities.
            if no_feasible_line(o) {
                let none = NO_FEASIBLE_LINE.to_string();
                return vec![
                    text(&o["name"], text(&o["id"], "?")).to_string(),
                    none.clone(),
                    none.clone(),
                    none.clone(),
                    none,
                    failed.join(", "),
                ];
            }
            vec![
                text(&o["name"], text(&o["id"], "?")).to_string(),
                m3(&v["dredge_m3"]),
                m3(&v["excav_m3"]),
                m3(&v["total_m3"]),
                val(&v["max_face_h_m"], 2, "not computed"),
                if failed.is_empty() {
                    "none".into()
                } else {
                    failed.join(", ")
                },
            ]
        })
        .collect();
    let body = if rows.is_empty() {
        vec![prose(
            "The design has no design line option, so it has no works.",
        )]
    } else {
        let mut t = table(
            "quantities-table",
            "Works and quantities for each design line option",
            &[
                ("Option", "left", None),
                ("Dredge", "right", Some("m³")),
                ("Dry cut", "right", Some("m³")),
                ("Total", "right", Some("m³")),
                ("Tallest cut face", "right", Some("m")),
                ("Failed checks", "left", None),
            ],
            rows,
        );
        if let Some(n) = ctx.session.depth_note() {
            t["notes"] = json!(format!("The depth is {}.", md(n)));
        }
        vec![
            prose(format!(
                "The quantities of the volumes stage, which is **{}**.",
                ctx.session
                    .status_with_dem(ctx.dem)
                    .stages
                    .iter()
                    .find(|(s, _)| *s == owd_format::design::Stage::Volumes)
                    .map_or("not run", |(_, x)| x.name())
            )),
            t,
        ]
    };
    section("works", "Works and quantities", body)
}
