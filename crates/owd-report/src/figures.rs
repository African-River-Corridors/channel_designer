//! Figures from the drawing sheets (spec 0014, "Figures"): each figure is the SVG that
//! `drawing::render` writes for `owd draw`, so a figure and its sheet are the same drawing. A sheet
//! that cannot be made (no DEM, no option) gives a one-line note instead of a figure.

use crate::Context;
use crate::fmt::md;
use owd_format::drawing::{self, DrawOptions, SheetKind};
use serde_json::{Value, json};

/// One sheet, ready to place: its number, its SVG, its drawing document and its extra files (the
/// setting-out CSV).
pub struct SheetFigure {
    pub number: String,
    pub svg: String,
    pub document: Value,
    pub extra: Vec<(String, Vec<u8>)>,
}

/// The sheets of `kind` for the report's option, or why there are none.
pub fn sheets(ctx: &Context, kind: SheetKind) -> Result<Vec<SheetFigure>, String> {
    let options = DrawOptions {
        option: ctx
            .option
            .as_ref()
            .and_then(|o| o["id"].as_str().map(String::from)),
        sheets: vec![kind],
        ..DrawOptions::default()
    };
    let made = drawing::sheets_from_text(&ctx.session.to_text(), ctx.dem, &options)
        .map_err(|e| e.to_string())?;
    made.iter()
        .map(|s| {
            let out = drawing::render(s).map_err(|e| e.to_string())?;
            Ok(SheetFigure {
                number: out.number,
                svg: out.svg,
                document: s.document.clone(),
                extra: out.extra,
            })
        })
        .collect()
}

/// The note that stands in for a figure whose sheet could not be made.
pub fn left_out(what: &str, why: &str) -> Value {
    json!({
        "type": "prose",
        "markdown": format!("The {what} is left out: {}.", md(why.trim_end_matches('.'))),
    })
}

/// The key map: the plan overview sheet as an ORD `map`, with its coordinate system, its scale on
/// the sheet and the attribution of what it is drawn over.
pub fn key_map(ctx: &Context, assets: &mut Vec<(String, Vec<u8>)>) -> Value {
    let sheets = match sheets(ctx, SheetKind::Plan) {
        Ok(s) => s,
        Err(e) => return left_out("key map", &e),
    };
    let Some(overview) = sheets.into_iter().next() else {
        return left_out("key map", "the design makes no plan sheet");
    };
    let scale = overview.document["viewports"]
        .as_array()
        .and_then(|v| v.iter().find(|p| p["id"] == "plan"))
        .and_then(|p| p["scale"].as_u64())
        .map(|n| {
            format!(
                "1:{} on the {} sheet; reduced here",
                crate::fmt::num(n as f64, 0),
                overview.document["sheet"]["size"]
                    .as_str()
                    .unwrap_or("full")
            )
        });
    let crs = ctx.doc["project"]["crs"]["name"]
        .as_str()
        .unwrap_or("the design's coordinate system")
        .to_string();
    let notices = ctx.session.notices();
    let attribution = if notices.is_empty() {
        "Drawn by Open Waterway Designer from the design's own inputs".to_string()
    } else {
        notices.join("; ")
    };
    assets.push(("key-map".into(), overview.svg.into_bytes()));
    let mut m = json!({
        "type": "map",
        "id": "key-map",
        "caption": format!("Key map: the plan overview, sheet {}", overview.number),
        "asset": "key-map",
        "alt": "The plan overview sheet: the river, the design line option and the constraint stretches, with a north arrow and a scale bar",
        "crs": crs,
        "attribution": attribution,
    });
    if let Some(s) = scale {
        m["scale"] = json!(s);
    }
    m
}

/// The long section of the report's option.
pub fn long_section(ctx: &Context, assets: &mut Vec<(String, Vec<u8>)>) -> Value {
    let sheets = match sheets(ctx, SheetKind::LongSection) {
        Ok(s) => s,
        Err(e) => return left_out("long section", &e),
    };
    let Some(ls) = sheets.into_iter().next() else {
        return left_out("long section", "the design makes no long-section sheet");
    };
    assets.push(("long-section".into(), ls.svg.into_bytes()));
    json!({
        "type": "figure",
        "id": "long-section",
        "caption": format!("Long section: the bed on the design line, the water level and the design bed, sheet {}", ls.number),
        "asset": "long-section",
        "alt": "The long section sheet: bed, water level and design bed levels against chainage",
    })
}

