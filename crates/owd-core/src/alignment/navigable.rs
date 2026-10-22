//! Navigable water and the per-station width check (spec 0005 Q4, R2-Q3, L3).
//!
//! Navigable water is the part of the water a design line may use: the water shrunk by half the
//! channel's straight bottom width plus a clearance, then smoothed by an opening and a closing,
//! so that projecting a line onto its edge does not invent tight bends. The width check then
//! tests each station of a line: the bend-widened bottom width at the station's measured local
//! radius, against the width of the water across the line there.

use super::PyError;
use super::radius_check::radius_ok;
use crate::geom::offset::{SHAPELY_QUAD_SEGS, buffer_polygons_q, close, open};
use crate::geom::overlay::contains_xy;
use crate::geom::{Polygon, Pt, linear};
use crate::rules::rulesets::ChannelShape;

/// Spacing of the stations the width check reads (m). Stretch ends are refined to `EDGE_TOL_M`.
pub const STATION_STEP_M: f64 = 10.0;
/// How closely a stretch's start and end chainage are found (m).
pub const EDGE_TOL_M: f64 = 0.01;

/// Why a stretch of a design line is infeasible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchReason {
    /// The water is narrower than the bend-widened channel.
    TooNarrow,
    /// The channel fits, but the line turns tighter than R_min there.
    RMinNotPossible,
}

impl StretchReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StretchReason::TooNarrow => "too_narrow",
            StretchReason::RMinNotPossible => "r_min_not_possible",
        }
    }
}

/// An infeasible stretch of a line: chainage and position of each end, and the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Stretch {
    pub start_chainage_m: f64,
    pub end_chainage_m: f64,
    pub start: Pt,
    pub end: Pt,
    pub reason: StretchReason,
}

/// The navigable water of `water`: shrunk by `half_width_m + clearance_m`, then opened and closed
/// by `smooth_m` (no smoothing at 0). Empty when nothing is left. Keep `smooth_m` under half the
/// narrowest navigable width you want kept: an opening removes anything narrower than twice it.
pub fn navigable_water(
    water: &[Polygon],
    half_width_m: f64,
    clearance_m: f64,
    smooth_m: f64,
) -> Result<Vec<Polygon>, PyError> {
    for (name, v) in [
        ("half_width_m", half_width_m),
        ("clearance_m", clearance_m),
        ("smooth_m", smooth_m),
    ] {
        if !(v.is_finite() && v >= 0.0) {
            return Err(PyError::value(format!(
                "{name} must be finite and not negative."
            )));
        }
    }
    let shrink = half_width_m + clearance_m;
    let mut nav = buffer_polygons_q(water, -shrink, SHAPELY_QUAD_SEGS);
    if smooth_m > 0.0 {
        nav = open(&nav, smooth_m, SHAPELY_QUAD_SEGS);
        nav = close(&nav, smooth_m, SHAPELY_QUAD_SEGS);
    }
    Ok(nav)
}

/// A line with its cumulative chainage, for reading points and directions by chainage.
pub(crate) struct Chained<'a> {
    pub line: &'a [Pt],
    pub cum: Vec<f64>,
}

impl<'a> Chained<'a> {
    pub fn new(line: &'a [Pt]) -> Self {
        let mut cum = Vec::with_capacity(line.len());
        let mut s = 0.0;
        cum.push(0.0);
        for w in line.windows(2) {
            s += crate::geom::dist(w[0], w[1]);
            cum.push(s);
        }
        Chained { line, cum }
    }

    pub fn length(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }

