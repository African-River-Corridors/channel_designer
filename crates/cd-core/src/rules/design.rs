//! Channel design: centreline geometry + barge size -> navigable channel section. Mirrors
//! `rules/design.py`.
//!
//! One-lane inland-waterway channel sizing along a surveyed centreline. Adds curvature-driven
//! bend widening to the straight and single-bend rules in [`super::simple`].
//!
//! Two-stage precompute: LOA and beam touch *different* parts of the geometry, so they are swept
//! separately to avoid recomputing the expensive curvature profile:
//!
//! ```text
//! Stage 0  compute_curvature(centreline)   resample -> circumcircle radius -> rolling-average
//!                                          smoothing. ONCE.
//! Stage 1  precompute_loa(curv, total_loa) R_min = radius_multiple x total_loa, tight-bend
//!                                          detection, banded extra width.
//! Stage 2  apply_beam(curv, loa, beam)     W_base = width_multiple x beam, bottom/top widths,
//!                                          half-offsets.
//! ```
//!
//! Combined sweep API: [`compute_scenario_grid`]. Single scenario: [`compute`].
//!
//! Design basis:
//! - Straight bottom width = `width_multiple x beam`.
//! - Design bend radius R2 = `radius_multiple x total_loa` (the target the centreline is
//!   straightened to).
//! - Bend widening = the banded `extra_width_coefficient x loa^2 / R` allowance on the smoothed
//!   radius, zero at and above `max_extra_width_radius`.
//! - Depth = draft + under-keel clearance, UKC = max(ukc_min, ukc_fraction x draft).
//! - Side slope: top width = bottom + 2 x side_slope x design_depth.
//!
//! A registered width method ([`super::methods`]) can replace the first and third items: set
//! `ChannelDesignParams::width_method`. The straight width then comes from the method, and each
//! discrete bend (a run of one turn direction) whose radius R1 is below R2 is widened to
//! `method.bend_width(loa, beam, options, R1, R2, deflection)`. Where a bend width exceeds the
//! LOA, a cut-through is indicated.
//!
//! The numpy of the Python (`linspace`, `interp`, `unwrap`, cumulative sums) is written out with
//! the same arithmetic, in the same order.

use super::methods::{MethodRegistry, Options};
use super::{RuleError, value_err};
use crate::numpy::{interp, linspace, np_mod};
use crate::py_max;
use ovcd_core::checks::Check;
use ovcd_core::pyfmt::{fixed, py_str, round_to};
use std::collections::HashMap;

// =============================================================================
// Under-keel clearance / depth <-> draft (carried over verbatim from pianc.py)
// =============================================================================

pub fn under_keel_clearance(draft_m: f64, ukc_fraction: f64, ukc_min_m: f64) -> f64 {
    py_max(ukc_min_m, ukc_fraction * draft_m)
}

/// Channel depth (below min water level) needed to float a given laden draft.
pub fn required_depth(draft_m: f64, ukc_fraction: f64, ukc_min_m: f64) -> f64 {
    draft_m + under_keel_clearance(draft_m, ukc_fraction, ukc_min_m)
}

/// Inverse: the deepest laden draft a given channel depth allows.
pub fn max_draft_for_depth(depth_m: f64, ukc_fraction: f64, ukc_min_m: f64) -> f64 {
    if depth_m <= 0.0 {
        return 0.0;
    }
    // fractional branch: depth = draft x (1 + ukc_fraction)
    let d_frac = depth_m / (1.0 + ukc_fraction);
    if ukc_fraction * d_frac >= ukc_min_m {
        return d_frac;
    }
    // absolute-UKC branch: depth = draft + ukc_min
    py_max(0.0, depth_m - ukc_min_m)
}

// =============================================================================
// Parameters (shared design assumptions, swept across the grid)
// =============================================================================

