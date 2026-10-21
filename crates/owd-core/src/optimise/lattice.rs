//! Pool-by-pool lattice alignment search: the least-cost channel through each pool that never
//! bends tighter than R_min. Mirrors `optimise/lattice.py`.
//!
//! Per pool (the reach between two weirs, one water level), search an (x, y, heading) state
//! lattice with A*:
//!
//! - **State**: position snapped to `pitch_m`, heading snapped to one of `n_heading` bins, plus
//!   the turn direction that arrived (so an arc may not reverse straight into the opposite arc; a
//!   straight must sit between them).
//! - **Edges**: left and right arcs at `r_min x radius_multiples`, each turning EXACTLY one
//!   heading bin (so forward integration lands on bin headings and never drifts), plus one
//!   straight. R_min holds BY CONSTRUCTION; no radius check after the fact.
//! - **Edge cost**: the earthwork field of a trapezoidal channel swept along the primitive
//!   ([`FieldScorer`]): fairway cut to grade everywhere; side-slope cut only in sections where
//!   the fairway bites dry ground, and only on dry cells; wet and dry weighted separately; an
//!   optional deep-cut multiplier on dry cut. Weights, not prices.
//! - **Heuristic**: `"c_min"`: straight-line distance x the cheapest per-metre cost anywhere in
//!   the corridor; `"reverse_dijkstra"`: a relaxed 2-D cost-to-go from the goal (tighter, but an
//!   8-connected grid can slightly over-estimate); `"none"`: plain Dijkstra.
//! - **Endpoints**: pinned to the baseline's position and heading at both pool ends, so each
//!   result splices back into the baseline. The goal is reached within `goal_pos_tol_m` (default
//!   half a pitch) and one heading bin; `end_err_m` reports the miss.
//!
//! The search cost is a SEARCH objective. Re-measure the winner with
//! [`crate::volumes::sections::section_volumes`] before you quote a quantity.
//!
//! Terrain comes in through the sampler interface `terrain(xs, ys) -> z` (NaN where unknown). It
//! is sampled once onto a regular grid over the corridor ([`build_terrain_grid`]); despiking a DEM
//! is the caller's job before that.
//!
//! The array code of the Python (numpy sums, `arange`, `linspace`, `rint`) is written out with
//! the same arithmetic in the same order, so that the search takes the same decisions.

use crate::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::offset::{Cap, buffer_line};
use crate::geom::overlay::{Op, contains_xy, overlay};
use crate::geom::{Geometry, Polygon, Pt, linear};
use crate::volumes::sections::{TerrainSampler, arange};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// A refusal: a Python `ValueError`, with its text.
pub type Refusal = String;

/// `(x, y, heading)`.
pub type Pose = (f64, f64, f64);

// =====================================================================================
// Small numpy and Python helpers
// =====================================================================================

/// numpy's `DOUBLE_pairwise_sum` (what `ndarray.sum()` does on a contiguous array).
fn np_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0;
        for &x in a {
            res += x;
        }
        return res;
    }
    if n <= 128 {
        let mut r = [0.0; 8];
        r.copy_from_slice(&a[..8]);
        let mut i = 8;
        while i < n - (n % 8) {
            for (j, rj) in r.iter_mut().enumerate() {
                *rj += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        return res;
    }
    let mut n2 = n / 2;
    n2 -= n2 % 8;
    np_sum(&a[..n2]) + np_sum(&a[n2..])
}

/// Python's `round(x)` to an int: half to even.
fn round_i(x: f64) -> i64 {
    libm::rint(x) as i64
}

/// Python's float `%`: the result takes the sign of the divisor.
fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) { m + b } else { m }
    } else {
        libm::copysign(0.0, b)
    }
}

const TAU: f64 = 2.0 * core::f64::consts::PI;

fn hypot(a: f64, b: f64) -> f64 {
    libm::hypot(a, b)
}

// =====================================================================================
// Configuration
// =====================================================================================

/// The bottom half-width: a number, or `half_width(radius_m)` so arcs are widened in bends.
pub enum HalfWidthSpec {
    Fixed(f64),
    Func(Box<dyn Fn(f64) -> f64>),
}

/// The channel the search sweeps, and how its cut is weighted.
///
/// Each arc is costed at the half-width for its own radius; the straight at `half_width(inf)`.
pub struct SectionSpec {
    pub half_width: HalfWidthSpec,
    /// Template side slope above the bottom edge.
    pub side_slope_h_per_v: f64,
    /// Added to the half-width (e.g. to match a rasterised volume integrator's footprint).
    pub footprint_corr_m: f64,
    /// Per m3 cut inside the river polygon.
    pub wet_weight: f64,
    /// Per m3 cut outside it.
    pub dry_weight: f64,
    /// Dry cut deeper than this ...
    pub deep_threshold_m: f64,
    /// ... is weighted by this as well.
    pub deep_multiplier: f64,
    /// Transverse reach of each cross-section.
    pub rake_half_m: f64,
    /// Transverse sampling step.
    pub rake_step_m: f64,
    /// Cross-section spacing along a primitive.
    pub ds_step_m: f64,
    /// Fairway samples that must be on known terrain.
    pub min_valid_fraction: f64,
}

impl SectionSpec {
    /// The Python defaults for everything but the half-width.
    pub fn new(half_width: HalfWidthSpec) -> Self {
        Self {
            half_width,
            side_slope_h_per_v: 2.0,
            footprint_corr_m: 0.0,
            wet_weight: 1.0,
            dry_weight: 1.0,
            deep_threshold_m: f64::INFINITY,
            deep_multiplier: 1.0,
            rake_half_m: 250.0,
            rake_step_m: 4.0,
            ds_step_m: 8.0,
            min_valid_fraction: 0.6,
        }
    }

    pub fn half_width_at(&self, radius_m: f64) -> f64 {
        match &self.half_width {
            HalfWidthSpec::Fixed(v) => *v,
            HalfWidthSpec::Func(f) => f(radius_m),
        }
    }
}

