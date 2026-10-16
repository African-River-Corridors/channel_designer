//! Least-cost channel route: re-route a centreline laterally to reduce cut, holding R_min
//! (`alignment/route.py`).
//!
//! A cost grid (stations x lateral offsets, one dense terrain transect per station), a dynamic
//! programme over offsets with a heading budget, smoothing, Laplacian radius enforcement, and a
//! corridor sweep. The target radius is never relaxed to the baseline's own radius (issue #9);
//! the winner reports `radius_ok`, the measured radius and the chord it was read on (issue #10).
//!
//! No-cut zones: a bank where no dry ground may be dug over a stretch of chainage (a town, a
//! landholding, a permit that will not be granted). A zone is read against a bank line, normally
//! the river's own centreline: a dry ground point is on the protected bank when its nearest point
//! on the bank line lies in the zone's chainage and it is on the zone's side of that line. (Not the
//! side of the baseline: a re-route across the inside of a bend has dry ground on both of its
//! sides, and all of it is the inner bank.) The grid records the template's dry cut on each
//! protected bank, the search refuses every offset that digs there, and the result reports the
//! protected cut the finished line still has (the smoothing can move it back), so a breach is loud.
//!
//! Search class (ADR 0005): the discrete choices are the DP's predecessor at every state and the
//! corridor the sweep picks. Ties go to the first candidate, as in the Python (strict `<`).

use super::PyError;
use super::radius_check::{RADIUS_TOL, min_radius_on_vertices, radius_ok};
use crate::geom::overlay::contains_xy;
use crate::geom::{Polygon, Pt, linear};
use crate::numpy::{arange, argmin, np_min, np_mod, np_sum};
use crate::py_max;

pub const DEFAULT_CORRIDOR_SWEEP: [f64; 7] = [40.0, 60.0, 80.0, 120.0, 160.0, 220.0, 300.0];

/// A bank of the bank line, looking along it from its first vertex to its last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bank {
    /// The side of the left normal (positive offsets).
    Left,
    /// The side of negative offsets.
    Right,
}

/// No dry ground may be cut on `bank` between chainages `from_m` and `to_m` on the bank line
/// (inclusive). Cut below the river's water (dredging) is still allowed there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoCutZone {
    pub from_m: f64,
    pub to_m: f64,
    pub bank: Bank,
}

impl NoCutZone {
    fn covers(&self, s: f64) -> bool {
        s >= self.from_m.min(self.to_m) - 1e-9 && s <= self.from_m.max(self.to_m) + 1e-9
    }

    fn holds(&self, s: f64, side: f64) -> bool {
        self.covers(s)
            && match self.bank {
                Bank::Left => side > 0.0,
                Bank::Right => side < 0.0,
            }
    }
}

/// The chainage of `p` on `line` (its nearest point; the first nearest segment wins a tie) and
/// the side it is on: > 0 left, < 0 right, 0 on the line.
pub fn chainage_and_side(line: &[Pt], p: Pt) -> (f64, f64) {
    let (mut best, mut s_best, mut side_best) = (f64::INFINITY, f64::NAN, 0.0);
    let mut start = 0.0;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let len = len2.sqrt();
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + t * dx, a[1] + t * dy];
        let d = hypot(p[0] - q[0], p[1] - q[1]);
        if d < best {
            best = d;
            s_best = start + t * len;
            side_best = dx * (p[1] - a[1]) - dy * (p[0] - a[0]);
        }
        start += len;
    }
    (s_best, side_best)
}

/// A terrain sampler: heights for the points `(xs[i], ys[i])`, NaN where unknown.
pub type Terrain<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// `CostGrid`.
#[derive(Debug, Clone, PartialEq)]
pub struct CostGrid {
    pub offsets: Vec<f64>,
    pub stations_m: Vec<f64>,
    pub centres: Vec<Pt>,
    pub normals: Vec<Pt>,
    pub grades: Vec<f64>,
    /// Transect-approximation cut volume per chunk, m3: `[station][offset]`.
    pub vol: Vec<Vec<f64>>,
    /// The in-river (dredge) part of `vol`.
    pub wet: Vec<Vec<f64>>,
    /// The dry part of `vol` on a protected bank (a `NoCutZone`), m3; empty when there are none.
    pub protected: Vec<Vec<f64>>,
    pub station_step: f64,
    pub offset_step: f64,
}

impl CostGrid {
    /// True where the cell digs dry ground on a protected bank.
    pub fn blocked(&self, i: usize, k: usize) -> bool {
        self.protected
            .get(i)
            .and_then(|r| r.get(k))
            .is_some_and(|&v| v > 0.0)
    }

