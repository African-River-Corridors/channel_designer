//! Move the mask banks onto the water edge seen in Sentinel-2 imagery (spec 0012, ticket T4).
//!
//! The method is ported from earlier bank-fitting work (the idea, not the code or the data); the
//! default limits are the ones the spec names. The mask banks are a seed: they give the search corridor
//! and the colour samples. Per bank:
//!
//! 1. Stations every 5 m along the bank, with a normal that points out of the river.
//! 2. A water score for every pixel: the log-likelihood ratio of a water colour model, fitted to the
//!    pixels well inside the mask polygon, against land colour models (three, split by brightness),
//!    fitted to a ring of pixels 40 to 90 m outside it. The score is clipped to 6 either way.
//! 3. A step-edge matched filter along each normal: the mean score just inside the offset minus the
//!    mean just outside.
//! 4. A globally best path of offsets (Viterbi) with a hard slope limit between stations, then a second
//!    path that is pulled back to the bank's own running-median trend, so a landing, moored boats or a
//!    bar cannot pull the bank in.
//! 5. Two passes: the second re-seeds the colour models from the first pass's banks.
//! 6. Confidence from the contrast of the edge, its lead over the best competing edge, and the share
//!    of valid pixels. A stretch of low confidence keeps the mask bank and is flagged `no-edge` (cloud,
//!    shadow, turbid water with no clear edge); a short gap takes the smoothed fit.
//! 7. A binomial smoother on the offsets and a slope-limited envelope through the kept stations.
//!
//! Provisional defaults until calibrated against a surveyed reach: +-40 m search, at most 30 m shift,
//! at most 1.5 m change per 5 m, under 3 pixels (30 m) wide after the fit is low-confidence.

use crate::banks::{MaskBanks, Stretch, StretchKind};
use crate::error::ReachError;
use crate::sentinel2::Imagery;
use owd_format::design::crs::Utm;

/// The settings of the edge fit. Every value here is a provisional default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitParams {
    pub station_step_m: f64,
    pub tangent_half_m: f64,
    /// The farthest a bank may move from the mask bank.
    pub max_shift_m: f64,
    /// A reach longer than this is fitted in parts this long, each on the scene that covers it.
    pub part_m: f64,
    /// The overlap of two parts, where their shifts are blended.
    pub overlap_m: f64,
    /// The most the offset may change from one station to the next one `station_step_m` away.
    pub max_step_m: f64,
    pub sample_m: f64,
    pub filter_half_m: f64,
    pub prior_per_m: f64,
    pub continuity_window_m: f64,
    pub continuity_deadband_m: f64,
    pub continuity_per_m: f64,
    /// The closest a bank may come to the centreline, in total width.
    pub min_width_m: f64,
    pub min_confidence: f64,
    pub keep_run_min_m: f64,
    pub conf_window_stations: usize,
    pub contrast_ref: f64,
    pub margin_ref: f64,
    pub competitor_gap_m: f64,
    /// The mean water score (log-likelihood ratio) that shows water, or with its sign turned land, all
    /// along the last stretch before the shift limit.
    pub beyond_score: f64,
    pub smooth_passes: usize,
    pub passes: usize,
    /// A width under this many metres after the fit is low-confidence (3 pixels of 10 m).
    pub min_trusted_width_m: f64,
}

impl Default for FitParams {
    fn default() -> Self {
        FitParams {
            station_step_m: 5.0,
            tangent_half_m: 20.0,
            max_shift_m: 30.0,
            part_m: 15_000.0,
            overlap_m: 2_000.0,
            max_step_m: 1.5,
            sample_m: 1.0,
            filter_half_m: 10.0,
            prior_per_m: 0.02,
            continuity_window_m: 105.0,
            continuity_deadband_m: 2.0,
            continuity_per_m: 0.6,
            min_width_m: 20.0,
            min_confidence: 0.5,
            keep_run_min_m: 60.0,
            conf_window_stations: 5,
            contrast_ref: 2.0,
            margin_ref: 1.0,
            competitor_gap_m: 20.0,
            beyond_score: 0.5,
            smooth_passes: 3,
            passes: 2,
            min_trusted_width_m: 30.0,
        }
    }
}

/// What the fit gives.
#[derive(Debug, Clone)]
pub struct EdgeFit {
    pub utm: Utm,
    pub params: FitParams,
    pub left: Vec<[f64; 2]>,
    pub right: Vec<[f64; 2]>,
    pub centreline: Vec<[f64; 2]>,
    /// Closed ring: left bank, then right bank back.
    pub polygon: Vec<[f64; 2]>,
    /// Per station of the left then the right bank: shift in metres from the mask bank (positive is
    /// outward), confidence 0 to 1, and whether the mask bank was kept.
    pub left_shift: Vec<f64>,
    pub right_shift: Vec<f64>,
    pub left_kept: Vec<bool>,
    pub right_kept: Vec<bool>,
    /// Moved by the full shift limit, the water edge lying beyond it.
    pub left_beyond: Vec<bool>,
    pub right_beyond: Vec<bool>,
    /// Width at each left station, in metres.
    pub widths: Vec<f64>,
    pub stretches: Vec<Stretch>,
}

type P2 = [f64; 2];

fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Arc length at each vertex, and the total.
fn arc(line: &[P2]) -> Vec<f64> {
    let mut s = vec![0.0];
    for w in line.windows(2) {
        s.push(s.last().unwrap() + dist(w[0], w[1]));
    }
    s
}

/// The point at arc length `at` (clamped).
fn point_at(line: &[P2], s: &[f64], at: f64) -> P2 {
    let at = at.clamp(0.0, *s.last().unwrap());
    let i = s
        .partition_point(|v| *v <= at)
        .saturating_sub(1)
        .min(line.len() - 2);
    let seg = s[i + 1] - s[i];
    let f = if seg > 0.0 { (at - s[i]) / seg } else { 0.0 };
    [
        line[i][0] + f * (line[i + 1][0] - line[i][0]),
        line[i][1] + f * (line[i + 1][1] - line[i][1]),
    ]
}

/// Uniform stations, spacing at least `step`: arc lengths and points.
fn stations(line: &[P2], step: f64) -> (Vec<f64>, Vec<P2>) {
    let a = arc(line);
    let len = *a.last().unwrap();
    let n = ((len / step).floor() as usize).max(1);
    let s: Vec<f64> = (0..=n).map(|i| len * i as f64 / n as f64).collect();
    let pts = s.iter().map(|v| point_at(line, &a, *v)).collect();
    (s, pts)
}

