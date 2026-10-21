//! Plan sheets (spec 0004, ticket 5): an overview of the whole design line with the strip sheets as
//! a key, then strips along chainage at the style's scale, each over shaded relief of the design DEM.
//!
//! Always shown: the relief, the water edge (the river bank lines of the design), the design line
//! with chainage ticks, the channel footprint (toe and top of cut at the stations of the volumes
//! stage), the section lines of the cross-section gallery, a UTM grid, a north arrow and a scale bar.
//! Shown only where a label rule of the style finds something: bends tighter than `k x R_min`, and
//! infeasible stretches. A clean line has no label.
//!
//! Every drawn value is the design's: the line and its chainages, the station walk, the bend
//! catalogue of the setting-out table, the stored stretches of `checks.fits_in_water`. The relief is
//! the DEM, sampled at the pixel centres through the same terrain the stages use; `otd` makes it a
//! JPEG. The strips are north up, as the standard draws an image only in a viewport with no rotation
//! (clause 9.10.4). A strip ends on a whole 10 m of chainage, so its match line is written exactly.

use super::furniture::{self, Titles, clean};
use super::gallery::{self, GalleryParams, chainage_text};
use super::setting_out::{self, BendRow};
use super::{DrawError, DrawOptions, Sheet, SheetKind};
use crate::design::Session;
use crate::design::alignment::Terrain;
use crate::design::geojson::all_positions;
use otd::raster::{Backdrop, Grid, Light};
use otd::rules::{Rect, text_width};
use owd_core::alignment::limits::compute_chainages;
use owd_core::geom::{Pt, linear};
use owd_core::volumes::sections::{CrossSection, SectionVolumes};
use serde_json::{Value, json};

/// The numbers the plan reads, all from the sheet style (`Style::plan`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanParams {
    pub sheet_size: String,
    pub landscape: bool,
    pub strip_scale: u32,
    pub overlap_m: f64,
    pub margin_m: f64,
    pub tick_mm: f64,
    pub tick_label_every: usize,
    pub light: Light,
    pub pixel_mm: f64,
    pub quality: u8,
    pub rules: Vec<LabelRule>,
    pub grid: bool,
    pub water: bool,
}

/// A label rule of the style (`plan.label_rules`).
#[derive(Debug, Clone, PartialEq)]
pub enum LabelRule {
    /// Label a bend whose least radius is below `k x R_min`.
    BendTighterThan { k: f64 },
    /// Label each stretch where the channel does not fit in the water.
    InfeasibleStretch,
}

impl LabelRule {
    pub fn name(&self) -> &'static str {
        match self {
            LabelRule::BendTighterThan { .. } => "bend-tighter-than",
            LabelRule::InfeasibleStretch => "infeasible-stretch",
        }
    }
}

/// One label a rule found: its rule, its chainage range and its one short text.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanLabel {
    pub rule: &'static str,
    pub start_m: f64,
    pub end_m: f64,
    pub text: String,
}

/// Text heights of the standard (clause 11.2) used here.
const H_LABEL: f64 = 3.5;
const H_TEXT: f64 = 2.5;
/// Chainage ticks are this long on paper, in millimetres.
const TICK_LEN_MM: f64 = 2.0;
/// A label stands this far from the line on paper, in millimetres.
const LABEL_OFFSET_MM: f64 = 6.0;
/// Text keeps this far inside the frame, in millimetres.
const INSET_MM: f64 = 3.0;
/// Strip ends fall on this step of chainage, in metres.
const STRIP_STEP_M: f64 = 10.0;
/// The layer of the relief (a layer of the document, not of the standard's table).
const RELIEF_LAYER: &str = "RELIEF";