/// How the lattice is built and searched. Every field can change the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticeConfig {
    pub r_min_m: f64,
    pub radius_multiples: Vec<f64>,
    pub pitch_m: f64,
    pub n_heading: i64,
    /// Default 2 x pitch.
    pub straight_len_m: Option<f64>,
    /// Sampling step along each primitive.
    pub arc_step_m: f64,
    /// Default half a pitch.
    pub goal_pos_tol_m: Option<f64>,
    pub goal_head_tol_bins: f64,
    /// `"c_min"`, `"reverse_dijkstra"` or `"none"`.
    pub heuristic: String,
    /// More than 1 is weighted A*: faster, not optimal.
    pub heuristic_weight: f64,
    /// Drop states with g + h above this.
    pub prune_bound: f64,
    pub max_expand: i64,
    pub wall_cap_s: Option<f64>,
    pub bounds_margin_m: f64,
}

impl LatticeConfig {
    /// The Python defaults for everything but `r_min_m`.
    pub fn new(r_min_m: f64) -> Self {
        Self {
            r_min_m,
            radius_multiples: vec![1.0, 1.5, 3.0],
            pitch_m: 12.0,
            n_heading: 16,
            straight_len_m: None,
            arc_step_m: 10.0,
            goal_pos_tol_m: None,
            goal_head_tol_bins: 1.0,
            heuristic: "c_min".into(),
            heuristic_weight: 1.0,
            prune_bound: f64::INFINITY,
            max_expand: 4_000_000,
            wall_cap_s: None,
            bounds_margin_m: 20.0,
        }
    }

    /// The Python `__post_init__`: the checks and the defaults that depend on the pitch.
    pub fn validated(mut self) -> Result<Self, Refusal> {
        if self.r_min_m <= 0.0 {
            return Err("r_min_m must be > 0".into());
        }
        let lowest = self
            .radius_multiples
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        if lowest < 1.0 {
            return Err("radius_multiples must all be >= 1 so no arc is tighter than R_min".into());
        }
        if !matches!(
            self.heuristic.as_str(),
            "c_min" | "reverse_dijkstra" | "none"
        ) {
            return Err("heuristic must be 'c_min', 'reverse_dijkstra' or 'none'".into());
        }
        if self.straight_len_m.is_none() {
            self.straight_len_m = Some(2.0 * self.pitch_m);
        }
        if self.goal_pos_tol_m.is_none() {
            self.goal_pos_tol_m = Some(0.5 * self.pitch_m);
        }
        Ok(self)
    }

    fn straight_len(&self) -> f64 {
        self.straight_len_m.unwrap_or(2.0 * self.pitch_m)
    }

    fn goal_tol(&self) -> f64 {
        self.goal_pos_tol_m.unwrap_or(0.5 * self.pitch_m)
    }
}

// =====================================================================================
// Terrain on a grid
// =====================================================================================

/// Terrain sampled on a regular north-up grid. `(x0, y0)` is the CENTRE of the top-left cell;
/// row index grows southward. `valid` marks known ground inside the search corridor.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainGrid {
    pub ground: Vec<Vec<f64>>,
    pub wet: Vec<Vec<bool>>,
    pub valid: Vec<Vec<bool>>,
    pub x0: f64,
    pub y0: f64,
    pub cell_m: f64,
}

impl TerrainGrid {
    fn rows(&self) -> usize {
        self.ground.len()
    }

    fn cols(&self) -> usize {
        self.ground.first().map_or(0, Vec::len)
    }

    fn rc(&self, x: f64, y: f64) -> (f64, f64) {
        ((self.y0 - y) / self.cell_m, (x - self.x0) / self.cell_m)
    }

    /// Bilinear ground level between cell centres; NaN off the grid or on unknown ground.
    pub fn sample_ground_at(&self, x: f64, y: f64) -> f64 {
        let (row, col) = self.rc(x, y);
        let (h, w) = (self.rows(), self.cols());
        let (c0, r0) = (col.floor(), row.floor());
        let ok = c0 >= 0.0 && r0 >= 0.0 && c0 < (w as f64 - 1.0) && r0 < (h as f64 - 1.0);
        if !ok {
            return f64::NAN;
        }
        let (fc, fr) = (col - c0, row - r0);
        let (cc, rr) = (c0 as usize, r0 as usize);
        let g = &self.ground;
        g[rr][cc] * (1.0 - fc) * (1.0 - fr)
            + g[rr][cc + 1] * fc * (1.0 - fr)
            + g[rr + 1][cc] * (1.0 - fc) * fr
            + g[rr + 1][cc + 1] * fc * fr
    }

    pub fn sample_ground(&self, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| self.sample_ground_at(x, y))
            .collect()
    }

    /// Nearest-cell wet mask; false off the grid.
    pub fn sample_wet_at(&self, x: f64, y: f64) -> bool {
        let (row, col) = self.rc(x, y);
        let (h, w) = (self.rows() as f64, self.cols() as f64);
        let (r, c) = (libm::rint(row), libm::rint(col));
        if c >= 0.0 && r >= 0.0 && c < w && r < h {
            self.wet[r as usize][c as usize]
        } else {
            false
        }
    }

    pub fn sample_wet(&self, xs: &[f64], ys: &[f64]) -> Vec<bool> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| self.sample_wet_at(x, y))
            .collect()
    }

    /// Cell centres for the given row and column indices.
    pub fn cell_centres(&self, rows: &[usize], cols: &[usize]) -> (Vec<f64>, Vec<f64>) {
        (
            cols.iter()
                .map(|&c| self.x0 + c as f64 * self.cell_m)
                .collect(),
            rows.iter()
                .map(|&r| self.y0 - r as f64 * self.cell_m)
                .collect(),
        )
    }

    /// `(xmin, ymin, xmax, ymax)` of the cell centres.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (h, w) = (self.rows(), self.cols());
        (
            self.x0,
            self.y0 - (h as f64 - 1.0) * self.cell_m,
            self.x0 + (w as f64 - 1.0) * self.cell_m,
            self.y0,
        )
    }
}

