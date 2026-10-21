//! Pool levels: the longitudinal bed profile and the weir cascade that sets each pool's water
//! level. Mirrors `volumes/pools.py` (flat-pool assumption, lift = H - d).
//!
//! The numpy steps (`nanmean`, `percentile`, `maximum.accumulate`, `interp`) are written out so
//! that they round as numpy's do.

use crate::geom::{Pt, linear};
use crate::numpy::{interp, np_sum, searchsorted_left};

/// `np.nanmean`: the sum with NaNs as zero, over the count of the others (NaN if none).
fn nanmean(z: &[f64]) -> f64 {
    let n = z.iter().filter(|x| !x.is_nan()).count();
    if n == 0 {
        return f64::NAN;
    }
    let zeroed: Vec<f64> = z
        .iter()
        .map(|&x| if x.is_nan() { 0.0 } else { x })
        .collect();
    np_sum(&zeroed) / n as f64
}

/// `np.percentile(seg, q)` (method "linear") of finite values.
fn percentile(seg: &[f64], q: f64) -> f64 {
    let mut a = seg.to_vec();
    a.sort_by(f64::total_cmp);
    let n = a.len();
    let vi = (n as f64 - 1.0) * (q / 100.0);
    let (prev, next) = if vi >= n as f64 - 1.0 {
        (-1isize, -1isize)
    } else if vi < 0.0 {
        (0, 0)
    } else {
        let p = vi.floor() as isize;
        (p, p + 1)
    };
    let at = |i: isize| if i < 0 { a[n - 1] } else { a[i as usize] };
    let (lo, hi) = (at(prev), at(next));
    let t = vi - prev as f64;
    // `_lerp`: from the nearer end.
    let diff = hi - lo;
    if t >= 0.5 {
        hi - diff * (1.0 - t)
    } else {
        lo + diff * t
    }
}

/// `_fill_nan`: interior NaNs interpolated linearly, the ends filled from the nearest value.
fn fill_nan(z: &[f64]) -> Vec<f64> {
    let good: Vec<usize> = (0..z.len()).filter(|&i| z[i].is_finite()).collect();
    if good.is_empty() {
        return vec![0.0; z.len()];
    }
    let xp: Vec<f64> = good.iter().map(|&i| i as f64).collect();
    let fp: Vec<f64> = good.iter().map(|&i| z[i]).collect();
    z.iter()
        .enumerate()
        .map(|(i, &v)| {
            if v.is_finite() {
                v
            } else {
                interp(i as f64, &xp, &fp)
            }
        })
        .collect()
}

fn window(n: usize, i: usize, half: usize) -> core::ops::Range<usize> {
    i.saturating_sub(half)..(i + half + 1).min(n)
}

/// `_rolling_percentile`: the centred rolling percentile of the finite values.
fn rolling_percentile(z: &[f64], win: usize, q: f64) -> Vec<f64> {
    let half = win / 2;
    (0..z.len())
        .map(|i| {
            let seg: Vec<f64> = z[window(z.len(), i, half)]
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .collect();
            if seg.is_empty() {
                f64::NAN
            } else {
                percentile(&seg, q)
            }
        })
        .collect()
}

/// `_rolling_mean`: the centred rolling `nanmean`.
fn rolling_mean(z: &[f64], win: usize) -> Vec<f64> {
    let half = win / 2;
    (0..z.len())
        .map(|i| nanmean(&z[window(z.len(), i, half)]))
        .collect()
}

/// Python `int(round(x))` (half to even) floored at 1, as the window sizes are.
fn window_samples(window_m: f64, spacing_m: f64) -> usize {
    ((window_m / spacing_m).round_ties_even() as i64).max(1) as usize
}

/// `overall_bottom_slope`: low rolling percentile, wide rolling mean, then the running maximum.
pub fn overall_bottom_slope(
    z_raw: &[f64],
    spacing_m: f64,
    percentile_q: f64,
    perc_window_m: f64,
    smooth_window_m: f64,
) -> Vec<f64> {
    let z = fill_nan(z_raw);
    let base = fill_nan(&rolling_percentile(
        &z,
        window_samples(perc_window_m, spacing_m),
        percentile_q,
    ));
    let smooth = rolling_mean(&base, window_samples(smooth_window_m, spacing_m));
    // `np.maximum.accumulate`: NaN propagates.
    let mut out = Vec::with_capacity(smooth.len());
    for (i, &v) in smooth.iter().enumerate() {
        if i == 0 {
            out.push(v);
        } else {
            let m: f64 = out[i - 1];
            out.push(if m.is_nan() || v.is_nan() {
                f64::NAN
            } else if v > m {
                v
            } else {
                m
            });
        }
    }
    out
}