    /// The point at chainage `s`, clamped to the line.
    pub fn at(&self, s: f64) -> Pt {
        let s = s.clamp(0.0, self.length());
        let i = self
            .cum
            .partition_point(|&c| c <= s)
            .clamp(1, self.line.len() - 1);
        let (a, b) = (self.line[i - 1], self.line[i]);
        let seg = self.cum[i] - self.cum[i - 1];
        if seg <= 0.0 {
            return a;
        }
        let t = (s - self.cum[i - 1]) / seg;
        [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
    }

    /// The part of the line from chainage `a` to `b` (clamped, `a <= b`), with interpolated ends.
    pub fn slice(&self, a: f64, b: f64) -> Vec<Pt> {
        let (a, b) = (a.clamp(0.0, self.length()), b.clamp(0.0, self.length()));
        let mut out = vec![self.at(a)];
        for (i, &c) in self.cum.iter().enumerate() {
            if c > a && c < b {
                out.push(self.line[i]);
            }
        }
        out.push(self.at(b));
        out
    }

    /// Points every `length / n` metres, `n = ceil(length / step)` (at least `min_segs`), ends
    /// included.
    pub fn resample(&self, step: f64, min_segs: usize) -> Vec<Pt> {
        let len = self.length();
        let n = ((len / step).ceil() as usize).max(min_segs);
        (0..=n)
            .map(|i| self.at(len * i as f64 / n as f64))
            .collect()
    }

    /// The unit direction at chainage `s`, read across `±half` metres. A window that runs off an
    /// end of the line is moved back onto it, so an end still reads the line's own direction.
    pub fn direction(&self, s: f64, half: f64) -> Pt {
        let len = self.length();
        let w = (2.0 * half).min(len);
        let a = (s - half).clamp(0.0, (len - w).max(0.0));
        let (p, q) = (self.at(a), self.at(a + w));
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let l = (dx * dx + dy * dy).sqrt();
        if l > 0.0 {
            [dx / l, dy / l]
        } else {
            [1.0, 0.0]
        }
    }

    /// The circumradius through the points half a chord before and after `s` and `s` itself:
    /// the local radius read on the chord, as `min_radius_on_chord` reads it. The window is
    /// kept on the line; infinity on a line shorter than the chord, or a straight window.
    pub fn local_radius(&self, s: f64, chord_m: f64) -> f64 {
        let half = chord_m / 2.0;
        let len = self.length();
        if len < chord_m {
            return f64::INFINITY;
        }
        let s = s.clamp(half, len - half);
        circumradius(self.at(s - half), self.at(s), self.at(s + half))
    }
}

/// The radius of the circle through three points; infinity when they are (nearly) collinear.
pub(crate) fn circumradius(a_: Pt, b_: Pt, c_: Pt) -> f64 {
    let a = libm::hypot(b_[0] - c_[0], b_[1] - c_[1]);
    let b = libm::hypot(a_[0] - c_[0], a_[1] - c_[1]);
    let c = libm::hypot(a_[0] - b_[0], a_[1] - b_[1]);
    let area = 0.5 * ((b_[0] - a_[0]) * (c_[1] - a_[1]) - (c_[0] - a_[0]) * (b_[1] - a_[1])).abs();
    if area > 1e-9 {
        a * b * c / (4.0 * area)
    } else {
        f64::INFINITY
    }
}

/// The distances from `p` to the water's edge along `+n` and `-n` (the cross-section through `p`
/// along unit normal `n`). Both zero when `p` is not inside the water.
pub fn cross_section(p: Pt, n: Pt, water: &[Polygon]) -> (f64, f64) {
    if !contains_xy(water, p) {
        return (0.0, 0.0);
    }
    let (mut pos, mut neg) = (f64::INFINITY, f64::INFINITY);
    let rings = water
        .iter()
        .flat_map(|poly| core::iter::once(&poly.exterior).chain(&poly.holes));
    for ring in rings {
        for w in ring.windows(2) {
            let (a, b) = (w[0], w[1]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let den = n[0] * d[1] - n[1] * d[0];
            if den == 0.0 {
                continue;
            }
            let ap = [a[0] - p[0], a[1] - p[1]];
            let u = (ap[0] * d[1] - ap[1] * d[0]) / den;
            let v = (ap[0] * n[1] - ap[1] * n[0]) / den;
            if (0.0..=1.0).contains(&v) {
                if u >= 0.0 {
                    pos = pos.min(u);
                } else {
                    neg = neg.min(-u);
                }
            }
        }
    }
    (pos, neg)
}

/// [`cross_section`] for many stations in one water: the same distances, found through a grid of
/// the rings' segments. The ray is walked through the grid cell by cell, each way from `p`, and a
/// way stops once the next cell starts farther than its nearest hit so far (with a metre to spare,
/// so a hit is never missed for a rounding in the cell walk): every segment that could be nearest
/// has then been read, with the very formula [`cross_section`] reads it with.
pub struct WaterIndex<'a> {
    water: &'a [Polygon],
    inside: crate::geom::overlay::PolygonIndex,
    segs: Vec<(Pt, Pt)>,
    origin: Pt,
    cell: f64,
    cols: i64,
    rows: i64,
    cells: Vec<Vec<u32>>,
}

impl<'a> WaterIndex<'a> {
    pub fn new(water: &'a [Polygon]) -> WaterIndex<'a> {
        let segs: Vec<(Pt, Pt)> = water
            .iter()
            .flat_map(|poly| core::iter::once(&poly.exterior).chain(&poly.holes))
            .flat_map(|ring| ring.windows(2).map(|w| (w[0], w[1])))
            .collect();
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (a, b) in &segs {
            for q in [a, b] {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let cell = (span / (segs.len().max(1) as f64).sqrt()).max(1.0);
        let cols = ((hi[0] - lo[0]) / cell).floor() as i64 + 1;
        let rows = ((hi[1] - lo[1]) / cell).floor() as i64 + 1;
        let mut cells = vec![
            Vec::new();
            if segs.is_empty() {
                0
            } else {
                (cols * rows) as usize
            }
        ];
        for (i, (a, b)) in segs.iter().enumerate() {
            let c0 = ((a[0].min(b[0]) - lo[0]) / cell).floor() as i64;
            let c1 = ((a[0].max(b[0]) - lo[0]) / cell).floor() as i64;
            let r0 = ((a[1].min(b[1]) - lo[1]) / cell).floor() as i64;
            let r1 = ((a[1].max(b[1]) - lo[1]) / cell).floor() as i64;
            for r in r0..=r1.min(rows - 1) {
                for c in c0..=c1.min(cols - 1) {
                    cells[(r * cols + c) as usize].push(i as u32);
                }
            }
        }
        WaterIndex {
            water,
            inside: crate::geom::overlay::PolygonIndex::new(water),
            segs,
            origin: lo,
            cell,
            cols,
            rows,
            cells,
        }
    }

    /// The hit of the ray `p + u n` on segment `i`, as [`cross_section`] computes it.
    fn hit(&self, i: usize, p: Pt, n: Pt) -> Option<f64> {
        let (a, b) = self.segs[i];
        let d = [b[0] - a[0], b[1] - a[1]];
        let den = n[0] * d[1] - n[1] * d[0];
        if den == 0.0 {
            return None;
        }
        let ap = [a[0] - p[0], a[1] - p[1]];
        let u = (ap[0] * d[1] - ap[1] * d[0]) / den;
        let v = (ap[0] * n[1] - ap[1] * n[0]) / den;
        (0.0..=1.0).contains(&v).then_some(u)
    }

    /// [`water_width_at`].
    pub(crate) fn width_at(&self, c: &Chained, s: f64) -> f64 {
        let t = c.direction(s, STATION_STEP_M / 2.0);
        let (l, r) = self.cross_section(c.at(s), [-t[1], t[0]]);
        l + r
    }

    /// [`cross_section`].
    pub fn cross_section(&self, p: Pt, n: Pt) -> (f64, f64) {
        if !self.inside.contains_xy(p) {
            return (0.0, 0.0);
        }
        if !(n[0].is_finite() && n[1].is_finite()) || n == [0.0, 0.0] || self.segs.is_empty() {
            return cross_section(p, n, self.water);
        }
        let (mut pos, mut neg) = (f64::INFINITY, f64::INFINITY);
        let mut seen = vec![false; self.segs.len()];
        for dir in [1.0, -1.0] {
            let m = [n[0] * dir, n[1] * dir];
            // Amanatides-Woo: the cells the ray from `p` along `m` passes through, in order.
            let fx = (p[0] - self.origin[0]) / self.cell;
            let fy = (p[1] - self.origin[1]) / self.cell;
            let (mut cx, mut cy) = (fx.floor() as i64, fy.floor() as i64);
            let step_x: i64 = if m[0] > 0.0 { 1 } else { -1 };
            let step_y: i64 = if m[1] > 0.0 { 1 } else { -1 };
            let next = |f: f64, c: i64, md: f64, st: i64| {
                if md == 0.0 {
                    f64::INFINITY
                } else {
                    let edge = if st > 0 { (c + 1) as f64 } else { c as f64 };
                    (edge - f) * self.cell / md
                }
            };
            let mut t_x = next(fx, cx, m[0], step_x);
            let mut t_y = next(fy, cy, m[1], step_y);
            let dt_x = if m[0] == 0.0 {
                f64::INFINITY
            } else {
                self.cell / m[0].abs()
            };
            let dt_y = if m[1] == 0.0 {
                f64::INFINITY
            } else {
                self.cell / m[1].abs()
            };
            let mut t_enter = 0.0;
            loop {
                let best = if dir > 0.0 { pos } else { neg };
                if t_enter > best + 1.0 {
                    break;
                }
                if cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows {
                    for &i in &self.cells[(cy * self.cols + cx) as usize] {
                        let i = i as usize;
                        if seen[i] {
                            continue;
                        }
                        seen[i] = true;
                        if let Some(u) = self.hit(i, p, n) {
                            if u >= 0.0 {
                                pos = pos.min(u);
                            } else {
                                neg = neg.min(-u);
                            }
                        }
                    }
                } else {
                    // Outside the grid and moving away from it: nothing more to read.
                    let away_x = (cx < 0 && step_x < 0)
                        || (cx >= self.cols && step_x > 0)
                        || m[0] == 0.0 && (cx < 0 || cx >= self.cols);
                    let away_y = (cy < 0 && step_y < 0)
                        || (cy >= self.rows && step_y > 0)
                        || m[1] == 0.0 && (cy < 0 || cy >= self.rows);
                    if away_x || away_y {
                        break;
                    }
                }
                if t_x < t_y {
                    t_enter = t_x;
                    t_x += dt_x;
                    cx += step_x;
                } else {
                    t_enter = t_y;
                    t_y += dt_y;
                    cy += step_y;
                }
            }
        }
        (pos, neg)
    }
}

/// [`local_radius_on`] for many chainages of one line: the line chained once.
pub struct RadiusOn<'a>(Chained<'a>);

impl<'a> RadiusOn<'a> {
    pub fn new(line: &'a [Pt]) -> RadiusOn<'a> {
        RadiusOn(Chained::new(line))
    }

    /// [`local_radius_on`] at chainage `s`, read on `chord_m`.
    pub fn at(&self, s: f64, chord_m: f64) -> f64 {
        self.0.local_radius(s, chord_m)
    }
}

/// The width of the water across a line at its stations, for many stations: [`water_width_on`],
/// with the line chained once and the water indexed once.
pub struct WaterWidth<'a> {
    line: Chained<'a>,
    water: WaterIndex<'a>,
}

impl<'a> WaterWidth<'a> {
    pub fn new(line: &'a [Pt], water: &'a [Polygon]) -> WaterWidth<'a> {
        WaterWidth {
            line: Chained::new(line),
            water: WaterIndex::new(water),
        }
    }

    /// [`water_width_on`] at chainage `s`.
    pub fn at(&self, s: f64) -> f64 {
        self.water.width_at(&self.line, s)
    }
}

/// The width of the water across `line` at chainage `s` (m): the cross-section at right angles
/// to the line. Zero when the station is not in the water.
pub(crate) fn water_width_at(c: &Chained, s: f64, water: &[Polygon]) -> f64 {
    let t = c.direction(s, STATION_STEP_M / 2.0);
    let (l, r) = cross_section(c.at(s), [-t[1], t[0]], water);
    l + r
}

/// The local radius of `line` at chainage `s`, read on `chord_m` (as the radius checks read it):
/// infinity on a straight or a line shorter than the chord.
pub fn local_radius_on(line: &[Pt], s: f64, chord_m: f64) -> f64 {
    Chained::new(line).local_radius(s, chord_m)
}

/// The width of the water across `line` at chainage `s` (m); zero where the station is not in the
/// water.
pub fn water_width_on(line: &[Pt], s: f64, water: &[Polygon]) -> f64 {
    water_width_at(&Chained::new(line), s, water)
}

/// The per-station width check. At each station (every `STATION_STEP_M`, and the end), the
/// bend-widened bottom width at the measured local radius (read on `chord_m`) is compared with
/// the width of the water across the line. A station is `too_narrow` when the water is narrower
/// than that width, else `r_min_not_possible` when the local radius is under R_min (beyond the
/// reading tolerance), else it passes. Runs of failing stations become stretches; each end is
/// found to `EDGE_TOL_M` between the last passing and the first failing station.
///
pub fn width_check(
    line: &[Pt],
    water: &[Polygon],
    shape: &ChannelShape,
    r_min: f64,
    chord_m: f64,
) -> Result<Vec<Stretch>, PyError> {
    if !(chord_m.is_finite() && chord_m > 0.0) {
        return Err(PyError::value("chord_m must be finite and positive."));
    }
    if line.len() < 2 {
        return Err(PyError::value("the line needs at least two points."));
    }
    let c = Chained::new(line);
    let len = c.length();
    let index = WaterIndex::new(water);
    let status = |s: f64| -> Option<StretchReason> {
        let r = c.local_radius(s, chord_m);
        if index.width_at(&c, s) < shape.bottom_width_at_m(r) {
            Some(StretchReason::TooNarrow)
        } else if !radius_ok(r, r_min) {
            Some(StretchReason::RMinNotPossible)
        } else {
            None
        }
    };
    let n = (len / STATION_STEP_M).ceil() as usize;
    let stations: Vec<f64> = (0..=n)
        .map(|i| (i as f64 * STATION_STEP_M).min(len))
        .collect();
    // The chainage, between a and b, where the status changes from status(a).
    let edge = |mut a: f64, mut b: f64| {
        let sa = status(a);
        while b - a > EDGE_TOL_M {
            let m = (a + b) / 2.0;
            if status(m) == sa { a = m } else { b = m }
        }
        (a + b) / 2.0
    };
    let mut out: Vec<Stretch> = Vec::new();
    let mut open_run: Option<(f64, StretchReason)> = None;
    let mut prev: Option<(f64, Option<StretchReason>)> = None;
    for &s in &stations {
        let st = status(s);
        let changed = prev.map(|(_, p)| p != st);
        if let Some((ps, _)) = prev
            && changed == Some(true)
        {
            let at = edge(ps, s);
            if let Some((start, reason)) = open_run.take() {
                out.push(stretch(&c, start, at, reason));
            }
            if let Some(r) = st {
                open_run = Some((at, r));
            }
        } else if prev.is_none()
            && let Some(r) = st
        {
            open_run = Some((0.0, r));
        }
        prev = Some((s, st));
    }
    if let Some((start, reason)) = open_run {
        out.push(stretch(&c, start, len, reason));
    }
    Ok(out)
}

fn stretch(c: &Chained, start: f64, end: f64, reason: StretchReason) -> Stretch {
    Stretch {
        start_chainage_m: start,
        end_chainage_m: end,
        start: c.at(start),
        end: c.at(end),
        reason,
    }
}

/// Whether every station of `line` (every `step_m`, and the end) lies inside navigable water,
/// outside the chainage ranges in `skip`. The property the fairway fit promises.
pub fn inside_outside_stretches(
    line: &[Pt],
    navigable: &[Polygon],
    skip: &[Stretch],
    step_m: f64,
) -> bool {
    let len = linear::length(line);
    let c = Chained::new(line);
    let n = (len / step_m).ceil() as usize;
    (0..=n).map(|i| (i as f64 * step_m).min(len)).all(|s| {
        skip.iter()
            .any(|st| s >= st.start_chainage_m && s <= st.end_chainage_m)
            || contains_xy(navigable, c.at(s))
    })
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (fixture answers in `synthetic::fixtures`; PIANC-B with
    //! CEMT IV gives R_min 255 m and a 30.4 m straight bottom width, spec 0005 worked example)
    use super::*;
    use crate::synthetic::{RiverParams, narrowing_reach, synthetic_river};

    fn shape() -> ChannelShape {
        crate::rules::rulesets::channel_shape_for(
            &crate::rules::engine::selection_for_old_ruleset("PIANC-B").unwrap(),
            "CEMT_IV",
            Some(2.5),
        )
        .unwrap()
    }

    #[test]
    fn the_worked_example_shape_is_r_min_255_and_30_4_m_wide() {
        let s = shape();
        assert!((s.min_bend_radius_m - 255.0).abs() < 1e-9);
        assert!((s.bottom_width_straight_m - 30.4).abs() < 1e-9);
    }

    #[test]
    fn on_the_narrowing_reach_exactly_the_narrow_stretch_is_too_narrow() {
        let f = narrowing_reach(60.0, 20.0);
        let water = core::slice::from_ref(&f.water);
        let out = width_check(&f.legs[0], water, &shape(), 255.0, 85.0).unwrap();
        assert_eq!(out.len(), 1, "{out:?}");
        let st = &out[0];
        assert_eq!(st.reason, StretchReason::TooNarrow);
        assert!((st.start_chainage_m - 974.0).abs() <= EDGE_TOL_M, "{st:?}");
        assert!((st.end_chainage_m - 1526.0).abs() <= EDGE_TOL_M, "{st:?}");
        assert!((st.start[0] - 974.0).abs() <= EDGE_TOL_M && st.start[1] == 0.0);
        assert!((st.end[0] - 1526.0).abs() <= EDGE_TOL_M && st.end[1] == 0.0);
    }

    #[test]
    fn the_check_fails_on_a_corrupted_input() {
        // Widen the narrow reach to 40 m: nothing is reported, so the check above can fail.
        let f = narrowing_reach(60.0, 40.0);
        let out = width_check(
            &f.legs[0],
            core::slice::from_ref(&f.water),
            &shape(),
            255.0,
            85.0,
        )
        .unwrap();
        assert!(out.is_empty(), "{out:?}");
        // A line pushed onto land is too narrow there (width zero).
        let land = vec![
            [0.0, 0.0],
            [500.0, 0.0],
            [600.0, 80.0],
            [700.0, 0.0],
            [3000.0, 0.0],
        ];
        let f = narrowing_reach(60.0, 60.0);
        let out = width_check(
            &land,
            core::slice::from_ref(&f.water),
            &shape(),
            255.0,
            85.0,
        )
        .unwrap();
        assert!(out.iter().any(|s| s.reason == StretchReason::TooNarrow
            && s.start_chainage_m < 600.0
            && s.end_chainage_m > 600.0));
    }

    #[test]
    fn a_tight_bend_in_wide_water_is_r_min_not_possible() {
        // A 100 m radius quarter circle inside a 2 km square of water.
        let mut line: Vec<Pt> = (0..=60)
            .map(|i| {
                let th = core::f64::consts::FRAC_PI_2 * i as f64 / 60.0;
                [100.0 * libm::sin(th), 100.0 * (1.0 - libm::cos(th))]
            })
            .collect();
        line.insert(0, [-500.0, 0.0]);
        line.push([100.0, 600.0]);
        let water = Polygon {
            exterior: vec![
                [-1000.0, -1000.0],
                [1000.0, -1000.0],
                [1000.0, 1000.0],
                [-1000.0, 1000.0],
                [-1000.0, -1000.0],
            ],
            holes: vec![],
        };
        let out = width_check(&line, &[water], &shape(), 255.0, 85.0).unwrap();
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].reason, StretchReason::RMinNotPossible);
        // The arc runs from chainage 500 to 500 + 50π.
        assert!(out[0].start_chainage_m > 450.0 && out[0].end_chainage_m < 700.0);
    }