/// `(xmin, ymin, xmax, ymax)` of the polygons' vertices (shapely `.bounds`).
fn polygon_bounds(polys: &[Polygon]) -> (f64, f64, f64, f64) {
    let mut b = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in polys {
        for pt in &p.exterior {
            b.0 = b.0.min(pt[0]);
            b.1 = b.1.min(pt[1]);
            b.2 = b.2.max(pt[0]);
            b.3 = b.3.max(pt[1]);
        }
    }
    b
}

/// Sample `terrain` at cell centres over the corridor's bounds (plus `margin_m`).
///
/// Ground outside the corridor is kept (a side slope may daylight there); `valid` is known
/// ground INSIDE the corridor, which is where the heuristic samples.
pub fn build_terrain_grid(
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    corridor: &[Polygon],
    cell_m: f64,
    margin_m: f64,
) -> Result<TerrainGrid, Refusal> {
    let (mut xmin, mut ymin, mut xmax, mut ymax) = polygon_bounds(corridor);
    xmin -= margin_m;
    ymin -= margin_m;
    xmax += margin_m;
    ymax += margin_m;
    let w = ((xmax - xmin) / cell_m).ceil() as usize + 1;
    let h = ((ymax - ymin) / cell_m).ceil() as usize + 1;
    let xs: Vec<f64> = (0..w).map(|i| xmin + i as f64 * cell_m).collect();
    let ys: Vec<f64> = (0..h).map(|i| ymax - i as f64 * cell_m).collect();
    let mut px = Vec::with_capacity(h * w);
    let mut py = Vec::with_capacity(h * w);
    for &y in &ys {
        for &x in &xs {
            px.push(x);
            py.push(y);
        }
    }
    let z = terrain(&px, &py);
    if z.len() != h * w {
        return Err(format!(
            "cannot reshape array of size {} into shape ({h},{w})",
            z.len()
        ));
    }
    let mut ground = Vec::with_capacity(h);
    let mut wet = Vec::with_capacity(h);
    let mut valid = Vec::with_capacity(h);
    for r in 0..h {
        let row = &z[r * w..(r + 1) * w];
        let mut wet_row = Vec::with_capacity(w);
        let mut valid_row = Vec::with_capacity(w);
        for (c, &zc) in row.iter().enumerate() {
            let p = [px[r * w + c], py[r * w + c]];
            wet_row.push(contains_xy(river_polygon, p));
            valid_row.push(zc.is_finite() && contains_xy(corridor, p));
        }
        ground.push(row.to_vec());
        wet.push(wet_row);
        valid.push(valid_row);
    }
    Ok(TerrainGrid {
        ground,
        wet,
        valid,
        x0: xmin,
        y0: ymax,
        cell_m,
    })
}

// =====================================================================================
// Primitives
// =====================================================================================

/// One motion primitive: an arc (or a straight) turning at most one heading bin.
#[derive(Debug, Clone, PartialEq)]
pub struct Prim {
    /// Radius; infinity for a straight.
    pub r: f64,
    /// +1 left, -1 right, 0 straight.
    pub sign: i8,
    /// `(s, x, y, heading)` along the primitive, in its own frame.
    pub pts: Vec<[f64; 4]>,
    /// `(x, y, heading)` at its end.
    pub end: (f64, f64, f64),
    pub arc_len: f64,
}

/// Left/right arcs at `r_min x radius_multiples` turning exactly one heading bin, plus one
/// straight of `straight_len_m`.
pub fn lattice_primitives(cfg: &LatticeConfig) -> Vec<Prim> {
    let mut prims = Vec::new();
    let bs = TAU / cfg.n_heading as f64;
    for &m in &cfg.radius_multiples {
        let r = cfg.r_min_m * m;
        let arc_len = r * bs;
        let n = std::cmp::max(2, round_i(arc_len / cfg.arc_step_m)) as usize;
        for sgn in [1_i8, -1] {
            let k = f64::from(sgn) / r;
            let pts: Vec<[f64; 4]> = (1..=n)
                .map(|i| {
                    let i = i as f64;
                    let nn = n as f64;
                    [
                        arc_len * i / nn,
                        libm::sin(k * arc_len * i / nn) / k,
                        (1.0 - libm::cos(k * arc_len * i / nn)) / k,
                        k * arc_len * i / nn,
                    ]
                })
                .collect();
            prims.push(Prim {
                r,
                sign: sgn,
                pts,
                end: (
                    libm::sin(k * arc_len) / k,
                    (1.0 - libm::cos(k * arc_len)) / k,
                    k * arc_len,
                ),
                arc_len,
            });
        }
    }
    let l = cfg.straight_len();
    let n = std::cmp::max(2, round_i(l / cfg.arc_step_m)) as usize;
    prims.push(straight_prim(l, n));
    prims
}

fn straight_prim(length: f64, n: usize) -> Prim {
    Prim {
        r: f64::INFINITY,
        sign: 0,
        pts: (1..=n)
            .map(|i| {
                let v = length * i as f64 / n as f64;
                [v, v, 0.0, 0.0]
            })
            .collect(),
        end: (length, 0.0, 0.0),
        arc_len: length,
    }
}

fn unit_straight(length: f64) -> Prim {
    let n = std::cmp::max(2, round_i(length / 10.0)) as usize;
    straight_prim(length, n)
}

// =====================================================================================
// Edge cost
// =====================================================================================

/// Weighted cut of a constant-width trapezoidal channel swept along one primitive.
///
/// `score(prim, pose) -> (cost, ok)`; `ok` is false when less than `min_valid_fraction` of the
/// fairway samples are on known terrain.
pub struct FieldScorer<'a> {
    grid: &'a TerrainGrid,
    grade: f64,
    spec: &'a SectionSpec,
    offs: Vec<f64>,
    tmpl: Vec<f64>,
    in_fairway: Vec<bool>,
}