    /// The protected cut of an offset path (each offset snapped to the grid), m3.
    pub fn protected_cut(&self, ovals: &[f64]) -> f64 {
        if self.protected.is_empty() {
            return 0.0;
        }
        let k = self.offsets.len();
        ovals
            .iter()
            .enumerate()
            .map(|(i, &o)| {
                let r = ((o - self.offsets[0]) / self.offset_step).round_ties_even();
                self.protected[i][r.max(0.0).min(k as f64 - 1.0) as usize]
            })
            .sum()
    }

    /// `wet * wet_weight + dry * dry_weight`, where `dry = vol - wet`.
    pub fn weighted(&self, wet_weight: f64, dry_weight: f64) -> Vec<Vec<f64>> {
        self.vol
            .iter()
            .zip(&self.wet)
            .map(|(v, w)| {
                v.iter()
                    .zip(w)
                    .map(|(&v, &w)| w * wet_weight + (v - w) * dry_weight)
                    .collect()
            })
            .collect()
    }
}

/// The keyword arguments of `build_cost_grid`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridParams {
    pub bottom_width: f64,
    pub depth: f64,
    pub side_slope: f64,
    pub corridor_half: f64,
    pub station_step: f64,
    pub offset_step: f64,
    pub transect_step: f64,
    pub daylight_m: Option<f64>,
}

/// `RouteResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteResult {
    pub line: Vec<Pt>,
    pub offsets: Vec<f64>,
    pub min_radius_m: f64,
    pub corridor_half_m: f64,
    pub approx_saving_pct: f64,
    pub smooth_passes: i64,
    /// `(corridor, saving %, min R)` per candidate.
    pub sweep: Vec<(f64, f64, f64)>,
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    pub radius_ok: bool,
    /// Grid-approx dry cut of `line` on a protected bank, m3 (0 with no zones).
    pub protected_cut_m3: f64,
    /// `protected_cut_m3 == 0`: the line digs no protected bank.
    pub no_cut_ok: bool,
}

fn hypot(a: f64, b: f64) -> f64 {
    libm::hypot(a, b)
}

/// Transect-approximation cut volume for every (station, offset).
pub fn build_cost_grid(
    line: &[Pt],
    terrain: Terrain,
    river: &[Polygon],
    grade_at: &dyn Fn(f64) -> f64,
    p: &GridParams,
) -> Result<CostGrid, PyError> {
    build_cost_grid_with(line, terrain, river, grade_at, p, &[], None)
}