/// The nearest point of a polyline to `p`.
fn nearest_on(line: &[P2], p: P2) -> P2 {
    let mut best = (f64::MAX, line[0]);
    for w in line.windows(2) {
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 == 0.0 {
            0.0
        } else {
            (((p[0] - w[0][0]) * d[0] + (p[1] - w[0][1]) * d[1]) / l2).clamp(0.0, 1.0)
        };
        let q = [w[0][0] + t * d[0], w[0][1] + t * d[1]];
        let dd = dist(p, q);
        if dd < best.0 {
            best = (dd, q);
        }
    }
    best.1
}

/// Unit normals that point away from the centreline, and the distance to it.
fn normals(line: &[P2], s: &[f64], half: f64, centre: &[P2], pts: &[P2]) -> (Vec<P2>, Vec<f64>) {
    let a = arc(line);
    let mut ns = Vec::new();
    let mut dcs = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        let (b0, b1) = (
            point_at(line, &a, s[i] - half),
            point_at(line, &a, s[i] + half),
        );
        let (dx, dy) = (b1[0] - b0[0], b1[1] - b0[1]);
        let l = (dx * dx + dy * dy).sqrt().max(1e-12);
        let mut n = [-dy / l, dx / l];
        let q = nearest_on(centre, *p);
        if n[0] * (q[0] - p[0]) + n[1] * (q[1] - p[1]) > 0.0 {
            n = [-n[0], -n[1]];
        }
        ns.push(n);
        dcs.push(dist(*p, q));
    }
    (ns, dcs)
}

// ---- the colour model ---------------------------------------------------------------------

const NF: usize = 5;

fn features(im: &Imagery, i: usize) -> [f64; NF] {
    let (g, r, n, s) = (
        f64::from(im.bands[0][i]) * 100.0,
        f64::from(im.bands[1][i]) * 100.0,
        f64::from(im.bands[2][i]) * 100.0,
        f64::from(im.bands[3][i]) * 100.0,
    );
    let mndwi = if g + s > 1e-6 {
        10.0 * (g - s) / (g + s)
    } else {
        0.0
    };
    [g, r, n, s, mndwi]
}

struct Gauss {
    mean: [f64; NF],
    cov: [[f64; NF]; NF],
}

/// Added to each variance of the shared covariance, in (percent reflectance) squared: sensor noise, so
/// a band that barely varies in the samples cannot dominate the discriminant.
const VARIANCE_FLOOR: f64 = 0.25;

fn invert(mut a: [[f64; NF]; NF]) -> [[f64; NF]; NF] {
    let mut inv = [[0.0; NF]; NF];
    for (i, row) in inv.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for c in 0..NF {
        let p = (c..NF)
            .max_by(|x, y| a[*x][c].abs().total_cmp(&a[*y][c].abs()))
            .unwrap();
        a.swap(c, p);
        inv.swap(c, p);
        let d = a[c][c];
        for k in 0..NF {
            a[c][k] /= d;
            inv[c][k] /= d;
        }
        for r in 0..NF {
            if r != c {
                let f = a[r][c];
                for k in 0..NF {
                    a[r][k] -= f * a[c][k];
                    inv[r][k] -= f * inv[c][k];
                }
            }
        }
    }
    inv
}

fn gauss(samples: &[[f64; NF]]) -> Gauss {
    let n = samples.len() as f64;
    let mut mean = [0.0; NF];
    for s in samples {
        for k in 0..NF {
            mean[k] += s[k] / n;
        }
    }
    let mut cov = [[0.0; NF]; NF];
    for s in samples {
        for i in 0..NF {
            for j in 0..NF {
                cov[i][j] += (s[i] - mean[i]) * (s[j] - mean[j]) / (n - 1.0).max(1.0);
            }
        }
    }
    Gauss { mean, cov }
}

/// Chamfer 3-4 distance, in pixels, from each pixel to the nearest pixel where `target` is true.
fn distance_to(target: &[bool], cols: usize, rows: usize) -> Vec<f32> {
    let inf = f32::MAX / 4.0;
    let mut d: Vec<f32> = target.iter().map(|t| if *t { 0.0 } else { inf }).collect();
    let (a, b) = (1.0f32, std::f32::consts::SQRT_2);
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            let mut v = d[i];
            if c > 0 {
                v = v.min(d[i - 1] + a);
            }
            if r > 0 {
                v = v.min(d[i - cols] + a);
                if c > 0 {
                    v = v.min(d[i - cols - 1] + b);
                }
                if c + 1 < cols {
                    v = v.min(d[i - cols + 1] + b);
                }
            }
            d[i] = v;
        }
    }
    for r in (0..rows).rev() {
        for c in (0..cols).rev() {
            let i = r * cols + c;
            let mut v = d[i];
            if c + 1 < cols {
                v = v.min(d[i + 1] + a);
            }
            if r + 1 < rows {
                v = v.min(d[i + cols] + a);
                if c + 1 < cols {
                    v = v.min(d[i + cols + 1] + b);
                }
                if c > 0 {
                    v = v.min(d[i + cols - 1] + b);
                }
            }
            d[i] = v;
        }
    }
    d
}