/// Design assumptions shared across a LOA x beam sweep.
///
/// Defaults are examples, not recommendations: set your own for your waterway. Note the default
/// `width_multiple` 2.8 is below WG 141's 3.0 x beam single-lane minimum;
/// `rulesets::channel_shape` with a named ruleset is the standard-backed path.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDesignParams {
    /// One-lane straight bottom width = k_w x beam.
    pub width_multiple: f64,
    /// Design R_min = k_r x total_loa (the single radius knob; set 4.0 / 5.0 for the future
    /// comparison scenarios).
    pub radius_multiple: f64,
    /// Bank slope (horizontal per vertical).
    pub side_slope: f64,
    /// `None`: straight = width_multiple x beam, banded extra width below. A name: the method
    /// sizes the straight width and each discrete bend; R2 stays radius_multiple x LOA.
    pub width_method: Option<String>,
    /// Passed to the method untouched.
    pub width_options: Options,
    /// Laden draft -> design depth (if not overridden).
    pub draft_m: f64,
    /// Explicit channel depth; else from draft + UKC.
    pub design_depth_m: Option<f64>,
    pub ukc_fraction: f64,
    pub ukc_min_m: f64,
    /// Bend-widening coefficient (x loa^2 / R).
    pub extra_width_coefficient: f64,
    /// No widening for radii >= this.
    pub max_extra_width_radius: f64,
    /// Banded extra-width resolution.
    pub n_bands: i64,
    /// Centreline resample spacing (m).
    pub radius_sample_length: f64,
    /// Half-window for radius smoothing (m).
    pub rolling_window_m: f64,
    /// Ignore widening/tight bends within this of the ends.
    pub edge_margin_m: f64,
    /// Water-surface datum for bed_level reporting.
    pub reference_level_m: f64,
    /// One passing place every ~2 km.
    pub passing_place_spacing_m: f64,
    /// Passing-place length = k x total_loa.
    pub passing_place_length_multiple: f64,
    /// Two-lane width = k x base width.
    pub passing_place_width_multiple: f64,
    /// Warn if max extra > ratio x base width.
    pub extra_width_reasonable_ratio: f64,
}

impl Default for ChannelDesignParams {
    fn default() -> Self {
        Self {
            width_multiple: 2.8,
            radius_multiple: 3.0,
            side_slope: 4.0,
            width_method: None,
            width_options: Options::new(),
            draft_m: 1.75,
            design_depth_m: None,
            ukc_fraction: 0.15,
            ukc_min_m: 0.5,
            extra_width_coefficient: 0.6,
            max_extra_width_radius: 2000.0,
            n_bands: 4,
            radius_sample_length: 10.0,
            rolling_window_m: 50.0,
            edge_margin_m: 100.0,
            reference_level_m: 0.0,
            passing_place_spacing_m: 2000.0,
            passing_place_length_multiple: 5.0,
            passing_place_width_multiple: 2.0,
            extra_width_reasonable_ratio: 1.0,
        }
    }
}

impl ChannelDesignParams {
    /// The Python `__post_init__` checks, in its order. An unknown width method is refused now.
    pub fn validate(&self, methods: &MethodRegistry) -> Result<(), RuleError> {
        if self.width_multiple <= 0.0 {
            return value_err("width_multiple must be > 0".into());
        }
        if self.radius_multiple <= 0.0 {
            return value_err("radius_multiple must be > 0".into());
        }
        if self.side_slope < 0.0 {
            return value_err("side_slope must be >= 0".into());
        }
        if self.radius_sample_length <= 0.0 {
            return value_err("radius_sample_length must be > 0".into());
        }
        if self.max_extra_width_radius <= 0.0 {
            return value_err("max_extra_width_radius must be > 0".into());
        }
        if self.n_bands < 1 {
            return value_err("n_bands must be >= 1".into());
        }
        if let Some(d) = self.design_depth_m
            && d <= 0.0
        {
            return value_err("design_depth_m must be > 0 when given".into());
        }
        if let Some(name) = &self.width_method {
            methods.get(name)?;
        }
        Ok(())
    }
}

/// Design depth = explicit override, else draft + under-keel clearance.
pub fn resolve_design_depth(params: &ChannelDesignParams) -> f64 {
    match params.design_depth_m {
        Some(d) => d,
        None => required_depth(params.draft_m, params.ukc_fraction, params.ukc_min_m),
    }
}

// =============================================================================
// Inputs / Result (the standard module contract)
// =============================================================================

/// Plan coordinates of a centreline in a projected CRS (metres), with an optional third column:
/// the water-surface / terrain level used for `bed_level` reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct Centreline {
    pub xy: Vec<[f64; 2]>,
    pub z: Option<Vec<f64>>,
}

/// Inputs for a single channel-design scenario. `centreline` of `None` (or fewer than 3 points)
/// yields a straight channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDesignInputs {
    /// Convoy length (barge + pusher): drives bends.
    pub total_loa: f64,
    /// Moulded beam: drives base width.
    pub beam: f64,
    pub centreline: Option<Centreline>,
    pub params: ChannelDesignParams,
}

impl ChannelDesignInputs {
    pub fn validate(&self) -> Result<(), RuleError> {
        if self.total_loa <= 0.0 {
            return value_err("total_loa must be > 0".into());
        }
        if self.beam <= 0.0 {
            return value_err("beam must be > 0".into());
        }
        Ok(())
    }
}

