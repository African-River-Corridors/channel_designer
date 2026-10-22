//! The long section (spec 0004, ticket 10): the design line option along its chainage, with the
//! river bed, the water level and the design bed (the invert), drawn with a vertical exaggeration
//! (Drawing Format Standard 1.2, clause 7.3.6).
//!
//! Every number is a series of [`Session::long_section`], the call the app's long-section view
//! makes, so the sheet and the app draw the same values: the station walk the volumes stage sums
//! (`SectionVolumes`), with the bed read at the design line (the offset nearest zero), as the
//! gallery reads its ground. Nothing is computed here but the layout and the factor.
//!
//! The scale. The horizontal scale is the first of the style's `scales` at which the whole line
//! fits the plot at one sheet; when none does, the last, and the line is split over sheets at it.
//! The vertical exaggeration `E` is the style's whole number, or, with `"auto"`, the smallest `E`
//! of 2 or more whose levels fill at least `auto_min_fill` of the plot height, with `N / E` a ratio
//! of the standard (clause 7.3.6.2). Levels that already fill it get no exaggeration.

use super::furniture::{self, Area, Titles, clean};
use super::gallery::chainage_text;
use super::{DrawError, DrawOptions, Sheet, SheetKind};
use crate::design::Session;
use otd::rules::{ratios, text_width, vertical_denominator};
use owd_core::volumes::sections::SectionVolumes;
use serde_json::{Value, json};

/// The vertical exaggeration the style asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exaggeration {
    /// The smallest factor that makes the depth readable (see the module notes).
    Auto,
    /// This factor, a whole number of 2 or more.
    Factor(u32),
}

/// The numbers the long section reads, all from the sheet style (`Style::long_section`).
#[derive(Debug, Clone, PartialEq)]
pub struct LongSectionParams {
    pub sheet_size: String,
    pub landscape: bool,
    /// The horizontal scales to try, largest scale first (denominators of the standard's ratios).
    pub scales: Vec<u32>,
    pub exaggeration: Exaggeration,
    /// With `Exaggeration::Auto`: the least share of the plot height that the levels must fill.
    pub auto_min_fill: f64,
    /// Room under the lowest level, and above the highest, in metres.
    pub below_invert_m: f64,
    pub above_ground_m: f64,
    /// The bands of the plot for axis labels, headings and the scale bars, in millimetres.
    pub band_left_mm: f64,
    pub band_right_mm: f64,
    pub band_top_mm: f64,
    pub band_bottom_mm: f64,
    /// The least paper distance between two chainage labels, and between two level labels.
    pub chainage_pitch_mm: f64,
    pub level_pitch_mm: f64,
    /// The optional layers, on or off.
    pub grid: bool,
    pub water: bool,
}

const LAYERS: [&str; 5] = ["OUTLINE", "OBJECT", "WATER", "TEXT", "GRID"];
/// Text heights of the standard (clause 11.2) used here.
const H_TITLE: f64 = 3.5;
const H_TEXT: f64 = 2.5;
/// Space between two items of one heading line, in millimetres.
const ITEM_GAP_MM: f64 = 4.0;
/// The standard version: the vertical exaggeration is from 1.2 (clause 18.2.1).
pub(super) const VERSION: &str = "1.2";
const VERSION_MINOR: u32 = 2;

/// The series of the long section, read from [`Session::long_section`]: one value a station,
/// `None` where the walk has none (a bed off the DEM).
pub(super) struct Series {
    pub chainage: Vec<f64>,
    pub bed: Vec<Option<f64>>,
    pub wl: Vec<Option<f64>>,
    pub invert: Vec<Option<f64>>,
    pub station_m: f64,
}

impl Series {
    pub(super) fn read(v: &Value) -> Series {
        let col = |k: &str| -> Vec<Option<f64>> {
            v[k].as_array()
                .map(|a| a.iter().map(Value::as_f64).collect())
                .unwrap_or_default()
        };
        Series {
            chainage: col("chainage_m").into_iter().flatten().collect(),
            bed: col("bed_m"),
            wl: col("water_level_m"),
            invert: col("design_invert_m"),
            station_m: v["station_m"].as_f64().unwrap_or(f64::NAN),
        }
    }
}

/// The level labels: whole metres, a step of 1, 2 or 5 times a power of ten.
fn nice_step(min: f64) -> f64 {
    let mut mag = 1.0;
    loop {
        for m in [1.0, 2.0, 5.0] {
            if m * mag >= min {
                return m * mag;
            }
        }
        mag *= 10.0;
    }
}