/// Which pixels (by centre) lie inside a closed ring in imagery coordinates (even-odd rule).
fn rasterize(ring: &[P2], im: &Imagery) -> Vec<bool> {
    let mut out = vec![false; im.cols * im.rows];
    let px = |p: P2| [(p[0] - im.x0) / im.px, (im.y1 - p[1]) / im.px];
    let pr: Vec<P2> = ring.iter().map(|p| px(*p)).collect();
    for r in 0..im.rows {
        let y = r as f64 + 0.5;
        let mut xs: Vec<f64> = Vec::new();
        for w in pr
            .windows(2)
            .chain(std::iter::once(&[pr[pr.len() - 1], pr[0]][..]))
        {
            let (a, b) = (w[0], w[1]);
            if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
                xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        xs.sort_by(f64::total_cmp);
        for [a, b] in xs.as_chunks::<2>().0 {
            let c0 = (a - 0.5).ceil().max(0.0) as usize;
            let c1 = ((b - 0.5).floor() + 1.0).min(im.cols as f64).max(0.0) as usize;
            for c in c0..c1 {
                out[r * im.cols + c] = true;
            }
        }
    }
    out
}

/// The water score of every pixel, seeded by a river polygon in imagery coordinates; `None` when the
/// seeds are too few.
fn water_score(im: &Imagery, polygon: &[P2]) -> Option<Vec<f32>> {
    let inside = rasterize(polygon, im);
    if !inside.iter().any(|v| *v) || inside.iter().all(|v| *v) {
        return None;
    }
    let outside: Vec<bool> = inside.iter().map(|v| !v).collect();
    let din: Vec<f32> = distance_to(&outside, im.cols, im.rows);
    let dout: Vec<f32> = distance_to(&inside, im.cols, im.rows);
    let dmax = din.iter().cloned().fold(0.0f32, f32::max) as f64 * im.px;
    let water_min = (0.3 * dmax).max(2.0 * im.px);
    let mut water = Vec::new();
    let mut land = Vec::new();
    for i in 0..inside.len() {
        if !im.valid[i] {
            continue;
        }
        if inside[i] && f64::from(din[i]) * im.px >= water_min {
            water.push(features(im, i));
        } else if !inside[i] && (4.0 * im.px..=9.0 * im.px).contains(&(f64::from(dout[i]) * im.px))
        {
            land.push((
                features(im, i),
                (im.bands[0][i] + im.bands[1][i] + im.bands[2][i]) / 3.0,
            ));
        }
    }
    if water.len() < 150 || land.len() < 150 {
        return None;
    }
    // One covariance shared by both classes (a linear discriminant): the log-likelihood ratio of two
    // Gaussians with a common covariance is linear in the colour. A covariance of its own for the water,
    // learnt from calm water mid-channel, is so tight that the shallower, muddier water near a bank
    // scores as land.
    let land: Vec<[f64; NF]> = land.into_iter().map(|l| l.0).collect();
    let (gw, gl) = (gauss(&water), gauss(&land));
    let mut pooled = [[0.0; NF]; NF];
    for (i, row) in pooled.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (gw.cov[i][j] + gl.cov[i][j]) / 2.0;
        }
        row[i] += VARIANCE_FLOOR;
    }
    let inv = invert(pooled);
    let mut w = [0.0; NF];
    for (wi, row) in w.iter_mut().zip(&inv) {
        *wi = (0..NF).map(|j| row[j] * (gw.mean[j] - gl.mean[j])).sum();
    }
    let mid: Vec<f64> = (0..NF).map(|k| (gw.mean[k] + gl.mean[k]) / 2.0).collect();
    Some(
        (0..im.cols * im.rows)
            .map(|i| {
                let f = features(im, i);
                let llr: f64 = (0..NF).map(|k| w[k] * (f[k] - mid[k])).sum();
                llr.clamp(-6.0, 6.0) as f32
            })
            .collect(),
    )
}

/// Bilinear score at a point, and whether the nearest pixel is valid.
fn sample(im: &Imagery, score: &[f32], x: f64, y: f64) -> (f64, bool) {
    let (fc, fr) = ((x - im.x0) / im.px - 0.5, (im.y1 - y) / im.px - 0.5);
    if fc < 0.0 || fr < 0.0 || fc > (im.cols - 1) as f64 || fr > (im.rows - 1) as f64 {
        return (0.0, false);
    }
    let (c0, r0) = (fc.floor() as usize, fr.floor() as usize);
    let (c1, r1) = ((c0 + 1).min(im.cols - 1), (r0 + 1).min(im.rows - 1));
    let (tx, ty) = (fc - c0 as f64, fr - r0 as f64);
    let s = |r: usize, c: usize| f64::from(score[r * im.cols + c]);
    let v = (s(r0, c0) * (1.0 - tx) + s(r0, c1) * tx) * (1.0 - ty)
        + (s(r1, c0) * (1.0 - tx) + s(r1, c1) * tx) * ty;
    let near = im.valid
        [(fr.round() as usize).min(im.rows - 1) * im.cols + (fc.round() as usize).min(im.cols - 1)];
    (v, near)
}

// ---- the dynamic programme and its helpers ------------------------------------------------

/// Min-sum DP over columns; the column may change by at most `j` per row. Ties go to the smaller change
/// then the lower column.
fn viterbi(cost: &[Vec<i64>], j: usize) -> Vec<usize> {
    let (n, m) = (cost.len(), cost[0].len());
    let big: i64 = 1 << 50;
    let mut acc = cost[0].clone();
    let mut back = vec![vec![0usize; m]; n];
    let mut deltas: Vec<i64> = (-(j as i64)..=j as i64).collect();
    deltas.sort_by_key(|d| (d.abs(), *d));
    for i in 1..n {
        let mut best = vec![big; m];
        for c in 0..m {
            for &dj in &deltas {
                let from = c as i64 - dj;
                if from < 0 || from >= m as i64 {
                    continue;
                }
                let cand = acc[from as usize] + dj.abs();
                if cand < best[c] {
                    best[c] = cand;
                    back[i][c] = from as usize;
                }
            }
        }
        acc = (0..m).map(|c| (best[c] + cost[i][c]).min(big)).collect();
    }
    let mut path = vec![0usize; n];
    path[n - 1] = acc
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| **v)
        .map(|(c, _)| c)
        .unwrap();
    for i in (1..n).rev() {
        path[i - 1] = back[i][path[i]];
    }
    path
}

fn running_median(v: &[f64], k: usize) -> Vec<f64> {
    let k = (k.max(1)) | 1;
    let h = k / 2;
    (0..v.len())
        .map(|i| {
            let mut w: Vec<f64> = (0..k)
                .map(|o| v[(i as i64 + o as i64 - h as i64).clamp(0, v.len() as i64 - 1) as usize])
                .collect();
            w.sort_by(f64::total_cmp);
            w[h]
        })
        .collect()
}

fn binomial(v: &[f64], passes: usize) -> Vec<f64> {
    let k = [1.0, 4.0, 6.0, 4.0, 1.0];
    let mut cur = v.to_vec();
    for _ in 0..passes {
        cur = (0..cur.len())
            .map(|i| {
                (0..5)
                    .map(|o| {
                        k[o] * cur
                            [(i as i64 + o as i64 - 2).clamp(0, cur.len() as i64 - 1) as usize]
                    })
                    .sum::<f64>()
                    / 16.0
            })
            .collect();
    }
    cur
}

/// Clip `d` (integer millimetres) into the slope-limited envelope through every fixed station at 0.
fn pinned_envelope(d: &[i64], fixed: &[bool], cap: i64) -> Vec<i64> {
    let n = d.len();
    let big = 1i64 << 40;
    let (mut hi, mut lo) = (vec![big; n], vec![-big; n]);
    for order in [(0..n).collect::<Vec<_>>(), (0..n).rev().collect::<Vec<_>>()] {
        let (mut ph, mut pl) = (big, -big);
        for i in order {
            let (mut h, mut l) = ((ph + cap).min(big), (pl - cap).max(-big));
            if fixed[i] {
                h = 0;
                l = 0;
            }
            hi[i] = hi[i].min(h);
            lo[i] = lo[i].max(l);
            ph = hi[i];
            pl = lo[i];
        }
    }
    (0..n).map(|i| d[i].clamp(lo[i], hi[i])).collect()
}

