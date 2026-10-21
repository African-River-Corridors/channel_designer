//! The cross-section gallery (spec 0004, ticket 1): sections along the design line, all at one
//! scale, packed onto as many sheets as needed.
//!
//! Which sections: one every `section_spacing_m`, and the middle (the tightest point) of each tight
//! bend. Each panel is a viewport that shows the ground, the design cut face, the water level and
//! the dredge and dry cut hatched, with their areas in the heading. Every number is a result of the
//! station walk (`SectionVolumes`); nothing is computed here but the layout of the panels.
//!
//! The standard sets one scale for a viewport and gives no clip, so each panel's window is the
//! extent of what it draws, with bands for its headings and axis labels, and its frame is the
//! window at the common scale. The sheet scale is the largest in `scales` at which every panel
//! fits and at least `min_columns` panels stand side by side.

use super::furniture::{self, Area, Titles, clean};
use super::{DrawError, DrawOptions, Sheet, SheetKind};
use crate::design::Session;
use crate::design::map::option_tight_bends;
use otd::rules::text_width;
use owd_core::volumes::sections::{CrossSection, SectionVolumes};
use serde_json::{Value, json};

/// The numbers the gallery reads, all from the sheet style (`Style::gallery`).
#[derive(Debug, Clone, PartialEq)]
pub struct GalleryParams {
    pub sheet_size: String,
    pub landscape: bool,
    /// Sections at this chainage spacing, in metres.
    pub section_spacing_m: f64,
    /// Add the middle of each tight bend.
    pub bend_sections: bool,
    /// Add the worst station of each constraint stretch of the screening.
    pub worst_sections: bool,
    /// The scales to try, largest scale first (denominators of the standard's ratios).
    pub scales: Vec<u32>,
    /// The fewest panels side by side that a scale must allow.
    pub min_columns: usize,
    /// Space between panel frames, in millimetres.
    pub gap_mm: f64,
    /// Room round the wet channel, in metres.
    pub margin_m: f64,
    /// The least half-width of a panel, in metres.
    pub min_half_m: f64,
    /// Room under the design invert, and above the highest ground, in metres.
    pub below_invert_m: f64,
    pub above_ground_m: f64,
    /// The bands of a panel for axis labels and headings, in millimetres.
    pub band_left_mm: f64,
    pub band_right_mm: f64,
    pub band_top_mm: f64,
    pub band_bottom_mm: f64,
    /// The least paper distance between two axis labels, in millimetres.
    pub label_pitch_mm: f64,
    /// A cut thinner than this, in metres, is not hatched.
    pub hatch_min_cut_m: f64,
    /// The optional layers, on or off.
    pub grid: bool,
    pub hatch: bool,
    pub water: bool,
}

const LAYERS: [&str; 7] = [
    "OUTLINE",
    "OBJECT",
    "WATER",
    "HATCH",
    "TEXT",
    "GRID",
    "DIMENSIONS",
];
/// Text heights of the standard (clause 11.2) used here.
const H_TITLE: f64 = 3.5;
const H_TEXT: f64 = 2.5;
/// Space between two items of one heading line, in millimetres.
const ITEM_GAP_MM: f64 = 4.0;
/// Panel `n` keeps its model coordinates `n` strides along x, so that no two windows overlap in the
/// DXF's model space (clause 15.4.9). The offset of a point is its x less this shift.
pub(super) const PANEL_STRIDE_M: f64 = 10_000.0;