/// `build_cost_grid` with no-cut zones: `protected` holds each cell's dry cut on a protected bank.
/// The zones are read on `bank_line` (the river centreline), or on the baseline when it is None.
pub fn build_cost_grid_with(
    line: &[Pt],
    terrain: Terrain,
    river: &[Polygon],
    grade_at: &dyn Fn(f64) -> f64,
    p: &GridParams,
    no_cut: &[NoCutZone],
    bank_line: Option<&[Pt]>,
) -> Result<CostGrid, PyError> {
    let bank_line = bank_line.unwrap_or(line);
    if !no_cut.is_empty() && bank_line.len() < 2 {
        return Err(PyError::value("the bank line needs at least two points."));
    }
    for z in no_cut {
        if !(z.from_m.is_finite() && z.to_m.is_finite()) {
            return Err(PyError::value("a no-cut zone needs finite chainages."));
        }
    }
    let half_bottom = p.bottom_width / 2.0;
    let total_m = linear::length(line);
    let stations_m: Vec<f64> = arange(0.0, total_m + 0.5 * p.station_step, p.station_step)
        .into_iter()
        .filter(|&s| s <= total_m)
        .collect();
    let offsets = arange(
        -p.corridor_half,
        p.corridor_half + p.offset_step,
        p.offset_step,
    );
    let daylight = p.daylight_m.unwrap_or(half_bottom + p.side_slope * p.depth);
    let t_half = p.corridor_half + daylight + p.transect_step;
    let t_off = arange(-t_half, t_half + p.transect_step, p.transect_step);
    let n_toff = t_off.len();
    let n = stations_m.len();

    let at = |d: f64| linear::interpolate(line, d).unwrap_or([f64::NAN, f64::NAN]);
    let mut centres = Vec::with_capacity(n);
    let mut normals = Vec::with_capacity(n);
    let mut sx = Vec::with_capacity(n * n_toff);
    let mut sy = Vec::with_capacity(n * n_toff);
    for &sm in &stations_m {
        let pt = at(sm);
        let p1 = at(py_max(0.0, sm - 5.0));
        let p2 = at(crate::py_min(total_m, sm + 5.0));
        let (dx, dy) = (p2[0] - p1[0], p2[1] - p1[1]);
        let mut norm = hypot(dx, dy);
        if norm == 0.0 {
            norm = 1.0;
        }
        let (tx, ty) = (dx / norm, dy / norm);
        let (nx, ny) = (-ty, tx);
        centres.push(pt);
        normals.push([nx, ny]);
        for &t in &t_off {
            sx.push(pt[0] + t * nx);
            sy.push(pt[1] + t * ny);
        }
    }
    let heights = terrain(&sx, &sy);
    if heights.len() != n * n_toff {
        return Err(PyError::value(format!(
            "cannot reshape array of size {} into shape ({n},{n_toff})",
            heights.len()
        )));
    }
    let wet_flags: Vec<bool> = sx
        .iter()
        .zip(&sy)
        .map(|(&x, &y)| contains_xy(river, [x, y]))
        .collect();
    let grades: Vec<f64> = stations_m.iter().map(|&s| grade_at(s)).collect();

    let n_off = offsets.len();
    let mut vol = vec![vec![f64::NAN; n_off]; n];
    let mut wet = vec![vec![f64::NAN; n_off]; n];
    let mut protected = if no_cut.is_empty() {
        Vec::new()
    } else {
        vec![vec![0.0; n_off]; n]
    };
    // Which transect points are protected ground: dry, and on a zone's bank.
    let guarded: Vec<bool> = if no_cut.is_empty() {
        Vec::new()
    } else {
        sx.iter()
            .zip(&sy)
            .zip(&wet_flags)
            .map(|((&x, &y), &wf)| {
                !wf && {
                    let (s, side) = chainage_and_side(bank_line, [x, y]);
                    no_cut.iter().any(|z| z.holds(s, side))
                }
            })
            .collect()
    };
    let da = p.transect_step * p.station_step;
    for i in 0..n {
        let g = &heights[i * n_toff..(i + 1) * n_toff];
        let w = &wet_flags[i * n_toff..(i + 1) * n_toff];
        let valid: Vec<bool> = g.iter().map(|z| z.is_finite()).collect();
        if valid.iter().filter(|&&v| v).count() < 5 {
            vol[i] = vec![0.0; n_off];
            wet[i] = vec![0.0; n_off];
            continue;
        }
        for (k, &off) in offsets.iter().enumerate() {
            let cut: Vec<f64> = (0..n_toff)
                .map(|j| {
                    let local = (t_off[j] - off).abs();
                    let over = local - half_bottom;
                    let template = grades[i] + (if over < 0.0 { 0.0 } else { over }) / p.side_slope;
                    let c = if valid[j] { g[j] - template } else { 0.0 };
                    // np.clip(cut, 0, None): NaN stays NaN
                    if c < 0.0 { 0.0 } else { c }
                })
                .collect();
            let wet_cut: Vec<f64> = cut
                .iter()
                .zip(w)
                .filter(|(_, wf)| **wf)
                .map(|(c, _)| *c)
                .collect();
            vol[i][k] = np_sum(&cut) * da;
            wet[i][k] = np_sum(&wet_cut) * da;
            if !guarded.is_empty() {
                let gd = &guarded[i * n_toff..(i + 1) * n_toff];
                let dug: f64 = (0..n_toff)
                    .filter(|&j| gd[j])
                    .map(|j| cut[j])
                    .filter(|c| c.is_finite())
                    .sum();
                protected[i][k] = dug * da;
            }
        }
    }
    Ok(CostGrid {
        offsets,
        stations_m,
        centres,
        normals,
        grades,
        vol,
        wet,
        protected,
        station_step: p.station_step,
        offset_step: p.offset_step,
    })
}

/// Three-point circumradius at every interior vertex (infinity on the ends and where the three
/// points are nearly collinear or coincident).
pub fn radii(pts: &[Pt]) -> Vec<f64> {
    let n = pts.len();
    let mut r = vec![f64::INFINITY; n];
    for i in 1..n.saturating_sub(1) {
        let (a_, b_, c_) = (pts[i - 1], pts[i], pts[i + 1]);
        let a = hypot(b_[0] - c_[0], b_[1] - c_[1]);
        let b = hypot(a_[0] - c_[0], a_[1] - c_[1]);
        let c = hypot(a_[0] - b_[0], a_[1] - b_[1]);
        let area =
            0.5 * ((b_[0] - a_[0]) * (c_[1] - a_[1]) - (c_[0] - a_[0]) * (b_[1] - a_[1])).abs();
        if area < 1e-9 || a * b * c < 1e-12 {
            continue;
        }
        r[i] = a * b * c / (4.0 * area);
    }
    r
}