/// `d` (integer millimetres, kept stations at 0) with no change over `cap` from one station to the
/// next: each part's fit keeps the limit by its own path, but blending two parts across an overlap
/// can step past it. Passes forward and back, kept stations held at 0, until nothing changes.
fn slope_limited(mut d: Vec<i64>, fixed: &[bool], cap: i64) -> Vec<i64> {
    let n = d.len();
    for _ in 0..n.max(1) {
        let before = d.clone();
        for i in 1..n {
            if !fixed[i] {
                d[i] = d[i].clamp(d[i - 1] - cap, d[i - 1] + cap);
            }
        }
        for i in (0..n.saturating_sub(1)).rev() {
            if !fixed[i] {
                d[i] = d[i].clamp(d[i + 1] - cap, d[i + 1] + cap);
            }
        }
        if d == before {
            break;
        }
    }
    d
}

struct OneBank {
    d: Vec<f64>,
    fixed: Vec<bool>,
    /// Moved by the full shift limit: the water edge lies beyond it.
    beyond: Vec<bool>,
}

#[allow(clippy::too_many_arguments)]
fn fit_one(
    im: &Imagery,
    score: &[f32],
    cfg: &FitParams,
    s: &[f64],
    pts: &[P2],
    nrm: &[P2],
    dc: &[f64],
) -> OneBank {
    let steps = (((cfg.max_shift_m + cfg.filter_half_m) * 2.0) / cfg.sample_m).round() as usize + 1;
    let offs: Vec<f64> = (0..steps)
        .map(|i| -(cfg.max_shift_m + cfg.filter_half_m) + i as f64 * cfg.sample_m)
        .collect();
    let nf = (cfg.filter_half_m / cfg.sample_m).round() as usize;
    let n = s.len();
    let mut matched = vec![vec![-12.0f64; steps]; n];
    let mut valid_share = vec![0.0f64; n];
    let mut profiles = vec![vec![0.0f64; steps]; n];
    for i in 0..n {
        let mut p = vec![0.0f64; steps];
        let mut ok = 0;
        for (j, o) in offs.iter().enumerate() {
            let (v, good) = sample(
                im,
                score,
                pts[i][0] + nrm[i][0] * o,
                pts[i][1] + nrm[i][1] * o,
            );
            if good {
                p[j] = v;
                ok += 1;
            }
        }
        valid_share[i] = f64::from(ok) / steps as f64;
        profiles[i].clone_from(&p);
        let mut cs = vec![0.0; steps + 1];
        for j in 0..steps {
            cs[j + 1] = cs[j] + p[j];
        }
        for j in nf..steps.saturating_sub(nf) {
            matched[i][j] = (cs[j] - cs[j - nf]) / nf as f64 - (cs[j + nf] - cs[j]) / nf as f64;
        }
    }
    let big: i64 = 1_000_000_000;
    let inward: Vec<f64> = dc
        .iter()
        .map(|d| (d - cfg.min_width_m / 2.0).max(0.0))
        .collect();
    let allowed = |i: usize, j: usize| {
        offs[j].abs() <= cfg.max_shift_m + 1e-9 && offs[j] >= -inward[i] - 1e-9
    };
    let unary: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            (0..steps)
                .map(|j| {
                    if allowed(i, j) {
                        (100.0 * (12.0 - matched[i][j]) + 100.0 * cfg.prior_per_m * offs[j].abs())
                            .round() as i64
                    } else {
                        big
                    }
                })
                .collect()
        })
        .collect();
    let ds = if n > 1 {
        s[1] - s[0]
    } else {
        cfg.station_step_m
    };
    let jmax = (cfg.max_step_m * ds / cfg.station_step_m / cfg.sample_m + 1e-9).floor() as usize;
    let j0 = viterbi(&unary, jmax);
    let d0: Vec<f64> = j0.iter().map(|j| offs[*j]).collect();
    let k = (cfg.continuity_window_m / ds).round() as usize;
    let trend = running_median(&d0, k);
    let cost2: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            (0..steps)
                .map(|j| {
                    if allowed(i, j) {
                        let excess =
                            ((offs[j] - trend[i]).abs() - cfg.continuity_deadband_m).max(0.0);
                        unary[i][j] + (100.0 * cfg.continuity_per_m * excess).round() as i64
                    } else {
                        big
                    }
                })
                .collect()
        })
        .collect();
    let j1 = viterbi(&cost2, jmax);
    let d1: Vec<f64> = j1.iter().map(|j| offs[*j]).collect();

    // Confidence: the weakest of contrast, lead over the best edge well apart from it, and valid
    // pixels. A path at the shift limit, or with no edge at all, is no edge, unless the image shows
    // the river running on past the limit: water all along the outward side, or land all along the
    // inward side. Then the edge lies beyond the limit, and the bank moves by the full limit (a path
    // through the same slope limit) and is marked `beyond`.
    let lim = cfg.max_shift_m;
    let mean_over = |i: usize, from: f64, to: f64| {
        let v: Vec<f64> = (0..steps)
            .filter(|j| offs[*j] >= from - 1e-9 && offs[*j] <= to + 1e-9)
            .map(|j| profiles[i][j])
            .collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    let confidence = |i: usize, d: f64, j: usize| {
        let peak = matched[i][j];
        let comp = (0..steps)
            .filter(|k| (offs[*k] - d).abs() > cfg.competitor_gap_m && allowed(i, *k))
            .map(|k| matched[i][k])
            .fold(-12.0, f64::max);
        let c_contrast = (peak / cfg.contrast_ref).clamp(0.0, 1.0);
        let c_margin = ((peak - comp) / cfg.margin_ref).clamp(0.0, 1.0);
        let c_valid = ((valid_share[i] - 0.5) / 0.5).clamp(0.0, 1.0);
        let c = (c_contrast.min(c_margin).min(c_valid) * 100.0).round() / 100.0;
        let at_limit = d.abs() >= lim - 1e-9 || (d <= -inward[i] + 1e-9 && inward[i] < lim);
        (if at_limit { 0.0 } else { c }, c_valid)
    };
    // Where the water (or land) runs on past the limit: +1 outward, -1 inward, 0 neither.
    let target: Vec<i8> = (0..n)
        .map(|i| {
            let (c, _) = confidence(i, d1[i], j1[i]);
            if c >= cfg.min_confidence {
                0
            } else if mean_over(i, 0.0, lim) >= cfg.beyond_score
                && mean_over(i, lim - cfg.filter_half_m, lim) >= cfg.beyond_score
            {
                1
            } else if inward[i] >= lim
                && mean_over(i, -lim, 0.0) <= -cfg.beyond_score
                && mean_over(i, -lim, -lim + cfg.filter_half_m) <= -cfg.beyond_score
            {
                -1
            } else {
                0
            }
        })
        .collect();
    let (j1, d1) = if target.iter().any(|t| *t != 0) {
        let cost3: Vec<Vec<i64>> = (0..n)
            .map(|i| {
                if target[i] == 0 {
                    return cost2[i].clone();
                }
                let want = lim * f64::from(target[i]);
                (0..steps)
                    .map(|j| {
                        if allowed(i, j) {
                            (100.0 * 12.0 * (offs[j] - want).abs()).round() as i64
                        } else {
                            big
                        }
                    })
                    .collect()
            })
            .collect();
        let j = viterbi(&cost3, jmax);
        let d: Vec<f64> = j.iter().map(|j| offs[*j]).collect();
        (j, d)
    } else {
        (j1, d1)
    };
    let mut conf = vec![0.0f64; n];
    let mut beyond = vec![false; n];
    for i in 0..n {
        let (c, c_valid) = confidence(i, d1[i], j1[i]);
        if target[i] != 0 {
            beyond[i] = true;
            conf[i] = c_valid;
        } else {
            conf[i] = c;
        }
    }
    let conf_s = running_median(&conf, cfg.conf_window_stations);
    let low: Vec<bool> = conf_s.iter().map(|c| *c < cfg.min_confidence).collect();
    let min_run = ((cfg.keep_run_min_m / ds).ceil() as usize).max(1);
    let mut fixed = vec![false; n];
    let mut i = 0;
    while i < n {
        if low[i] {
            let mut j = i;
            while j < n && low[j] {
                j += 1;
            }
            if j - i >= min_run || i == 0 || j == n {
                fixed[i..j].iter_mut().for_each(|f| *f = true);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let sm = binomial(&d1, cfg.smooth_passes);
    let dsm: Vec<f64> = (0..n)
        .map(|i| sm[i].clamp(-inward[i], cfg.max_shift_m))
        .collect();
    let cap = (cfg.max_step_m * ds / cfg.station_step_m * 1000.0 + 1e-6).floor() as i64;
    let mm: Vec<i64> = dsm.iter().map(|v| (v * 1000.0).round() as i64).collect();
    let d = pinned_envelope(&mm, &fixed, cap)
        .iter()
        .map(|v| *v as f64 / 1000.0)
        .collect();
    OneBank { d, fixed, beyond }
}

/// Fit the mask banks to the water edge in `imagery`.
pub fn fit_banks(
    banks: &MaskBanks,
    imagery: &Imagery,
    params: &FitParams,
) -> Result<EdgeFit, ReachError> {
    if banks.utm != imagery.utm {
        return Err(ReachError::Input(format!(
            "the banks are in UTM zone {} and the scene in zone {}: the edge fit needs one zone",
            banks.utm.zone, imagery.utm.zone
        )));
    }
    if banks.left.len() < 3 || banks.right.len() < 3 {
        return Err(ReachError::Input(
            "the mask banks are too short to fit".into(),
        ));
    }
    let geo = bank_geo(banks, params);
    let seed = ring(&banks.left, &banks.right);
    let all = [0..geo[0].0.len(), 0..geo[1].0.len()];
    let res = fit_range(imagery, params, &geo, &all, seed)?;
    Ok(finish(banks, params, &geo, res))
}

/// Per bank: the stations' arc lengths, points, outward normals and distances to the centreline.
type Geo = (Vec<f64>, Vec<P2>, Vec<P2>, Vec<f64>);

fn bank_geo(banks: &MaskBanks, cfg: &FitParams) -> [Geo; 2] {
    [&banks.left, &banks.right].map(|b| {
        let (s, pts) = stations(b, cfg.station_step_m);
        let (nrm, dc) = normals(b, &s, cfg.tangent_half_m, &banks.centreline, &pts);
        (s, pts, nrm, dc)
    })
}

/// A closed ring: one line, then the other back.
fn ring(l: &[P2], r: &[P2]) -> Vec<P2> {
    let mut v = l.to_vec();
    v.extend(r.iter().rev());
    v.push(l[0]);
    v
}

/// Fit the stations `ranges` of each bank to `imagery`, the colours learnt inside `seed` (a ring of
/// the river): the passes of the fit, each re-seeding from the last.
fn fit_range(
    imagery: &Imagery,
    cfg: &FitParams,
    geo: &[Geo; 2],
    ranges: &[std::ops::Range<usize>; 2],
    mut seed: Vec<P2>,
) -> Result<[OneBank; 2], ReachError> {
    let slice = |k: usize| {
        let (s, pts, nrm, dc) = &geo[k];
        let r = ranges[k].clone();
        (&s[r.clone()], &pts[r.clone()], &nrm[r.clone()], &dc[r])
    };
    let mut res: Option<[OneBank; 2]> = None;
    for pass in 0..cfg.passes {
        let score = water_score(imagery, &seed).ok_or_else(|| {
            ReachError::NoWater(
                "the scene has too few clear water or land pixels around the mask banks to learn their colours (cloud over the river?)"
                    .into(),
            )
        })?;
        let fitted = [0, 1].map(|k| {
            let (s, pts, nrm, dc) = slice(k);
            fit_one(imagery, &score, cfg, s, pts, nrm, dc)
        });
        if pass + 1 < cfg.passes {
            let lines: Vec<Vec<P2>> = (0..2)
                .map(|k| {
                    let (_, pts, nrm, _) = slice(k);
                    moved(pts, nrm, &fitted[k])
                })
                .collect();
            seed = ring(&lines[0], &lines[1]);
        }
        res = Some(fitted);
    }
    res.ok_or_else(|| ReachError::Input("the fit needs at least one pass".into()))
}

/// The fitted banks, centreline, widths and flagged stretches, from the shifts of every station.
fn finish(banks: &MaskBanks, cfg: &FitParams, geo: &[Geo; 2], res: [OneBank; 2]) -> EdgeFit {
    let left = moved(&geo[0].1, &geo[0].2, &res[0]);
    let right = moved(&geo[1].1, &geo[1].2, &res[1]);

    // The centreline: the middle between the banks, smoothed. The width at each left station: its
    // distance to the right bank.
    let mut centre: Vec<P2> = left
        .iter()
        .map(|p| {
            let q = nearest_on(&right, *p);
            [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]
        })
        .collect();
    centre = (0..centre.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(2), (i + 2).min(centre.len() - 1));
            let n = (b - a + 1) as f64;
            let sum = centre[a..=b]
                .iter()
                .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
            [sum[0] / n, sum[1] / n]
        })
        .collect();
    let widths: Vec<f64> = left
        .iter()
        .map(|p| dist(*p, nearest_on(&right, *p)))
        .collect();
    let chain = arc(&centre);

    // Flags: a kept station (no clear edge), a station moved by the full limit (the edge lies beyond
    // it), or a width under the minimum after the fit. A stretch takes the most serious kind in it.
    let rank = |k: StretchKind| match k {
        StretchKind::NoEdge => 3,
        StretchKind::AtShiftLimit => 2,
        _ => 1,
    };
    let half = cfg.station_step_m / 2.0;
    let mut stretches: Vec<Stretch> = Vec::new();
    // Flagged stations next to each other make one stretch (the chainage of the smoothed centreline
    // can jump where the banks bend apart, so adjacency is by station).
    let mut last_flagged: Option<usize> = None;
    for i in 0..left.len() {
        let (kept, beyond) = (
            res[0].fixed[i] || nearest_flag(&geo[1].1, &res[1].fixed, left[i]),
            res[0].beyond[i] || nearest_flag(&geo[1].1, &res[1].beyond, left[i]),
        );
        let kind = if kept {
            StretchKind::NoEdge
        } else if beyond {
            StretchKind::AtShiftLimit
        } else if widths[i] < cfg.min_trusted_width_m {
            StretchKind::Narrow
        } else {
            continue;
        };
        let w = widths[i];
        let (from, to) = ((chain[i] - half).max(0.0), chain[i] + half);
        let next_to_last = last_flagged == Some(i.wrapping_sub(1));
        last_flagged = Some(i);
        match stretches.last_mut() {
            Some(s) if next_to_last => {
                s.to_m = to;
                s.min_width_m = s.min_width_m.min(w);
                if rank(kind) > rank(s.kind) {
                    s.kind = kind;
                }
            }
            _ => stretches.push(Stretch {
                from_m: from,
                to_m: to,
                kind,
                min_width_m: w,
            }),
        }
    }
    let polygon = ring(&left, &right);
    EdgeFit {
        utm: banks.utm,
        params: *cfg,
        left,
        right,
        centreline: centre,
        polygon,
        left_shift: res[0].d.clone(),
        right_shift: res[1].d.clone(),
        left_kept: res[0].fixed.clone(),
        right_kept: res[1].fixed.clone(),
        left_beyond: res[0].beyond.clone(),
        right_beyond: res[1].beyond.clone(),
        widths,
        stretches,
    }
}

fn moved(pts: &[P2], nrm: &[P2], r: &OneBank) -> Vec<P2> {
    let out: Vec<P2> = (0..pts.len())
        .map(|i| {
            let p = if r.fixed[i] {
                pts[i]
            } else {
                [
                    pts[i][0] + nrm[i][0] * r.d[i],
                    pts[i][1] + nrm[i][1] * r.d[i],
                ]
            };
            [
                (p[0] * 1000.0).round() / 1000.0,
                (p[1] * 1000.0).round() / 1000.0,
            ]
        })
        .collect();
    let mut dedup: Vec<P2> = Vec::with_capacity(out.len());
    for p in out {
        if dedup.last() != Some(&p) {
            dedup.push(p);
        }
    }
    dedup
}

/// The flag of the right-bank station nearest to `p`.
fn nearest_flag(pts: &[P2], flag: &[bool], p: P2) -> bool {
    pts.iter()
        .zip(flag)
        .min_by(|a, b| dist(*a.0, p).total_cmp(&dist(*b.0, p)))
        .is_some_and(|(_, f)| *f)
}

// ---- from a scene to the document ---------------------------------------------------------

/// Which scenes to look at.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneQuery {
    /// The STAC search endpoint.
    pub endpoint: String,
    /// `YYYY-MM-DD`, inclusive.
    pub from: String,
    pub to: String,
    /// Months of the dry season.
    pub months: Vec<u32>,
    pub max_cloud_pct: f64,
}