/// One-lane channel geometry along the centreline + summary, checks, warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDesignResult {
    pub total_loa: f64,
    pub beam: f64,
    pub design_depth_m: f64,
    /// Straight one-lane bottom width = k_w x beam.
    pub base_width_m: f64,
    /// R_min threshold = k_r x total_loa.
    pub required_min_radius_m: f64,

    // Per-chainage arrays (parallel).
    pub chainage_m: Vec<f64>,
    /// Raw circumcircle radius.
    pub radius_m: Vec<f64>,
    /// Rolling-averaged radius (drives widening).
    pub smoothed_radius_m: Vec<f64>,
    pub extra_width_m: Vec<f64>,
    pub bottom_width_m: Vec<f64>,
    pub top_width_m: Vec<f64>,
    pub bed_level_m: Vec<f64>,
    pub half_offset_m: Vec<f64>,
    /// Not a sub-R_min bend.
    pub radius_ok: Vec<bool>,

    // Summary
    pub total_length_m: f64,
    /// Tightest actual smoothed radius on the route.
    pub min_bend_radius_m: f64,
    pub tight_bend_count: usize,
    /// Mean bottom width.
    pub mean_width_m: f64,
    pub max_extra_width_m: f64,
    pub max_top_width_m: f64,

    // Passing places
    pub passing_place_spacing_m: f64,
    pub passing_place_length_m: f64,
    pub passing_place_width_m: f64,

    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
}

/// JSON-safe float: inf and nan become `None`.
pub fn jnum(x: f64) -> Option<f64> {
    if x.is_infinite() || x.is_nan() {
        None
    } else {
        Some(x)
    }
}

/// The scalar summary of a result (`ChannelDesignResult.summary()`).
#[derive(Debug, Clone, PartialEq)]
pub struct DesignSummary {
    pub total_loa: f64,
    pub beam: f64,
    pub design_depth_m: f64,
    pub base_width_m: f64,
    pub required_min_radius_m: f64,
    pub total_length_m: f64,
    pub min_bend_radius_m: Option<f64>,
    pub tight_bend_count: usize,
    pub mean_width_m: f64,
    pub max_extra_width_m: f64,
    pub max_top_width_m: f64,
    pub passing_place_spacing_m: f64,
    pub passing_place_length_m: f64,
    pub passing_place_width_m: f64,
    pub ok: bool,
}

/// The per-chainage columns of a downsampled, rounded result.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignSections {
    pub chainage_m: Vec<f64>,
    pub smoothed_radius_m: Vec<Option<f64>>,
    pub extra_width_m: Vec<f64>,
    pub bottom_width_m: Vec<f64>,
    pub top_width_m: Vec<f64>,
    pub bed_level_m: Vec<f64>,
    pub half_offset_m: Vec<f64>,
    pub radius_ok: Vec<bool>,
}

/// `ChannelDesignResult.to_dict()`: summary + (optionally downsampled) sections + checks.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignDict {
    pub summary: DesignSummary,
    pub sections: DesignSections,
    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
}

impl ChannelDesignResult {
    /// All checks pass (every severity gates here, unlike `size_channel`'s result).
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    pub fn summary(&self) -> DesignSummary {
        DesignSummary {
            total_loa: self.total_loa,
            beam: self.beam,
            design_depth_m: self.design_depth_m,
            base_width_m: self.base_width_m,
            required_min_radius_m: self.required_min_radius_m,
            total_length_m: self.total_length_m,
            min_bend_radius_m: jnum(self.min_bend_radius_m),
            tight_bend_count: self.tight_bend_count,
            mean_width_m: self.mean_width_m,
            max_extra_width_m: self.max_extra_width_m,
            max_top_width_m: self.max_top_width_m,
            passing_place_spacing_m: self.passing_place_spacing_m,
            passing_place_length_m: self.passing_place_length_m,
            passing_place_width_m: self.passing_place_width_m,
            ok: self.ok(),
        }
    }

    /// Summary, sections (downsampled to about `n_samples`, rounded to 3 decimals) and checks.
    pub fn to_dict(&self, n_samples: Option<usize>) -> DesignDict {
        let idx = downsample_indices(self.chainage_m.len(), n_samples);
        let r3 = |v: &[f64]| -> Vec<f64> { idx.iter().map(|&i| round_to(v[i], 3)).collect() };
        let sections = DesignSections {
            chainage_m: idx.iter().map(|&i| self.chainage_m[i]).collect(),
            smoothed_radius_m: idx
                .iter()
                .map(|&i| jnum(self.smoothed_radius_m[i]))
                .collect(),
            extra_width_m: r3(&self.extra_width_m),
            bottom_width_m: r3(&self.bottom_width_m),
            top_width_m: r3(&self.top_width_m),
            bed_level_m: r3(&self.bed_level_m),
            half_offset_m: r3(&self.half_offset_m),
            radius_ok: idx.iter().map(|&i| self.radius_ok[i]).collect(),
        };
        DesignDict {
            summary: self.summary(),
            sections,
            checks: self.checks.clone(),
            warnings: self.warnings.clone(),
        }
    }
}