/// The labels of an option under the rules, in chainage order: the bends of the bend catalogue
/// (the setting-out table's) whose least radius is below `k x checks.r_min_m`, and the stored
/// stretches of `checks.fits_in_water`.
pub fn labels(option: &Value, rules: &[LabelRule]) -> Vec<PlanLabel> {
    let mut out = Vec::new();
    let line = all_positions(&option["line"]);
    for rule in rules {
        match rule {
            LabelRule::BendTighterThan { k } => {
                let Some(r_min) = option["checks"]["r_min_m"].as_f64() else {
                    continue;
                };
                if line.len() < 3 {
                    continue;
                }
                for b in setting_out::bend_rows(&line) {
                    if b.least_radius_m < k * r_min {
                        out.push(bend_label(&b, *k, r_min));
                    }
                }
            }
            LabelRule::InfeasibleStretch => {
                let stretches = option["checks"]["fits_in_water"]["stretches"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for s in stretches {
                    let (Some(a), Some(b)) =
                        (s["start_chainage_m"].as_f64(), s["end_chainage_m"].as_f64())
                    else {
                        continue;
                    };
                    let why = match s["reason"].as_str() {
                        Some("too_narrow") => "TOO NARROW".to_string(),
                        Some("r_min_not_possible") => "RMIN NOT POSSIBLE".to_string(),
                        Some(other) => clean(&other.replace('_', " ")),
                        None => "INFEASIBLE".to_string(),
                    };
                    out.push(PlanLabel {
                        rule: "infeasible-stretch",
                        start_m: a,
                        end_m: b,
                        text: format!("{why} CH {} TO {}", chainage_text(a), chainage_text(b)),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.start_m.total_cmp(&b.start_m));
    out
}

fn bend_label(b: &BendRow, k: f64, r_min: f64) -> PlanLabel {
    let limit = if (k - 1.0).abs() < 1e-12 {
        format!("RMIN {r_min:.0} M")
    } else {
        format!("{k} X RMIN {:.0} M", k * r_min)
    };
    PlanLabel {
        rule: "bend-tighter-than",
        start_m: b.start_m,
        end_m: b.end_m,
        text: format!(
            "BEND R {:.0} M < {limit} CH {} TO {}",
            b.least_radius_m,
            chainage_text(b.start_m),
            chainage_text(b.end_m)
        ),
    }
}

// ---- geometry on the line ---------------------------------------------------------------------

/// The design line with its chainages.
struct Line {
    pts: Vec<Pt>,
    chain: Vec<f64>,
    total: f64,
}

impl Line {
    fn new(pts: Vec<Pt>) -> Line {
        let chain = compute_chainages(&pts);
        let total = *chain.last().unwrap_or(&0.0);
        Line { pts, chain, total }
    }

    fn at(&self, s: f64) -> Pt {
        linear::interpolate(&self.pts, s.clamp(0.0, self.total)).unwrap_or(self.pts[0])
    }

    /// The unit direction of the segment at chainage `s`.
    fn dir(&self, s: f64) -> Pt {
        let i = self
            .chain
            .partition_point(|&c| c <= s)
            .saturating_sub(1)
            .min(self.pts.len() - 2);
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let len = libm::hypot(b[0] - a[0], b[1] - a[1]).max(1e-12);
        [(b[0] - a[0]) / len, (b[1] - a[1]) / len]
    }

    /// The left normal at chainage `s`, facing increasing chainage.
    fn normal(&self, s: f64) -> Pt {
        let d = self.dir(s);
        [-d[1], d[0]]
    }

    /// The line between two chainages, with its own vertices between.
    fn part(&self, s0: f64, s1: f64) -> Vec<Pt> {
        let (s0, s1) = (s0.max(0.0), s1.min(self.total));
        let mut out = vec![self.at(s0)];
        for (p, c) in self.pts.iter().zip(&self.chain) {
            if *c > s0 && *c < s1 {
                out.push(*p);
            }
        }
        out.push(self.at(s1));
        out
    }
}

/// A window `[xmin, ymin, xmax, ymax]` in model metres.
type Win = [f64; 4];

fn inside(p: Pt, w: &Win) -> bool {
    p[0] >= w[0] && p[0] <= w[2] && p[1] >= w[1] && p[1] <= w[3]
}

/// Clip a polyline to a window (Liang-Barsky on each segment), as runs of points.
fn clip(points: &[Pt], w: &Win) -> Vec<Vec<Pt>> {
    let mut runs: Vec<Vec<Pt>> = Vec::new();
    let mut current: Vec<Pt> = Vec::new();
    for seg in points.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut keep = true;
        for (p, q) in [
            (-dx, a[0] - w[0]),
            (dx, w[2] - a[0]),
            (-dy, a[1] - w[1]),
            (dy, w[3] - a[1]),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    keep = false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if !keep || t0 > t1 {
            if current.len() > 1 {
                runs.push(std::mem::take(&mut current));
            }
            current.clear();
            continue;
        }
        let p0 = [a[0] + t0 * dx, a[1] + t0 * dy];
        let p1 = [a[0] + t1 * dx, a[1] + t1 * dy];
        if current.last() != Some(&p0) {
            if current.len() > 1 {
                runs.push(std::mem::take(&mut current));
            }
            current = vec![p0];
        }
        current.push(p1);
        if t1 < 1.0 {
            runs.push(std::mem::take(&mut current));
        }
    }
    if current.len() > 1 {
        runs.push(current);
    }
    // a run of one repeated point draws nothing (and the standard refuses it, clause 9.4.1)
    runs.into_iter()
        .map(|mut r| {
            r.dedup();
            r
        })
        .filter(|r| r.len() > 1)
        .collect()
}

/// The station point at a signed offset (negative is the left bank in the walk's own sense).
fn at_offset(s: &CrossSection, o: f64) -> Pt {
    [
        s.x + o * -libm::sin(s.heading_rad),
        s.y + o * libm::cos(s.heading_rad),
    ]
}

/// The outermost offsets on each side where the cut face is at or below the ground: the top of cut.
fn top_of_cut(s: &CrossSection) -> (Option<f64>, Option<f64>) {
    let cut = |j: usize| {
        let (t, c) = (s.terrain[j], s.cutface[j]);
        t.is_finite() && c.is_finite() && c <= t
    };
    let lo = (0..s.offsets.len())
        .filter(|&j| s.offsets[j] < 0.0 && cut(j))
        .map(|j| s.offsets[j])
        .reduce(f64::min);
    let hi = (0..s.offsets.len())
        .filter(|&j| s.offsets[j] > 0.0 && cut(j))
        .map(|j| s.offsets[j])
        .reduce(f64::max);
    (lo, hi)
}

/// Polylines through the stations, broken where `f` gives nothing.
fn station_lines(walk: &SectionVolumes, f: impl Fn(&CrossSection) -> Option<Pt>) -> Vec<Vec<Pt>> {
    let mut out: Vec<Vec<Pt>> = Vec::new();
    let mut current = Vec::new();
    for s in &walk.stations {
        match f(s) {
            Some(p) => current.push(p),
            None => {
                if current.len() > 1 {
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
            }
        }
    }
    if current.len() > 1 {
        out.push(current);
    }
    out
}

/// A step of 1, 2 or 5 times a power of ten that is at least `min`.
fn nice_step(min: f64) -> f64 {
    let mut mag = 1e-3;
    loop {
        for m in [1.0, 2.0, 5.0] {
            if m * mag >= min {
                return m * mag;
            }
        }
        mag *= 10.0;
    }
}

fn pv(p: Pt) -> Value {
    json!([p[0], p[1]])
}

fn polyline(layer: &str, id: Option<String>, pts: &[Pt]) -> Value {
    let mut v = json!({"type": "polyline", "layer": layer, "points_m": pts.iter().map(|p| pv(*p)).collect::<Vec<_>>()});
    if let Some(id) = id {
        v["id"] = json!(id);
    }
    v
}

fn line_e(layer: &str, id: Option<String>, a: Pt, b: Pt) -> Value {
    let mut v = json!({"type": "line", "layer": layer, "from_m": pv(a), "to_m": pv(b)});
    if let Some(id) = id {
        v["id"] = json!(id);
    }
    v
}

/// A text, left and baseline aligned, moved inside the window so that its box keeps `INSET_MM` from
/// the frame. `None` when the window is too small for it.
fn text_in(id: &str, at: Pt, s: &str, h: f64, k: f64, w: &Win) -> Option<Value> {
    let tw = text_width(s, h)? / k;
    let (th, inset) = (h / k, INSET_MM / k);
    let x = at[0].clamp(w[0] + inset, (w[2] - inset - tw).max(w[0] + inset));
    let y = at[1].clamp(w[1] + inset, (w[3] - inset - th).max(w[1] + inset));
    (x + tw <= w[2] - inset + 1e-9 && y + th <= w[3] - inset + 1e-9).then(|| {
        json!({"type": "text", "layer": "TEXT", "id": id, "anchor_m": pv([x, y]), "text": s,
            "height_mm": h, "align_h": "left", "align_v": "baseline"})
    })
}

// ---- the sheets --------------------------------------------------------------------------------

/// What every viewport of the plan draws over its window.
struct Content<'a> {
    line: &'a Line,
    walk: &'a SectionVolumes,
    banks: Vec<Vec<Pt>>,
    sections: Vec<(usize, &'a CrossSection, f64)>,
    labels: Vec<PlanLabel>,
    p: &'a PlanParams,
}

/// The extent of the channel round the line: the widest top of cut (or toe) of the walk, plus the
/// margin.
fn half_width(walk: &SectionVolumes, margin_m: f64) -> f64 {
    walk.stations
        .iter()
        .map(|s| {
            let (lo, hi) = top_of_cut(s);
            lo.map_or(s.hb, f64::abs).max(hi.unwrap_or(s.hb)).max(s.hb)
        })
        .fold(0.0, f64::max)
        + margin_m
}

/// The box of the line between two chainages, grown by `grow` metres.
fn line_box(line: &Line, s0: f64, s1: f64, grow: f64) -> Win {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut s = s0.max(0.0);
    let end = s1.min(line.total);
    loop {
        let p = line.at(s);
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
        if s >= end {
            break;
        }
        s = (s + STRIP_STEP_M).min(end);
    }
    [b[0] - grow, b[1] - grow, b[2] + grow, b[3] + grow]
}

/// A window of the frame's size at `k` mm a metre, centred on a box. A thousandth of a millimetre
/// smaller than the frame, so that the standard's fit test (clause 7.5.1) is never a rounding away.
fn window_on(b: &Win, frame: &Rect, k: f64) -> Win {
    let (w, h) = ((frame.w - 1e-3) / k, (frame.h - 1e-3) / k);
    let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
}

/// The strips: chainage ranges on whole steps, each as long as its window allows.
fn strips(
    line: &Line,
    half: f64,
    frame: &Rect,
    k: f64,
    overlap: f64,
) -> Result<Vec<(f64, f64)>, DrawError> {
    let (ww, wh) = ((frame.w - 1e-3) / k, (frame.h - 1e-3) / k);
    let fits = |s0: f64, s1: f64| {
        let b = line_box(line, s0 - overlap, s1 + overlap, half);
        b[2] - b[0] <= ww && b[3] - b[1] <= wh
    };
    let mut out = Vec::new();
    let mut s0 = 0.0;
    while s0 < line.total - 1e-9 {
        let mut s1 = (s0 + STRIP_STEP_M).min(line.total);
        if !fits(s0, s1) {
            return Err(DrawError::Input(format!(
                "a plan strip at 1:{} does not hold {STRIP_STEP_M} m of the line with its footprint: use a smaller strip scale",
                (1000.0 / k).round()
            )));
        }
        while s1 < line.total && fits(s0, (s1 + STRIP_STEP_M).min(line.total)) {
            s1 = (s1 + STRIP_STEP_M).min(line.total);
        }
        out.push((s0, s1));
        s0 = s1;
    }
    Ok(out)
}

/// The relief of a window: the terrain at each pixel centre, shaded, as a JPEG.
fn relief(
    path: &str,
    terrain: Terrain,
    w: &Win,
    frame: &Rect,
    p: &PlanParams,
) -> Result<Backdrop, DrawError> {
    let cols = ((frame.w / p.pixel_mm).ceil() as usize).max(2);
    let rows = ((frame.h / p.pixel_mm).ceil() as usize).max(2);
    let (dx, dy) = ((w[2] - w[0]) / cols as f64, (w[3] - w[1]) / rows as f64);
    let mut xs = Vec::with_capacity(cols * rows);
    let mut ys = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            xs.push(w[0] + (c as f64 + 0.5) * dx);
            ys.push(w[3] - (r as f64 + 0.5) * dy);
        }
    }
    let heights = terrain(&xs, &ys);
    let dem = Grid::new(cols, rows, *w, heights)
        .map_err(|e| DrawError::Input(format!("the relief grid: {e}")))?;
    Backdrop::relief(path, &dem, &p.light, p.quality)
        .map_err(|e| DrawError::Input(format!("the relief: {e}")))
}

/// The UTM grid of a window: lines on a nice step, with their eastings and northings at the edges.
fn utm_grid(w: &Win, k: f64, out: &mut Vec<Value>) {
    let step = nice_step(60.0 / k);
    let mut i = 0;
    let mut e = (w[0] / step).ceil() * step;
    while e <= w[2] {
        out.push(line_e("GRID", None, [e, w[1]], [e, w[3]]));
        if let Some(t) = text_in(
            &format!("grid-e-{i}"),
            [e + 1.0 / k, w[3] - (INSET_MM + H_TEXT) / k],
            &format!("E {e:.0}"),
            H_TEXT,
            k,
            w,
        ) {
            out.push(t);
        }
        e += step;
        i += 1;
    }
    let mut n = (w[1] / step).ceil() * step;
    let mut j = 0;
    while n <= w[3] {
        out.push(line_e("GRID", None, [w[0], n], [w[2], n]));
        if let Some(t) = text_in(
            &format!("grid-n-{j}"),
            [w[2] - 30.0 / k, n + 1.0 / k],
            &format!("N {n:.0}"),
            H_TEXT,
            k,
            w,
        ) {
            out.push(t);
        }
        n += step;
        j += 1;
    }
}

impl Content<'_> {
    /// The entities common to every viewport, clipped to its window.
    fn draw(&self, w: &Win, k: f64, ticks: bool, out: &mut Vec<Value>) {
        if self.p.grid {
            utm_grid(w, k, out);
        }
        if self.p.water {
            for (i, bank) in self.banks.iter().enumerate() {
                for (j, run) in clip(bank, w).into_iter().enumerate() {
                    out.push(polyline("WATER", Some(format!("bank-{i}-{j}")), &run));
                }
            }
        }
        // the footprint: toe and top of cut on each side
        let toe_l = station_lines(self.walk, |s| Some(at_offset(s, -s.hb)));
        let toe_r = station_lines(self.walk, |s| Some(at_offset(s, s.hb)));
        let top_l = station_lines(self.walk, |s| top_of_cut(s).0.map(|o| at_offset(s, o)));
        let top_r = station_lines(self.walk, |s| top_of_cut(s).1.map(|o| at_offset(s, o)));
        for (name, lines) in [
            ("toe-l", toe_l),
            ("toe-r", toe_r),
            ("top-l", top_l),
            ("top-r", top_r),
        ] {
            for (i, l) in lines.iter().enumerate() {
                for (j, run) in clip(l, w).into_iter().enumerate() {
                    out.push(polyline("OBJECT", Some(format!("{name}-{i}-{j}")), &run));
                }
            }
        }
        for (j, run) in clip(&self.line.pts, w).into_iter().enumerate() {
            out.push(polyline("OUTLINE", Some(format!("line-{j}")), &run));
        }
        if ticks {
            let step = nice_step(self.p.tick_mm / k);
            let half = TICK_LEN_MM / 2.0 / k;
            let mut i = 0usize;
            loop {
                let s = i as f64 * step;
                if s > self.line.total + 1e-9 {
                    break;
                }
                let (c, nrm) = (self.line.at(s), self.line.normal(s));
                let a = [c[0] - nrm[0] * half, c[1] - nrm[1] * half];
                let b = [c[0] + nrm[0] * half, c[1] + nrm[1] * half];
                if inside(a, w) && inside(b, w) {
                    out.push(line_e("ANNO", Some(format!("tick-{i}")), a, b));
                    if i.is_multiple_of(self.p.tick_label_every) {
                        let at = [
                            c[0] + nrm[0] * (half + 1.0 / k),
                            c[1] + nrm[1] * (half + 1.0 / k),
                        ];
                        if inside(at, w)
                            && let Some(t) = text_in(
                                &format!("tick-label-{i}"),
                                at,
                                &chainage_text(s),
                                H_TEXT,
                                k,
                                w,
                            )
                        {
                            out.push(t);
                        }
                    }
                }
                i += 1;
            }
        }
    }

    /// The section lines of the gallery that lie wholly in the window.
    fn sections(&self, w: &Win, k: f64, out: &mut Vec<Value>) {
        for &(n, s, half) in &self.sections {
            let (a, b) = (at_offset(s, -half), at_offset(s, half));
            if !(inside(a, w) && inside(b, w)) {
                continue;
            }
            out.push(line_e("ANNO", Some(format!("xs-{n}")), a, b));
            if let Some(t) = text_in(
                &format!("xs-{n}-label"),
                [b[0] + 1.0 / k, b[1] + 1.0 / k],
                &format!("S{n}"),
                H_TEXT,
                k,
                w,
            ) {
                out.push(t);
            }
        }
    }
}

/// The plan sheets of a design line option.
pub(super) fn build(
    session: &Session,
    option_id: &str,
    walk: &SectionVolumes,
    terrain: Terrain,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let p = options.style.plan();
    let gp: GalleryParams = options.style.gallery();
    let doc = session.document();
    let option = doc["options"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["id"] == option_id))
        .ok_or_else(|| DrawError::Input(format!("there is no option `{option_id}`")))?;
    let pts = all_positions(&option["line"]);
    if pts.len() < 2 {
        return Err(DrawError::Missing(format!(
            "option `{option_id}` has no design line to draw"
        )));
    }
    if walk.stations.is_empty() {
        return Err(DrawError::Missing(format!(
            "option `{option_id}` has no stations to draw"
        )));
    }
    let crs = doc["project"]["crs"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let epsg = doc["project"]["crs"]["epsg"].as_u64().ok_or_else(|| {
        DrawError::Missing("the design has no project CRS: the plan is in its UTM zone".into())
    })?;
    let line = Line::new(pts);
    let banks: Vec<Vec<Pt>> = ["left_bank", "right_bank"]
        .iter()
        .map(|key| all_positions(&doc["inputs"][*key]))
        .filter(|b| b.len() > 1)
        .collect();
    let sections: Vec<(usize, &CrossSection, f64)> = gallery::pick(walk, option, &gp)
        .iter()
        .enumerate()
        .map(|(i, &(si, _))| {
            let s = &walk.stations[si];
            let (lo, hi, _, _) = gallery::extent(s, &gp);
            (i + 1, s, lo.abs().max(hi.abs()))
        })
        .collect();
    let content = Content {
        line: &line,
        walk,
        banks,
        sections,
        labels: labels(option, &p.rules),
        p: &p,
    };

    let notes_len = 5;
    let layout = furniture::layout(&p.sheet_size, p.landscape, notes_len).ok_or_else(|| {
        DrawError::Input(format!(
            "the sheet size `{}` is not one of the standard's",
            p.sheet_size
        ))
    })?;
    let frame = Rect {
        x: layout.free.x,
        y: layout.free.y,
        w: layout.free.w,
        h: layout.free.h,
    };
    let half = half_width(walk, p.margin_m);
    let k_strip = 1000.0 / f64::from(p.strip_scale);
    let ranges = strips(&line, half, &frame, k_strip, p.overlap_m)?;

    // the overview: the whole line at the first ratio of version 1.1 that holds it
    let whole = line_box(&line, 0.0, line.total, half);
    let over_scale = otd::rules::fit_scale_in(
        &otd::rules::ratios(1),
        frame,
        otd::rules::Window {
            xmin: whole[0],
            ymin: whole[1],
            xmax: whole[2],
            ymax: whole[3],
        },
        0.0,
    )
    .ok_or_else(|| {
        DrawError::Input("the line does not fit an overview at any ratio of the standard".into())
    })?;
    let k_over = 1000.0 / f64::from(over_scale);

    let title = doc["project"]["title"].as_str().unwrap_or("DESIGN");
    let option_name = option["name"].as_str().unwrap_or(option_id);
    let date = options
        .date
        .clone()
        .or_else(|| doc["project"]["created"].as_str().map(str::to_string))
        .ok_or_else(|| {
            DrawError::Missing("the design has no created date for the title block".into())
        })?;
    let code = furniture::design_code(&clean(title), SheetKind::Plan.code());
    let count = 1 + ranges.len();
    let numbers: Vec<String> = (1..=count)
        .map(|n| furniture::drawing_number(&code, SheetKind::Plan.code(), n))
        .collect();
    let rules = if p.rules.is_empty() {
        "NONE".to_string()
    } else {
        p.rules
            .iter()
            .map(|r| r.name().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let notes = |scale: u32| {
        vec![
            format!("PLAN AT 1:{scale} ON SHADED RELIEF OF THE DESIGN DEM. COORDINATES {} (EPSG {epsg}), METRES.", clean(&crs)),
            "WATER EDGE: THE RIVER BANK LINES OF THE DESIGN. FOOTPRINT: TOE AND TOP OF CUT AT THE STATIONS OF THE VOLUMES STAGE.".to_string(),
            format!("SN: SECTION N OF THE CROSS-SECTION GALLERY. LABEL RULES: {rules}."),
            "SCREENING – NOT FOR CONSTRUCTION.".to_string(),
            format!("SHEET STYLE {} SHA-256 {}", clean(options.style.name()), options.style.hash()),
        ]
    };

    let mut out = Vec::new();
    // sheet 1: the overview, with the strips as a key
    {
        let w = window_on(&whole, &frame, k_over);
        let path = format!("{}-relief.jpg", numbers[0].to_ascii_lowercase());
        let backdrop = relief(&path, terrain, &w, &frame, &p)?;
        let mut ents = vec![backdrop.entity(RELIEF_LAYER, 1.0, Some("relief"))];
        content.draw(&w, k_over, false, &mut ents);
        for (i, &(s0, s1)) in ranges.iter().enumerate() {
            let b = window_on(
                &line_box(&line, s0 - p.overlap_m, s1 + p.overlap_m, half),
                &frame,
                k_strip,
            );
            let r = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
            let rc: Vec<Pt> = r
                .iter()
                .map(|q| [q[0].clamp(w[0], w[2]), q[1].clamp(w[1], w[3])])
                .collect();
            let mut outline = polyline("ANNO", Some(format!("key-{}", i + 2)), &rc);
            outline["closed"] = json!(true);
            ents.push(outline);
            let name = numbers[i + 1].clone();
            if let Some(t) = text_in(
                &format!("key-{}-label", i + 2),
                [rc[3][0] + 1.0 / k_over, rc[3][1] - (1.0 + H_TEXT) / k_over],
                &name,
                H_TEXT,
                k_over,
                &w,
            ) {
                ents.push(t);
            }
        }
        out.push(sheet(
            &p,
            &layout,
            &numbers[0],
            "Plan overview",
            1,
            count,
            title,
            option_name,
            options,
            &date,
            &notes(over_scale),
            viewport("plan", &frame, &w, over_scale, ents),
            backdrop,
        ));
    }
    // sheets 2..: the strips
    for (i, &(s0, s1)) in ranges.iter().enumerate() {
        let n = i + 2;
        let w = window_on(
            &line_box(&line, s0 - p.overlap_m, s1 + p.overlap_m, half),
            &frame,
            k_strip,
        );
        let path = format!("{}-relief.jpg", numbers[n - 1].to_ascii_lowercase());
        let backdrop = relief(&path, terrain, &w, &frame, &p)?;
        let mut ents = vec![backdrop.entity(RELIEF_LAYER, 1.0, Some("relief"))];
        content.draw(&w, k_strip, true, &mut ents);
        content.sections(&w, k_strip, &mut ents);
        // match lines: at the start, to the strip before; at the end, to the strip after
        for (which, s, other) in [
            ("start", s0, (n > 2).then(|| n - 1)),
            ("end", s1, (n < count).then_some(n + 1)),
        ] {
            let Some(other) = other else { continue };
            let (c, nrm) = (line.at(s), line.normal(s));
            let a = [c[0] - nrm[0] * half, c[1] - nrm[1] * half];
            let b = [c[0] + nrm[0] * half, c[1] + nrm[1] * half];
            for run in clip(&[a, b], &w) {
                ents.push(line_e(
                    "ANNO",
                    Some(format!("match-{which}")),
                    run[0],
                    run[run.len() - 1],
                ));
            }
            let text = format!(
                "MATCH LINE CH {} SEE {}",
                chainage_text(s),
                numbers[other - 1]
            );
            if let Some(t) = text_in(
                &format!("match-{which}-label"),
                [b[0] + 1.0 / k_strip, b[1]],
                &text,
                H_TEXT,
                k_strip,
                &w,
            ) {
                ents.push(t);
            }
        }
        // the labels whose middle falls in this strip's own range (each label once in the set)
        for (j, l) in content.labels.iter().enumerate() {
            let mid = (l.start_m + l.end_m) / 2.0;
            let last = n == count;
            if !(mid >= s0 && (mid < s1 || (last && mid <= s1))) {
                continue;
            }
            if l.rule == "infeasible-stretch" {
                for (r, run) in clip(&line.part(l.start_m, l.end_m), &w)
                    .into_iter()
                    .enumerate()
                {
                    ents.push(polyline("ZONE", Some(format!("label-{j}-band-{r}")), &run));
                }
            }
            let (c, nrm) = (line.at(mid), line.normal(mid));
            let off = half + LABEL_OFFSET_MM / k_strip;
            if let Some(t) = text_in(
                &format!("label-{j}"),
                [c[0] + nrm[0] * off, c[1] + nrm[1] * off],
                &l.text,
                H_LABEL,
                k_strip,
                &w,
            ) {
                ents.push(t);
            }
        }
        let title_s = format!("Plan CH {} to {}", chainage_text(s0), chainage_text(s1));
        out.push(sheet(
            &p,
            &layout,
            &numbers[n - 1],
            &title_s,
            n,
            count,
            title,
            option_name,
            options,
            &date,
            &notes(p.strip_scale),
            viewport("plan", &frame, &w, p.strip_scale, ents),
            backdrop,
        ));
    }
    Ok(out)
}

fn viewport(id: &str, frame: &Rect, w: &Win, scale: u32, entities: Vec<Value>) -> Value {
    json!({
        "id": id,
        "frame": {"x_mm": frame.x, "y_mm": frame.y, "width_mm": frame.w, "height_mm": frame.h},
        "window": {"xmin_m": w[0], "ymin_m": w[1], "xmax_m": w[2], "ymax_m": w[3]},
        "scale": scale,
        "entities": entities,
        "scale_bar": {},
        "north": {},
    })
}

/// The layers of a plan sheet, with the colours of the colour print profile (clause 8.6.5).
fn layers(p: &PlanParams) -> Vec<Value> {
    let mut out = vec![
        json!({"name": RELIEF_LAYER, "role": "thin", "line_type": "continuous", "aci": 251}),
        json!({"name": "OUTLINE", "print_colour": "#C0392B"}),
        json!({"name": "OBJECT"}),
        json!({"name": "ANNO"}),
        json!({"name": "ZONE", "print_colour": "#E67E22"}),
        json!({"name": "TEXT", "halo_colour": "#FFFFFF"}),
    ];
    if p.water {
        out.push(json!({"name": "WATER", "print_colour": "#1F5FBF"}));
    }
    if p.grid {
        out.push(json!({"name": "GRID", "print_colour": "#7F7F7F"}));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn sheet(
    p: &PlanParams,
    layout: &furniture::Layout,
    number: &str,
    sheet_title: &str,
    sheet_number: usize,
    count: usize,
    title: &str,
    option_name: &str,
    options: &DrawOptions,
    date: &str,
    notes: &[String],
    viewport: Value,
    backdrop: Backdrop,
) -> Sheet {
    let titles = Titles {
        design: clean(title),
        option_name: option_name.to_string(),
        sheet_title: sheet_title.to_string(),
        number: number.to_string(),
        revision: options.revision.clone(),
        date: date.to_string(),
        sheet_number,
        sheet_count: count,
    };
    let mut document = furniture::document(
        &p.sheet_size,
        p.landscape,
        layout,
        &titles,
        notes,
        &[],
        vec![viewport],
    );
    // an image needs version 1.1 and the colour print profile (clauses 9.10.5 and 18.2.1)
    document["standard_version"] = json!("1.1");
    document["sheet"]["print_profile"] = json!("colour");
    document["layers"] = json!(layers(p));
    Sheet {
        kind: SheetKind::Plan,
        number: number.to_string(),
        document,
        extra: vec![(backdrop.path.clone(), backdrop.jpeg)],
    }
}