/// The two years up to `today` (`YYYY-MM-DD`; this crate has no clock): enough dry seasons to find a clear
/// scene.
pub fn default_range(today: &str) -> Result<(String, String), ReachError> {
    let year: i32 = today
        .get(0..4)
        .and_then(|y| y.parse().ok())
        .ok_or_else(|| ReachError::Input(format!("`{today}` is not a date (YYYY-MM-DD)")))?;
    Ok((format!("{}{}", year - 2, &today[4..]), today.to_string()))
}

/// A fit and the scene it used.
#[derive(Debug, Clone)]
pub struct Fitted {
    pub fit: EdgeFit,
    /// The scene of the first part that was fitted.
    pub scene: crate::sentinel2::Scene,
    /// The parts of the reach, along the mask centreline, each with the scene it was fitted on. One
    /// part for a reach no longer than `FitParams::part_m`.
    pub parts: Vec<Part>,
}

/// A part of a long reach, by chainage along the mask centreline, and the scene it was fitted on.
/// A part with no scene (none clear enough covers it, or its colours could not be learnt) keeps the
/// mask banks, flagged `no-edge`, and says why.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub from_m: f64,
    pub to_m: f64,
    pub scene: Option<crate::sentinel2::Scene>,
    pub note: Option<String>,
}