/// The text of a chainage label on the axis: kilometres, a plus, whole metres (`1+500`).
pub(super) fn axis_chainage_text(m: f64) -> String {
    let whole = m.round() as i64;
    format!("{}+{:03}", whole / 1000, whole % 1000)
}

/// The text of a level label on the axis: whole metres with a sign (`+38`).
pub(super) fn axis_level_text(z: f64) -> String {
    format!("{:+}", z.round() as i64)
}

/// The id part of a number: a minus is `m`, so an id has no `-` of its own meaning.
pub(super) fn id_number(x: f64) -> String {
    (x.round() as i64).to_string().replace('-', "m")
}

/// The words of the factor in the notes (clause 14.4.4: the scales and the factor in words).
pub(super) fn factor_note(n: u32, e: u32) -> String {
    if e > 1 {
        format!(
            "VERTICAL EXAGGERATION \u{00d7}{e}: LEVELS DRAWN AT 1:{}, CHAINAGE AT 1:{n}.",
            n / e
        )
    } else {
        format!("NO VERTICAL EXAGGERATION: LEVELS AND CHAINAGE BOTH AT 1:{n}.")
    }
}

/// The paper room of the plot: the frame less its bands.
fn plot_size(frame: Area, p: &LongSectionParams) -> (f64, f64) {
    (
        frame.w - p.band_left_mm - p.band_right_mm,
        frame.h - p.band_top_mm - p.band_bottom_mm,
    )
}

/// The horizontal scale, the factor, and whether the line must be split over sheets.
///
/// The scale is the first of `scales` at which the line's length and its level range both fit the
/// plot (at the factor, when the style fixes one); if none holds the length, the last that holds the
/// levels, and the line is split. With `Auto`, the factor is then the smallest `E >= 2` with an
/// allowed `N / E` whose level range fills at least `auto_min_fill` of the plot height and still
/// fits it; if none reaches that share, the largest that fits; and 1 when the levels at one scale
/// already fill it.
pub(super) fn choose(
    length_m: f64,
    range_m: f64,
    plot_w: f64,
    plot_h: f64,
    p: &LongSectionParams,
) -> Result<(u32, u32, bool), DrawError> {
    let allowed = ratios(VERSION_MINOR);
    let fixed = match p.exaggeration {
        Exaggeration::Auto => 1,
        Exaggeration::Factor(e) => e,
    };
    let height_fits = |n: u32| range_m * 1000.0 / f64::from(n) * f64::from(fixed) <= plot_h + 1e-9;
    let usable: Vec<u32> = p
        .scales
        .iter()
        .copied()
        .filter(|&n| fixed == 1 || vertical_denominator(n, fixed, &allowed).is_some())
        .filter(|&n| height_fits(n))
        .collect();
    let Some(&last) = usable.last() else {
        return Err(DrawError::Input(match p.exaggeration {
            Exaggeration::Factor(e) => format!(
                "the long section's vertical exaggeration \u{00d7}{e} fits no scale of the sheet style \
                 ({:?}): at each, either the levels do not fit the sheet or 1:N / {e} is not a ratio \
                 of the standard (clause 7.3.6.2); choose another factor, or \"auto\"",
                p.scales
            ),
            Exaggeration::Auto => format!(
                "the levels of the long section do not fit a {} sheet at any scale in the sheet \
                 style ({:?})",
                p.sheet_size, p.scales
            ),
        }));
    };
    let fits_length = |n: u32| length_m * 1000.0 / f64::from(n) <= plot_w + 1e-9;
    let (n, split) = match usable.iter().copied().find(|&n| fits_length(n)) {
        Some(n) => (n, false),
        None => (last, true),
    };
    let e = match p.exaggeration {
        Exaggeration::Factor(e) => e,
        Exaggeration::Auto => {
            let k = 1000.0 / f64::from(n);
            let want = p.auto_min_fill * plot_h;
            if range_m * k >= want {
                1
            } else {
                // the factors of N whose vertical scale is a ratio, smallest first
                let mut best = 1;
                for e in (2..=n).filter(|e| vertical_denominator(n, *e, &allowed).is_some()) {
                    let drawn = range_m * k * f64::from(e);
                    if drawn > plot_h + 1e-9 {
                        break;
                    }
                    best = e;
                    if drawn >= want {
                        break;
                    }
                }
                best
            }
        }
    };
    Ok((n, e, split))
}

