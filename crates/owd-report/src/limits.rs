//! The "Data and limits" box every report opens with (spec 0014): one item per fact the result
//! rests on, read from the document. A fact the document does not hold prints "not recorded"; a
//! default the provenance names provisional prints "provisional".

use crate::Context;
use crate::fmt::{km, num, position, text};
use owd_format::design::StageStatus;
use serde_json::{Value, json};

pub const NOT_RECORDED: &str = "not recorded";

/// The box as an ORD `data_box` block.
pub fn data_box(ctx: &Context) -> Value {
    let items: Vec<Value> = items(ctx)
        .into_iter()
        .map(|(label, value)| json!({"label": label, "value": value}))
        .collect();
    let mut b = json!({"type": "data_box", "id": "data-and-limits", "items": items});
    let notices = ctx.session.notices();
    if !notices.is_empty() {
        let list: Vec<String> = notices
            .iter()
            .map(|n| format!("- {}", crate::fmt::md(n)))
            .collect();
        b["notes"] = json!(format!("Licence notices:\n\n{}", list.join("\n")));
    }
    b
}

/// `(label, value)` per fact, in the spec's order.
pub fn items(ctx: &Context) -> Vec<(String, String)> {
    let d = ctx.doc;
    let mut out: Vec<(String, String)> = vec![
        ("Reach".into(), reach(d)),
        (
            "Coordinate system".into(),
            text(&d["project"]["crs"]["name"], NOT_RECORDED).to_string(),
        ),
        ("Bank lines".into(), banks(ctx)),
        ("Low-confidence width".into(), low_confidence(d)),
        ("DEM".into(), dem(ctx)),
        ("Water level".into(), water_level(ctx)),
        ("Depth".into(), depth(ctx)),
        ("Plan line".into(), plan_line(d)),
        ("Rule Sets".into(), rule_sets(d)),
        ("Stale stages".into(), stale(ctx)),
    ];
    // A custom vessel (spec 0015 T1): what its figures are, and any warning on them.
    if let Some(c) = ctx.session.custom_vessel() {
        let mut v = format!(
            "{}: user-entered dimensions, not a catalogue class",
            c.label()
        );
        for w in c.check().unwrap_or_default() {
            v.push_str(&format!("; warning: {w}"));
        }
        out.insert(out.len() - 1, ("Vessel".into(), v));
    }
    out
}

fn provisional(settings: &Value, key: &str) -> bool {
    settings["provisional"]
        .as_array()
        .is_some_and(|a| a.iter().any(|k| k == key))
}

/// A setting with its unit, marked provisional when the record says so.
fn setting(settings: &Value, key: &str, unit: &str) -> Option<String> {
    let x = settings.get(key)?.as_f64()?;
    let mark = if provisional(settings, key) {
        " (provisional)"
    } else {
        ""
    };
    Some(format!(
        "{}{unit}{mark}",
        num(x, if x.fract() == 0.0 { 0 } else { 2 })
    ))
}

fn reach(d: &Value) -> String {
    let r = &d["reach"];
    let mut parts = Vec::new();
    if r.is_object() {
        parts.push(text(&r["river"], "unnamed river").to_string());
        if let (Some(a), Some(b)) = (position(&r["start"]), position(&r["end"])) {
            parts.push(format!("from {a} to {b} (longitude, latitude)"));
        }
    }
    let route = r["length_m"].as_f64();
    if let Some(l) = route {
        parts.push(format!("route {} km", km(l)));
    }
    // The centreline's own length beside the route's: banks that wander make it much longer
    // (issue #360). A measure of the input, not a design result.
    if let Some(c) = line_length(&d["inputs"]["centreline"]["coordinates"]) {
        let ratio = route.filter(|l| *l > 0.0).map_or(String::new(), |l| {
            format!(", {} times the route", num(c / l, 2))
        });
        parts.push(format!("centreline {} km{ratio}", km(c)));
    }
    if parts.is_empty() {
        NOT_RECORDED.into()
    } else {
        parts.join("; ")
    }
}

/// The length in metres of a line of projected coordinates; `None` for no line.
fn line_length(coords: &Value) -> Option<f64> {
    let pts: Vec<(f64, f64)> = coords
        .as_array()?
        .iter()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect();
    (pts.len() > 1).then(|| {
        pts.windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum()
    })
}

fn banks(ctx: &Context) -> String {
    let Some(p) = ctx.session.provenance("left_bank") else {
        return if ctx.doc["inputs"]["left_bank"].is_object() {
            "the user's own".into()
        } else {
            NOT_RECORDED.into()
        };
    };
    let s = &p["settings"];
    let mut parts = vec![format!("source {}", text(&p["source"], NOT_RECORDED))];
    parts.push(format!(
        "seed {}",
        text(&s["seed"], text(&p["source"], NOT_RECORDED))
    ));
    if let Some(t) =
        setting(s, "mask_threshold_pct", " %").or_else(|| setting(s, "jrc_threshold_pct", " %"))
    {
        parts.push(format!("mask threshold {t}"));
    }
    match setting(s, "max_shift_m", " m") {
        Some(m) => parts.push(format!("moved by the edge fit, at most {m}")),
        None => parts.push("no edge fit".into()),
    }
    parts.join("; ")
}