impl<'a> FieldScorer<'a> {
    pub fn new(grid: &'a TerrainGrid, grade_m: f64, hb_m: f64, spec: &'a SectionSpec) -> Self {
        let offs = arange(-spec.rake_half_m, spec.rake_half_m + 1e-6, spec.rake_step_m);
        let tmpl: Vec<f64> = offs
            .iter()
            .map(|o| grade_m + crate::py_max(0.0, o.abs() - hb_m) / spec.side_slope_h_per_v)
            .collect();
        let in_fairway = offs.iter().map(|o| o.abs() <= hb_m).collect();
        Self {
            grid,
            grade: grade_m,
            spec,
            offs,
            tmpl,
            in_fairway,
        }
    }

    pub fn score(&self, prim: &Prim, pose: Pose) -> (f64, bool) {
        let (x0, y0, th0) = pose;
        let sp = self.spec;
        let pts = &prim.pts;
        let ds_native = prim.arc_len / pts.len() as f64;
        let stride = if ds_native > 0.0 {
            std::cmp::max(1, round_i(sp.ds_step_m / ds_native)) as usize
        } else {
            1
        };
        let mut sel: Vec<&[f64; 4]> = pts.iter().step_by(stride).collect();
        if !core::ptr::eq(
            *sel.last().expect("a primitive has points"),
            &pts[pts.len() - 1],
        ) {
            sel.push(&pts[pts.len() - 1]);
        }
        let n_sec = sel.len();
        let ds = prim.arc_len / n_sec as f64;
        let m = self.offs.len();

        let (ct, st) = (libm::cos(th0), libm::sin(th0));
        let mut gz = vec![0.0; n_sec * m];
        let mut wet = vec![false; n_sec * m];
        for (i, p) in sel.iter().enumerate() {
            let wx = x0 + ct * p[1] - st * p[2];
            let wy = y0 + st * p[1] + ct * p[2];
            let hth = th0 + p[3];
            let (nx, ny) = (-libm::sin(hth), libm::cos(hth));
            for (j, &o) in self.offs.iter().enumerate() {
                let gx = wx + o * nx;
                let gy = wy + o * ny;
                gz[i * m + j] = self.grid.sample_ground_at(gx, gy);
                wet[i * m + j] = self.grid.sample_wet_at(gx, gy);
            }
        }
        let n_fair = self.in_fairway.iter().filter(|&&f| f).count();
        if n_fair == 0 {
            return (f64::INFINITY, false);
        }
        let mut fair_finite = 0usize;
        for i in 0..n_sec {
            for j in 0..m {
                if self.in_fairway[j] && gz[i * m + j].is_finite() {
                    fair_finite += 1;
                }
            }
        }
        if (fair_finite as f64 / (n_sec * n_fair) as f64) < sp.min_valid_fraction {
            return (f64::INFINITY, false);
        }

        let mut cut = vec![0.0; n_sec * m];
        for i in 0..n_sec {
            let mut hits_dry = false;
            for j in 0..m {
                let z = gz[i * m + j];
                if z.is_finite() && self.in_fairway[j] {
                    let fc = crate::py_max(z - self.grade, 0.0);
                    cut[i * m + j] = fc;
                    if fc > 0.0 && !wet[i * m + j] {
                        hits_dry = true;
                    }
                }
            }
            if hits_dry {
                for j in 0..m {
                    let z = gz[i * m + j];
                    if z.is_finite() && !self.in_fairway[j] && !wet[i * m + j] {
                        cut[i * m + j] += crate::py_max(z - self.tmpl[j], 0.0);
                    }
                }
            }
        }

        let cell = sp.rake_step_m * ds;
        let dredge: Vec<f64> = cut
            .iter()
            .zip(&wet)
            .map(|(&c, &w)| c * if w { 1.0 } else { 0.0 })
            .collect();
        let excav: Vec<f64> = cut
            .iter()
            .zip(&wet)
            .map(|(&c, &w)| c * if w { 0.0 } else { 1.0 })
            .collect();
        let shallow: Vec<f64> = excav
            .iter()
            .map(|&e| if e > sp.deep_threshold_m { 0.0 } else { e })
            .collect();
        let deep: Vec<f64> = excav
            .iter()
            .map(|&e| if e > sp.deep_threshold_m { e } else { 0.0 })
            .collect();
        let total = (np_sum(&dredge) * sp.wet_weight
            + np_sum(&shallow) * sp.dry_weight
            + np_sum(&deep) * sp.dry_weight * sp.deep_multiplier)
            * cell;
        (total, true)
    }
}

// =====================================================================================
// Heuristics
// =====================================================================================

/// `(row index / s, col index / s, x, y)` of the valid cells on every `s`-th row and column.
struct ValidPoints {
    ir: Vec<usize>,
    ic: Vec<usize>,
    xs: Vec<f64>,
    ys: Vec<f64>,
}

fn valid_points(grid: &TerrainGrid, stride_m: f64, max_samples: Option<usize>) -> ValidPoints {
    let s = std::cmp::max(1, round_i(stride_m / grid.cell_m)) as usize;
    let (mut rr, mut cc) = (Vec::new(), Vec::new());
    for (r, row) in grid.valid.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            if v && r % s == 0 && c % s == 0 {
                rr.push(r);
                cc.push(c);
            }
        }
    }
    if let Some(max) = max_samples
        && rr.len() > max
    {
        // np.linspace(0, size - 1, max).astype(int)
        let (n, last) = (max, (rr.len() - 1) as f64);
        let step = last / (n - 1) as f64;
        let idx: Vec<usize> = (0..n)
            .map(|i| if i == n - 1 { last } else { i as f64 * step } as usize)
            .collect();
        rr = idx.iter().map(|&i| rr[i]).collect();
        cc = idx.iter().map(|&i| cc[i]).collect();
    }
    let (xs, ys) = grid.cell_centres(&rr, &cc);
    ValidPoints {
        ir: rr.iter().map(|r| r / s).collect(),
        ic: cc.iter().map(|c| c / s).collect(),
        xs,
        ys,
    }
}