fn pt(x: f64, y: f64) -> Value {
    json!([x, y])
}

fn text(id: &str, x: f64, y: f64, s: &str, h: f64, h_align: &str, v_align: &str) -> Value {
    json!({
        "type": "text", "layer": "TEXT", "id": id, "anchor_m": pt(x, y), "text": s,
        "height_mm": h, "align_h": h_align, "align_v": v_align,
    })
}

/// Runs of consecutive stations of `range` at which `series` has a value.
fn runs(range: &[usize], series: &[Option<f64>]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for &i in range {
        if series.get(i).copied().flatten().is_some_and(f64::is_finite) {
            cur.push(i);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Is this layer drawn? Only the optional layers of the style can be off.
fn layer_on(p: &LongSectionParams, layer: &str) -> bool {
    match layer {
        "GRID" => p.grid,
        "WATER" => p.water,
        _ => true,
    }
}

/// The geometry of one sheet's viewport.
struct Frame {
    area: Area,
    /// Horizontal and vertical millimetres per metre.
    k: f64,
    kv: f64,
    /// The model window.
    x0: f64,
    y0: f64,
}

impl Frame {
    fn mx(&self, px: f64) -> f64 {
        self.x0 + px / self.k
    }
    fn my(&self, py: f64) -> f64 {
        self.y0 + (self.area.h - py) / self.kv
    }
}

fn line_width(items: &[(String, String)], h: f64) -> f64 {
    let widths: f64 = items
        .iter()
        .map(|(_, t)| text_width(t, h).unwrap_or(0.0))
        .sum();
    widths + ITEM_GAP_MM * items.len().saturating_sub(1) as f64
}

/// The heading items at one end of a sheet: its chainage, then the bed, water level and invert
/// there (an item the walk has no value for is left out).
fn end_items(end: &str, s: &Series, i: usize, p: &LongSectionParams) -> Vec<(String, String)> {
    let mut items = vec![(
        format!("ls-{end}-chainage"),
        format!(
            "{} CH {}",
            end.to_ascii_uppercase(),
            chainage_text(s.chainage[i])
        ),
    )];
    let mut push = |key: &str, word: &str, v: Option<f64>| {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            items.push((format!("ls-{end}-{key}"), format!("{word} {v:+.2}")));
        }
    };
    push("bed", "BED", s.bed[i]);
    if p.water {
        push("wl", "WL", s.wl[i]);
    }
    push("invert", "INVERT", s.invert[i]);
    items
}

#[allow(clippy::too_many_arguments)]
fn viewport(
    f: &Frame,
    s: &Series,
    stations: &[usize],
    (first, last): (usize, usize),
    n: u32,
    e: u32,
    p: &LongSectionParams,
) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let (w_mm, h_mm) = (f.area.w, f.area.h);
    let (px0, px1) = (p.band_left_mm, w_mm - p.band_right_mm);
    let (py0, py1) = (p.band_top_mm, h_mm - p.band_bottom_mm);
    // the plot box, on paper and in the model
    let (cx0, cx1) = (f.mx(px0), f.mx(px1));
    let (cz0, cz1) = (f.my(py1), f.my(py0));
    out.push(json!({
        "type": "polyline", "layer": "OBJECT", "id": "ls-plot", "closed": true,
        "points_m": [pt(cx0, cz0), pt(cx1, cz0), pt(cx1, cz1), pt(cx0, cz1)],
    }));

    // headings: the title, then the values at the first and the last station of the sheet
    out.push(text(
        "ls-title",
        f.mx(px0),
        f.my(6.0),
        "LONG SECTION",
        H_TITLE,
        "left",
        "baseline",
    ));
    let start = end_items("start", s, first, p);
    let mut x = px0;
    for (id, t) in &start {
        out.push(text(id, f.mx(x), f.my(12.0), t, H_TEXT, "left", "baseline"));
        x += text_width(t, H_TEXT).unwrap_or(0.0) + ITEM_GAP_MM;
    }
    let end = end_items("end", s, last, p);
    let mut x = px1 - line_width(&end, H_TEXT);
    for (id, t) in &end {
        out.push(text(id, f.mx(x), f.my(12.0), t, H_TEXT, "left", "baseline"));
        x += text_width(t, H_TEXT).unwrap_or(0.0) + ITEM_GAP_MM;
    }

    // the level axis on the left of the plot, the chainage axis under it
    let z_step = nice_step(p.level_pitch_mm / f.kv);
    let mut z = (cz0 / z_step).ceil() * z_step;
    while z <= cz1 + 1e-9 {
        let id = id_number(z);
        out.push(json!({
            "type": "line", "layer": "GRID", "id": format!("ls-ztick-{id}"),
            "from_m": pt(f.mx(px0 - 1.0), z), "to_m": pt(cx0, z),
        }));
        out.push(text(
            &format!("ls-zlabel-{id}"),
            f.mx(px0 - 1.5),
            z,
            &axis_level_text(z),
            H_TEXT,
            "right",
            "middle",
        ));
        z += z_step;
    }
    let c_step = nice_step(p.chainage_pitch_mm / f.k);
    let mut c = (cx0.max(0.0) / c_step).ceil() * c_step;
    while c <= cx1 + 1e-9 {
        let id = id_number(c);
        out.push(json!({
            "type": "line", "layer": "GRID", "id": format!("ls-ctick-{id}"),
            "from_m": pt(c, cz0), "to_m": pt(c, f.my(py1 + 1.0)),
        }));
        out.push(text(
            &format!("ls-clabel-{id}"),
            c,
            f.my(py1 + 1.5),
            &axis_chainage_text(c),
            H_TEXT,
            "centre",
            "top",
        ));
        c += c_step;
    }

    // the series: river bed (thin), water level (dashed), design bed (thick)
    let mut series = |key: &str, layer: &str, values: &[Option<f64>], dashed: bool| {
        for (j, run) in runs(stations, values).into_iter().enumerate() {
            if run.len() < 2 {
                continue;
            }
            let pts: Vec<Value> = run
                .iter()
                .map(|&i| pt(s.chainage[i], values[i].unwrap_or(f64::NAN)))
                .collect();
            let mut e = json!({
                "type": "polyline", "layer": layer, "id": format!("ls-{key}-{}", j + 1),
                "closed": false, "points_m": pts,
            });
            if dashed {
                e["line_type"] = json!("dashed");
            }
            out.push(e);
        }
    };
    series("bed", "OBJECT", &s.bed, false);
    series("wl", "WATER", &s.wl, true);
    series("invert", "OUTLINE", &s.invert, false);

    out.retain(|x| layer_on(p, x["layer"].as_str().unwrap_or("")));
    let mut vp = json!({
        "id": "ls",
        "frame": {"x_mm": f.area.x, "y_mm": f.area.y, "width_mm": w_mm, "height_mm": h_mm},
        "window": {"xmin_m": f.x0, "ymin_m": f.y0,
                   "xmax_m": f.x0 + w_mm / f.k, "ymax_m": f.y0 + h_mm / f.kv},
        "scale": n,
        "scale_bar": {},
        "entities": out,
    });
    if e > 1 {
        vp["vertical_exaggeration"] = json!(e);
    }
    vp
}

/// The key viewport: what each line is. It has no scale bar: the long section's own carries both.
fn key_viewport(area: Area, k: f64, scale: u32, p: &LongSectionParams) -> Value {
    let (wm, hm) = (area.w / k, area.h / k);
    let mx = |px: f64| px / k;
    let my = |py: f64| (area.h - py) / k;
    let mut e: Vec<Value> = vec![text(
        "key-heading",
        mx(70.0),
        my(7.0),
        "KEY",
        H_TITLE,
        "left",
        "baseline",
    )];
    let all = [
        (
            "OBJECT",
            None,
            "RIVER BED: THE GROUND ON THE DESIGN LINE (BED)",
        ),
        ("WATER", Some("dashed"), "DESIGN WATER LEVEL (WL)"),
        ("OUTLINE", None, "DESIGN BED (INVERT)"),
    ];
    let rows = all.iter().filter(|(layer, _, _)| layer_on(p, layer));
    for (row, (layer, line_type, label)) in rows.enumerate() {
        let base = 14.0 + 6.0 * row as f64;
        let id = format!("key-row-{}", row + 1);
        let mut line = json!({
            "type": "line", "layer": layer, "id": format!("{id}-swatch"),
            "from_m": [mx(70.0), my(base - 1.2)], "to_m": [mx(80.0), my(base - 1.2)],
        });
        if let Some(t) = line_type {
            line["line_type"] = json!(t);
        }
        e.push(line);
        e.push(text(
            &id,
            mx(83.0),
            my(base),
            label,
            H_TEXT,
            "left",
            "baseline",
        ));
    }
    json!({
        "id": "key",
        "frame": {"x_mm": area.x, "y_mm": area.y, "width_mm": area.w, "height_mm": area.h},
        "window": {"xmin_m": 0, "ymin_m": 0, "xmax_m": wm, "ymax_m": hm},
        "scale": scale,
        "entities": e,
    })
}

pub(super) fn build(
    session: &Session,
    option_id: &str,
    walk: &SectionVolumes,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let p = &options.style.long_section();
    let doc = session.document();
    let option = doc["options"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["id"] == option_id))
        .ok_or_else(|| DrawError::Input(format!("there is no option `{option_id}`")))?;
    let s = Series::read(&session.long_section(option_id, walk)?);
    if s.chainage.len() < 2 {
        return Err(DrawError::Missing(format!(
            "option `{option_id}` has fewer than two stations to draw a long section"
        )));
    }
    // the level range of the whole line, so every sheet of it has the same levels
    let levels: Vec<f64> = s
        .bed
        .iter()
        .chain(&s.invert)
        .chain(if p.water { &s.wl[..] } else { &[] })
        .filter_map(|v| v.filter(|v| v.is_finite()))
        .collect();
    if levels.is_empty() {
        return Err(DrawError::Missing(format!(
            "option `{option_id}` has no levels to draw"
        )));
    }
    let zmin = levels.iter().copied().fold(f64::INFINITY, f64::min) - p.below_invert_m;
    let zmax = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max) + p.above_ground_m;
    let (c_first, c_last) = (s.chainage[0], s.chainage[s.chainage.len() - 1]);

    let notes_len = 5;
    let layout = furniture::layout(&p.sheet_size, p.landscape, notes_len).ok_or_else(|| {
        DrawError::Input(format!(
            "the sheet size `{}` is not one of the standard's",
            p.sheet_size
        ))
    })?;
    let frame_area = layout.free;
    let (plot_w, plot_h) = plot_size(frame_area, p);
    if plot_w <= 0.0 || plot_h <= 0.0 || frame_area.h < otd::rules::MIN_EXAGGERATED_FRAME_HEIGHT {
        return Err(DrawError::Input(format!(
            "a {} sheet leaves no room for the long section's plot and bands",
            p.sheet_size
        )));
    }
    let (n, e, split) = choose(c_last - c_first, zmax - zmin, plot_w, plot_h, p)?;
    let k = 1000.0 / f64::from(n);
    let kv = k * f64::from(e);

    // the stations of each sheet: the whole line, or runs of at most a plot's length; the last
    // station of a sheet is the first of the next, so the lines meet
    let last_i = s.chainage.len() - 1;
    let ranges: Vec<(usize, usize)> = if split {
        let len = plot_w / k;
        let mut out = Vec::new();
        let mut lo = 0;
        while lo < last_i {
            let hi = (lo + 1..=last_i)
                .take_while(|&i| s.chainage[i] - s.chainage[lo] <= len + 1e-9)
                .last()
                .unwrap_or(lo + 1);
            out.push((lo, hi));
            lo = hi;
        }
        out
    } else {
        vec![(0, last_i)]
    };

    let title = doc["project"]["title"].as_str().unwrap_or("DESIGN");
    let option_name = option["name"].as_str().unwrap_or(option_id);
    let date = options
        .date
        .clone()
        .or_else(|| doc["project"]["created"].as_str().map(str::to_string))
        .ok_or_else(|| {
            DrawError::Missing("the design has no created date for the title block".into())
        })?;
    let code = furniture::design_code(&clean(title), SheetKind::LongSection.code());
    let model = walk
        .stations
        .first()
        .map_or("?", |s| s.model.as_str())
        .trim_start_matches("cut_model_")
        .to_ascii_uppercase();
    let notes = vec![
        factor_note(n, e),
        "LEVELS AND CHAINAGES IN METRES. BED IS THE GROUND ON THE DESIGN LINE; INVERT IS THE DESIGN BED.".to_string(),
        format!(
            "VALUES AT THE STATIONS OF THE VOLUMES STAGE (CUT MODEL {model}), EVERY {} M.",
            s.station_m
        ),
        "SCREENING – NOT FOR CONSTRUCTION.".to_string(),
        format!(
            "SHEET STYLE {} SHA-256 {}",
            clean(options.style.name()),
            options.style.hash()
        ),
    ];
    debug_assert_eq!(notes.len(), notes_len);

    let layer_names: Vec<&str> = LAYERS.iter().copied().filter(|l| layer_on(p, l)).collect();
    let count = ranges.len();
    let mut out = Vec::new();
    for (si, &(lo, hi)) in ranges.iter().enumerate() {
        let stations: Vec<usize> = (lo..=hi).collect();
        // the window: the sheet's chainage from the plot's left edge, the levels centred in it
        let slack = plot_h - (zmax - zmin) * kv;
        let f = Frame {
            area: frame_area,
            k,
            kv,
            x0: s.chainage[lo] - p.band_left_mm / k,
            y0: zmin - (p.band_bottom_mm + slack / 2.0) / kv,
        };
        let mut viewports = vec![viewport(&f, &s, &stations, (lo, hi), n, e, p)];
        viewports.push(key_viewport(layout.key, k, n, p));
        let number = furniture::drawing_number(&code, SheetKind::LongSection.code(), si + 1);
        let titles = Titles {
            design: clean(title),
            option_name: option_name.to_string(),
            sheet_title: "Long section".into(),
            number: number.clone(),
            revision: options.revision.clone(),
            date: date.clone(),
            sheet_number: si + 1,
            sheet_count: count,
        };
        let mut document = furniture::document(
            &p.sheet_size,
            p.landscape,
            &layout,
            &titles,
            &notes,
            &layer_names,
            viewports,
        );
        document["standard_version"] = json!(VERSION);
        out.push(Sheet {
            kind: SheetKind::LongSection,
            number,
            document,
            extra: Vec::new(),
            warnings: Vec::new(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(e: Exaggeration) -> LongSectionParams {
        crate::drawing::Style::builtin()
            .long_section()
            .with_exaggeration(e)
    }

    impl LongSectionParams {
        fn with_exaggeration(mut self, e: Exaggeration) -> Self {
            self.exaggeration = e;
            self
        }
    }

    #[test]
    fn the_clause_7_3_6_2_example_ratios_hold() {
        // 1:5000 with E = 10 gives 1:500; with E = 3 it is not a ratio (clause 7.3.6.2)
        assert_eq!(vertical_denominator(5000, 10, &ratios(2)), Some(500));
        assert_eq!(vertical_denominator(5000, 3, &ratios(2)), None);
    }

    #[test]
    fn auto_picks_the_smallest_allowed_factor_that_fills_the_share() {
        let p = params(Exaggeration::Auto);
        // 3000 m on a 700 mm plot: 1:5000 holds it (600 mm). 10 m of levels at 1:5000 is 2 mm;
        // half of a 400 mm plot is 200 mm, so E = 100 (1:50, 200 mm) is the first that fills it.
        let (n, e, split) = choose(3000.0, 10.0, 700.0, 400.0, &p).unwrap();
        assert_eq!((n, split), (5000, false));
        assert_eq!(e, 100);
        assert!(vertical_denominator(n, e, &ratios(2)).is_some());
        // levels that already fill the plot get no exaggeration
        let (_, e, _) = choose(3000.0, 1500.0, 700.0, 400.0, &p).unwrap();
        assert_eq!(e, 1);
    }

    #[test]
    fn a_fixed_factor_skips_scales_where_n_over_e_is_not_a_ratio() {
        let p = params(Exaggeration::Factor(4));
        // 1:5000 / 4 = 1:1250 is a house ratio; 1:1000 / 4 = 1:250 too
        let (n, e, _) = choose(500.0, 10.0, 700.0, 400.0, &p).unwrap();
        assert_eq!((n, e), (1000, 4));
        let p = params(Exaggeration::Factor(3));
        let err = choose(500.0, 10.0, 700.0, 400.0, &p)
            .unwrap_err()
            .to_string();
        assert!(err.contains("vertical exaggeration ×3"), "{err}");
        assert!(err.contains("7.3.6.2"), "{err}");
    }

    #[test]
    fn axis_texts_are_whole_metres() {
        assert_eq!(axis_chainage_text(0.0), "0+000");
        assert_eq!(axis_chainage_text(12_500.0), "12+500");
        assert_eq!(axis_level_text(38.0), "+38");
        assert_eq!(axis_level_text(-2.0), "-2");
        assert_eq!(id_number(-2.0), "m2");
    }
}