fn low_confidence(d: &Value) -> String {
    let l = &d["low_confidence_width"];
    if !l.is_object() {
        return NOT_RECORDED.into();
    }
    let n = l["stretches"].as_array().map_or(0, Vec::len);
    let under = l["min_width_m"]
        .as_f64()
        .map(|w| {
            format!(
                " (narrower than {} m, or flagged by a bank check)",
                num(w, 0)
            )
        })
        .unwrap_or_default();
    let mark = if l["provisional"].as_bool() == Some(true) {
        "; threshold provisional"
    } else {
        ""
    };
    format!("{n} stretches{under}{mark}")
}

fn dem(ctx: &Context) -> String {
    let d = &ctx.doc["inputs"]["dem"];
    if !d.is_object() {
        return NOT_RECORDED.into();
    }
    let source = ctx
        .session
        .provenance("dem")
        .and_then(|p| p["source"].as_str())
        .or(d["path"].as_str())
        .unwrap_or(NOT_RECORDED);
    let hash = d["sha256"].as_str().map_or(String::new(), |h| {
        format!("; SHA-256 {}", &h[..h.len().min(12)])
    });
    format!("{source}{hash}")
}

fn water_level(ctx: &Context) -> String {
    if !ctx.doc["inputs"]["water_level"].is_object() {
        return NOT_RECORDED.into();
    }
    match ctx.session.provenance("water_level") {
        Some(p) => {
            let s = &p["settings"];
            let mut out = text(&p["source"], NOT_RECORDED).to_string();
            if let Some(x) = setting(s, "percentile", "th percentile") {
                out.push_str(&format!("; {x}"));
            }
            out
        }
        None => "the user's own".into(),
    }
}

fn depth(ctx: &Context) -> String {
    let Some(b) = ctx.session.bed_model_value() else {
        return NOT_RECORDED.into();
    };
    let pts = b["points"].as_array().cloned().unwrap_or_default();
    let depths: Vec<f64> = pts.iter().filter_map(|p| p[1].as_f64()).collect();
    let value = match depths.as_slice() {
        [] => NOT_RECORDED.to_string(),
        [one] => format!("{} m", num(*one, 2)),
        many => {
            let lo = many.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = many.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            format!("{} to {} m", num(lo, 2), num(hi, 2))
        }
    };
    let season = b["season"]
        .as_str()
        .map_or(String::new(), |s| format!(", {s} season"));
    let label = ctx.session.depth_note().unwrap_or("assumed, not measured");
    format!("{value}{season}; {label}")
}

fn plan_line(d: &Value) -> String {
    let fits = &d["fits"];
    if !fits.is_object() {
        return NOT_RECORDED.into();
    }
    text(
        &fits["results"]["plan_line"],
        text(&fits["settings"]["plan_line"], "centreline"),
    )
    .to_string()
}

fn rule_sets(d: &Value) -> String {
    let sizing = &d["stages"]["sizing"];
    let s = &sizing["settings"];
    let mut out = Vec::new();
    if let Some(sets) = s["rule_sets"].as_array() {
        let mut parts: Vec<String> = sets
            .iter()
            .map(|r| format!("{}@{}", text(&r["id"], "?"), text(&r["version"], "?")))
            .collect();
        for i in s["set_inputs"].as_array().into_iter().flatten() {
            let v = match &i["value"] {
                Value::String(x) => x.clone(),
                other => other.to_string(),
            };
            parts.push(format!("{} {}", text(&i["name"], "?"), v));
        }
        let ran = if sizing["results"].is_object() {
            ""
        } else {
            " (selected; sizing not run)"
        };
        out.push(format!("sizing: {}{ran}", parts.join(", ")));
    }
    // The fits answer names its own rule sets: they are the ones its verdicts used.
    if let Some(sets) = d["fits"]["results"]["rule_sets"].as_array() {
        let names: Vec<&str> = sets.iter().filter_map(|r| r["rule_set"].as_str()).collect();
        out.push(format!("fits answer: {}", names.join(", ")));
    }
    if out.is_empty() {
        return NOT_RECORDED.into();
    }
    let mut text = out.join("; ");
    if sizing["results"]["rules_notice"].is_string()
        || d["fits"]["results"]["rules_notice"].is_string()
    {
        text.push_str("; every Rule unverified");
    }
    text
}

fn stale(ctx: &Context) -> String {
    let st = ctx.session.status_with_dem(ctx.dem);
    let bad: Vec<String> = st
        .stages
        .iter()
        .filter(|(_, x)| *x != StageStatus::Fresh)
        .map(|(s, x)| format!("{} {}", s.name(), x.name()))
        .collect();
    if bad.is_empty() {
        "none".into()
    } else {
        bad.join(", ")
    }
}