/// The cheapest per-metre straight cost anywhere in the corridor (0 if none is valid). Straight
/// cost is symmetric under a half turn, so half the heading bins are swept.
pub fn min_cost_per_metre(
    grid: &TerrainGrid,
    scorer: &FieldScorer,
    n_heading: i64,
    length_m: f64,
    stride_m: f64,
    max_samples: usize,
) -> f64 {
    let v = valid_points(grid, stride_m, Some(max_samples));
    let unit = unit_straight(length_m);
    let bs = TAU / n_heading as f64;
    let mut best = f64::INFINITY;
    for (&x, &y) in v.xs.iter().zip(&v.ys) {
        for b in 0..std::cmp::max(1, n_heading / 2) {
            let (c, ok) = scorer.score(&unit, (x, y, b as f64 * bs));
            if ok {
                best = crate::py_min(best, c / length_m);
            }
        }
    }
    if best.is_finite() { best } else { 0.0 }
}

/// Defaults of `min_cost_per_metre`.
pub const DEFAULT_C_MIN_STRIDE_M: f64 = 24.0;
pub const DEFAULT_C_MIN_MAX_SAMPLES: usize = 4000;

/// A 2-D cost-to-go from the goal on a coarse grid, curvature dropped and the cheapest heading
/// taken per cell.
pub struct ReverseDijkstra {
    d: Vec<Vec<f64>>,
    s: usize,
    x0: f64,
    y0: f64,
    cell_m: f64,
}

impl ReverseDijkstra {
    /// The cost to go from `(x, y)` (a large number off the reachable grid).
    pub fn h(&self, x: f64, y: f64) -> f64 {
        let s = self.s as f64;
        let i = round_i((self.y0 - y) / self.cell_m / s);
        let j = round_i((x - self.x0) / self.cell_m / s);
        let (nr, nc) = (
            self.d.len() as i64,
            self.d.first().map_or(0, Vec::len) as i64,
        );
        if 0 <= i && i < nr && 0 <= j && j < nc && self.d[i as usize][j as usize].is_finite() {
            return self.d[i as usize][j as usize];
        }
        1e18
    }
}

#[derive(PartialEq)]
struct Node(f64, usize, usize);

impl Eq for Node {}

impl Ord for Node {
    /// Reversed, so the `BinaryHeap` pops the smallest `(d, i, j)`, as Python's `heapq`.
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0)
            .unwrap_or(Ordering::Equal)
            .then(o.1.cmp(&self.1))
            .then(o.2.cmp(&self.2))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

pub fn reverse_dijkstra_heuristic(
    grid: &TerrainGrid,
    scorer: &FieldScorer,
    goal_xy: Pt,
    n_heading: i64,
    length_m: f64,
    stride_m: f64,
) -> ReverseDijkstra {
    let s = std::cmp::max(1, round_i(stride_m / grid.cell_m)) as usize;
    let (h, w) = (grid.rows(), grid.cols());
    let (nr, nc) = (h.div_ceil(s), w.div_ceil(s));
    let mut cg = vec![vec![f64::INFINITY; nc]; nr];
    let unit = unit_straight(length_m);
    let bs = TAU / n_heading as f64;
    let v = valid_points(grid, stride_m, None);
    for k in 0..v.xs.len() {
        let mut best = f64::INFINITY;
        for b in 0..std::cmp::max(1, n_heading / 2) {
            let (c, ok) = scorer.score(&unit, (v.xs[k], v.ys[k], b as f64 * bs));
            if ok {
                best = crate::py_min(best, c / length_m);
            }
        }
        cg[v.ir[k]][v.ic[k]] = best;
    }
    let step = s as f64 * grid.cell_m;
    let mut gr = round_i((grid.y0 - goal_xy[1]) / grid.cell_m / s as f64);
    let mut gc = round_i((goal_xy[0] - grid.x0) / grid.cell_m / s as f64);
    gr = gr.clamp(0, nr as i64 - 1);
    gc = gc.clamp(0, nc as i64 - 1);
    let (mut gr, mut gc) = (gr as usize, gc as usize);
    if !cg[gr][gc].is_finite() {
        // The finite cell nearest the goal in grid steps; the first one wins a tie.
        let mut best: Option<(i64, usize, usize)> = None;
        for (i, row) in cg.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                if v.is_finite() {
                    let d = (i as i64 - gr as i64).abs() + (j as i64 - gc as i64).abs();
                    if best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, i, j));
                    }
                }
            }
        }
        if let Some((_, i, j)) = best {
            gr = i;
            gc = j;
        }
    }
    let mut d = vec![vec![f64::INFINITY; nc]; nr];
    d[gr][gc] = 0.0;
    let mut pq = BinaryHeap::new();
    pq.push(Node(0.0, gr, gc));
    let diag = hypot(step, step);
    let neigh: [(i64, i64, f64); 8] = [
        (-1, 0, step),
        (1, 0, step),
        (0, -1, step),
        (0, 1, step),
        (-1, -1, diag),
        (-1, 1, diag),
        (1, -1, diag),
        (1, 1, diag),
    ];
    while let Some(Node(dist, i, j)) = pq.pop() {
        if dist > d[i][j] + 1e-9 {
            continue;
        }
        for &(di, dj, sl) in &neigh {
            let (a, b) = (i as i64 + di, j as i64 + dj);
            if 0 <= a && a < nr as i64 && 0 <= b && b < nc as i64 {
                let (a, b) = (a as usize, b as usize);
                if cg[a][b].is_finite() {
                    let nd = dist + sl * 0.5 * (cg[i][j] + cg[a][b]);
                    if nd < d[a][b] - 1e-9 {
                        d[a][b] = nd;
                        pq.push(Node(nd, a, b));
                    }
                }
            }
        }
    }
    ReverseDijkstra {
        d,
        s,
        x0: grid.x0,
        y0: grid.y0,
        cell_m: grid.cell_m,
    }
}