/// The sections the gallery shows, in chainage order, each with whether a bend added it.
pub(super) fn pick(
    walk: &SectionVolumes,
    option: &Value,
    worst: &[f64],
    params: &GalleryParams,
) -> Vec<(usize, bool)> {
    let st = &walk.stations;
    if st.is_empty() {
        return Vec::new();
    }
    let nearest = |s: f64| {
        (0..st.len())
            .min_by(|&a, &b| {
                (st[a].chainage_m - s)
                    .abs()
                    .total_cmp(&(st[b].chainage_m - s).abs())
            })
            .unwrap_or(0)
    };
    let last = st[st.len() - 1].chainage_m;
    let mut picked: std::collections::BTreeMap<usize, bool> = std::collections::BTreeMap::new();
    if params.section_spacing_m > 0.0 {
        let mut i = 0usize;
        loop {
            let at = i as f64 * params.section_spacing_m;
            if at > last + 1e-9 {
                break;
            }
            picked.entry(nearest(at)).or_insert(false);
            i += 1;
        }
    }
    if params.bend_sections {
        for (chainage, _) in option_tight_bends(option) {
            picked.insert(nearest(chainage), true);
        }
    }
    if params.worst_sections {
        // a station already drawn is drawn once (a map keeps one entry a station)
        for &chainage in worst {
            picked.entry(nearest(chainage)).or_insert(false);
        }
    }
    picked.into_iter().collect()
}

/// What one panel draws and how big it is on paper.
struct Panel<'a> {
    section: &'a CrossSection,
    number: usize,
    bend: bool,
    lo: f64,
    hi: f64,
    zmin: f64,
    zmax: f64,
    /// Frame size at the common scale.
    w_mm: f64,
    h_mm: f64,
}

/// The ground after construction: the cut face where it is below the ground, else the ground.
fn design_surface(s: &CrossSection, j: usize) -> f64 {
    let (t, c) = (s.terrain[j], s.cutface[j]);
    if c.is_nan() { t } else { c.min(t) }
}

fn finite(x: f64) -> bool {
    x.is_finite()
}

/// The extent a panel must show: the whole wet channel and the cut, with room round them.
pub(super) fn extent(s: &CrossSection, p: &GalleryParams) -> (f64, f64, f64, f64) {
    let idx: Vec<usize> = (0..s.offsets.len())
        .filter(|&j| design_surface(s, j) < s.wl || (s.terrain[j] - s.cutface[j]) > 1e-6)
        .collect();
    let (lo, hi) = if idx.is_empty() {
        (-p.min_half_m, p.min_half_m)
    } else {
        (
            s.offsets[idx[0]].min(s.offsets[idx[idx.len() - 1]]) - p.margin_m,
            s.offsets[idx[0]].max(s.offsets[idx[idx.len() - 1]]) + p.margin_m,
        )
    };
    let half = lo.abs().max(hi.abs()).max(p.min_half_m);
    let (lo, hi) = (-half, half);
    let sel: Vec<usize> = (0..s.offsets.len())
        .filter(|&j| s.offsets[j] >= lo && s.offsets[j] <= hi)
        .collect();
    let vals = |f: &dyn Fn(usize) -> f64| -> Vec<f64> {
        sel.iter().map(|&j| f(j)).filter(|v| finite(*v)).collect()
    };
    let ground = vals(&|j| s.terrain[j]);
    let cut = vals(&|j| s.cutface[j]);
    let hi_z = ground.iter().copied().fold(s.wl.max(s.grade), f64::max) + p.above_ground_m;
    let lo_z = ground
        .iter()
        .chain(cut.iter())
        .copied()
        .fold(s.grade - p.below_invert_m, f64::min)
        .min(s.wl - 1.0)
        - 1.0;
    (lo, hi, lo_z, hi_z)
}

fn n1(x: f64) -> String {
    format!("{x:.1}")
}

/// The text of a chainage: kilometres, a plus, metres to 0.1 (`1+500.3`).
pub(super) fn chainage_text(m: f64) -> String {
    let tenths = (m * 10.0).round() as i64;
    format!(
        "{}+{:05.1}",
        tenths / 10_000,
        (tenths % 10_000) as f64 / 10.0
    )
}