/// The box `[west, south, east, north]` in degrees of `pts` (UTM), with a margin in metres; and the
/// same box in UTM, `(min, max)`.
fn boxes(pts: &[P2], utm: Utm, margin: f64) -> Result<([f64; 4], P2, P2), ReachError> {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let (min, max) = (
        [lo[0] - margin, lo[1] - margin],
        [hi[0] + margin, hi[1] + margin],
    );
    let c = utm
        .inverse_all(&[min, max, [min[0], max[1]], [max[0], min[1]]])
        .map_err(ReachError::Input)?;
    let bbox = [
        c.iter().map(|p| p[0]).fold(f64::MAX, f64::min),
        c.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
        c.iter().map(|p| p[0]).fold(f64::MIN, f64::max),
        c.iter().map(|p| p[1]).fold(f64::MIN, f64::max),
    ];
    Ok((bbox, min, max))
}

/// The chainage of `p` along `line` (with its arc lengths `s`): of the nearest point.
fn chainage_on(line: &[P2], s: &[f64], p: P2) -> f64 {
    let mut best = (f64::MAX, 0.0);
    for (i, w) in line.windows(2).enumerate() {
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 == 0.0 {
            0.0
        } else {
            (((p[0] - w[0][0]) * d[0] + (p[1] - w[0][1]) * d[1]) / l2).clamp(0.0, 1.0)
        };
        let q = [w[0][0] + t * d[0], w[0][1] + t * d[1]];
        let dd = dist(p, q);
        if dd < best.0 {
            best = (dd, s[i] + t * (s[i + 1] - s[i]));
        }
    }
    best.1
}