// =====================================================================================
// The search
// =====================================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct LatticeResult {
    pub converged: bool,
    /// Forward-integrated, tangent-continuous.
    pub line: Option<Vec<Pt>>,
    /// The search objective (weights, not prices).
    pub cost: f64,
    /// `(R or None for a straight, sign, arc_len)` per primitive.
    pub prim_seq: Vec<(Option<f64>, i8, f64)>,
    pub start_pose: Pose,
    pub goal_pose: Pose,
    pub end_err_m: f64,
    pub expansions: usize,
    pub n_states: usize,
    pub seconds: f64,
    pub hit_cap: bool,
    pub timed_out: bool,
    /// `c_min_per_m` when the heuristic is `"c_min"`.
    pub c_min_per_m: Option<f64>,
    /// The line's minimum radius read on the chord (`min_radius_on_chord`, chord R_min/3); NaN
    /// when there is no line.
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    /// The chord reading holds R_min (within `RADIUS_TOL`); false when there is no line.
    pub radius_ok: bool,
}

type Key = (i64, i64, usize, i8);

struct Entry(f64, f64, Key);

impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Entry {}

impl Ord for Entry {
    /// Reversed `(f, g, key)`: the heap pops the smallest tuple, as Python's `heapq`.
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0)
            .unwrap_or(Ordering::Equal)
            .then(o.1.partial_cmp(&self.1).unwrap_or(Ordering::Equal))
            .then(o.2.cmp(&self.2))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn bin(theta: f64, n: i64) -> usize {
    (round_i(py_mod(theta, TAU) / TAU * n as f64) % n) as usize
}

/// The points along an arc (or straight) of `arc_len` from a pose, `n` steps.
fn arc_points(x0: f64, y0: f64, th0: f64, r: f64, sgn: i8, arc_len: f64, n: usize) -> Vec<Pose> {
    let mut out = Vec::with_capacity(n);
    if sgn == 0 || r.is_infinite() {
        for k in 1..=n {
            let s = arc_len * k as f64 / n as f64;
            out.push((x0 + s * libm::cos(th0), y0 + s * libm::sin(th0), th0));
        }
        return out;
    }
    let kap = f64::from(sgn) / r;
    let (ct, st) = (libm::cos(th0), libm::sin(th0));
    for k in 1..=n {
        let s = arc_len * k as f64 / n as f64;
        let lx = libm::sin(kap * s) / kap;
        let ly = (1.0 - libm::cos(kap * s)) / kap;
        out.push((
            x0 + ct * lx - st * ly,
            y0 + st * lx + ct * ly,
            th0 + kap * s,
        ));
    }
    out
}

enum Heuristic {
    CMin { c_min: f64, gx: f64, gy: f64 },
    Reverse(ReverseDijkstra),
    None,
}

impl Heuristic {
    fn h(&self, x: f64, y: f64) -> f64 {
        match self {
            Heuristic::CMin { c_min, gx, gy } => c_min * hypot(x - gx, y - gy),
            Heuristic::Reverse(r) => r.h(x, y),
            Heuristic::None => 0.0,
        }
    }
}

