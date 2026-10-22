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
        let rs = f.rule_sets.to_ascii_uppercase();
        for start in ["BOTTOM WIDTH:", "DESIGN DEPTH:", "UNDER-KEEL CLEARANCE:"] {
            if !has(start, &format!("RULE SETS {rs}")) {
                note(
                    "typical-source",
                    format!("no note `{start}` names the Rule Sets {rs}"),
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

/// The checks of the setting-out table: every drawn value is the table's value to the precision it is
/// written to, every row of the table is drawn once, and the CSV holds the same rows.
pub fn setting_out(sheets: &[Sheet], t: &super::setting_out::Table) -> Vec<Finding> {
    use super::setting_out::{bearing_text, chainage_text, coord_text, radius_text};
    let mut found = Vec::new();
    let mut drawn_stations: Vec<Vec<String>> = Vec::new();
    let mut drawn_bends: Vec<Vec<String>> = Vec::new();
    for sheet in sheets {
        let mut note = |check: &'static str, message: String| {
            found.push(Finding {
                sheet: sheet.number.clone(),
                check,
                message,
            });
        };
        for (vp_id, prefix, cols) in [
            ("so-stations", "so-s", 5usize),
            ("so-bends", "so-b", 4usize),
        ] {
            let Some(vp) = sheet.document["viewports"]
                .as_array()
                .and_then(|a| a.iter().find(|v| v["id"] == vp_id))
            else {
                continue;
            };
            let mut r = 0;
            while let Some(first) = text_of(vp, &format!("{prefix}-r{r}-c0")) {
                let cells: Vec<String> = (0..cols)
                    .map(|c| {
                        text_of(vp, &format!("{prefix}-r{r}-c{c}"))
                            .unwrap_or("")
                            .to_string()
                    })
                    .collect();
                let _ = first;
                if vp_id == "so-stations" {
                    drawn_stations.push(cells);
                } else {
                    drawn_bends.push(cells);
                }
                r += 1;
            }
        }
        let header = sheet.document["notes"][0].as_str().unwrap_or("");
        if !header.contains(&format!("EPSG:{}", t.epsg)) || !header.contains("0.001 M") {
            note(
                "setting-out-header",
                "the notes do not state the UTM zone, datum and precision".into(),
            );
        }
    }
    let mut note = |check: &'static str, message: String| {
        found.push(Finding {
            sheet: sheets.first().map(|s| s.number.clone()).unwrap_or_default(),
            check,
            message,
        });
    };
    let want_stations: Vec<Vec<String>> = t
        .stations
        .iter()
        .map(|r| {
            vec![
                chainage_text(r.chainage_m),
                coord_text(r.easting_m),
                coord_text(r.northing_m),
                bearing_text(r.bearing_deg),
                radius_text(r.radius_m),
            ]
        })
        .collect();
    if drawn_stations != want_stations {
        let at = drawn_stations
            .iter()
            .zip(&want_stations)
            .position(|(a, b)| a != b)
            .map_or_else(
                || {
                    format!(
                        "{} rows drawn, {} in the result",
                        drawn_stations.len(),
                        want_stations.len()
                    )
                },
                |i| {
                    format!(
                        "station row {} is drawn {:?}, result {:?}",
                        i + 1,
                        drawn_stations[i],
                        want_stations[i]
                    )
                },
            );
        note("setting-out-station", at);
    }
    if drawn_bends.len() != t.bends.len() {
        note(
            "setting-out-bend",
            format!(
                "{} bend rows drawn, {} in the result",
                drawn_bends.len(),
                t.bends.len()
            ),
        );
    } else {
        for (i, (row, b)) in drawn_bends.iter().zip(&t.bends).enumerate() {
            let want = [
                chainage_text(b.start_m),
                chainage_text(b.end_m),
                radius_text(Some(b.least_radius_m)),
                format!("{:.3}", b.deflection_deg),
            ];
            if row.as_slice() != want {
                note(
                    "setting-out-bend",
                    format!("bend row {} is drawn {row:?}, result {want:?}", i + 1),
                );
            }
        }
    }
    // The CSV: the same rows as the sheets.
    let csv_text = sheets
        .iter()
        .flat_map(|s| s.extra.iter())
        .find(|(n, _)| n.ends_with(".csv"))
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    let rows: Vec<Vec<&str>> = csv_text
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .collect();
    let csv_stations: Vec<&Vec<&str>> = rows.iter().filter(|r| r[0] == "station").collect();
    let csv_bends: Vec<&Vec<&str>> = rows.iter().filter(|r| r[0] == "bend").collect();
    if csv_stations.len() != drawn_stations.len() || csv_bends.len() != drawn_bends.len() {
        note(
            "setting-out-csv",
            "the CSV and the sheets do not hold the same number of rows".into(),
        );
    } else {
        for (c, d) in csv_stations.iter().zip(&drawn_stations) {
            let ch = chainage_in(&format!("CH {}", d[0])).unwrap_or(f64::NAN);
            let same = (c[1].parse::<f64>().unwrap_or(f64::NAN) - ch).abs() < 5e-4
                && c[2] == d[1]
                && c[3] == d[2]
                && c[4] == d[3];
            if !same {
                note(
                    "setting-out-csv",
                    format!("CSV station row {c:?} differs from the sheet row {d:?}"),
                );
                break;
            }
        }
    }
    found
}

fn point_of(v: &Value) -> Option<(f64, f64)> {
    Some((v[0].as_f64()?, v[1].as_f64()?))
}

/// The distance from a point to the infinite line through two points.
fn off_line(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = libm::hypot(dx, dy);
    if len == 0.0 {
        return libm::hypot(p.0 - a.0, p.1 - a.1);
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len
}

/// The chainage of a match-line label `MATCH LINE CH 1+250.0 SEE OWD-X-PL-003`, and the sheet.
fn match_label(text: &str) -> Option<(f64, &str)> {
    let rest = text.strip_prefix("MATCH LINE ")?;
    let (ch, see) = rest.split_once(" SEE ")?;
    Some((chainage_in(ch)?, see))
}

/// A match line's chainage and the sheet it names, if the sheet has one at that end.
type MatchEnd = Option<(f64, String)>;

/// The checks of the plan sheets. Each match line crosses the design line at the chainage it
/// names, and names the sheet that has the matching line; each strip's end is the next strip's
/// start. Each section line crosses the line at the gallery station it is numbered after.
pub fn plan(
    sheets: &[Sheet],
    walk: &SectionVolumes,
    session: &crate::design::Session,
    option_id: &str,
    options: &super::DrawOptions,
) -> Vec<Finding> {
    use crate::design::geojson::all_positions;
    use owd_core::geom::linear;
    let mut out = Vec::new();
    let option = session.document()["options"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["id"] == option_id))
        .cloned()
        .unwrap_or(Value::Null);
    let line = all_positions(&option["line"]);
    let picks = super::gallery::pick(
        walk,
        &option,
        &super::worst_chainages(session, option_id),
        &options.style.gallery(),
    );
    let mut ends: Vec<(String, MatchEnd, MatchEnd)> = Vec::new();
    for sheet in sheets {
        let Some(vp) = sheet.document["viewports"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["id"] == "plan"))
        else {
            out.push(Finding {
                sheet: sheet.number.clone(),
                check: "plan-viewport",
                message: "the sheet has no plan viewport".into(),
            });
            continue;
        };
        let mut found = (None, None);
        for which in ["start", "end"] {
            let Some(e) = entity(vp, &format!("match-{which}")) else {
                continue;
            };
            let Some((ch, see)) =
                text_of(vp, &format!("match-{which}-label")).and_then(match_label)
            else {
                out.push(Finding {
                    sheet: sheet.number.clone(),
                    check: "match-line",
                    message: format!("the {which} match line has no chainage label"),
                });
                continue;
            };
            let at = linear::interpolate(&line, ch).map(|p| (p[0], p[1]));
            let (a, b) = (point_of(&e["from_m"]), point_of(&e["to_m"]));
            let crosses = match (at, a, b) {
                (Some(at), Some(a), Some(b)) => off_line(at, a, b) < 1e-6,
                _ => false,
            };
            if !crosses {
                out.push(Finding {
                    sheet: sheet.number.clone(),
                    check: "match-line",
                    message: format!(
                        "the {which} match line does not cross the design line at CH {ch}"
                    ),
                });
            }
            if which == "start" {
                found.0 = Some((ch, see.to_string()));
            } else {
                found.1 = Some((ch, see.to_string()));
            }
        }
        ends.push((sheet.number.clone(), found.0, found.1));
        for (n, &(si, _)) in picks.iter().enumerate() {
            let Some(e) = entity(vp, &format!("xs-{}", n + 1)) else {
                continue;
            };
            let s = &walk.stations[si];
            let mid = match (point_of(&e["from_m"]), point_of(&e["to_m"])) {
                (Some(a), Some(b)) => ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                _ => (f64::NAN, f64::NAN),
            };
            if libm::hypot(mid.0 - s.x, mid.1 - s.y) > 1e-6
                || text_of(vp, &format!("xs-{}-label", n + 1)) != Some(&format!("S{}", n + 1))
            {
                out.push(Finding {
                    sheet: sheet.number.clone(),
                    check: "section-line",
                    message: format!(
                        "section line S{} is not at the gallery station CH {:.1}",
                        n + 1,
                        s.chainage_m
                    ),
                });
            }
        }
    }
    out.extend(plan_stretches(sheets, session, option_id, options));
    for pair in ends.windows(2) {
        let (this, _, end) = &pair[0];
        let (next, start, _) = &pair[1];
        if end.is_none() && start.is_none() {
            continue; // the overview and the first strip
        }
        let ok = match (end, start) {
            (Some((a, see_a)), Some((b, see_b))) => {
                (a - b).abs() < 1e-9 && see_a == next && see_b == this
            }
            _ => false,
        };
        if !ok {
            out.push(Finding {
                sheet: this.clone(),
                check: "match-line",
                message: format!("its end match line does not meet the start match line of {next}"),
            });
        }
    }
    out
}

