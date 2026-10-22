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