fn downsample_indices(n: usize, n_samples: Option<usize>) -> Vec<usize> {
    match n_samples {
        Some(s) if n > s && n != 0 => {
            let step = n as f64 / s as f64;
            (0..s).map(|i| (i as f64 * step) as usize).collect()
        }
        _ => (0..n).collect(),
    }
}

// =============================================================================
// numpy, written out (the shared parts are in `crate::numpy`)
// =============================================================================

/// `np.unwrap(p)` with the default period 2 pi.
fn unwrap(p: &[f64]) -> Vec<f64> {
    use core::f64::consts::PI;
    let n = p.len();
    let mut up = p.to_vec();
    let mut acc = 0.0;
    for i in 1..n {
        let dd = p[i] - p[i - 1];
        let mut ddmod = np_mod(dd + PI, 2.0 * PI) - PI;
        if ddmod == -PI && dd > 0.0 {
            ddmod = PI;
        }
        let mut ph = ddmod - dd;
        if dd.abs() < PI {
            ph = 0.0;
        }
        acc += ph;
        up[i] = p[i] + acc;
    }
    up
}

fn norm2(dx: f64, dy: f64) -> f64 {
    (dx * dx + dy * dy).sqrt()
}

fn nan_min(xs: &[f64]) -> f64 {
    let mut m = xs[0];
    for &x in xs {
        if x.is_nan() {
            return f64::NAN;
        }
        if x < m {
            m = x;
        }
    }
    m
}

fn nan_max(xs: &[f64]) -> f64 {
    let mut m = xs[0];
    for &x in xs {
        if x.is_nan() {
            return f64::NAN;
        }
        if x > m {
            m = x;
        }
    }
    m
}

// =============================================================================
// Core math (extracted + optimised from RiverMeanderAnalysis)
// =============================================================================