/// Minimum radius of curvature of the polyline and its vertex index.
pub fn min_radius_of_line(pts: &[Pt]) -> (f64, usize) {
    let r = radii(pts);
    let i = argmin(&r);
    (r[i], i)
}

/// Signed per-station turn (rad, > 0 left) of the baseline between station chords.
pub fn baseline_turn_angles(centres: &[Pt]) -> Vec<f64> {
    let n = centres.len();
    let pi = core::f64::consts::PI;
    let mut beta = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        let a1 = libm::atan2(
            centres[i][1] - centres[i - 1][1],
            centres[i][0] - centres[i - 1][0],
        );
        let a2 = libm::atan2(
            centres[i + 1][1] - centres[i][1],
            centres[i + 1][0] - centres[i][0],
        );
        beta[i] = np_mod(a2 - a1 + pi, 2.0 * pi) - pi;
    }
    beta
}

/// Per-station admissible offset window `[o_lo, o_hi]` so a parallel offset cannot fold the
/// alignment below `target_r` on the inside of a baseline bend.
pub fn offset_bounds(centres: &[Pt], target_r: f64) -> (Vec<f64>, Vec<f64>) {
    let n = centres.len();
    let r = radii(centres);
    let beta = baseline_turn_angles(centres);
    let mut lo = vec![f64::NEG_INFINITY; n];
    let mut hi = vec![f64::INFINITY; n];
    for i in 1..n.saturating_sub(1) {
        let rho = r[i];
        if !rho.is_finite() {
            continue;
        }
        let margin = py_max(rho - target_r, 0.0);
        if beta[i] > 1e-6 {
            hi[i] = margin;
        } else if beta[i] < -1e-6 {
            lo[i] = -margin;
        }
    }
    (lo, hi)
}

/// Minimise the summed per-station cost over an offset path, subject to R >= `r_min`. DP over
/// states (o[i-1], o[i]). Returns the path as offset indices and its cost.
pub fn dp_optimise(
    offsets: &[f64],
    centres: &[Pt],
    cost: &[Vec<f64>],
    r_min: f64,
    station_step: f64,
    offset_step: f64,
) -> Result<(Vec<usize>, f64), PyError> {
    let ds = station_step;
    let heading_budget = ds / r_min;
    let beta = baseline_turn_angles(centres);
    let n = centres.len();
    let k = offsets.len();
    let step = offset_step;
    let half_pi = core::f64::consts::FRAC_PI_2;

    let d0 = k - 1;
    let phi_lut: Vec<f64> = (0..2 * k - 1)
        .map(|j| libm::atan2((j as f64 - d0 as f64) * step, ds))
        .collect();

    let mut dp: Vec<f64> = vec![f64::INFINITY; k * k];
    for a in 0..k {
        for b in 0..k {
            dp[a * k + b] = cost[0][a] + cost[1][b];
        }
    }
    let mut back: Vec<Vec<i32>> = vec![Vec::new(); n];
    for i in 2..n {
        let mut ndp = vec![f64::INFINITY; k * k];
        let mut nb = vec![0_i32; k * k];
        let bi = beta[i - 1];
        let budget = py_max(heading_budget, bi.abs());
        for jp in 0..k {
            for jc in 0..k {
                let base_val = dp[jp * k + jc];
                if !base_val.is_finite() {
                    continue;
                }
                let phi_prev = phi_lut[d0 + jc - jp];
                let phi_lo = -budget - bi + phi_prev;
                let phi_hi = budget - bi + phi_prev;
                let d_lo = if phi_lo.abs() < half_pi {
                    (libm::tan(phi_lo) * ds / step).ceil() as i64
                } else {
                    -(k as i64 - 1)
                };
                let d_hi = if phi_hi.abs() < half_pi {
                    (libm::tan(phi_hi) * ds / step).floor() as i64
                } else {
                    k as i64 - 1
                };
                let jn_lo = (jc as i64 + d_lo).max(0);
                let jn_hi = (jc as i64 + d_hi).min(k as i64 - 1);
                if jn_lo > jn_hi {
                    continue;
                }
                for jn in jn_lo as usize..=jn_hi as usize {
                    let cand = base_val + cost[i][jn];
                    if cand < ndp[jc * k + jn] {
                        ndp[jc * k + jn] = cand;
                        nb[jc * k + jn] = jp as i32;
                    }
                }
            }
        }
        dp = ndp;
        back[i] = nb;
    }
    let flat = argmin(&dp);
    let (jp_end, jc_end) = (flat / k, flat % k);
    let best_cost = dp[flat];
    if !best_cost.is_finite() {
        return Err(PyError::runtime(
            "DP found no feasible path \u{2014} corridor/curvature too tight",
        ));
    }
    let mut path = vec![0usize; n];
    path[n - 1] = jc_end;
    path[n - 2] = jp_end;
    let (mut jc, mut jp) = (jc_end, jp_end);
    for i in (2..n).rev() {
        let j_prev = back[i][jp * k + jc] as usize;
        path[i - 2] = j_prev;
        jc = jp;
        jp = j_prev;
    }
    Ok((path, best_cost))
}

