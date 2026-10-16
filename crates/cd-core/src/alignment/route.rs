//! Least-cost channel route: re-route a centreline laterally to reduce cut, holding R_min
//! (`alignment/route.py`).
//!
//! A cost grid (stations x lateral offsets, one dense terrain transect per station), a dynamic
//! programme over offsets with a heading budget, smoothing, Laplacian radius enforcement, and a
//! corridor sweep. The target radius is never relaxed to the baseline's own radius (issue #9);
//! the winner reports `radius_ok`, the measured radius and the chord it was read on (issue #10).
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
    pub station_step: f64,
    pub offset_step: f64,
}

impl CostGrid {
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
    dp_optimise_fair(
        offsets,
        centres,
        cost,
        r_min,
        station_step,
        offset_step,
        0.0,
    )
}

/// [`dp_optimise`] with a fairness weight (spec 0005 Q9): each step also costs
/// `fairness_weight` × the squared change of heading (radians) at the station, so that where cut
/// is equal (open water) the smoother path wins. At 0 the term is never added, so the result is
/// exactly `dp_optimise`'s.
pub fn dp_optimise_fair(
    offsets: &[f64],
    centres: &[Pt],
    cost: &[Vec<f64>],
    r_min: f64,
    station_step: f64,
    offset_step: f64,
    fairness_weight: f64,
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
                    let mut cand = base_val + cost[i][jn];
                    if fairness_weight != 0.0 {
                        let turn = phi_lut[d0 + jn - jc] - phi_prev + bi;
                        cand += fairness_weight * turn * turn;
                    }
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
    solve_at_corridor_fair(grid, cost, corridor_half, r_min, 0.0)
}

/// [`solve_at_corridor`] with the DP's fairness weight ([`dp_optimise_fair`]).
pub fn solve_at_corridor_fair(
    grid: &CostGrid,
    cost: &[Vec<f64>],
    corridor_half: f64,
    r_min: f64,
    fairness_weight: f64,
) -> Result<(Vec<f64>, i64, f64, f64), PyError> {
    let offsets = &grid.offsets;
    let (o_lo, o_hi) = offset_bounds(&grid.centres, r_min);
    let mut g: Vec<Vec<f64>> = cost.to_vec();
    for (i, row) in g.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            let o = offsets[k];
            if o.abs() > corridor_half + 1e-6 || o < o_lo[i] - 1e-6 || o > o_hi[i] + 1e-6 {
                *v = f64::INFINITY;
            }
        }
    }
    let (path, _) = dp_optimise_fair(
        offsets,
        &grid.centres,
        &g,
        r_min,
        grid.station_step,
        grid.offset_step,
        fairness_weight,
    )?;
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
    /// Weight on the squared change of heading between stations in the DP (spec 0005 Q9).
    /// Default 0: today's results.
    pub fairness_weight: f64,
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
    let mut sweep = Vec::new();
    for &ch in &p.corridor_sweep {
        let (sm, passes, rmin, sav) =
            solve_at_corridor_fair(grid, &cost, ch, p.r_min, p.fairness_weight)?;
        sweep.push((ch, sav, rmin));
        let pts = build_line_from_offsets(&grid.centres, &grid.normals, &sm);
        let measured = min_radius_on_vertices(&pts, k as i64)?;
        let ok = radius_ok(measured, p.r_min);
        // Python `(ok, sav) > (best_ok, best_sav)`
        let better = best
            .as_ref()
            .is_none_or(|b| (ok && !b.ok) || (ok == b.ok && sav > b.sav));
        if better {
            best = Some(Best {
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
    let b = best.ok_or_else(|| PyError::value("cannot unpack non-iterable NoneType object"))?;
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
    })
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

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
            station_step: 100.0,
            offset_step: 10.0,
        };
        let p = RouteParams {
            r_min: 300.0,
            wet_weight: 1.0,
            dry_weight: 1.0,
            corridor_sweep: vec![40.0, 80.0],
            check_chord_m: None,
            fairness_weight: 0.0,
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

    /// Bending energy of a polyline: Σ (turn angle)² / (mean arm length), a discrete ∫κ² ds.
    fn bending_energy(pts: &[Pt]) -> f64 {
        pts.windows(3)
            .map(|w| {
                let (a, b) = (
                    [w[1][0] - w[0][0], w[1][1] - w[0][1]],
                    [w[2][0] - w[1][0], w[2][1] - w[1][1]],
                );
                let t = libm::atan2(a[0] * b[1] - a[1] * b[0], a[0] * b[0] + a[1] * b[1]);
                let l = (libm::hypot(a[0], a[1]) + libm::hypot(b[0], b[1])) / 2.0;
                t * t / l
            })
            .sum()
    }

    #[test]
    fn in_the_open_basin_a_fairness_weight_gives_a_smoother_line_at_the_same_cut() {
        // Label: property. Open water everywhere (bed 10 m below grade): every path cuts nothing,
        // so at weight 0 the DP keeps the first of equal paths, here the seed's own digitising
        // jitter (±15 m every 100 m along both legs); with a weight it takes a smoother one.
        let f = crate::synthetic::open_basin_90(1500.0, 600.0);
        let mut rng = crate::synthetic::fixtures::SplitMix::new(5);
        let mut line: Vec<Pt> = (0..15)
            .map(|i| [15.0 * rng.signed_unit(), -1500.0 + 100.0 * i as f64])
            .collect();
        line[0][0] = 0.0;
        line.push([0.0, 0.0]);
        line.extend((1..=15).map(|i| [100.0 * i as f64, 15.0 * rng.signed_unit()]));
        line.last_mut().unwrap()[1] = 0.0;
        let bed = |xs: &[f64], _: &[f64]| vec![-10.0; xs.len()];
        let gp = GridParams {
            bottom_width: 30.4,
            depth: 3.0,
            side_slope: 3.0,
            corridor_half: 160.0,
            station_step: 50.0,
            offset_step: 10.0,
            transect_step: 2.0,
            daylight_m: None,
        };
        let grid =
            build_cost_grid(&line, &bed, core::slice::from_ref(&f.water), &|_| 0.0, &gp).unwrap();
        assert!(grid.vol.iter().flatten().all(|v| *v == 0.0));
        let run = |w: f64| {
            optimise_route(
                &grid,
                &RouteParams {
                    r_min: 255.0,
                    wet_weight: 1.0,
                    dry_weight: 1.0,
                    corridor_sweep: vec![160.0],
                    check_chord_m: None,
                    fairness_weight: w,
                },
            )
            .unwrap()
        };
        let (plain, fair) = (run(0.0), run(1.0));
        let path_cut = |r: &RouteResult| -> f64 {
            r.offsets
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let k = grid.offsets.iter().position(|x| (x - o).abs() < 1e-6);
                    k.map_or(0.0, |k| grid.vol[i][k])
                })
                .sum()
        };
        assert_eq!(path_cut(&plain), 0.0);
        assert_eq!(path_cut(&fair), 0.0);
        let (e0, e1) = (bending_energy(&plain.line), bending_energy(&fair.line));
        assert!(e1 < e0, "bending energy {e1} not below {e0}");
        // Weight 0 is today's DP exactly.
        let today = dp_optimise(
            &grid.offsets,
            &grid.centres,
            &grid.weighted(1.0, 1.0),
            255.0,
            50.0,
            10.0,
        );
        let zero = dp_optimise_fair(
            &grid.offsets,
            &grid.centres,
            &grid.weighted(1.0, 1.0),
            255.0,
            50.0,
            10.0,
            0.0,
        );
        assert_eq!(today.unwrap(), zero.unwrap());
    }
}