    #[test]
    fn the_40_m_synthetic_river_is_under_10_m_navigable_and_too_narrow_at_every_apex() {
        let river = synthetic_river(RiverParams::default()).unwrap();
        let water = core::slice::from_ref(&river.polygon);
        let s = shape();
        let nav = navigable_water(water, s.bottom_width_straight_m / 2.0, 0.0, 2.0).unwrap();
        assert!(!nav.is_empty());
        let c = Chained::new(&river.centreline);
        let mut widest: f64 = 0.0;
        let mut st = 0.0;
        while st <= c.length() {
            widest = widest.max(water_width_at(&c, st, &nav));
            st += 25.0;
        }
        assert!(
            widest > 0.0 && widest < 10.0,
            "widest navigable cross-section {widest}"
        );
        let out = width_check(&river.centreline, water, &s, 255.0, 85.0).unwrap();
        // Apexes of y = 260 sin(2πx/1500): x = 375 + 750 k.
        for k in 0..8 {
            let x = 375.0 + 750.0 * k as f64;
            let at = linear::project(
                &river.centreline,
                [x, if k % 2 == 0 { 260.0 } else { -260.0 }],
            );
            assert!(
                out.iter().any(|st| st.reason == StretchReason::TooNarrow
                    && st.start_chainage_m < at
                    && at < st.end_chainage_m),
                "apex at x = {x} (chainage {at}) not too narrow: {out:?}"
            );
        }
    }