/// The parameters of `sample_bed_profile` beyond the line, with the Python defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileParams {
    pub spacing_m: f64,
    pub slope_percentile: f64,
    pub slope_perc_window_m: f64,
    pub slope_smooth_window_m: f64,
}

impl Default for ProfileParams {
    fn default() -> Self {
        ProfileParams {
            spacing_m: 25.0,
            slope_percentile: 20.0,
            slope_perc_window_m: 2500.0,
            slope_smooth_window_m: 5000.0,
        }
    }
}

/// A terrain sampler: heights for the points `(xs[i], ys[i])`, NaN where unknown.
pub type Terrain<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// The bed sampled along a centreline (`BedProfile`), chainage from the downstream end.
#[derive(Debug, Clone, PartialEq)]
pub struct BedProfile {
    pub s: Vec<f64>,
    pub z_raw: Vec<f64>,
    pub z_overall: Vec<f64>,
    pub points: Vec<Pt>,
    pub line: Vec<Pt>,
}

/// `sample_bed_profile`: bed heights from `terrain` every `spacing_m`, the line turned so that
/// chainage 0 is the lower-bed end, and the overall bottom slope.
pub fn sample_bed_profile(
    centreline: &[Pt],
    terrain: Terrain<'_>,
    p: &ProfileParams,
) -> BedProfile {
    let length = linear::length(centreline);
    // `np.arange(0.0, length + spacing, spacing)`
    let n = ((length + p.spacing_m) / p.spacing_m).ceil().max(0.0) as usize;
    let mut s: Vec<f64> = (0..n).map(|i| i as f64 * p.spacing_m).collect();
    if let Some(last) = s.last_mut() {
        *last = crate::py_min(*last, length);
    }
    let mut pts: Vec<Pt> = s
        .iter()
        .map(|&d| linear::interpolate(centreline, d).unwrap_or([f64::NAN; 2]))
        .collect();
    let xs: Vec<f64> = pts.iter().map(|q| q[0]).collect();
    let ys: Vec<f64> = pts.iter().map(|q| q[1]).collect();
    let mut z: Vec<f64> = terrain(&xs, &ys)
        .into_iter()
        .map(|v| if v.is_finite() { v } else { f64::NAN })
        .collect();
    let k = 3.max(z.len() / 10);
    let head = nanmean(&z[..k.min(z.len())]);
    let tail = nanmean(&z[z.len().saturating_sub(k)..]);
    let mut line = centreline.to_vec();
    if head.is_finite() && tail.is_finite() && head > tail {
        line.reverse();
        z.reverse();
        pts.reverse();
    }
    let z_overall = overall_bottom_slope(
        &z,
        p.spacing_m,
        p.slope_percentile,
        p.slope_perc_window_m,
        p.slope_smooth_window_m,
    );
    BedProfile {
        s,
        z_raw: z,
        z_overall,
        points: pts,
        line,
    }
}

/// One weir and the pool upstream of it (`Weir`).
#[derive(Debug, Clone, PartialEq)]
pub struct Weir {
    pub index: usize,
    pub chainage_m: f64,
    pub point: Pt,
    pub bed_m: f64,
    pub foundation_m: f64,
    pub crest_m: f64,
    pub pool_water_level_m: f64,
    pub pool_start_m: f64,
    pub pool_end_m: f64,
}

impl Weir {
    pub fn pool_length_m(&self) -> f64 {
        self.pool_end_m - self.pool_start_m
    }
}

/// `_first_crossing`: the first chainage upstream of `i0` where the bed reaches `level`.
fn first_crossing(s: &[f64], z: &[f64], i0: usize, level: f64) -> Option<(f64, usize)> {
    let j = (i0 + 1..z.len()).find(|&j| z[j] >= level)?;
    let (z0, z1) = (z[j - 1], z[j]);
    if z1 == z0 {
        return Some((s[j], j));
    }
    let frac = (level - z0) / (z1 - z0);
    Some((s[j - 1] + frac * (s[j] - s[j - 1]), j))
}

fn stitch(weirs: &mut [Weir]) {
    for i in 1..weirs.len() {
        weirs[i - 1].pool_end_m = weirs[i].pool_start_m;
    }
}