/// Resample a polyline at fixed arc-length spacing (RMA `_resample_line`).
fn resample_points(coords: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    if coords.len() < 2 {
        return coords.to_vec();
    }
    let mut cum = Vec::with_capacity(coords.len());
    cum.push(0.0);
    let mut run = 0.0;
    for w in coords.windows(2) {
        run += norm2(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        cum.push(run);
    }
    let total = run;
    if total <= 0.0 {
        return coords.to_vec();
    }
    let n = std::cmp::max((total / step).ceil() as i64 + 1, 3) as usize;
    let xs: Vec<f64> = coords.iter().map(|p| p[0]).collect();
    let ys: Vec<f64> = coords.iter().map(|p| p[1]).collect();
    linspace(0.0, total, n)
        .into_iter()
        .map(|t| [interp(t, &cum, &xs), interp(t, &cum, &ys)])
        .collect()
}

/// Circumcircle radius at every interior point; the ends copy their neighbour. Collinear
/// points give infinity.
fn compute_curvature_radius(points: &[[f64; 2]]) -> Vec<f64> {
    let n = points.len();
    let mut radii = vec![f64::INFINITY; n];
    if n < 3 {
        return radii;
    }
    for i in 0..n - 2 {
        let (a, b, c) = (points[i], points[i + 1], points[i + 2]);
        let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
        let a2 = a[0] * a[0] + a[1] * a[1];
        let b2 = b[0] * b[0] + b[1] * b[1];
        let c2 = c[0] * c[0] + c[1] * c[1];
        let ux = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
        let uy = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
        let mut r = libm::hypot(ux - a[0], uy - a[1]);
        if d.abs() < 1e-9 {
            r = f64::INFINITY;
        }
        if r.is_nan() || r <= 0.0 {
            r = f64::INFINITY;
        }
        radii[i + 1] = r;
    }
    radii[0] = radii[1];
    radii[n - 1] = radii[n - 2];
    radii
}

/// Cumulative chainage along a polyline (RMA `_compute_chainages`).
fn compute_chainages(points: &[[f64; 2]]) -> Vec<f64> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(points.len());
    out.push(0.0);
    let mut run = 0.0;
    for w in points.windows(2) {
        run += norm2(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        out.push(run);
    }
    out
}

/// Unwrapped tangent bearing (rad) at each point, from forward differences.
fn compute_heading(points: &[[f64; 2]]) -> Vec<f64> {
    let n = points.len();
    if n < 2 {
        return vec![0.0; n];
    }
    let mut ang: Vec<f64> = points
        .windows(2)
        .map(|w| libm::atan2(w[1][1] - w[0][1], w[1][0] - w[0][0]))
        .collect();
    ang.push(ang[ang.len() - 1]); // pad to length n
    unwrap(&ang)
}

/// `math.degrees`.
fn degrees(rad: f64) -> f64 {
    rad * (180.0 / core::f64::consts::PI)
}

/// Segment the centreline into discrete bends by turn direction (inflection points).
///
/// A bend is a maximal run of consistently-signed heading change (left- or right-turning). For
/// every point in a bend, returns its total deflection phi (deg, |heading_end - start|) and the
/// bend's characteristic radius R1 (the tightest smoothed radius in the run). Straight points
/// get phi = 0 and R1 = infinity. This is the discrete-bend basis a width method's
/// `bend_width(R1, R2, phi)` needs: a whole bend, not only its "below R_min" run.
fn segment_bends(heading: &[f64], smoothed: &[f64]) -> (Vec<f64>, Vec<f64>) {
    const TURN_EPS: f64 = 1e-3;
    let n = heading.len();
    let mut phi = vec![0.0; n];
    let mut r1 = vec![f64::INFINITY; n];
    if n < 3 {
        return (phi, r1);
    }
    let sign: Vec<i8> = heading
        .windows(2)
        .map(|w| {
            let turn = w[1] - w[0];
            if turn > TURN_EPS {
                1
            } else if turn < -TURN_EPS {
                -1
            } else {
                0
            }
        })
        .collect();
    let mut i = 0;
    while i < n - 1 {
        if sign[i] == 0 {
            i += 1;
            continue;
        }
        let s = sign[i];
        let mut j = i;
        while j + 1 < n - 1 && sign[j + 1] == s {
            j += 1;
        }
        let (a, b) = (i, j + 1); // bend spans points a..b
        let deflection = (heading[b] - heading[a]).abs();
        let rr = smoothed[a..=b]
            .iter()
            .copied()
            .filter(|x| x.is_finite())
            .fold(None, |m: Option<f64>, x| match m {
                Some(m) if m <= x => Some(m),
                _ => Some(x),
            })
            .unwrap_or(f64::INFINITY);
        for k in a..=b {
            phi[k] = degrees(deflection);
            r1[k] = rr;
        }
        i = j + 1;
    }
    (phi, r1)
}

/// Distance-windowed mean of finite radii. Resampling is uniform, so a distance window maps to
/// a fixed sample half-width `k`.
fn rolling_average_radius(radii: &[f64], chainages: &[f64], window: f64) -> Vec<f64> {
    let n = radii.len();
    let mut avg = vec![f64::INFINITY; n];
    if n == 0 {
        return avg;
    }
    let last = chainages[n - 1];
    let spacing = if n > 1 && last > 0.0 {
        last / (n - 1) as f64
    } else {
        1.0
    };
    // Python's `round()` of a float to int: half to even.
    let k = std::cmp::max(libm::rint(window / spacing) as i64, 0) as usize;

    let mut csum = vec![0.0; n + 1];
    let mut ccnt = vec![0.0; n + 1];
    for i in 0..n {
        let valid = radii[i].is_finite() && radii[i] > 0.0;
        csum[i + 1] = csum[i] + if valid { radii[i] } else { 0.0 };
        ccnt[i + 1] = ccnt[i] + if valid { 1.0 } else { 0.0 };
    }
    for (i, a) in avg.iter_mut().enumerate() {
        let lo = i.saturating_sub(k);
        let hi = std::cmp::min(n, i + k + 1);
        let s = csum[hi] - csum[lo];
        let c = ccnt[hi] - ccnt[lo];
        if c > 0.0 {
            *a = s / c;
        }
    }
    avg
}

/// Radius band edges `[R_min, ..., upper]`: `n_bands + 1` values.
fn band_edges(total_loa: f64, params: &ChannelDesignParams) -> Vec<f64> {
    let r_min = params.radius_multiple * total_loa;
    let upper = py_max(params.max_extra_width_radius, r_min + 1.0);
    linspace(r_min, upper, params.n_bands as usize + 1)
}

/// Extra width per band: coefficient x loa^2 / lower_edge; outermost band = 0.
fn band_widths(total_loa: f64, params: &ChannelDesignParams) -> Vec<f64> {
    let edges = band_edges(total_loa, params);
    let coeff = params.extra_width_coefficient;
    let mut widths: Vec<f64> = edges[..edges.len() - 1]
        .iter()
        .map(|e| coeff * (total_loa * total_loa) / e)
        .collect();
    widths.push(0.0);
    widths
}

/// `np.searchsorted(limits, x, side="right")`, clipped to the band count.
fn band_index(limits: &[f64], x: f64, n_widths: usize) -> usize {
    let i = limits.partition_point(|&l| l <= x);
    std::cmp::min(i, n_widths - 1)
}

/// Banded bend-widening for a single radius (m). 0 if straight / above threshold.
pub fn extra_width(radius: f64, total_loa: f64, params: &ChannelDesignParams) -> f64 {
    if !radius.is_finite() || radius <= 0.0 {
        return 0.0;
    }
    if params.extra_width_coefficient <= 0.0 || total_loa <= 0.0 {
        return 0.0;
    }
    if radius >= params.max_extra_width_radius {
        return 0.0;
    }
    let edges = band_edges(total_loa, params);
    let widths = band_widths(total_loa, params);
    widths[band_index(&edges[1..], radius, widths.len())]
}

/// Required (design) bend radius R2 = radius_multiple x total_loa: the target the centreline is
/// straightened to (cut-throughs). Bends below it are widened; where the width exceeds the LOA,
/// a cut-through is indicated.
pub fn min_bend_radius(total_loa: f64, params: &ChannelDesignParams) -> f64 {
    params.radius_multiple * total_loa
}

fn in_margin(chainages: &[f64], total_length: f64, margin: f64) -> Vec<bool> {
    let hi = py_max(total_length - margin, 0.0);
    chainages.iter().map(|&c| c >= margin && c <= hi).collect()
}

/// Banded extra width along the route (RMA `_compute_extra_width` equivalent).
fn extra_width_array(
    smoothed: &[f64],
    chainages: &[f64],
    total_length: f64,
    total_loa: f64,
    params: &ChannelDesignParams,
) -> Vec<f64> {
    let mut extra = vec![0.0; smoothed.len()];
    if params.extra_width_coefficient <= 0.0 || total_loa <= 0.0 {
        return extra;
    }
    let inner = in_margin(chainages, total_length, params.edge_margin_m);
    let threshold = params.max_extra_width_radius;
    let edges = band_edges(total_loa, params);
    let widths = band_widths(total_loa, params);
    let limits = &edges[1..];
    for i in 0..smoothed.len() {
        let s = smoothed[i];
        if inner[i] && s.is_finite() && s > 0.0 && s < threshold {
            extra[i] = widths[band_index(limits, s, widths.len())];
        }
    }
    extra
}

// =============================================================================
// Stage 0 / 1 / 2
// =============================================================================

/// LOA/beam-independent centreline geometry, computed once.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvatureProfile {
    pub points: Vec<[f64; 2]>,
    pub chainages: Vec<f64>,
    pub radius: Vec<f64>,
    pub smoothed_radius: Vec<f64>,
    pub total_length: f64,
    pub z: Option<Vec<f64>>,
    /// Tangent bearing (rad) at each resampled point.
    pub heading: Vec<f64>,
}