/// The constraint stretches on the plan strips: each stretch the `constraint-stretch` rule covers has
/// exactly one label and one band, no label or band names a stretch that is not there, and every
/// value a label writes equals the stretch's own (the deficit and the chainages, to their 0.1 m).
fn plan_stretches(
    sheets: &[Sheet],
    session: &crate::design::Session,
    option_id: &str,
    options: &super::DrawOptions,
) -> Vec<Finding> {
    use super::plan::{CONSTRAINT_KINDS, LabelRule};
    let mut out = Vec::new();
    let stretches = super::stretches(session, option_id);
    let fields = options
        .style
        .plan()
        .rules
        .into_iter()
        .find_map(|r| match r {
            LabelRule::ConstraintStretch { fields } => Some(fields),
            _ => None,
        });
    let strips: Vec<&Sheet> = sheets
        .iter()
        .filter(|s| !s.number.ends_with("-001"))
        .collect();
    let mut labels: std::collections::BTreeMap<usize, Vec<(String, String)>> = Default::default();
    let mut bands: std::collections::BTreeMap<usize, usize> = Default::default();
    for sheet in &strips {
        let Some(vp) = sheet.document["viewports"]
            .as_array()
            .and_then(|a| a.first())
        else {
            continue;
        };
        for e in vp["entities"].as_array().into_iter().flatten() {
            let Some(rest) = e["id"].as_str().and_then(|i| i.strip_prefix("stretch-")) else {
                continue;
            };
            let (n, band) = match rest.split_once("-band-") {
                Some((n, r)) => (n, Some(r)),
                None => (rest, None),
            };
            let Ok(i) = n.parse::<usize>() else { continue };
            match band {
                Some("0") => *bands.entry(i).or_default() += 1,
                Some(_) => {}
                None => labels.entry(i).or_default().push((
                    sheet.number.clone(),
                    e["text"].as_str().unwrap_or("").to_string(),
                )),
            }
        }
    }
    let covered = |i: usize| {
        fields.is_some()
            && stretches
                .get(i)
                .and_then(|x| x["kind"].as_str())
                .is_some_and(|k| CONSTRAINT_KINDS.contains(&k))
    };
    let first = strips.first().map_or(String::new(), |s| s.number.clone());
    for i in 0..stretches
        .len()
        .max(labels.keys().chain(bands.keys()).max().map_or(0, |m| m + 1))
    {
        let (n_labels, n_bands) = (
            labels.get(&i).map_or(0, Vec::len),
            bands.get(&i).copied().unwrap_or(0),
        );
        let want = usize::from(covered(i));
        if n_labels != want || n_bands != want {
            out.push(Finding {
                sheet: labels.get(&i).and_then(|l| l.first()).map_or(first.clone(), |l| l.0.clone()),
                check: "constraint-stretch",
                message: format!(
                    "stretch {i} has {n_labels} labels and {n_bands} bands on the plan; it should have {want} of each"
                ),
            });
            continue;
        }
        let (Some((sheet, text)), Some(x)) =
            (labels.get(&i).and_then(|l| l.first()), stretches.get(i))
        else {
            continue;
        };
        let mut wrong = Vec::new();
        for part in text.split(" · ") {
            if let Some(ch) = part.strip_prefix("CH ") {
                let (a, b) = ch.split_once(" TO ").unwrap_or((ch, ch));
                let start = x["start_chainage_m"].as_f64().unwrap_or(f64::NAN);
                let end = x["end_chainage_m"].as_f64().unwrap_or(f64::NAN);
                let ok = |t: &str, v: f64| {
                    chainage_in(&format!("CH {t}")).is_some_and(|c| (c - v).abs() <= half_step(1))
                };
                if !ok(a, start) || !ok(b, end) {
                    wrong.push(format!("chainage `{part}`"));
                }
            } else if let Some(d) = part.strip_suffix(" M").and_then(|d| d.parse::<f64>().ok()) {
                let want = x["deficit"].as_f64().unwrap_or(f64::NAN);
                if (d - want).abs() > half_step(1) {
                    wrong.push(format!("deficit {d} (the result is {want})"));
                }
            }
        }
        if !wrong.is_empty() {
            out.push(Finding {
                sheet: sheet.clone(),
                check: "constraint-stretch",
                message: format!(
                    "the label of stretch {i} differs from its result: {}",
                    wrong.join(", ")
                ),
            });
        }
    }
    out
}