/// `centres + offset * normals`.
pub fn build_line_from_offsets(centres: &[Pt], normals: &[Pt], offset_vals: &[f64]) -> Vec<Pt> {
    centres
        .iter()
        .zip(normals)
        .zip(offset_vals)
        .map(|((c, n), &o)| [c[0] + o * n[0], c[1] + o * n[1]])
        .collect()
}

fn combined_turn(o: &[f64], beta: &[f64], ds: f64) -> Vec<f64> {
    let n = o.len();
    let phi: Vec<f64> = o.windows(2).map(|w| libm::atan2(w[1] - w[0], ds)).collect();
    let mut turn = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        turn[i] = beta[i] + phi[i] - phi[i - 1];
    }
    turn
}

/// Gentle 3-point smoothing that keeps the DP path's bend-radius feasibility.
pub fn smooth_offsets(offset_vals: &[f64], heading_budget: f64, beta: &[f64], ds: f64) -> Vec<f64> {
    let mut o = offset_vals.to_vec();
    let n = o.len();
    for _ in 0..3 {
        let mut cand = o.clone();
        for i in 1..n.saturating_sub(1) {
            cand[i] = (o[i - 1] + 2.0 * o[i] + o[i + 1]) / 4.0;
        }
        let turn = combined_turn(&cand, beta, ds);
        for i in 0..n {
            let budget = py_max(heading_budget, beta[i].abs());
            if turn[i].abs() > budget + 1e-4 {
                cand[i] = o[i];
            }
        }
        o = cand;
    }
    o
}

fn apply_taper(a: &mut [f64], taper: i64) {
    let n = a.len();
    let count = (taper.max(0) as usize).min(n / 2);
    for k in 0..count {
        let f = k as f64 / taper as f64;
        a[k] *= f;
        a[n - 1 - k] *= f;
    }
}

fn min_radius_of(centres: &[Pt], normals: &[Pt], o: &[f64]) -> f64 {
    np_min(&radii(&build_line_from_offsets(centres, normals, o)))
}

/// Laplacian passes on the built geometry until its measured min radius clears `target_r`.
/// Returns `(offsets, passes, min_r)`; stops after `max_iter` passes even when not cleared.
pub fn enforce_min_radius(
    offset_vals: &[f64],
    centres: &[Pt],
    normals: &[Pt],
    target_r: f64,
    max_iter: i64,
    taper_stations: i64,
) -> (Vec<f64>, i64, f64) {
    let mut o = offset_vals.to_vec();
    let n = o.len();
    let omin = np_min(&o);
    let omax = o.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let corridor = py_max(py_max(omin.abs(), omax.abs()), 1.0);
    apply_taper(&mut o, taper_stations);
    let mut passes = 0;
    let mut rmin = min_radius_of(centres, normals, &o);
    for _ in 0..max_iter {
        if rmin >= target_r - 1e-6 {
            break;
        }
        let mut sm = o.clone();
        for i in 1..n.saturating_sub(1) {
            sm[i] = 0.25 * o[i - 1] + 0.5 * o[i] + 0.25 * o[i + 1];
        }
        for v in &mut sm {
            *v = v.max(-corridor).min(corridor);
        }
        apply_taper(&mut sm, taper_stations);
        o = sm;
        rmin = min_radius_of(centres, normals, &o);
        passes += 1;
    }
    (o, passes, rmin)
}

/// Grid-approx cost of `ovals` vs the baseline (offset 0 everywhere), as a percentage.
pub fn approx_saving_pct(grid: &CostGrid, cost: &[Vec<f64>], ovals: &[f64]) -> f64 {
    let k = grid.offsets.len();
    let idx: Vec<usize> = ovals
        .iter()
        .map(|&o| {
            let r = ((o - grid.offsets[0]) / grid.offset_step).round_ties_even();
            r.max(0.0).min(k as f64 - 1.0) as usize
        })
        .collect();
    let opt_vals: Vec<f64> = idx.iter().enumerate().map(|(i, &j)| cost[i][j]).collect();
    let abs_off: Vec<f64> = grid.offsets.iter().map(|o| o.abs()).collect();
    let zero = argmin(&abs_off);
    let base_vals: Vec<f64> = (0..ovals.len()).map(|i| cost[i][zero]).collect();
    let (opt, base) = (np_sum(&opt_vals), np_sum(&base_vals));
    if base != 0.0 {
        100.0 * (base - opt) / base
    } else {
        0.0
    }
}