/// LOA-dependent stage: tight-bend detection + banded extra width.
#[derive(Debug, Clone, PartialEq)]
pub struct LoaProfile {
    pub total_loa: f64,
    pub required_min_radius: f64,
    pub radius_ok: Vec<bool>,
    pub tight_mask: Vec<bool>,
    /// Per-point discrete-bend deflection phi (deg).
    pub bend_phi_deg: Vec<f64>,
    /// Per-point bend characteristic radius R1 (m).
    pub bend_r1_m: Vec<f64>,
}

/// Stage 0: resample, curvature radius, rolling-average smoothing (ONCE).
pub fn compute_curvature(
    centreline: Option<&Centreline>,
    params: &ChannelDesignParams,
) -> CurvatureProfile {
    let cl = match centreline {
        Some(c) if c.xy.len() >= 3 => c,
        _ => {
            // Straight channel: a single representative section, no curvature.
            return CurvatureProfile {
                points: vec![[0.0, 0.0]],
                chainages: vec![0.0],
                radius: vec![f64::INFINITY],
                smoothed_radius: vec![f64::INFINITY],
                total_length: 0.0,
                z: None,
                heading: vec![0.0],
            };
        }
    };
    let rp = resample_points(&cl.xy, params.radius_sample_length);
    let chain = compute_chainages(&rp);
    let radius = compute_curvature_radius(&rp);
    let smoothed = rolling_average_radius(&radius, &chain, params.rolling_window_m);
    let heading = compute_heading(&rp);
    let z = cl.z.as_ref().map(|z_in| {
        let orig_chain = compute_chainages(&cl.xy);
        chain
            .iter()
            .map(|&c| interp(c, &orig_chain, z_in))
            .collect()
    });
    let total_length = chain.last().copied().unwrap_or(0.0);
    CurvatureProfile {
        points: rp,
        chainages: chain,
        radius,
        smoothed_radius: smoothed,
        total_length,
        z,
        heading,
    }
}