/// Find a dry-season, low-cloud scene over the banks, read its window and fit the banks to it. A
/// reach longer than `FitParams::part_m` is fitted in overlapping parts, each on the clearest scene
/// that covers it (a Sentinel-2 tile is about 110 km across): the shifts are blended across each
/// overlap, and the slope limit then holds along the whole bank.
pub fn fit_to_scene(
    fetch: &dyn crate::fetch::Fetch,
    banks: &MaskBanks,
    params: &FitParams,
    q: &SceneQuery,
) -> Result<Fitted, ReachError> {
    use crate::sentinel2::{Scene, parse_scenes, pick_scene, read_imagery, search_url};
    let none = |q: &SceneQuery, what: &str| {
        format!(
            "no Sentinel-2 scene between {} and {} over {what} in the dry-season months {:?} with {}% cloud or less",
            q.from, q.to, q.months, q.max_cloud_pct
        )
    };
    let find = |pts: &[P2]| -> Result<(Option<Scene>, P2, P2), ReachError> {
        let (bbox, min, max) = boxes(pts, banks.utm, 250.0)?;
        let body = fetch.get(&search_url(
            &q.endpoint,
            bbox,
            &q.from,
            &q.to,
            q.max_cloud_pct,
            100,
        ))?;
        let scenes = parse_scenes(&body)?;
        Ok((
            pick_scene(&scenes, &q.months, q.max_cloud_pct, Some(bbox)).cloned(),
            min,
            max,
        ))
    };
    let centre = &banks.centreline;
    let cs = arc(centre);
    let length = *cs.last().unwrap_or(&0.0);

    // A reach that fits in one part: one scene over it all, as for a short reach.
    if length <= params.part_m {
        let all: Vec<P2> = banks.left.iter().chain(&banks.right).copied().collect();
        let (scene, min, max) = find(&all)?;
        let scene = scene.ok_or_else(|| {
            ReachError::NoWater(format!(
                "{} (it may cross a scene edge). Raise the cloud limit, widen the dates, or keep the mask banks",
                none(q, "the reach")
            ))
        })?;
        let imagery = read_imagery(fetch, &scene, min, max)?;
        let fit = fit_banks(banks, &imagery, params)?;
        return Ok(Fitted {
            fit,
            scene: scene.clone(),
            parts: vec![Part {
                from_m: 0.0,
                to_m: length,
                scene: Some(scene),
                note: None,
            }],
        });
    }

    let geo = bank_geo(banks, params);
    let chain: Vec<Vec<f64>> = geo
        .iter()
        .map(|g| g.1.iter().map(|p| chainage_on(centre, &cs, *p)).collect())
        .collect();
    let count = (length / params.part_m).ceil() as usize;
    let mut parts: Vec<Part> = Vec::new();
    // Per part: the station ranges of each bank and their fit, where there is one.
    let mut fits: Vec<([std::ops::Range<usize>; 2], [OneBank; 2])> = Vec::new();
    let mut fit_of_part: Vec<Option<usize>> = Vec::new();
    for k in 0..count {
        let from = (k as f64 * params.part_m - params.overlap_m / 2.0).max(0.0);
        let to = ((k + 1) as f64 * params.part_m + params.overlap_m / 2.0).min(length);
        let range = |b: usize| -> std::ops::Range<usize> {
            let inside: Vec<usize> = (0..chain[b].len())
                .filter(|i| chain[b][*i] >= from && chain[b][*i] <= to)
                .collect();
            match (inside.first(), inside.last()) {
                (Some(a), Some(z)) => *a..*z + 1,
                _ => 0..0,
            }
        };
        let ranges = [range(0), range(1)];
        let mut part = Part {
            from_m: from,
            to_m: to,
            scene: None,
            note: None,
        };
        if ranges.iter().any(|r| r.len() < 3) {
            part.note = Some("too few mask bank stations".into());
            parts.push(part);
            fit_of_part.push(None);
            continue;
        }
        let (l, r) = (&geo[0].1[ranges[0].clone()], &geo[1].1[ranges[1].clone()]);
        let pts: Vec<P2> = l.iter().chain(r).copied().collect();
        let (scene, min, max) = find(&pts)?;
        let Some(scene) = scene else {
            part.note = Some(none(q, "this part"));
            parts.push(part);
            fit_of_part.push(None);
            continue;
        };
        let fitted = read_imagery(fetch, &scene, min, max)
            .and_then(|im| fit_range(&im, params, &geo, &ranges, ring(l, r)));
        part.scene = Some(scene);
        match fitted {
            Ok(res) => {
                fit_of_part.push(Some(fits.len()));
                fits.push((ranges, res));
            }
            Err(ReachError::NoWater(m)) => {
                part.note = Some(m);
                fit_of_part.push(None);
            }
            Err(e) => return Err(e),
        }
        parts.push(part);
    }
    let first = parts
        .iter()
        .find_map(|p| p.note.is_none().then(|| p.scene.clone()).flatten());
    let Some(first) = first else {
        return Err(ReachError::NoWater(format!(
            "{}. Raise the cloud limit, widen the dates, or keep the mask banks",
            none(q, "any part of the reach")
        )));
    };

    // Blend the parts' shifts station by station, each weighted by how far inside its part the
    // station is; a station no part fitted keeps its mask bank. Then the slope limit, through every
    // kept station, along the whole bank.
    let res = [0, 1].map(|b| {
        let n = geo[b].0.len();
        let (mut d, mut fixed, mut beyond) = (vec![0.0; n], vec![true; n], vec![false; n]);
        for i in 0..n {
            let c = chain[b][i];
            let (mut sum, mut wsum, mut best) = (0.0, 0.0, (0.0, false));
            for (k, part) in parts.iter().enumerate() {
                let Some(f) = fit_of_part[k] else { continue };
                let (ranges, res) = &fits[f];
                if !ranges[b].contains(&i) {
                    continue;
                }
                let j = i - ranges[b].start;
                if res[b].fixed[j] {
                    continue;
                }
                let w = (c - part.from_m).min(part.to_m - c).max(0.0) + 1e-3;
                sum += w * res[b].d[j];
                wsum += w;
                if w > best.0 {
                    best = (w, res[b].beyond[j]);
                }
            }
            if wsum > 0.0 {
                d[i] = sum / wsum;
                fixed[i] = false;
                beyond[i] = best.1;
            }
        }
        let ds = if n > 1 {
            geo[b].0[1] - geo[b].0[0]
        } else {
            params.station_step_m
        };
        let cap = (params.max_step_m * ds / params.station_step_m * 1000.0 + 1e-6).floor() as i64;
        let mm: Vec<i64> = d.iter().map(|v| (v * 1000.0).round() as i64).collect();
        let d = slope_limited(pinned_envelope(&mm, &fixed, cap), &fixed, cap)
            .iter()
            .map(|v| *v as f64 / 1000.0)
            .collect();
        OneBank { d, fixed, beyond }
    });
    Ok(Fitted {
        fit: finish(banks, params, &geo, res),
        scene: first,
        parts,
    })
}