    #[test]
    fn navigable_water_refuses_negative_inputs_and_keeps_the_open_basin_corner() {
        assert!(navigable_water(&[], -1.0, 0.0, 0.0).is_err());
        let f = crate::synthetic::open_basin_90(1500.0, 200.0);
        let nav = navigable_water(core::slice::from_ref(&f.water), 15.2, 0.0, 0.0).unwrap();
        let r = crate::synthetic::open_basin_max_radius(200.0, 15.2);
        let arc = |r: f64| -> Vec<Pt> {
            (0..=90)
                .map(|i| {
                    let th = core::f64::consts::FRAC_PI_2 * i as f64 / 90.0;
                    [r - r * libm::cos(th), -r + r * libm::sin(th)]
                })
                .collect()
        };
        assert!(arc(r - 0.5).iter().all(|p| contains_xy(&nav, *p)));
        assert!(!arc(r + 0.5).iter().all(|p| contains_xy(&nav, *p)));
    }
}

#[cfg(test)]
mod index_tests {
    //! Label: property (the indexes give what the scans give, to the last bit)
    use super::*;
    use crate::geom::overlay::{PolygonIndex, locate_all};

    /// A river-like polygon: banks either side of a wandering line, with a hole (an island) and
    /// a run of horizontal edges.
    fn river(next: &mut impl FnMut() -> f64, n: usize) -> Polygon {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let (mut x, mut y, mut h): (f64, f64, f64) = (0.0, 0.0, 0.3);
        for i in 0..n {
            h += (next() - 0.5) * 0.5;
            x += 40.0 * libm::cos(h);
            y += 40.0 * libm::sin(h);
            let w = 80.0 + next() * 200.0;
            let (nx, ny) = (-libm::sin(h), libm::cos(h));
            // Every 15th pair at a round height, so horizontal edges and exact ties occur.
            let yy = if i % 15 == 0 { y.round() } else { y };
            left.push([x + nx * w, yy + ny * w]);
            right.push([x - nx * w, yy - ny * w]);
        }
        let mut ring = left.clone();
        ring.extend(right.into_iter().rev());
        ring.push(ring[0]);
        let c = left[n / 2];
        let island = vec![
            [c[0] - 30.0, c[1] - 30.0],
            [c[0] - 10.0, c[1] - 30.0],
            [c[0] - 10.0, c[1] - 10.0],
            [c[0] - 30.0, c[1] - 10.0],
            [c[0] - 30.0, c[1] - 30.0],
        ];
        Polygon {
            exterior: ring,
            holes: vec![island],
        }
    }