/// Stage 1: given the curvature profile, derive the LOA-dependent geometry.
///
/// R_min = radius_multiple x total_loa; tight bends are those below it, and each discrete bend
/// gets a deflection angle phi for a width method's bend rule.
pub fn precompute_loa(
    curv: &CurvatureProfile,
    total_loa: f64,
    params: &ChannelDesignParams,
) -> LoaProfile {
    let r_min = min_bend_radius(total_loa, params);
    let is_tight: Vec<bool> = curv
        .smoothed_radius
        .iter()
        .map(|&s| s.is_finite() && s < r_min)
        .collect();
    let radius_ok: Vec<bool> = is_tight.iter().map(|&t| !t).collect(); // straight (inf) is ok
    let inner = in_margin(&curv.chainages, curv.total_length, params.edge_margin_m);
    let tight_mask: Vec<bool> = is_tight.iter().zip(&inner).map(|(&t, &i)| t && i).collect();
    let (bend_phi, bend_r1) = segment_bends(&curv.heading, &curv.smoothed_radius);
    LoaProfile {
        total_loa,
        required_min_radius: r_min,
        radius_ok,
        tight_mask,
        bend_phi_deg: bend_phi,
        bend_r1_m: bend_r1,
    }
}

/// Stage 2: add the beam-dependent widths to a LOA profile -> full result.
///
/// Default: straight bottom width = `width_multiple x beam`, plus the banded extra width on the
/// smoothed radius. With `params.width_method` set: straight = the method's straight width, and
/// at each discrete bend whose radius R1 is below R_min the bottom widens to
/// `method.bend_width(loa, beam, options, R1, R_min, phi)`.
pub fn apply_beam(
    methods: &MethodRegistry,
    curv: &CurvatureProfile,
    loa: &LoaProfile,
    beam: f64,
    params: &ChannelDesignParams,
) -> Result<ChannelDesignResult, RuleError> {
    let depth = resolve_design_depth(params);
    let r_min = loa.required_min_radius;
    let smoothed = &curv.smoothed_radius;
    let n = smoothed.len();

    let (base, bottom): (f64, Vec<f64>) = match &params.width_method {
        None => {
            let base = params.width_multiple * beam;
            let extra_band = extra_width_array(
                smoothed,
                &curv.chainages,
                curv.total_length,
                loa.total_loa,
                params,
            );
            (base, extra_band.iter().map(|e| base + e).collect())
        }
        Some(name) => {
            let method = methods.get(name)?;
            let opts = &params.width_options;
            let base = (method.straight_width)(loa.total_loa, beam, opts)?;
            let mut b = vec![base; n];
            // Cache by (rounded R1, rounded phi) so each distinct bend is evaluated once.
            let mut cache: HashMap<(u64, u64), f64> = HashMap::new();
            for (i, bi) in b.iter_mut().enumerate() {
                let r1 = loa.bend_r1_m[i];
                if !(r1 < r_min && r1.is_finite() && r1 > 0.0) {
                    continue;
                }
                let key = (round_to(r1, 1), round_to(loa.bend_phi_deg[i], 2));
                let bits = (key.0.to_bits(), key.1.to_bits());
                let bw = match cache.get(&bits) {
                    Some(&v) => v,
                    None => {
                        let v =
                            (method.bend_width)(loa.total_loa, beam, opts, key.0, r_min, key.1)?;
                        cache.insert(bits, v);
                        v
                    }
                };
                if bw > *bi {
                    *bi = bw;
                }
            }
            (base, b)
        }
    };
    let extra: Vec<f64> = bottom.iter().map(|b| b - base).collect();
    let side = 2.0 * params.side_slope * depth;
    let top: Vec<f64> = bottom.iter().map(|b| b + side).collect();
    let half: Vec<f64> = top.iter().map(|t| t / 2.0).collect();

    let bed: Vec<f64> = match &curv.z {
        Some(z) => z.iter().map(|r| r - depth).collect(),
        None => vec![params.reference_level_m - depth; curv.chainages.len()],
    };

    let finite_min = smoothed.iter().copied().filter(|x| x.is_finite()).fold(
        None,
        |m: Option<f64>, x| match m {
            Some(m) if m <= x => Some(m),
            _ => Some(x),
        },
    );
    let min_radius = finite_min.unwrap_or(f64::INFINITY);
    let tight_count = loa.tight_mask.iter().filter(|&&t| t).count();
    let max_extra = if extra.is_empty() {
        0.0
    } else {
        nan_max(&extra)
    };

    // ---- checks (data, not buried ifs) ----
    let req_depth = required_depth(params.draft_m, params.ukc_fraction, params.ukc_min_m);
    let max_bottom = if bottom.is_empty() {
        base
    } else {
        nan_max(&bottom)
    };
    let min_bottom = nan_min(&bottom);
    let checks = vec![
        Check::new(
            "bends_navigable",
            tight_count == 0,
            format!(
                "{tight_count} bend(s) tighter than the design radius R2={} m \
                 (widened; cut-through if width > LOA)",
                fixed(loa.required_min_radius, 0)
            ),
        ),
        Check::new(
            "width_positive",
            min_bottom > 0.0,
            format!("min bottom width {} m > 0", fixed(min_bottom, 1)),
        ),
        Check::new(
            "depth_sufficient",
            depth + 1e-9 >= req_depth,
            format!(
                "design depth {} m >= required {} m for draft {} m",
                fixed(depth, 2),
                fixed(req_depth, 2),
                fixed(params.draft_m, 2)
            ),
        ),
        Check::with_severity(
            "bend_width_within_loa",
            max_bottom <= loa.total_loa + 1e-6,
            format!(
                "max bend width {} m vs LOA {} m \u{2014} above LOA, a cut-through is indicated",
                fixed(max_bottom, 1),
                fixed(loa.total_loa, 0)
            ),
            "warning",
        ),
    ];

    let mean_width = if bottom.is_empty() {
        base
    } else {
        bottom.iter().sum::<f64>() / bottom.len() as f64
    };
    let max_top = if top.is_empty() {
        base + 2.0 * params.side_slope * depth
    } else {
        nan_max(&top)
    };
    Ok(ChannelDesignResult {
        total_loa: loa.total_loa,
        beam,
        design_depth_m: depth,
        base_width_m: base,
        required_min_radius_m: loa.required_min_radius,
        chainage_m: curv.chainages.clone(),
        radius_m: curv.radius.clone(),
        smoothed_radius_m: smoothed.clone(),
        extra_width_m: extra,
        bottom_width_m: bottom,
        top_width_m: top,
        bed_level_m: bed,
        half_offset_m: half,
        radius_ok: loa.radius_ok.clone(),
        total_length_m: curv.total_length,
        min_bend_radius_m: min_radius,
        tight_bend_count: tight_count,
        mean_width_m: mean_width,
        max_extra_width_m: max_extra,
        max_top_width_m: max_top,
        passing_place_spacing_m: params.passing_place_spacing_m,
        passing_place_length_m: params.passing_place_length_multiple * loa.total_loa,
        passing_place_width_m: params.passing_place_width_multiple * base,
        checks,
        warnings: Vec::new(),
    })
}