/// The size sweep's two figures (spec 0016 T3): each size along the reference line, green where it
/// passes and red where a stretch blocks it; and the per-kilometre strip, darker for a larger
/// size. Drawn here as SVG from the study's numbers.
pub fn envelope(e: &Value, assets: &mut Vec<(String, Vec<u8>)>) -> Vec<Value> {
    const PASS: &str = "#409c68";
    const BLOCKED: &str = "#c44034";
    let sizes: Vec<&Value> = e["sizes"].as_array().into_iter().flatten().collect();
    let km: Vec<&Value> = e["per_km"].as_array().into_iter().flatten().collect();
    let len = e["length_m"].as_f64().unwrap_or(1.0).max(1.0);
    let (left, plot_w, row_h) = (70.0, 660.0, 22.0);
    let x_of = |m: f64| left + (m / len).clamp(0.0, 1.0) * plot_w;
    let width = left + plot_w + 20.0;
    let axis = |y: f64| {
        format!(
            r#"<text x="{left}" y="{y}" font-size="11">0 km</text><text x="{}" y="{y}" font-size="11" text-anchor="end">{} km</text>"#,
            left + plot_w,
            crate::fmt::num(len / 1000.0, 1)
        )
    };
    let svg = |h: f64, inner: String| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{h}" viewBox="0 0 {width} {h}" font-family="sans-serif"><rect width="{width}" height="{h}" fill="white"/>{inner}</svg>"#
        )
    };
    // The sizes, the largest on top.
    let mut inner = String::new();
    for (r, s) in sizes.iter().rev().enumerate() {
        let y = 10.0 + r as f64 * row_h;
        inner.push_str(&format!(
            r#"<text x="8" y="{}" font-size="11">{}</text><rect x="{left}" y="{}" width="{plot_w}" height="{}" fill="{PASS}"/>"#,
            y + 15.0,
            s["label"].as_str().unwrap_or("?"),
            y + 3.0,
            row_h - 6.0
        ));
        for st in s["stretches"].as_array().into_iter().flatten() {
            let (a, b) = (
                x_of(st["start_chainage_m"].as_f64().unwrap_or(0.0)),
                x_of(st["end_chainage_m"].as_f64().unwrap_or(0.0)),
            );
            inner.push_str(&format!(
                r#"<rect x="{a:.2}" y="{}" width="{:.2}" height="{}" fill="{BLOCKED}"/>"#,
                y + 3.0,
                (b - a).max(0.8),
                row_h - 6.0
            ));
        }
    }
    let h1 = 10.0 + sizes.len() as f64 * row_h + 22.0;
    inner.push_str(&axis(h1 - 6.0));
    assets.push(("envelope-sizes".into(), svg(h1, inner).into_bytes()));
    // The strip.
    let n = sizes.len().max(1) as f64;
    let mut inner = String::new();
    for k in &km {
        let (a, b) = (
            x_of(k["from_m"].as_f64().unwrap_or(0.0)),
            x_of(k["to_m"].as_f64().unwrap_or(0.0)),
        );
        let i = k["largest"]["label"]
            .as_str()
            .and_then(|l| sizes.iter().position(|s| s["label"] == l));
        let fill = match i {
            Some(i) => {
                let t = (i + 1) as f64 / n;
                let v = |lo: f64, hi: f64| (lo + (hi - lo) * t).round() as u8;
                format!(
                    "#{:02x}{:02x}{:02x}",
                    v(198.0, 8.0),
                    v(219.0, 48.0),
                    v(239.0, 107.0)
                )
            }
            None => BLOCKED.to_string(),
        };
        inner.push_str(&format!(
            r#"<rect x="{a:.2}" y="8" width="{:.2}" height="20" fill="{fill}"/>"#,
            (b - a).max(0.8)
        ));
    }
    inner.push_str(r#"<text x="8" y="22" font-size="11">per km</text>"#);
    inner.push_str(&axis(44.0));
    assets.push(("envelope-strip".into(), svg(52.0, inner).into_bytes()));
    vec![
        json!({
            "type": "figure",
            "id": "envelope-sizes",
            "caption": "Each size of the sweep along the reference line: green where it passes, red where a stretch blocks it",
            "asset": "envelope-sizes",
            "alt": "One bar per size, largest on top, along the river; red marks are the stretches that block a size",
        }),
        json!({
            "type": "figure",
            "id": "envelope-strip",
            "caption": "The largest size that passes each kilometre: darker for a larger size, red where none passes",
            "asset": "envelope-strip",
            "alt": "A strip along the river, one cell per kilometre, shaded by the largest size that passes it",
        }),
    ]
}