/// `place_weirs`: the cascade by the water-level recursion WL_n = WL_{n-1} + (H - d). `Err`
/// carries the lift when it is not positive (the caller words the Python message).
pub fn place_weirs(
    profile: &BedProfile,
    weir_height_m: f64,
    implantation_depth_m: f64,
    start_chainage_m: Option<f64>,
    start_elevation_m: f64,
    max_weir_elevation_m: Option<f64>,
) -> Result<Vec<Weir>, f64> {
    let (s, z) = (&profile.s, &profile.z_overall);
    let (h, d) = (weir_height_m, implantation_depth_m);
    let lift = h - d;
    if lift <= 0.0 {
        return Err(lift);
    }
    let i0 = match start_chainage_m {
        None => z.iter().position(|&v| v >= start_elevation_m).unwrap_or(0),
        Some(c) => searchsorted_left(s, c).min(s.len() - 1),
    };
    let bed = z[i0];
    let end = s[s.len() - 1];
    let mut weirs = vec![Weir {
        index: 1,
        chainage_m: s[i0],
        point: profile.points[i0],
        bed_m: bed,
        foundation_m: bed - d,
        crest_m: bed + lift,
        pool_water_level_m: bed + lift,
        pool_start_m: s[i0],
        pool_end_m: end,
    }];
    let mut target = bed + lift;
    let mut cur = i0;
    while let Some((chainage, j)) = first_crossing(s, z, cur, target) {
        if max_weir_elevation_m.is_some_and(|m| target >= m) {
            break;
        }
        let crest = target + lift;
        weirs.push(Weir {
            index: weirs.len() + 1,
            chainage_m: chainage,
            point: linear::interpolate(&profile.line, chainage).unwrap_or([f64::NAN; 2]),
            bed_m: target,
            foundation_m: target - d,
            crest_m: crest,
            pool_water_level_m: crest,
            pool_start_m: chainage,
            pool_end_m: end,
        });
        target = crest;
        cur = j;
    }
    stitch(&mut weirs);
    Ok(weirs)
}

/// One weir of an explicit cascade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExplicitWeir {
    pub chainage_m: f64,
    pub pool_mwl_m: f64,
    /// `None` (or zero) is no structure height.
    pub structure_height_m: Option<f64>,
}

/// `place_weirs_explicit`: the cascade from explicit weirs, sorted by chainage.
pub fn place_weirs_explicit(profile: &BedProfile, cascade: &[ExplicitWeir]) -> Vec<Weir> {
    let (s, z) = (&profile.s, &profile.z_overall);
    let mut items = cascade.to_vec();
    items.sort_by(|a, b| {
        a.chainage_m
            .partial_cmp(&b.chainage_m)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    let end = s[s.len() - 1];
    let mut weirs: Vec<Weir> = items
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let h = w.structure_height_m.filter(|&h| h != 0.0).unwrap_or(0.0);
            let j = searchsorted_left(s, w.chainage_m).min(s.len() - 1);
            Weir {
                index: i + 1,
                chainage_m: w.chainage_m,
                point: linear::interpolate(&profile.line, w.chainage_m).unwrap_or([f64::NAN; 2]),
                bed_m: z[j],
                foundation_m: w.pool_mwl_m - h,
                crest_m: w.pool_mwl_m,
                pool_water_level_m: w.pool_mwl_m,
                pool_start_m: w.chainage_m,
                pool_end_m: end,
            }
        })
        .collect();
    stitch(&mut weirs);
    weirs
}

/// `water_level_at`: the pool level of the last weir at or below `s`; below weir 1, the
/// downstream level.
pub fn water_level_at(weirs: &[Weir], s: f64, downstream_water_level_m: f64) -> f64 {
    let mut sorted: Vec<&Weir> = weirs.iter().collect();
    sorted.sort_by(|a, b| {
        a.pool_start_m
            .partial_cmp(&b.pool_start_m)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    let mut wl = downstream_water_level_m;
    for w in sorted {
        if s >= w.pool_start_m {
            wl = w.pool_water_level_m;
        } else {
            break;
        }
    }
    wl
}

/// `grade_at`: the design invert, pool level less depth and overdepth.
pub fn grade_at(
    weirs: &[Weir],
    s: f64,
    depth_m: f64,
    downstream_water_level_m: f64,
    overdepth_m: f64,
) -> f64 {
    water_level_at(weirs, s, downstream_water_level_m) - depth_m - overdepth_m
}

#[cfg(test)]
mod tests {
    //! Label: sourced - numpy 2.x `np.percentile` and `np.nanmean` for the same inputs
    use super::*;

    #[test]
    fn percentile_and_nanmean_follow_numpy() {
        // np.percentile([4, 1, 3, 2], 20) == 1.6; np.percentile([1, 2], 75) == 1.75
        assert!((percentile(&[4.0, 1.0, 3.0, 2.0], 20.0) - 1.6).abs() < 1e-15);
        assert_eq!(percentile(&[1.0, 2.0], 75.0), 1.75);
        assert_eq!(percentile(&[5.0], 20.0), 5.0);
        assert_eq!(nanmean(&[1.0, f64::NAN, 3.0]), 2.0);
        assert!(nanmean(&[f64::NAN]).is_nan());
    }
}