    #[test]
    fn the_indexes_locate_and_cross_as_the_scans_do() {
        let mut x: u64 = 3;
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        for case in 0..25 {
            let n = 10 + (next() * 300.0) as usize;
            let water = vec![river(&mut next, n)];
            let idx = PolygonIndex::new(&water);
            let wi = WaterIndex::new(&water);
            let ring = water[0].exterior.clone();
            for k in 0..600 {
                let p = if k % 10 == 0 {
                    // On a vertex, or on a horizontal level of one.
                    let v = ring[(next() * ring.len() as f64) as usize % ring.len()];
                    if k % 20 == 0 {
                        v
                    } else {
                        [v[0] + (next() - 0.5) * 50.0, v[1]]
                    }
                } else {
                    let v = ring[(next() * ring.len() as f64) as usize % ring.len()];
                    [v[0] + (next() - 0.5) * 600.0, v[1] + (next() - 0.5) * 600.0]
                };
                assert_eq!(
                    idx.locate_all(p),
                    locate_all(p, &water),
                    "case {case}, {p:?}"
                );
                let a = next() * std::f64::consts::TAU;
                let dir = [libm::cos(a), libm::sin(a)];
                let (u, v) = wi.cross_section(p, dir);
                let (u0, v0) = cross_section(p, dir, &water);
                assert_eq!(
                    (u.to_bits(), v.to_bits()),
                    (u0.to_bits(), v0.to_bits()),
                    "case {case}, {p:?} {dir:?}"
                );
                // Axis-aligned rays too.
                for d in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
                    let (u, v) = wi.cross_section(p, d);
                    let (u0, v0) = cross_section(p, d, &water);
                    assert_eq!(
                        (u.to_bits(), v.to_bits()),
                        (u0.to_bits(), v0.to_bits()),
                        "case {case}, {p:?} {d:?}"
                    );
                }
            }
        }
    }
}