/// DP within `+-corridor_half` and the fold band, then smooth to `r_min` (less the fixed reading
/// tolerance, never relaxed to the baseline's own radius). Returns
/// `(offsets, passes, min_radius, saving_pct)`.
pub fn solve_at_corridor(
    grid: &CostGrid,
    cost: &[Vec<f64>],
    corridor_half: f64,
    r_min: f64,
) -> Result<(Vec<f64>, i64, f64, f64), PyError> {
    let offsets = &grid.offsets;
    let (o_lo, o_hi) = offset_bounds(&grid.centres, r_min);
    let mut g: Vec<Vec<f64>> = cost.to_vec();
    for (i, row) in g.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            let o = offsets[k];
            if o.abs() > corridor_half + 1e-6
                || o < o_lo[i] - 1e-6
                || o > o_hi[i] + 1e-6
                || grid.blocked(i, k)
            {
                *v = f64::INFINITY;
            }
        }
    }
    let (path, _) = dp_optimise(
        offsets,
        &grid.centres,
        &g,
        r_min,
        grid.station_step,
        grid.offset_step,
    )
    .map_err(|e| {
        if grid.protected.is_empty() {
            e
        } else {
            PyError::runtime(format!(
                "{}, or a no-cut zone leaves no offset that clears its bank",
                e.message
            ))
        }
    })?;
    let beta = baseline_turn_angles(&grid.centres);
    let chosen: Vec<f64> = path.iter().map(|&j| offsets[j]).collect();
    let sm = smooth_offsets(&chosen, grid.station_step / r_min, &beta, grid.station_step);
    let (sm, passes, rmin) = enforce_min_radius(
        &sm,
        &grid.centres,
        &grid.normals,
        (1.0 - RADIUS_TOL) * r_min,
        600,
        8,
    );
    let sav = approx_saving_pct(grid, cost, &sm);
    Ok((sm, passes, rmin, sav))
}

/// The keyword arguments of `optimise_route` that are not the grid's.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteParams {
    pub r_min: f64,
    pub wet_weight: f64,
    pub dry_weight: f64,
    pub corridor_sweep: Vec<f64>,
    pub check_chord_m: Option<f64>,
}

