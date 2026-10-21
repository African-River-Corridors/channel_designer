//! The domain checks of a drawing (spec 0004): read the finished drawing documents back and compare
//! every drawn value with the design's result. A drawing that holds a wrong number fails `owd draw`
//! and names the sheet and the check.
//!
//! The comparison reads the document as the standard defines it (entities, with their ids and
//! texts), not what the builder meant to draw. A number in text is compared to the precision it is
//! printed to. A point in a line is compared exactly, as a result of the same arithmetic.

use super::Sheet;
use super::gallery::PANEL_STRIDE_M;
use owd_core::volumes::sections::{CrossSection, SectionVolumes};
use serde_json::Value;

/// A drawn value that differs from its result.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// The drawing number of the sheet.
    pub sheet: String,
    /// The name of the check.
    pub check: &'static str,
    pub message: String,
}

impl core::fmt::Display for Finding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "sheet {}: check `{}`: {}",
            self.sheet, self.check, self.message
        )
    }
}

/// The half step of a printed digit: a value printed to `d` decimals may differ from its result by
/// this much.
fn half_step(decimals: i32) -> f64 {
    0.5 * libm::pow(10.0, f64::from(-decimals)) + 1e-9
}

/// The entities of a viewport by id.
fn entity<'a>(viewport: &'a Value, id: &str) -> Option<&'a Value> {
    viewport["entities"]
        .as_array()?
        .iter()
        .find(|e| e["id"] == id)
}

fn text_of<'a>(viewport: &'a Value, id: &str) -> Option<&'a str> {
    entity(viewport, id)?["text"].as_str()
}

/// The number written second from the end of a text such as `DRY CUT 56.7 M²` or `WL +38.00`.
fn number_in(text: &str, from_end: usize) -> Option<f64> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    tokens
        .len()
        .checked_sub(from_end)
        .and_then(|i| tokens.get(i))
        .and_then(|t| t.parse().ok())
}

/// A chainage written `km+m.m`, in metres.
fn chainage_in(text: &str) -> Option<f64> {
    let (km, m) = text.strip_prefix("CH ")?.split_once('+')?;
    Some(km.parse::<f64>().ok()? * 1000.0 + m.parse::<f64>().ok()?)
}

