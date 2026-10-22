//! The section pack (spec 0014, report 3): the typical section, the cross-section gallery sheets
//! (one figure per sheet, its number in the caption) and the setting-out table. The table's cells
//! are the setting-out CSV that `owd draw` writes, so the report and the CSV never differ.

use crate::Context;
use crate::figures::{SheetFigure, left_out, sheets};
use owd_format::drawing::SheetKind;
use serde_json::{Value, json};

fn section(id: &str, title: &str, body: Vec<Value>) -> Value {
    json!({"type": "section", "id": id, "title": title, "body": body})
}

/// A sheet as a figure, its drawing number in the caption.
fn figure(id: &str, caption: String, alt: &str, f: &SheetFigure) -> Value {
    json!({"type": "figure", "id": id, "caption": format!("{caption}, sheet {}", f.number),
        "asset": id, "alt": alt})
}

/// The body and no appendix; the sheets' SVG go into `assets`.
pub fn pack(ctx: &Context, assets: &mut Vec<(String, Vec<u8>)>) -> (Vec<Value>, Vec<Value>) {
    let mut body = vec![
        json!({"type": "section", "id": "data", "title": "Data and limits",
        "numbered": false, "body": [crate::limits::data_box(ctx)]}),
    ];

    // The typical section.
    let typical = match sheets(ctx, SheetKind::TypicalSection) {
        Ok(list) => match list.first() {
            Some(f) => {
                assets.push(("typical-section".into(), f.svg.clone().into_bytes()));
                figure(
                    "typical-section",
                    "The typical section".into(),
                    "The typical section sheet: the channel's bottom width, side slopes and design depth",
                    f,
                )
            }
            None => left_out(
                "typical section",
                "the design makes no typical-section sheet",
            ),
        },
        Err(e) => left_out("typical section", &e),
    };
    body.push(section("typical", "Typical section", vec![typical]));

    // The cross sections, one figure per sheet.
    let gallery = match sheets(ctx, SheetKind::CrossSections) {
        Ok(list) if !list.is_empty() => {
            let n = list.len();
            list.iter()
                .enumerate()
                .map(|(i, f)| {
                    let id = format!("cross-sections-{}", i + 1);
                    assets.push((id.clone(), f.svg.clone().into_bytes()));
                    figure(
                        &id,
                        format!("Cross sections, {} of {n}", i + 1),
                        "A sheet of cross sections at chainages: the ground, the water level and the design cut face",
                        f,
                    )
                })
                .collect()
        }
        Ok(_) => vec![left_out(
            "cross-section gallery",
            "the design makes no cross-section sheet",
        )],
        Err(e) => vec![left_out("cross-section gallery", &e)],
    };
    body.push(section("cross-sections", "Cross sections", gallery));

    body.push(section(
        "setting-out",
        "Setting-out table",
        setting_out(ctx),
    ));
    (body, Vec::new())
}

/// The setting-out tables from the CSV the setting-out sheet carries.
fn setting_out(ctx: &Context) -> Vec<Value> {
    let list = match sheets(ctx, SheetKind::SettingOut) {
        Ok(l) => l,
        Err(e) => return vec![left_out("setting-out table", &e)],
    };
    let Some(csv) = list
        .iter()
        .flat_map(|f| f.extra.iter())
        .find(|(name, _)| name.ends_with(".csv"))
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
    else {
        return vec![left_out(
            "setting-out table",
            "the setting-out sheet has no CSV",
        )];
    };
    let mut stations = Vec::new();
    let mut bends = Vec::new();
    for line in csv.lines().skip(1) {
        let c: Vec<String> = line.split(',').map(String::from).collect();
        match c.first().map(String::as_str) {
            Some("station") => stations.push(c[1..6].to_vec()),
            Some("bend") => {
                bends.push(vec![c[1].clone(), c[6].clone(), c[7].clone(), c[8].clone()])
            }
            _ => {}
        }
    }
    let zone = ctx.doc["project"]["crs"]["name"]
        .as_str()
        .unwrap_or("the design's coordinate system");
    let col = |t: &str, a: &str, u: Option<&str>| {
        let mut c = json!({"title": t, "align": a});
        if let Some(u) = u {
            c["unit"] = json!(u);
        }
        c
    };
    let mut out = vec![json!({"type": "prose", "markdown": format!(
        "The design line of the option, station by station, then one row per bend: the same values \
         as the setting-out sheets {} and their CSV. Coordinates are in {zone}.",
        list.iter().map(|f| f.number.as_str()).collect::<Vec<_>>().join(", ")
    )})];
    out.push(json!({
        "type": "table", "id": "setting-out-stations", "caption": "Setting out: the stations",
        "columns": [col("Chainage", "right", Some("m")), col("Easting", "right", Some("m")),
            col("Northing", "right", Some("m")), col("Bearing", "right", Some("°")),
            col("Radius", "right", Some("m"))],
        "rows": stations,
        "notes": "A radius of \"straight\" is a straight; an empty radius is where the check chord does not fit on the line.",
    }));
    if !bends.is_empty() {
        out.push(json!({
            "type": "table", "id": "setting-out-bends", "caption": "Setting out: the bends",
            "columns": [col("Start", "right", Some("m")), col("End", "right", Some("m")),
                col("Least radius", "right", Some("m")), col("Deflection", "right", Some("°"))],
            "rows": bends,
        }));
    }
    out
}