/// A* from `start_pose` to `goal_pose` over the lattice. See the module docs.
pub fn solve_lattice(
    grid: &TerrainGrid,
    start_pose: Pose,
    goal_pose: Pose,
    grade_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
) -> LatticeResult {
    let t0 = Instant::now();
    let prims = lattice_primitives(cfg);
    // One scorer per distinct half-width (rounded to 0.1 mm), as Python caches them.
    let hbs: Vec<f64> = prims
        .iter()
        .map(|p| section.half_width_at(p.r) + section.footprint_corr_m)
        .collect();
    let scorers: Vec<FieldScorer> = hbs
        .iter()
        .map(|&hb| FieldScorer::new(grid, grade_m, hb, section))
        .collect();
    let straight_scorer = &scorers[prims.len() - 1];

    let (gx0, gy0, gth) = goal_pose;
    let mut c_min_per_m = None;
    let heur = match cfg.heuristic.as_str() {
        "c_min" => {
            let c_min = min_cost_per_metre(
                grid,
                straight_scorer,
                cfg.n_heading,
                cfg.pitch_m,
                DEFAULT_C_MIN_STRIDE_M,
                DEFAULT_C_MIN_MAX_SAMPLES,
            );
            c_min_per_m = Some(c_min);
            Heuristic::CMin {
                c_min,
                gx: gx0,
                gy: gy0,
            }
        }
        "reverse_dijkstra" => Heuristic::Reverse(reverse_dijkstra_heuristic(
            grid,
            straight_scorer,
            [gx0, gy0],
            cfg.n_heading,
            cfg.pitch_m,
            cfg.pitch_m,
        )),
        _ => Heuristic::None,
    };
    let w = cfg.heuristic_weight;

    let bs = TAU / cfg.n_heading as f64;
    let head_tol = cfg.goal_head_tol_bins * bs;
    let goal_tol = cfg.goal_tol();

    let key = |x: f64, y: f64, th: f64, insign: i8| -> Key {
        (
            round_i(x / cfg.pitch_m),
            round_i(y / cfg.pitch_m),
            bin(th, cfg.n_heading),
            insign,
        )
    };
    let is_goal = |x: f64, y: f64, th: f64| -> bool {
        if hypot(x - gx0, y - gy0) > goal_tol {
            return false;
        }
        (py_mod(th - gth + core::f64::consts::PI, TAU) - core::f64::consts::PI).abs() <= head_tol
    };

    let (xmin, ymin, xmax, ymax) = grid.bounds();
    let m = cfg.bounds_margin_m;
    let sk = key(start_pose.0, start_pose.1, start_pose.2, 0);
    let mut g: HashMap<Key, f64> = HashMap::new();
    let mut pose: HashMap<Key, Pose> = HashMap::new();
    let mut prev: HashMap<Key, Option<Key>> = HashMap::new();
    let mut prev_prim: HashMap<Key, Option<usize>> = HashMap::new();
    g.insert(sk, 0.0);
    pose.insert(sk, start_pose);
    prev.insert(sk, None);
    prev_prim.insert(sk, None);
    let mut pq = BinaryHeap::new();
    pq.push(Entry(w * heur.h(start_pose.0, start_pose.1), 0.0, sk));
    let mut expanded: i64 = 0;
    let mut best: Option<Key> = None;
    let mut res = LatticeResult {
        converged: false,
        line: None,
        cost: f64::INFINITY,
        prim_seq: Vec::new(),
        start_pose,
        goal_pose,
        end_err_m: f64::INFINITY,
        expansions: 0,
        n_states: 0,
        seconds: 0.0,
        hit_cap: false,
        timed_out: false,
        c_min_per_m,
        measured_min_radius_m: f64::NAN,
        radius_check_chord_m: default_check_chord(cfg.r_min_m),
        radius_ok: false,
    };
    while let Some(Entry(_, gc, k)) = pq.pop() {
        if gc > g.get(&k).copied().unwrap_or(f64::INFINITY) + 1e-9 {
            continue;
        }
        let (x, y, th) = pose[&k];
        if is_goal(x, y, th) {
            best = Some(k);
            break;
        }
        expanded += 1;
        if expanded >= cfg.max_expand {
            res.hit_cap = true;
            break;
        }
        if let Some(cap) = cfg.wall_cap_s
            && expanded % 1000 == 0
            && t0.elapsed().as_secs_f64() > cap
        {
            res.timed_out = true;
            break;
        }
        let insign = k.3;
        let (ct, st) = (libm::cos(th), libm::sin(th));
        for (pi, p) in prims.iter().enumerate() {
            let sgn = p.sign;
            if insign != 0 && sgn != 0 && sgn == -insign {
                continue; // no arc straight into the opposite arc
            }
            let (ex, ey, eth) = p.end;
            let nx = x + ct * ex - st * ey;
            let ny = y + st * ex + ct * ey;
            if !(xmin - m <= nx && nx <= xmax + m && ymin - m <= ny && ny <= ymax + m) {
                continue;
            }
            let (c, ok) = scorers[pi].score(p, (x, y, th));
            if !ok {
                continue;
            }
            let ng = gc + c;
            let hn = heur.h(nx, ny);
            if ng + hn > cfg.prune_bound {
                continue;
            }
            let nk = key(nx, ny, th + eth, sgn);
            if ng < g.get(&nk).copied().unwrap_or(f64::INFINITY) - 1e-6 {
                g.insert(nk, ng);
                pose.insert(nk, (nx, ny, th + eth));
                prev.insert(nk, Some(k));
                prev_prim.insert(nk, Some(pi));
                pq.push(Entry(ng + w * hn, ng, nk));
            }
        }
    }

    res.expansions = expanded as usize;
    res.n_states = g.len();
    let Some(best) = best else {
        res.seconds = t0.elapsed().as_secs_f64();
        return res;
    };
    let mut chain = Vec::new();
    let mut cur = Some(best);
    while let Some(k) = cur {
        chain.push(k);
        cur = prev[&k];
    }
    chain.reverse();
    let seq: Vec<usize> = chain[1..]
        .iter()
        .map(|k| prev_prim[k].expect("a non-start state has a primitive"))
        .collect();
    // Forward-integrate the exact primitive sequence from the true start pose: tangent-continuous
    // by construction, and the geometry the costs were computed on.
    let (mut cx, mut cy, mut cth) = start_pose;
    let mut dense: Vec<Pt> = vec![[cx, cy]];
    for &pi in &seq {
        let p = &prims[pi];
        let pts = arc_points(cx, cy, cth, p.r, p.sign, p.arc_len, 10);
        dense.extend(pts.iter().map(|q| [q.0, q.1]));
        (cx, cy, cth) = pts[pts.len() - 1];
    }
    res.converged = true;
    res.cost = g[&best];
    if let Ok(m) = min_radius_on_chord(&dense, res.radius_check_chord_m) {
        res.measured_min_radius_m = m;
        res.radius_ok = radius_ok(m, cfg.r_min_m);
    }
    res.line = Some(dense);
    res.prim_seq = seq
        .iter()
        .map(|&pi| {
            let p = &prims[pi];
            (
                if p.r.is_infinite() { None } else { Some(p.r) },
                p.sign,
                p.arc_len,
            )
        })
        .collect();
    res.end_err_m = hypot(cx - gx0, cy - gy0);
    res.seconds = t0.elapsed().as_secs_f64();
    res
}

// =====================================================================================
// Front door: per pool, then the whole line
// =====================================================================================

/// Tangent bearing (rad) at chainage `s`.
pub fn heading_at(line: &[Pt], s: f64, arm_m: f64) -> f64 {
    let a = linear::interpolate(line, crate::py_max(0.0, s - arm_m)).unwrap_or([f64::NAN; 2]);
    let b = linear::interpolate(line, crate::py_min(linear::length(line), s + arm_m))
        .unwrap_or([f64::NAN; 2]);
    libm::atan2(b[1] - a[1], b[0] - a[0])
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoolAlignment {
    pub start_m: f64,
    pub end_m: f64,
    pub water_level_m: f64,
    pub grade_m: f64,
    pub result: LatticeResult,
    /// The baseline segment under the same scorer (straight width).
    pub baseline_cost: f64,
}

fn segment_cost(seg: &[Pt], grid: &TerrainGrid, grade: f64, section: &SectionSpec) -> f64 {
    let hb = section.half_width_at(f64::INFINITY) + section.footprint_corr_m;
    let sc = FieldScorer::new(grid, grade, hb, section);
    let mut total = 0.0;
    for w in seg.windows(2) {
        let ([x0, y0], [x1, y1]) = (w[0], w[1]);
        let l = hypot(x1 - x0, y1 - y0);
        if l <= 0.0 {
            continue;
        }
        let n = std::cmp::max(1, round_i(l / 2.0)) as usize;
        let prim = Prim {
            r: f64::INFINITY,
            sign: 0,
            pts: (1..=n)
                .map(|k| {
                    let v = l * k as f64 / n as f64;
                    [v, v, 0.0, 0.0]
                })
                .collect(),
            end: (l, 0.0, 0.0),
            arc_len: l,
        };
        let (c, ok) = sc.score(&prim, (x0, y0, libm::atan2(y1 - y0, x1 - x0)));
        total += if ok { c } else { 0.0 };
    }
    total
}

fn polygons_of(g: Geometry) -> Result<Vec<Polygon>, Refusal> {
    match g {
        Geometry::Polygon(p) => Ok(vec![p]),
        Geometry::MultiPolygon(ps) => Ok(ps),
        Geometry::Empty(_) => Ok(Vec::new()),
        other => Err(format!("a buffer came out as {other:?}")),
    }
}

fn line_of(g: Geometry) -> Result<Vec<Pt>, Refusal> {
    match g {
        Geometry::LineString(l) => Ok(l),
        Geometry::Point(p) => Ok(vec![p]),
        other => Err(format!("a substring came out as {other:?}")),
    }
}

/// The corridor, cell size and half-width of `optimise_pool`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolOptions {
    pub corridor_half_m: f64,
    pub cell_m: f64,
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            corridor_half_m: 350.0,
            cell_m: 2.0,
        }
    }
}