// =============================================================================
// Public API
// =============================================================================

/// Single-scenario channel design (Stage 0 -> 1 -> 2).
pub fn compute(
    methods: &MethodRegistry,
    inputs: &ChannelDesignInputs,
) -> Result<ChannelDesignResult, RuleError> {
    inputs.params.validate(methods)?;
    inputs.validate()?;
    let curv = compute_curvature(inputs.centreline.as_ref(), &inputs.params);
    let loa = precompute_loa(&curv, inputs.total_loa, &inputs.params);
    apply_beam(methods, &curv, &loa, inputs.beam, &inputs.params)
}

/// The result of [`compute_scenario_grid`], keyed `loa{L}_beam{B}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioGrid {
    pub design_depth_m: f64,
    pub total_length_m: f64,
    pub loa_values: Vec<f64>,
    pub beam_values: Vec<f64>,
    pub scenarios: Vec<(String, DesignDict)>,
}

/// Sweep LOA x beam over one centreline.
///
/// Curvature is computed once (Stage 0), then for each LOA the tight-bend / extra-width stage
/// runs once (Stage 1), then each beam is applied (Stage 2).
pub fn compute_scenario_grid(
    methods: &MethodRegistry,
    centreline: Option<&Centreline>,
    loa_values: &[f64],
    beam_values: &[f64],
    params: Option<&ChannelDesignParams>,
    n_samples: Option<usize>,
) -> Result<ScenarioGrid, RuleError> {
    let default = ChannelDesignParams::default();
    let params = params.unwrap_or(&default);
    params.validate(methods)?;
    let curv = compute_curvature(centreline, params);
    let depth = resolve_design_depth(params);
    let mut scenarios: Vec<(String, DesignDict)> = Vec::new();
    for &loa_val in loa_values {
        let loa = precompute_loa(&curv, loa_val, params);
        for &beam_val in beam_values {
            let res = apply_beam(methods, &curv, &loa, beam_val, params)?;
            let key = format!("loa{}_beam{}", tag(loa_val), tag(beam_val));
            let dict = res.to_dict(n_samples);
            match scenarios.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => *v = dict,
                None => scenarios.push((key, dict)),
            }
        }
    }
    Ok(ScenarioGrid {
        design_depth_m: depth,
        total_length_m: curv.total_length,
        loa_values: loa_values.to_vec(),
        beam_values: beam_values.to_vec(),
        scenarios,
    })
}

/// `str(int(f)) if f == int(f) else str(f)`.
fn tag(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        py_str(v)
    }
}