fn points(e: &Value) -> Vec<(f64, f64)> {
    e["points_m"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// The checks of the cross-section gallery, for every sheet of it.
pub fn gallery(sheets: &[Sheet], walk: &SectionVolumes) -> Vec<Finding> {
    let mut found = Vec::new();
    for sheet in sheets {
        let mut note = |check: &'static str, message: String| {
            found.push(Finding {
                sheet: sheet.number.clone(),
                check,
                message,
            });
        };
        let viewports = sheet.document["viewports"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut drawn = 0usize;
        for vp in viewports
            .iter()
            .filter(|v| v["id"].as_str().is_some_and(|i| i.starts_with("xs-")))
        {
            drawn += 1;
            let id = vp["id"].as_str().unwrap_or("?");
            let Some(ch) = text_of(vp, &format!("{id}-chainage")).and_then(chainage_in) else {
                note("section-chainage", format!("{id} has no readable chainage"));
                continue;
            };
            let station: Option<&CrossSection> = walk
                .stations
                .iter()
                .find(|s| (s.chainage_m - ch).abs() <= half_step(1));
            let Some(s) = station else {
                note(
                    "section-chainage",
                    format!("{id} is drawn at chainage {ch} m, which is not a station of the walk"),
                );
                continue;
            };
            let at = format!("{id} (chainage {ch} m)");
            let mut value = |check: &'static str,
                             what: &str,
                             key: &str,
                             from_end: usize,
                             want: f64,
                             decimals: i32| {
                match text_of(vp, &format!("{id}-{key}")).and_then(|t| number_in(t, from_end)) {
                    Some(got) if (got - want).abs() <= half_step(decimals) => {}
                    Some(got) => note(check, format!("{at}: {what} drawn {got}, result {want}")),
                    None => note(check, format!("{at}: {what} is not drawn")),
                }
            };
            value(
                "section-area",
                "dredge area",
                "dredge",
                2,
                s.dredge_area_m2,
                1,
            );
            value("section-area", "dry cut area", "dry", 2, s.excav_area_m2, 1);
            value("section-level", "water level", "wl", 1, s.wl, 2);
            value("section-level", "design invert", "invert", 1, s.grade, 2);
            value("section-width", "bottom width", "bottom", 2, 2.0 * s.hb, 1);

            // The ground and the design cut face are the walk's profile, point for point.
            let shift = id
                .strip_prefix("xs-")
                .and_then(|n| n.parse::<f64>().ok())
                .unwrap_or(0.0)
                * PANEL_STRIDE_M;
            let on = |series: &[f64], x: f64, z: f64| {
                let x = x - shift;
                s.offsets
                    .iter()
                    .position(|o| (o - x).abs() <= 1e-6)
                    .is_some_and(|j| (series[j] - z).abs() <= 1e-9)
            };
            for e in vp["entities"].as_array().into_iter().flatten() {
                let eid = e["id"].as_str().unwrap_or("");
                let series = if eid.starts_with(&format!("{id}-ground-")) {
                    Some((&s.terrain, "ground"))
                } else if eid.starts_with(&format!("{id}-design-")) {
                    Some((&s.cutface, "design cut face"))
                } else {
                    None
                };
                if let Some((series, what)) = series {
                    for (x, z) in points(e) {
                        if !on(series, x, z) {
                            note(
                                "section-profile",
                                format!(
                                    "{at}: a {what} point ({x}, {z}) is not in the walk's profile"
                                ),
                            );
                            break;
                        }
                    }
                }
            }
            match entity(vp, &format!("{id}-wl-line")) {
                Some(e) => {
                    let ys = [e["from_m"][1].as_f64(), e["to_m"][1].as_f64()];
                    if ys.iter().any(|y| y.is_none_or(|y| (y - s.wl).abs() > 1e-9)) {
                        note(
                            "section-level",
                            format!("{at}: the water level line is not at {}", s.wl),
                        );
                    }
                }
                None => note("section-level", format!("{at}: no water level line")),
            }
        }
        if drawn == 0 {
            note("section-count", "the sheet draws no section".into());
        }
    }
    found
}

/// The checks of the typical section: each dimension is the length of the geometry it points at, and
/// that length is the design's result; the slope and the water level are in the geometry; each value
/// has a note that names its source.
pub fn typical(sheets: &[Sheet], f: &super::typical::Facts) -> Vec<Finding> {
    let mut found = Vec::new();
    for sheet in sheets {
        let mut note = |check: &'static str, message: String| {
            found.push(Finding {
                sheet: sheet.number.clone(),
                check,
                message,
            });
        };
        let vp = sheet.document["viewports"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["id"] == "ts"))
            .cloned()
            .unwrap_or(Value::Null);
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b.abs().max(1.0);
        // Dimensions: the standard computes the text from the two points, so the points are checked.
        let detail = sheet.document["viewports"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["id"] == "ts-detail"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut dim =
            |vp: &Value, id: &str, axis: usize, what: &str, want: f64| match entity(vp, id) {
                Some(e) => {
                    let (a, b) = (
                        e["from"]["point_m"][axis].as_f64(),
                        e["to"]["point_m"][axis].as_f64(),
                    );
                    match (a, b) {
                        (Some(a), Some(b)) if close((b - a).abs(), want) => {}
                        (Some(a), Some(b)) => note(
                            "typical-dimension",
                            format!("{what} is drawn as {}, result {want}", (b - a).abs()),
                        ),
                        _ => note("typical-dimension", format!("{what} has no points")),
                    }
                }
                None => note("typical-dimension", format!("{what} is not drawn")),
            };
        dim(&vp, "ts-width", 0, "bottom width", f.bottom_width_m);
        dim(&vp, "ts-depth", 1, "design depth", f.design_depth_m);
        dim(&detail, "ts-ukc", 1, "under-keel clearance", f.ukc_m());
        // The channel: its bottom is the width, its slopes the slope, its top the water level.
        match entity(&vp, "ts-channel").map(points) {
            Some(p) if p.len() == 4 => {
                if !close(p[2].0 - p[1].0, f.bottom_width_m) {
                    note(
                        "typical-dimension",
                        "the channel bottom is not the bottom width".into(),
                    );
                }
                let rise = p[0].1 - p[1].1;
                let (left, right) = (p[1].0 - p[0].0, p[3].0 - p[2].0);
                if !close(rise, f.design_depth_m)
                    || !close(left / rise, f.side_slope_h_per_v)
                    || !close(right / rise, f.side_slope_h_per_v)
                {
                    note(
                        "typical-slope",
                        format!(
                            "the side slopes are not {}:1 over the design depth",
                            f.side_slope_h_per_v
                        ),
                    );
                }
            }
            _ => note(
                "typical-dimension",
                "the channel outline is not drawn".into(),
            ),
        }
        match entity(&vp, "ts-wl-line") {
            Some(e)
                if e["from_m"][1]
                    .as_f64()
                    .is_some_and(|y| close(y, f.design_depth_m)) => {}
            _ => note(
                "typical-level",
                "the water level line is not at the design depth above the invert".into(),
            ),
        }
        let drawn_slope = text_of(&vp, "ts-slope").and_then(|t| {
            t.split_whitespace()
                .find_map(|w| w.strip_suffix(":1").and_then(|n| n.parse::<f64>().ok()))
        });
        if !drawn_slope.is_some_and(|n| (n - f.side_slope_h_per_v).abs() <= half_step(1)) {
            note(
                "typical-slope",
                "the slope label is missing or differs from the result".into(),
            );
        }
        // Notes: each value names its source.
        let notes: Vec<&str> = sheet.document["notes"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let has = |start: &str, source: &str| {
            notes
                .iter()
                .any(|n| n.starts_with(start) && n.contains(source))
        };
        let rs = f.ruleset.to_ascii_uppercase();
        for start in ["BOTTOM WIDTH:", "DESIGN DEPTH:", "UNDER-KEEL CLEARANCE:"] {
            if !has(start, &format!("RULESET {rs}")) {
                note(
                    "typical-source",
                    format!("no note `{start}` names ruleset {rs}"),
                );
            }
        }
        if !has("SIDE SLOPE:", "VOLUMES") || !has("DESIGN WATER LEVEL:", "WATER PROFILE") {
            note(
                "typical-source",
                "the side slope and the water level need a note that names their source".into(),
            );
        }
    }
    found
}