/// The heading items of a panel: `(entity id suffix, text)`, one list per heading line.
fn heading(s: &CrossSection, number: usize, bend: bool) -> [Vec<(&'static str, String)>; 3] {
    let title = if bend {
        format!("SECTION {number}  MID-BEND")
    } else {
        format!("SECTION {number}")
    };
    [
        vec![
            ("title", title),
            ("chainage", format!("CH {}", chainage_text(s.chainage_m))),
        ],
        vec![
            ("dredge", format!("DREDGE {} M²", n1(s.dredge_area_m2))),
            ("dry", format!("DRY CUT {} M²", n1(s.excav_area_m2))),
        ],
        vec![
            ("wl", format!("WL {:+.2}", s.wl)),
            ("invert", format!("INVERT {:+.2}", s.grade)),
            ("bottom", format!("BOTTOM {} M", n1(2.0 * s.hb))),
        ],
    ]
}

fn line_width(items: &[(&'static str, String)], h: f64) -> f64 {
    let widths: f64 = items
        .iter()
        .map(|(_, t)| text_width(t, h).unwrap_or(0.0))
        .sum();
    widths + ITEM_GAP_MM * items.len().saturating_sub(1) as f64
}

fn panel_size(panel: &Panel, k: f64, p: &GalleryParams) -> (f64, f64) {
    let content_w = (panel.hi - panel.lo) * k + p.band_left_mm + p.band_right_mm;
    let content_h = (panel.zmax - panel.zmin) * k + p.band_top_mm + p.band_bottom_mm;
    let lines = heading(panel.section, panel.number, panel.bend);
    let text_w = lines
        .iter()
        .enumerate()
        .map(|(i, l)| line_width(l, if i == 0 { H_TITLE } else { H_TEXT }))
        .fold(0.0, f64::max)
        + 2.0 * 2.0;
    (content_w.max(text_w), content_h)
}

/// A step of 1, 2 or 5 times a power of ten that is at least `min` (for axis labels).
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

fn pt(x: f64, y: f64) -> Value {
    json!([x, y])
}

fn text(id: &str, x: f64, y: f64, s: &str, h: f64, h_align: &str, v_align: &str) -> Value {
    json!({
        "type": "text", "layer": "TEXT", "id": id, "anchor_m": pt(x, y), "text": s,
        "height_mm": h, "align_h": h_align, "align_v": v_align,
    })
}

/// Runs of consecutive indices of `sel` for which `keep` holds.
fn runs(sel: &[usize], keep: impl Fn(usize) -> bool) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for &j in sel {
        if keep(j) {
            if cur.last().is_some_and(|&l| j != l + 1) {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(j);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The viewport of one panel, with its frame at `(fx, fy)` on the sheet.
fn viewport(panel: &Panel, k: f64, scale: u32, fx: f64, fy: f64, p: &GalleryParams) -> Value {
    let s = panel.section;
    let n = format!("xs-{:03}", panel.number);
    let (w_mm, h_mm) = (panel.w_mm, panel.h_mm);
    // The window: the content, then the bands. Model x runs with the offset, model y with the
    // elevation, so the paper map is a scale by `k` and a flip of y.
    let shift = panel.number as f64 * PANEL_STRIDE_M;
    let x0 = shift + panel.lo - p.band_left_mm / k;
    let y0 = panel.zmin - p.band_bottom_mm / k;
    let (wm, hm) = (w_mm / k, h_mm / k);
    let mx = |px: f64| x0 + px / k;
    let my = |py: f64| y0 + hm - py / k;
    let mut e: Vec<Value> = Vec::new();

    // The panel border, a hair inside the frame.
    let inset = 0.3;
    e.push(json!({
        "type": "polyline", "layer": "OBJECT", "id": format!("{n}-border"), "closed": true,
        "points_m": [pt(mx(inset), my(inset)), pt(mx(w_mm - inset), my(inset)),
                     pt(mx(w_mm - inset), my(h_mm - inset)), pt(mx(inset), my(h_mm - inset))],
    }));

    // Headings.
    let lines = heading(s, panel.number, panel.bend);
    let baselines = [5.0, 10.5, 15.0];
    for (i, items) in lines.iter().enumerate() {
        let h = if i == 0 { H_TITLE } else { H_TEXT };
        if i == 0 {
            // The title at the left and the chainage at the right.
            e.push(text(
                &format!("{n}-{}", items[0].0),
                mx(2.0),
                my(baselines[0]),
                &items[0].1,
                h,
                "left",
                "baseline",
            ));
            e.push(text(
                &format!("{n}-{}", items[1].0),
                mx(w_mm - 2.0),
                my(baselines[0]),
                &items[1].1,
                h,
                "right",
                "baseline",
            ));
            continue;
        }
        let mut x = 2.0;
        for (suffix, t) in items {
            e.push(text(
                &format!("{n}-{suffix}"),
                mx(x),
                my(baselines[i]),
                t,
                h,
                "left",
                "baseline",
            ));
            x += text_width(t, h).unwrap_or(0.0) + ITEM_GAP_MM;
        }
    }

    // Elevation labels along the left band, offset labels along the bottom band.
    let z_step = nice_step(p.label_pitch_mm / k / 2.0);
    let mut z = (panel.zmin / z_step).ceil() * z_step;
    while z <= panel.zmax + 1e-9 {
        e.push(json!({
            "type": "line", "layer": "GRID", "id": format!("{n}-ztick-{}", z.round() as i64),
            "from_m": pt(mx(p.band_left_mm - 1.0), z), "to_m": pt(mx(p.band_left_mm), z),
        }));
        e.push(text(
            &format!("{n}-zlabel-{}", z.round() as i64),
            mx(p.band_left_mm - 1.5),
            z,
            &format!("{}", z.round() as i64),
            H_TEXT,
            "right",
            "middle",
        ));
        z += z_step;
    }
    let o_step = nice_step(p.label_pitch_mm / k);
    let mut o = (panel.lo / o_step).ceil() * o_step;
    while o <= panel.hi + 1e-9 {
        let label = if o.abs() < 1e-9 {
            "0".to_string()
        } else {
            format!("{:+}", o.round() as i64)
        };
        e.push(text(
            &format!(
                "{n}-olabel-{}",
                (o.round() as i64).to_string().replace('-', "m")
            ),
            shift + o,
            my(h_mm - p.band_bottom_mm + 1.5),
            &label,
            H_TEXT,
            "centre",
            "top",
        ));
        o += o_step;
    }

    // The cut, hatched: dredge where the ground is wet, dry cut elsewhere.
    let sel: Vec<usize> = (0..s.offsets.len())
        .filter(|&j| s.offsets[j] >= panel.lo && s.offsets[j] <= panel.hi)
        .collect();
    let cut_run = runs(&sel, |j| {
        finite(s.terrain[j])
            && finite(s.cutface[j])
            && s.terrain[j] - s.cutface[j] > p.hatch_min_cut_m
    });
    let mut hatch_no = 0;
    for run in cut_run {
        // Split a run where the ground changes between wet and dry.
        let mut parts: Vec<Vec<usize>> = Vec::new();
        for j in run {
            match parts.last_mut() {
                Some(last) if s.wet[*last.last().unwrap_or(&j)] == s.wet[j] => last.push(j),
                _ => parts.push(vec![j]),
            }
        }
        for part in parts.into_iter().filter(|p| p.len() >= 2) {
            let mut boundary: Vec<[f64; 2]> = part
                .iter()
                .map(|&j| [shift + s.offsets[j], s.terrain[j]])
                .chain(
                    part.iter()
                        .rev()
                        .map(|&j| [shift + s.offsets[j], s.cutface[j]]),
                )
                .collect();
            boundary.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
            if boundary.len() < 3 {
                continue;
            }
            hatch_no += 1;
            e.push(json!({
                "type": "hatch", "layer": "HATCH", "id": format!("{n}-hatch-{hatch_no}"),
                "boundary_m": boundary,
                "pattern": if s.wet[part[0]] { "water" } else { "soil" },
            }));
        }
    }

    // Ground (thin) and the design cut face (thick).
    for (i, run) in runs(&sel, |j| finite(s.terrain[j])).into_iter().enumerate() {
        if run.len() < 2 {
            continue;
        }
        let pts: Vec<Value> = run
            .iter()
            .map(|&j| pt(shift + s.offsets[j], s.terrain[j]))
            .collect();
        e.push(json!({
            "type": "polyline", "layer": "OBJECT", "id": format!("{n}-ground-{}", i + 1),
            "closed": false, "points_m": pts,
        }));
    }
    let face = runs(&sel, |j| {
        finite(s.cutface[j])
            && finite(s.terrain[j])
            && s.cutface[j] <= s.terrain[j] + p.hatch_min_cut_m
    });
    for (i, run) in face.into_iter().enumerate() {
        if run.len() < 2 {
            continue;
        }
        let pts: Vec<Value> = run
            .iter()
            .map(|&j| pt(shift + s.offsets[j], s.cutface[j]))
            .collect();
        e.push(json!({
            "type": "polyline", "layer": "OUTLINE", "id": format!("{n}-design-{}", i + 1),
            "closed": false, "points_m": pts,
        }));
    }

    // The water level.
    e.push(json!({
        "type": "line", "layer": "WATER", "id": format!("{n}-wl-line"), "line_type": "dashed",
        "from_m": pt(shift + panel.lo, s.wl), "to_m": pt(shift + panel.hi, s.wl),
    }));

    e.retain(|x| layer_on(p, x["layer"].as_str().unwrap_or("")));
    json!({
        "id": n,
        "frame": {"x_mm": fx, "y_mm": fy, "width_mm": w_mm, "height_mm": h_mm},
        "window": {"xmin_m": x0, "ymin_m": y0, "xmax_m": x0 + wm, "ymax_m": y0 + hm},
        "scale": scale,
        "entities": e,
    })
}

/// Is this layer drawn? Only the optional layers of the style can be off.
pub(super) fn layer_on(p: &GalleryParams, layer: &str) -> bool {
    match layer {
        "GRID" => p.grid,
        "HATCH" => p.hatch,
        "WATER" => p.water,
        _ => true,
    }
}

/// The key viewport: the scale bar (the standard puts it in the frame) and the legend.
fn key_viewport(area: Area, k: f64, scale: u32, p: &GalleryParams) -> Value {
    let (wm, hm) = (area.w / k, area.h / k);
    let mx = |px: f64| px / k;
    let my = |py: f64| (area.h - py) / k;
    let mut e: Vec<Value> = Vec::new();
    e.push(text(
        "key-heading",
        mx(70.0),
        my(7.0),
        "KEY",
        H_TITLE,
        "left",
        "baseline",
    ));
    let all = [
        (0, "HATCH", "DREDGE: CUT BELOW THE WATER LEVEL IN THE RIVER"),
        (1, "HATCH", "DRY CUT: BANK, BERM AND FACE"),
        (2, "OBJECT", "EXISTING GROUND"),
        (3, "OUTLINE", "DESIGN CUT FACE"),
        (4, "WATER", "DESIGN WATER LEVEL (WL)"),
    ];
    let rows: Vec<(usize, &str)> = all
        .iter()
        .filter(|(_, layer, _)| layer_on(p, layer))
        .map(|(kind, _, label)| (*kind, *label))
        .collect();
    for (row, (i, label)) in rows.iter().enumerate() {
        let i = *i;
        let base = 14.0 + 6.0 * row as f64;
        let id = format!("key-row-{}", row + 1);
        let (a, b) = (70.0, 80.0);
        match i {
            0 | 1 => e.push(json!({
                "type": "hatch", "layer": "HATCH", "id": format!("{id}-swatch"),
                "boundary_m": [pt(mx(a), my(base - 3.5)), pt(mx(b), my(base - 3.5)),
                               pt(mx(b), my(base)), pt(mx(a), my(base))],
                "pattern": if i == 0 { "water" } else { "soil" },
            })),
            _ => {
                let (layer, line_type) = match i {
                    2 => ("OBJECT", None),
                    3 => ("OUTLINE", None),
                    _ => ("WATER", Some("dashed")),
                };
                let mut line = json!({
                    "type": "line", "layer": layer, "id": format!("{id}-swatch"),
                    "from_m": pt(mx(a), my(base - 1.2)), "to_m": pt(mx(b), my(base - 1.2)),
                });
                if let Some(t) = line_type {
                    line["line_type"] = json!(t);
                }
                e.push(line);
            }
        }
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
        "scale_bar": {},
        "entities": e,
    })
}

/// The panels of each sheet: index, and the paper position of the frame.
type Placed = Vec<Vec<(usize, f64, f64)>>;

/// Place panels in rows, row by row, sheet by sheet, in the order given. Returns, for each sheet,
/// the panels' indices and the paper position of each frame.
fn pack(sizes: &[(f64, f64)], free: Area, gap: f64) -> Option<Placed> {
    let mut sheets: Vec<Vec<(usize, f64, f64)>> = vec![Vec::new()];
    let (mut cx, mut cy, mut row_h) = (0.0, 0.0, 0.0f64);
    for (i, &(w, h)) in sizes.iter().enumerate() {
        if w > free.w + 1e-9 || h > free.h + 1e-9 {
            return None;
        }
        if cx > 0.0 && cx + w > free.w + 1e-9 {
            cx = 0.0;
            cy += row_h + gap;
            row_h = 0.0;
        }
        if cy + h > free.h + 1e-9 {
            sheets.push(Vec::new());
            cx = 0.0;
            cy = 0.0;
            row_h = 0.0;
        }
        sheets
            .last_mut()
            .expect("one sheet at least")
            .push((i, free.x + cx, free.y + cy));
        cx += w + gap;
        row_h = row_h.max(h);
    }
    Some(sheets)
}

pub(super) fn build(
    session: &Session,
    option_id: &str,
    walk: &SectionVolumes,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let params = options.style.gallery();
    let p = &params;
    let doc = session.document();
    let option = doc["options"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["id"] == option_id))
        .ok_or_else(|| DrawError::Input(format!("there is no option `{option_id}`")))?;
    let picks = pick(walk, option, &super::worst_chainages(session, option_id), p);
    if picks.is_empty() {
        return Err(DrawError::Missing(format!(
            "option `{option_id}` has no stations to draw"
        )));
    }
    let mut panels: Vec<Panel> = picks
        .iter()
        .enumerate()
        .map(|(i, &(si, bend))| {
            let section = &walk.stations[si];
            let (lo, hi, zmin, zmax) = extent(section, p);
            Panel {
                section,
                number: i + 1,
                bend,
                lo,
                hi,
                zmin,
                zmax,
                w_mm: 0.0,
                h_mm: 0.0,
            }
        })
        .collect();

    let notes_len = 5;
    let layout = furniture::layout(&p.sheet_size, p.landscape, notes_len).ok_or_else(|| {
        DrawError::Input(format!(
            "the sheet size `{}` is not one of the standard's",
            p.sheet_size
        ))
    })?;
    // The largest scale at which every panel fits, with room for `min_columns` side by side.
    let mut chosen: Option<(u32, Placed)> = None;
    for &scale in &p.scales {
        let k = 1000.0 / f64::from(scale);
        for panel in &mut panels {
            let (w, h) = panel_size(panel, k, p);
            panel.w_mm = w;
            panel.h_mm = h;
        }
        let cols = p.min_columns.max(1) as f64;
        let widest = panels.iter().map(|q| q.w_mm).fold(0.0, f64::max);
        if widest * cols + p.gap_mm * (cols - 1.0) > layout.free.w + 1e-9 {
            continue;
        }
        let sizes: Vec<(f64, f64)> = panels.iter().map(|q| (q.w_mm, q.h_mm)).collect();
        if let Some(sheets) = pack(&sizes, layout.free, p.gap_mm) {
            chosen = Some((scale, sheets));
            break;
        }
    }
    let Some((scale, sheets)) = chosen else {
        return Err(DrawError::Input(format!(
            "the sections do not fit a {} sheet at any scale in the sheet style ({:?})",
            p.sheet_size, p.scales
        )));
    };
    let k = 1000.0 / f64::from(scale);

    let title = doc["project"]["title"].as_str().unwrap_or("DESIGN");
    let option_name = option["name"].as_str().unwrap_or(option_id);
    let date = options
        .date
        .clone()
        .or_else(|| doc["project"]["created"].as_str().map(str::to_string))
        .ok_or_else(|| {
            DrawError::Missing("the design has no created date for the title block".into())
        })?;
    let code = furniture::design_code(&clean(title), SheetKind::CrossSections.code());
    let model = walk
        .stations
        .first()
        .map_or("?", |s| s.model.as_str())
        .trim_start_matches("cut_model_")
        .to_ascii_uppercase();
    let notes = vec![
        format!("ALL SECTIONS AT 1:{scale}, VERTICAL = HORIZONTAL. ELEVATIONS, OFFSETS AND CHAINAGES IN METRES."),
        "VIEW LOOKS TOWARDS DECREASING CHAINAGE. OFFSET + IS LEFT OF THE LINE FACING INCREASING CHAINAGE.".to_string(),
        format!("AREAS ARE THOSE OF THE VOLUMES STAGE (CUT MODEL {model}), ON THE PROFILE SHOWN."),
        "SCREENING – NOT FOR CONSTRUCTION.".to_string(),
        format!(
            "SHEET STYLE {} SHA-256 {}",
            clean(options.style.name()),
            options.style.hash()
        ),
    ];
    debug_assert_eq!(notes.len(), notes_len);

    let layer_names: Vec<&str> = LAYERS.iter().copied().filter(|l| layer_on(p, l)).collect();
    let count = sheets.len();
    let mut out = Vec::new();
    for (si, placed) in sheets.iter().enumerate() {
        let mut viewports: Vec<Value> = placed
            .iter()
            .map(|&(i, x, y)| viewport(&panels[i], k, scale, x, y, p))
            .collect();
        viewports.push(key_viewport(layout.key, k, scale, p));
        let number = furniture::drawing_number(&code, SheetKind::CrossSections.code(), si + 1);
        let titles = Titles {
            design: clean(title),
            option_name: option_name.to_string(),
            sheet_title: "Cross-section gallery".into(),
            number: number.clone(),
            revision: options.revision.clone(),
            date: date.clone(),
            sheet_number: si + 1,
            sheet_count: count,
        };
        let document = furniture::document(
            &p.sheet_size,
            p.landscape,
            &layout,
            &titles,
            &notes,
            &layer_names,
            viewports,
        );
        out.push(Sheet {
            kind: SheetKind::CrossSections,
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

    #[test]
    fn a_chainage_is_written_as_kilometres_plus_metres_to_a_tenth() {
        assert_eq!(chainage_text(0.0), "0+000.0");
        assert_eq!(chainage_text(25.0), "0+025.0");
        assert_eq!(chainage_text(1500.34), "1+500.3");
        // Rounding to a tenth carries into the kilometre, never writes `0+1000.0`.
        assert_eq!(chainage_text(999.96), "1+000.0");
        assert_eq!(chainage_text(12_345.6), "12+345.6");
    }

    #[test]
    fn axis_steps_are_one_two_or_five_times_a_power_of_ten() {
        assert_eq!(nice_step(6.0), 10.0);
        assert_eq!(nice_step(12.0), 20.0);
        assert_eq!(nice_step(0.3), 1.0); // never finer than a metre
        assert_eq!(nice_step(50.0), 50.0);
    }
}