/// Least-cost lateral re-route over a cost grid (the caller builds it, with `corridor_half` set to
/// the widest sweep corridor, as `optimise_route` does when it is given none).
pub fn optimise_route(grid: &CostGrid, p: &RouteParams) -> Result<RouteResult, PyError> {
    let cost = grid.weighted(p.wet_weight, p.dry_weight);
    let k: usize = match p.check_chord_m {
        None => 1,
        Some(c) if !(c.is_finite() && c > 0.0) => {
            return Err(PyError::value("check_chord_m must be finite and positive."));
        }
        Some(c) => ((c / (2.0 * grid.station_step) + 0.5).floor().max(1.0)) as usize,
    };
    struct Best {
        clean: bool,
        ok: bool,
        ch: f64,
        sav: f64,
        sm: Vec<f64>,
        rmin: f64,
        passes: i64,
        pts: Vec<Pt>,
        measured: f64,
    }
    let mut best: Option<Best> = None;
    let mut last_err: Option<PyError> = None;
    let mut sweep = Vec::new();
    for &ch in &p.corridor_sweep {
        // With no-cut zones a narrow corridor may hold no offset clear of the bank: skip it and
        // let a wider one win. Without zones a failure is an error, as before.
        let (sm, passes, rmin, sav) = match solve_at_corridor(grid, &cost, ch, p.r_min) {
            Ok(v) => v,
            Err(e) if grid.protected.is_empty() => return Err(e),
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        sweep.push((ch, sav, rmin));
        let pts = build_line_from_offsets(&grid.centres, &grid.normals, &sm);
        let measured = min_radius_on_vertices(&pts, k as i64)?;
        let ok = radius_ok(measured, p.r_min);
        // `(clean, ok, sav) > (best_clean, best_ok, best_sav)`; `clean` is always true with no
        // zones, so the ranking is then the Python `(ok, sav) > (best_ok, best_sav)`.
        let clean = grid.protected_cut(&sm) == 0.0;
        let better = best.as_ref().is_none_or(|b| {
            (clean && !b.clean)
                || (clean == b.clean && ((ok && !b.ok) || (ok == b.ok && sav > b.sav)))
        });
        if better {
            best = Some(Best {
                clean,
                ok,
                ch,
                sav,
                sm,
                rmin,
                passes,
                pts,
                measured,
            });
        }
    }
    let b = match (best, last_err) {
        (Some(b), _) => b,
        (None, Some(e)) => return Err(e),
        (None, None) => {
            return Err(PyError::value("cannot unpack non-iterable NoneType object"));
        }
    };
    let protected_cut_m3 = grid.protected_cut(&b.sm);
    Ok(RouteResult {
        line: b.pts,
        offsets: b.sm,
        min_radius_m: b.rmin,
        corridor_half_m: b.ch,
        approx_saving_pct: b.sav,
        smooth_passes: b.passes,
        sweep,
        measured_min_radius_m: b.measured,
        radius_check_chord_m: 2.0 * k as f64 * grid.station_step,
        radius_ok: b.ok,
        protected_cut_m3,
        no_cut_ok: b.clean,
    })
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::geom::Polygon;

    fn straight(n: usize, ds: f64) -> Vec<Pt> {
        (0..n).map(|i| [i as f64 * ds, 0.0]).collect()
    }

    #[test]
    fn points_on_a_circle_read_its_radius_and_a_straight_line_reads_infinity() {
        let r = 300.0;
        let arc: Vec<Pt> = (0..12)
            .map(|i| {
                let t = i as f64 * 0.1;
                [r * libm::sin(t), r * (1.0 - libm::cos(t))]
            })
            .collect();
        let (min, at) = min_radius_of_line(&arc);
        assert!(
            (min - r).abs() < 1e-6 && (1..11).contains(&at),
            "{min} {at}"
        );
        assert!(radii(&straight(6, 100.0)).iter().all(|r| r.is_infinite()));
    }

    #[test]
    fn a_dp_with_no_finite_path_is_a_runtime_error() {
        let cost = vec![vec![f64::INFINITY; 3]; 10];
        let err = dp_optimise(
            &[-10.0, 0.0, 10.0],
            &straight(10, 100.0),
            &cost,
            300.0,
            100.0,
            10.0,
        )
        .unwrap_err();
        assert_eq!(err.kind, "RuntimeError");
        assert!(err.message.starts_with("DP found no feasible path"));
    }

    #[test]
    fn on_flat_cost_the_dp_keeps_the_first_of_equal_paths() {
        let offsets = [-10.0, 0.0, 10.0];
        let cost = vec![vec![1.0; 3]; 6];
        let (path, c) =
            dp_optimise(&offsets, &straight(6, 100.0), &cost, 300.0, 100.0, 10.0).unwrap();
        assert_eq!((path, c), (vec![0; 6], 6.0));
    }

    #[test]
    fn the_sweep_reports_every_corridor_and_a_bad_chord_is_refused() {
        // One-cell-wide grid: nothing to move, both corridors save 0 and the first wins the tie.
        let grid = CostGrid {
            offsets: vec![0.0],
            stations_m: vec![0.0, 100.0, 200.0],
            centres: straight(3, 100.0),
            normals: vec![[0.0, 1.0]; 3],
            grades: vec![0.0; 3],
            vol: vec![vec![5.0]; 3],
            wet: vec![vec![1.0]; 3],
            protected: vec![],
            station_step: 100.0,
            offset_step: 10.0,
        };
        let p = RouteParams {
            r_min: 300.0,
            wet_weight: 1.0,
            dry_weight: 1.0,
            corridor_sweep: vec![40.0, 80.0],
            check_chord_m: None,
        };
        let r = optimise_route(&grid, &p).unwrap();
        assert_eq!(
            (r.corridor_half_m, r.radius_ok, r.radius_check_chord_m),
            (40.0, true, 200.0)
        );
        assert_eq!(r.sweep.len(), 2);
        let bad = RouteParams {
            check_chord_m: Some(f64::NAN),
            ..p
        };
        assert_eq!(
            optimise_route(&grid, &bad).unwrap_err().message,
            "check_chord_m must be finite and positive."
        );
    }

    // A straight baseline along +x (left = +y) in a river 40 m wide (|y| < 20). Dry ground is 1 m
    // high on the left bank and 4 m on the right, so an unconstrained route moves left. Template:
    // bottom 20 m, 1:2 sides, invert -1: it meets 1 m ground 10 + 2 x 2 = 14 m from its centre.
    fn bank_case(no_cut: &[NoCutZone]) -> CostGrid {
        let line = straight(11, 100.0);
        let river = [Polygon {
            exterior: vec![
                [-50.0, -20.0],
                [1050.0, -20.0],
                [1050.0, 20.0],
                [-50.0, 20.0],
                [-50.0, -20.0],
            ],
            holes: vec![],
        }];
        let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
            xs.iter()
                .zip(ys)
                .map(|(_, &y)| if y < 0.0 { 4.0 } else { 1.0 })
                .collect()
        };
        let p = GridParams {
            bottom_width: 20.0,
            depth: 2.0,
            side_slope: 2.0,
            corridor_half: 40.0,
            station_step: 100.0,
            offset_step: 2.0,
            transect_step: 1.0,
            daylight_m: Some(30.0),
        };
        build_cost_grid_with(&line, &terrain, &river, &|_| -1.0, &p, no_cut, None).unwrap()
    }

    fn route_of(grid: &CostGrid) -> Result<RouteResult, PyError> {
        optimise_route(
            grid,
            &RouteParams {
                r_min: 300.0,
                wet_weight: 1.0,
                dry_weight: 1.0,
                corridor_sweep: vec![10.0, 40.0],
                check_chord_m: None,
            },
        )
    }

    #[test]
    fn no_zones_leave_the_grid_and_the_route_as_they_were() {
        let a = bank_case(&[]);
        assert!(a.protected.is_empty());
        let r = route_of(&a).unwrap();
        assert!(r.no_cut_ok && r.protected_cut_m3 == 0.0);
        // Unconstrained, the line takes the low left bank.
        assert!(r.offsets[5] > 6.0, "{:?}", r.offsets);
    }

    #[test]
    fn a_protected_left_bank_holds_the_line_where_its_cut_stays_in_the_water() {
        // Golden: the template's left top of cut is 14 m from its centre and the bank is at +20 m,
        // so the widest offset that digs no left-bank ground is +6 m (worked by hand above).
        let zone = NoCutZone {
            from_m: 0.0,
            to_m: 1000.0,
            bank: Bank::Left,
        };
        let g = bank_case(&[zone]);
        let k6 = g
            .offsets
            .iter()
            .position(|&o| (o - 6.0).abs() < 1e-9)
            .unwrap();
        assert!(!g.blocked(5, k6) && g.blocked(5, k6 + 1));
        assert!(g.protected[5][k6 + 1] > 0.0 && g.protected[5][0] == 0.0);
        let r = route_of(&g).unwrap();
        assert!(r.no_cut_ok && r.protected_cut_m3 == 0.0);
        assert!(
            r.offsets.iter().all(|&o| o <= 6.0 + 1e-6),
            "{:?}",
            r.offsets
        );
        assert!((r.offsets[5] - 6.0).abs() < 1e-6, "{:?}", r.offsets);
    }

    #[test]
    fn a_zone_only_covers_its_chainage_and_names_its_bank() {
        let z = NoCutZone {
            from_m: 300.0,
            to_m: 500.0,
            bank: Bank::Right,
        };
        let g = bank_case(&[z]);
        // Right bank protected at stations 3..=5 only; the left bank is never protected.
        let far_right = 0;
        assert!(g.blocked(3, far_right) && g.blocked(5, far_right) && !g.blocked(6, far_right));
        let far_left = g.offsets.len() - 1;
        assert!((0..g.stations_m.len()).all(|i| !g.blocked(i, far_left)));
    }

    #[test]
    fn a_zone_that_leaves_no_clear_offset_is_a_loud_failure() {
        // Both banks protected and a river 10 m wide: the template is 28 m across at the ground,
        // so every offset digs one bank or the other.
        let both = [
            NoCutZone {
                from_m: 0.0,
                to_m: 1000.0,
                bank: Bank::Left,
            },
            NoCutZone {
                from_m: 0.0,
                to_m: 1000.0,
                bank: Bank::Right,
            },
        ];
        let line = straight(11, 100.0);
        let river = [Polygon {
            exterior: vec![
                [-50.0, -5.0],
                [1050.0, -5.0],
                [1050.0, 5.0],
                [-50.0, 5.0],
                [-50.0, -5.0],
            ],
            holes: vec![],
        }];
        let flat = |xs: &[f64], _ys: &[f64]| -> Vec<f64> { vec![1.0; xs.len()] };
        let p = GridParams {
            bottom_width: 20.0,
            depth: 2.0,
            side_slope: 2.0,
            corridor_half: 40.0,
            station_step: 100.0,
            offset_step: 2.0,
            transect_step: 1.0,
            daylight_m: Some(30.0),
        };
        let g = build_cost_grid_with(&line, &flat, &river, &|_| -1.0, &p, &both, None).unwrap();
        let err = route_of(&g).unwrap_err();
        assert_eq!(err.kind, "RuntimeError");
        assert!(err.message.contains("no-cut zone"), "{}", err.message);
        let bad = NoCutZone {
            from_m: f64::NAN,
            to_m: 1.0,
            bank: Bank::Left,
        };
        assert!(build_cost_grid_with(&line, &flat, &river, &|_| -1.0, &p, &[bad], None).is_err());
    }
}