impl EdgeFit {
    /// The `low_confidence_width` value of the document.
    pub fn low_confidence_value(&self) -> serde_json::Value {
        use serde_json::json;
        json!({
            "min_width_m": self.params.min_trusted_width_m,
            "provisional": true,
            "source": "sentinel2-l2a",
            "stretches": self.stretches.iter().map(|s| json!({
                "from_m": s.from_m, "to_m": s.to_m,
                "kind": match s.kind {
                    StretchKind::Narrow => "narrow",
                    StretchKind::NoWater => "no-water",
                    StretchKind::NoEdge => "no-edge",
                    StretchKind::AtShiftLimit => "at-shift-limit",
                },
                "min_width_m": s.min_width_m,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Record the fitted banks in a document, replacing the mask banks: the four inputs, each marked
/// `default` with the scene id and date, the Sentinel notice and the JRC notice of the seed, and the
/// limits that are provisional; and the stretches to check.
pub fn record_edge_fit(
    session: &mut owd_format::design::Session,
    f: &Fitted,
) -> Result<(), ReachError> {
    use owd_format::design::Role;
    use owd_format::design::provenance::{Mark, record, with_settings};
    use serde_json::json;
    let u = f.fit.utm;
    let geo = |kind: &str, coords: Vec<[f64; 2]>| {
        let c: Vec<serde_json::Value> = coords.iter().map(|p| json!([p[0], p[1]])).collect();
        let mut g = if kind == "Polygon" {
            json!({"type": "Polygon", "coordinates": [c]})
        } else {
            json!({"type": kind, "coordinates": c})
        };
        g["crs"] = json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", u.epsg())}});
        g.to_string()
    };
    let (l, r, c, p) = (
        geo("LineString", f.fit.left.clone()),
        geo("LineString", f.fit.right.clone()),
        geo("LineString", f.fit.centreline.clone()),
        geo("Polygon", f.fit.polygon.clone()),
    );
    session
        .import(&[
            (Role::LeftBank, l.as_str()),
            (Role::RightBank, r.as_str()),
            (Role::Centreline, c.as_str()),
            (Role::RiverPolygon, p.as_str()),
        ])
        .map_err(|e| ReachError::Input(e.to_string()))?;
    let s2 = crate::sources::source("sentinel2-l2a").expect("sentinel2-l2a is in the registry");
    let jrc = crate::sources::source("jrc-gsw").expect("jrc-gsw is in the registry");
    // Every scene used: the notice once per year, and the dates as a range when there are several.
    let used: Vec<&crate::sentinel2::Scene> = f
        .parts
        .iter()
        .filter(|p| p.note.is_none())
        .filter_map(|p| p.scene.as_ref())
        .collect();
    let mut years: Vec<String> = used
        .iter()
        .map(|sc| sc.date.chars().take(4).collect())
        .collect();
    years.sort();
    years.dedup();
    let mut notices: Vec<String> = years.iter().map(|y| s2.notice_for(y)).collect();
    notices.push(jrc.notice.to_string());
    let notice = notices.join("; ");
    let mut dates: Vec<&str> = used.iter().map(|sc| sc.date.as_str()).collect();
    dates.sort();
    let date = match (dates.first(), dates.last()) {
        (Some(a), Some(b)) if a != b => format!("{a}/{b}"),
        _ => f.scene.date.clone(),
    };
    let rec = record("sentinel2-l2a", &date, &notice, Mark::Default, true);
    let q = &f.fit.params;
    let settings = json!({
        "scene_id": f.scene.id,
        "scene_cloud_pct": f.scene.cloud_pct,
        "parts": f.parts.iter().map(|p| json!({
            "from_m": p.from_m,
            "to_m": p.to_m,
            "scene_id": p.scene.as_ref().map(|sc| sc.id.clone()),
            "scene_date": p.scene.as_ref().map(|sc| sc.date.clone()),
            "note": p.note,
        })).collect::<Vec<_>>(),
        "seed": "jrc-gsw",
        "search_m": q.max_shift_m + q.filter_half_m,
        "max_shift_m": q.max_shift_m,
        "max_step_m_per_station": q.max_step_m,
        "station_step_m": q.station_step_m,
        "min_trusted_width_m": q.min_trusted_width_m,
        "provisional": ["max_shift_m", "max_step_m_per_station", "search_m", "min_trusted_width_m"],
    });
    for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
        session
            .set_provenance(input, with_settings(rec.clone(), settings.clone()))
            .map_err(|e| ReachError::Input(e.to_string()))?;
    }
    session
        .set_low_confidence_width(f.fit.low_confidence_value())
        .map_err(|e| ReachError::Input(e.to_string()))
}

#[cfg(test)]
mod join_tests {
    //! Label: property
    use super::*;

    #[test]
    fn the_slope_limit_holds_after_a_join_and_kept_stations_stay_at_zero() {
        let mut x: u64 = 7;
        let mut next = || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) as i64
        };
        for _ in 0..300 {
            let n = 5 + (next() % 200) as usize;
            let cap = 1 + next() % 2000;
            let fixed: Vec<bool> = (0..n).map(|_| next() % 7 == 0).collect();
            let d: Vec<i64> = (0..n)
                .map(|i| if fixed[i] { 0 } else { next() % 60_001 - 30_000 })
                .collect();
            let out = slope_limited(pinned_envelope(&d, &fixed, cap), &fixed, cap);
            for i in 0..n {
                if fixed[i] {
                    assert_eq!(out[i], 0);
                }
                assert!(out[i].abs() <= 30_000);
            }
            for w in out.windows(2) {
                assert!((w[1] - w[0]).abs() <= cap, "{w:?} over {cap}");
            }
        }
    }
}