/// Least-cost R_min-feasible alignment for one pool, pinned to `baseline` at `start_m` and
/// `end_m` (position and heading). Grade = `water_level_m - depth_m`.
///
/// The corridor is `corridor_half_m` around the baseline segment, unioned with the same buffer
/// around `corridor_line` (e.g. the natural river) between the projections of the two end points,
/// when given.
#[allow(clippy::too_many_arguments)]
pub fn optimise_pool(
    baseline: &[Pt],
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    start_m: f64,
    end_m: f64,
    water_level_m: f64,
    depth_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
    corridor_line: Option<&[Pt]>,
    opts: PoolOptions,
) -> Result<PoolAlignment, Refusal> {
    let blen = linear::length(baseline);
    if !(0.0 <= start_m && start_m < end_m && end_m <= blen) {
        return Err("need 0 <= start_m < end_m <= baseline.length".into());
    }
    let seg = line_of(linear::substring(baseline, start_m, end_m))?;
    let geo = |e: crate::geom::GeomError| e.to_string();
    let mut corridor =
        polygons_of(buffer_line(&seg, opts.corridor_half_m, 16, Cap::Round).map_err(geo)?)?;
    if let Some(cl) = corridor_line {
        let a = linear::project(cl, seg[0]);
        let b = linear::project(cl, seg[seg.len() - 1]);
        let part = line_of(linear::substring(
            cl,
            crate::py_min(a, b),
            crate::py_max(a, b),
        ))?;
        let other =
            polygons_of(buffer_line(&part, opts.corridor_half_m, 16, Cap::Round).map_err(geo)?)?;
        corridor = overlay(&corridor, &other, Op::Union);
    }
    let grid = build_terrain_grid(terrain, river_polygon, &corridor, opts.cell_m, 20.0)?;
    let grade = water_level_m - depth_m;
    let p0 = linear::interpolate(baseline, start_m).unwrap_or([f64::NAN; 2]);
    let p1 = linear::interpolate(baseline, end_m).unwrap_or([f64::NAN; 2]);
    let start = (p0[0], p0[1], heading_at(baseline, start_m, 20.0));
    let goal = (p1[0], p1[1], heading_at(baseline, end_m, 20.0));
    let result = solve_lattice(&grid, start, goal, grade, section, cfg);
    Ok(PoolAlignment {
        start_m,
        end_m,
        water_level_m,
        grade_m: grade,
        result,
        baseline_cost: segment_cost(&seg, &grid, grade, section),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoolsResult {
    pub line: Vec<Pt>,
    pub pools: Vec<PoolAlignment>,
    /// The joined line's minimum radius read on the chord (R_min/3).
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    pub radius_ok: bool,
}

impl PoolsResult {
    pub fn n_replaced(&self) -> usize {
        self.pools.iter().filter(|p| p.result.converged).count()
    }
}

/// Run [`optimise_pool`] on each `(start_m, end_m, water_level_m)` and splice the results into
/// `baseline`. A pool keeps its baseline segment when the search does not converge, or
/// (`keep_if_dearer`) when the result costs more than the baseline under the same scorer.
///
/// Each spliced segment joins the baseline within `cfg.goal_pos_tol_m` at its far end; the join
/// is a straight step of at most that length.
#[allow(clippy::too_many_arguments)]
pub fn optimise_pools(
    baseline: &[Pt],
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    pools: &[(f64, f64, f64)],
    depth_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
    corridor_line: Option<&[Pt]>,
    opts: PoolOptions,
    keep_if_dearer: bool,
) -> Result<PoolsResult, Refusal> {
    let mut sorted = pools.to_vec();
    sorted.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .then(a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal))
    });
    let blen = linear::length(baseline);
    let mut out_pools = Vec::new();
    let mut pieces: Vec<Vec<Pt>> = Vec::new();
    let mut cursor = 0.0;
    for (a, b, wl) in sorted {
        let pa = optimise_pool(
            baseline,
            terrain,
            river_polygon,
            a,
            b,
            wl,
            depth_m,
            section,
            cfg,
            corridor_line,
            opts,
        )?;
        let converged = pa.result.converged;
        let dearer = pa.result.cost > pa.baseline_cost;
        let line = pa.result.line.clone();
        out_pools.push(pa);
        let use_it = converged && !(keep_if_dearer && dearer);
        if !use_it {
            continue;
        }
        if a > cursor {
            pieces.push(line_of(linear::substring(baseline, cursor, a))?);
        }
        pieces.push(line.unwrap_or_default());
        cursor = b;
    }
    if cursor < blen {
        pieces.push(line_of(linear::substring(baseline, cursor, blen))?);
    }
    let mut coords: Vec<Pt> = Vec::new();
    for pc in &pieces {
        for xy in pc {
            let far = coords
                .last()
                .is_none_or(|l| hypot(xy[0] - l[0], xy[1] - l[1]) > 1e-9);
            if far {
                coords.push(*xy);
            }
        }
    }
    let chord = default_check_chord(cfg.r_min_m);
    let measured = min_radius_on_chord(&coords, chord).unwrap_or(f64::NAN);
    Ok(PoolsResult {
        radius_ok: radius_ok(measured, cfg.r_min_m),
        measured_min_radius_m: measured,
        radius_check_chord_m: chord,
        line: coords,
        pools: out_pools,
    })
}
