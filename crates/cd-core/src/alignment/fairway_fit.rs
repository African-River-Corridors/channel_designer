//! The fairway fit (spec 0005 Q3, Q5, L1-L4, L6): a smooth design line inside navigable water
//! between fixed poses, never tighter than R_min.
//!
//! An elastic band, coarse to fine (point spacing 200, 100, 50, 25 and 10 m; L2). Each step
//! moves the free points sideways, along their normals, by the offsets that minimise bending
//! energy (the discrete fourth difference; no second-difference step, L6), plus a weak length
//! term and an optional one-sided preferred-radius penalty max(0, κ - 1/R_pref)², off by
//! default. The step is a small convex problem solved by ADMM (Boyd et al. 2011): a banded solve
//! for the offsets, then projections. The projections are the limits:
//! - each point stays inside the navigable cross-section through it;
//! - no step moves a point further than a cap (a trust region, L4);
//! - the bend at each point is clamped to 1/R_min, by projection, never by a soft cost.
//!
//! Bending energy is linearised about the current line; the next step re-linearises.
//!
//! A fixed pose is met by a Dubins piece (`dubins_paths`) of radius R_min from the seed line to
//! the pose, and held by a fixed ghost point one spacing beyond it along the heading, so the line
//! meets the pose tangentially (L1). Must-follow segments are copied exactly and the band works
//! only between them. Where no compliant line fits, the step ends on the line that breaks the
//! limits least, and the stretch is returned.

use super::PyError;
use super::navigable::{
    Chained, EDGE_TOL_M, Stretch, StretchReason, inside_outside_stretches, navigable_water,
    width_check,
};
use super::radius_check::{RADIUS_TOL, default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::grid::EdgeGrid;
use crate::geom::{Polygon, Pt, dist, linear};
use crate::rules::rulesets::ChannelShape;
use dubins_paths::f64::{DubinsPath, PosRot};

/// A position and a heading (radians, counter-clockwise from +x).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub at: Pt,
    pub heading_rad: f64,
}

impl Pose {
    fn dir(&self) -> Pt {
        [libm::cos(self.heading_rad), libm::sin(self.heading_rad)]
    }
}

/// The fixed pieces of a fit: a start pose, an end pose and must-follow segments (in order along
/// the line). All optional. Without a pose the line's end point is held, its heading is free.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fixed {
    pub start_pose: Option<Pose>,
    pub end_pose: Option<Pose>,
    pub must_follow: Vec<Vec<Pt>>,
}

/// One resolution of the band: point spacing (m) and number of steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitLevel {
    pub spacing_m: f64,
    pub steps: usize,
}

/// Settings of the fit. `FitParams::new(r_min)` gives the defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct FitParams {
    pub r_min: f64,
    /// The chord the reported radius is read on (default R_min/3).
    pub chord_m: f64,
    /// A preferred radius: bends tighter than it are penalised, softly. Off by default.
    pub r_pref_m: Option<f64>,
    /// Point spacing and number of steps of each level, coarse to fine.
    pub levels: Vec<FitLevel>,
    /// Weight of the length term relative to bending energy (weak).
    pub length_weight: f64,
    /// Weight of the preferred-radius penalty relative to bending energy.
    pub pref_weight: f64,
    /// The most one step moves a point sideways (m), as a multiple of the level's spacing (L4).
    pub max_jump_spacings: f64,
    /// How far inside the edge of navigable water the finest level keeps the line (m), so its
    /// chords stay off a tightly curved bank.
    pub contact_margin_m: f64,
    /// ADMM penalty and iterations of each step.
    pub admm_rho: f64,
    pub admm_iters: usize,
    /// A level whose line grows past this multiple of the seed's length is dropped.
    pub max_growth: f64,
    /// Station spacing of the final checks (m).
    pub check_step_m: f64,
    /// How far outside navigable water a station may be and still count as inside (m).
    pub inside_tol_m: f64,
}

impl FitParams {
    pub fn new(r_min: f64) -> Self {
        FitParams {
            r_min,
            chord_m: default_check_chord(r_min),
            r_pref_m: None,
            levels: [(200.0, 6), (100.0, 6), (50.0, 6), (25.0, 6), (10.0, 4)]
                .iter()
                .map(|&(spacing_m, steps)| FitLevel { spacing_m, steps })
                .collect(),
            length_weight: 1e-3,
            pref_weight: 10.0,
            max_jump_spacings: 1.0,
            contact_margin_m: 0.25,
            admm_rho: 1.0,
            admm_iters: 300,
            max_growth: 1.5,
            check_step_m: 10.0,
            inside_tol_m: 0.05,
        }
    }
}

/// The fitted line and what it achieved.
#[derive(Debug, Clone, PartialEq)]
pub struct FitResult {
    pub line: Vec<Pt>,
    /// Stretches where no compliant line fits: outside navigable water (`too_narrow`) or tighter
    /// than R_min (`r_min_not_possible`). Empty when the whole line complies.
    pub infeasible: Vec<Stretch>,
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    pub radius_ok: bool,
    /// Heading error (radians, absolute) at the start and end poses, when given.
    pub start_heading_error_rad: Option<f64>,
    pub end_heading_error_rad: Option<f64>,
    /// Plain-English notes (a level dropped for growth, a Dubins piece that could not be built).
    pub notes: Vec<String>,
}

/// The heading of a direction vector.
fn heading(d: Pt) -> f64 {
    libm::atan2(d[1], d[0])
}

/// The absolute difference between two headings, in [0, π].
pub fn heading_error(a: f64, b: f64) -> f64 {
    let d = libm::remainder(a - b, 2.0 * core::f64::consts::PI);
    d.abs()
}

/// Fit `line` inside `navigable` between the fixed pieces. See the module notes.
pub fn fairway_fit(
    line: &[Pt],
    navigable: &[Polygon],
    fixed: &Fixed,
    params: &FitParams,
) -> Result<FitResult, PyError> {
    if !(params.r_min.is_finite() && params.r_min > 0.0) {
        return Err(PyError::value("r_min must be finite and positive."));
    }
    if line.len() < 2 || linear::length(line) <= 0.0 {
        return Err(PyError::value(
            "the line needs at least two distinct points.",
        ));
    }
    if params.levels.is_empty() {
        return Err(PyError::value("the fit needs at least one level."));
    }
    let grid = EdgeGrid::new(navigable);
    let seed = Chained::new(line);
    let mut notes = Vec::new();

    // Free pieces between must-follow segments, each with the pose at either end.
    let mut out: Vec<Pt> = Vec::new();
    let mut from_s = 0.0;
    let mut from_pt = line[0];
    let mut from_pose = fixed.start_pose;
    if let Some(p) = from_pose {
        from_pt = p.at;
    }
    for mf in &fixed.must_follow {
        if mf.len() < 2 {
            return Err(PyError::value(
                "a must-follow segment needs at least two points.",
            ));
        }
        let to_s = linear::project(line, mf[0]);
        let n = mf.len();
        let to_pose = Pose {
            at: mf[0],
            heading_rad: heading([mf[1][0] - mf[0][0], mf[1][1] - mf[0][1]]),
        };
        let piece = piece_seed(&seed, from_s, to_s, from_pt, mf[0]);
        let fitted = fit_piece(&piece, from_pose, Some(to_pose), &grid, params, &mut notes);
        append(&mut out, &fitted);
        append(&mut out, mf);
        from_s = linear::project(line, mf[n - 1]);
        from_pt = mf[n - 1];
        from_pose = Some(Pose {
            at: mf[n - 1],
            heading_rad: heading([mf[n - 1][0] - mf[n - 2][0], mf[n - 1][1] - mf[n - 2][1]]),
        });
    }
    let end_pt = fixed.end_pose.map_or(line[line.len() - 1], |p| p.at);
    // After a must-follow segment that runs to (or past) the seed's end, there is nothing left
    // to fit unless an end pose asks for more.
    let tail_left =
        out.is_empty() || from_s < seed.length() - EDGE_TOL_M || fixed.end_pose.is_some();
    if tail_left && from_pt != end_pt {
        let piece = piece_seed(&seed, from_s, seed.length(), from_pt, end_pt);
        let fitted = fit_piece(&piece, from_pose, fixed.end_pose, &grid, params, &mut notes);
        append(&mut out, &fitted);
    }

    let infeasible = check_line(&out, &grid, params);
    let measured = min_radius_on_chord(&out, params.chord_m)?;
    let head = |i: usize, j: usize| heading([out[j][0] - out[i][0], out[j][1] - out[i][1]]);
    let n = out.len();
    Ok(FitResult {
        start_heading_error_rad: fixed
            .start_pose
            .map(|p| heading_error(head(0, 1), p.heading_rad)),
        end_heading_error_rad: fixed
            .end_pose
            .map(|p| heading_error(head(n - 2, n - 1), p.heading_rad)),
        line: out,
        infeasible,
        measured_min_radius_m: measured,
        radius_check_chord_m: params.chord_m,
        radius_ok: radius_ok(measured, params.r_min),
        notes,
    })
}

/// The fairway fit from the water itself: builds navigable water (half the straight bottom width
/// plus `clearance_m`, smoothed by `smooth_m`), fits the line, and adds the per-station width
/// check of the result against the water (the bend-widened width at each station's local
/// radius, R2-Q3). Overlapping stretches merge; `too_narrow` wins over `r_min_not_possible`.
#[allow(clippy::too_many_arguments)]
pub fn fairway_fit_in_water(
    line: &[Pt],
    water: &[Polygon],
    shape: &ChannelShape,
    clearance_m: f64,
    smooth_m: f64,
    fixed: &Fixed,
    params: &FitParams,
) -> Result<FitResult, PyError> {
    let nav = navigable_water(
        water,
        shape.bottom_width_straight_m / 2.0,
        clearance_m,
        smooth_m,
    )?;
    let mut r = fairway_fit(line, &nav, fixed, params)?;
    let wide = width_check(&r.line, water, shape, params.r_min, params.chord_m)?;
    r.infeasible = merge_stretches(&r.line, r.infeasible.into_iter().chain(wide).collect());
    Ok(r)
}

/// Sort stretches by start and merge those that overlap; `too_narrow` wins.
pub fn merge_stretches(line: &[Pt], mut st: Vec<Stretch>) -> Vec<Stretch> {
    st.sort_by(|a, b| a.start_chainage_m.total_cmp(&b.start_chainage_m));
    let c = Chained::new(line);
    let mut out: Vec<Stretch> = Vec::new();
    for s in st {
        if let Some(last) = out.last_mut()
            && s.start_chainage_m <= last.end_chainage_m
        {
            if s.end_chainage_m > last.end_chainage_m {
                last.end_chainage_m = s.end_chainage_m;
                last.end = c.at(s.end_chainage_m);
            }
            if s.reason == StretchReason::TooNarrow {
                last.reason = StretchReason::TooNarrow;
            }
            continue;
        }
        out.push(s);
    }
    out
}

/// The fit's promises, checked on a result: outside its infeasible stretches the line lies inside
/// navigable water (stations every `step_m`), and, when no stretch is `r_min_not_possible`, the
/// radius read on the chord is at least (1 - RADIUS_TOL) R_min. `Err` says which failed.
pub fn check_fit_properties(
    r: &FitResult,
    navigable: &[Polygon],
    r_min: f64,
    chord_m: f64,
    step_m: f64,
) -> Result<(), String> {
    if !inside_outside_stretches(&r.line, navigable, &r.infeasible, step_m) {
        return Err("the line leaves navigable water outside its infeasible stretches".into());
    }
    let tight = r
        .infeasible
        .iter()
        .any(|s| s.reason == StretchReason::RMinNotPossible);
    let measured = min_radius_on_chord(&r.line, chord_m).map_err(|e| e.message)?;
    if !tight && !radius_ok(measured, r_min) {
        return Err(format!(
            "the radius read on a {chord_m} m chord is {measured:.1} m, under R_min {r_min} m"
        ));
    }
    Ok(())
}

/// Append `pts` to `out`, dropping a point within 1e-6 m of the last one there (the later one,
/// a fixed point, wins).
fn append(out: &mut Vec<Pt>, pts: &[Pt]) {
    for &p in pts {
        if let Some(l) = out.last_mut()
            && dist(*l, p) < 1e-6
        {
            *l = p;
            continue;
        }
        out.push(p);
    }
}

/// The seed of one free piece: the seed line from chainage `a` to `b`, its ends moved to the
/// fixed points. A piece whose chainages run backwards is the straight segment between them.
fn piece_seed(seed: &Chained, a: f64, b: f64, start: Pt, end: Pt) -> Vec<Pt> {
    let mut pts = if b > a {
        seed.slice(a, b)
    } else {
        vec![start, end]
    };
    pts[0] = start;
    let n = pts.len();
    pts[n - 1] = end;
    pts.dedup();
    if pts.len() < 2 {
        pts = vec![start, end];
    }
    pts
}

/// A Dubins piece of radius `rho` from `q0` to `q1`, sampled every `step` metres (ends included).
fn dubins(q0: Pose, q1: Pose, rho: f64, step: f64) -> Option<Vec<Pt>> {
    let a = PosRot::from_floats(q0.at[0], q0.at[1], q0.heading_rad);
    let b = PosRot::from_floats(q1.at[0], q1.at[1], q1.heading_rad);
    let path = DubinsPath::shortest_from(a, b, rho).ok()?;
    let mut pts: Vec<Pt> = path
        .sample_many(step)
        .into_iter()
        .map(|q| [q.x(), q.y()])
        .collect();
    if pts.last().is_some_and(|l| dist(*l, q1.at) < 1e-6) {
        pts.pop();
    }
    pts.push(q1.at);
    Some(pts)
}

/// The initial line of a free piece: the seed with a Dubins piece at each posed end, from the
/// seed's own pose a blend length in from that end. The blend length is the shorter of 2 R_min
/// and a third of the piece.
fn with_dubins_ends(
    piece: &[Pt],
    start: Option<Pose>,
    end: Option<Pose>,
    params: &FitParams,
    notes: &mut Vec<String>,
) -> Vec<Pt> {
    let c = Chained::new(piece);
    let len = c.length();
    let blend = (2.0 * params.r_min).min(len / 3.0);
    let step = 5.0_f64.min(blend / 4.0).max(0.5);
    let (mut a, mut b) = (0.0, len);
    let mut head: Vec<Pt> = vec![];
    let mut tail: Vec<Pt> = vec![];
    if let Some(p) = start {
        let q = Pose {
            at: c.at(blend),
            heading_rad: heading(c.direction(blend, step)),
        };
        match dubins(p, q, params.r_min, step) {
            Some(d) => {
                head = d;
                a = blend;
            }
            None => notes.push("no Dubins piece to the start pose; the band alone meets it".into()),
        }
    }
    if let Some(p) = end {
        let q = Pose {
            at: c.at(len - blend),
            heading_rad: heading(c.direction(len - blend, step)),
        };
        match dubins(q, p, params.r_min, step) {
            Some(d) => {
                tail = d;
                b = len - blend;
            }
            None => notes.push("no Dubins piece to the end pose; the band alone meets it".into()),
        }
    }
    let mut out = Vec::new();
    append(&mut out, &head);
    append(&mut out, &c.slice(a, b));
    append(&mut out, &tail);
    if let Some(p) = start {
        out[0] = p.at;
    }
    if let Some(p) = end {
        let n = out.len();
        out[n - 1] = p.at;
    }
    out
}

/// The band's points: an optional fixed ghost, the fixed first point, free points, the fixed last
/// point, an optional fixed ghost. `lo..=hi` are the free points.
struct Band {
    p: Vec<Pt>,
    lo: usize,
    hi: usize,
}

impl Band {
    fn new(line: &[Pt], start: Option<Pose>, end: Option<Pose>, spacing: f64) -> Band {
        let mut p = Vec::with_capacity(line.len() + 2);
        if let Some(s) = start {
            let d = s.dir();
            p.push([line[0][0] - spacing * d[0], line[0][1] - spacing * d[1]]);
        }
        p.extend_from_slice(line);
        if let Some(e) = end {
            let d = e.dir();
            let l = line[line.len() - 1];
            p.push([l[0] + spacing * d[0], l[1] + spacing * d[1]]);
        }
        let lo = usize::from(start.is_some()) + 1;
        let hi = p.len() - usize::from(end.is_some()) - 2;
        Band { p, lo, hi }
    }

    fn line(&self, start: Option<Pose>, end: Option<Pose>) -> Vec<Pt> {
        let a = usize::from(start.is_some());
        let b = self.p.len() - usize::from(end.is_some());
        self.p[a..b].to_vec()
    }

    /// The unit left normal at point `i`, from its neighbours' chord.
    fn normal(&self, i: usize) -> Pt {
        let (a, c) = (self.p[i - 1], self.p[i + 1]);
        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
        let l = (dx * dx + dy * dy).sqrt();
        if l > 0.0 {
            [-dy / l, dx / l]
        } else {
            [0.0, 1.0]
        }
    }
}

/// A symmetric band matrix with half-bandwidth 2: `a[i][k]` = A[i][i + k - 2].
type Band5 = Vec<[f64; 5]>;

/// Solve `A x = r` for a symmetric positive-definite band matrix (Gaussian elimination without
/// pivoting).
fn solve_band(mut a: Band5, mut r: Vec<f64>) -> Vec<f64> {
    let m = r.len();
    for i in 0..m {
        let piv = a[i][2];
        for j in i + 1..(i + 3).min(m) {
            let f = a[j][i + 2 - j] / piv;
            if f == 0.0 {
                continue;
            }
            for col in i..(i + 3).min(m) {
                a[j][col + 2 - j] -= f * a[i][col + 2 - i];
            }
            r[j] -= f * r[i];
        }
    }
    let mut x = vec![0.0; m];
    for i in (0..m).rev() {
        let mut s = r[i];
        for j in i + 1..(i + 3).min(m) {
            s -= a[i][j + 2 - i] * x[j];
        }
        x[i] = s / a[i][2];
    }
    x
}

/// One sideways step of the band, as a small convex problem in the offsets `u` of the free
/// points along their normals (a trust region of `cap`):
///
/// minimise Σ q² + w Σ (Δu)² + β Σ max(0, |q| - q_pref)²
/// subject to |u| <= cap, u inside the navigable cross-section at each point, |q| <= q_max,
///
/// where `q = (Δ²p)·n + Δ²u` is the bend at each point in length units (κh² on equal spacing),
/// so the first sum is the bending energy, linearised about the current line. Solved by ADMM:
/// a banded solve for `u`, then projections (clipping) onto the water and curvature limits, the
/// bending energy and the preferred-radius penalty taken in closed form. Where the limits leave
/// nothing, ADMM ends on the line that breaks them least.
// Several arrays are indexed in step, by free point and by bend row.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn offset_step(
    b: &mut Band,
    grid: &EdgeGrid,
    cap: f64,
    margin: f64,
    q_max: f64,
    q_pref: Option<f64>,
    params: &FitParams,
) {
    let (lo, hi) = (b.lo, b.hi);
    if hi < lo {
        return;
    }
    let n = b.p.len();
    let m = hi - lo + 1;
    let normals: Vec<Pt> = (0..n)
        .map(|i| {
            if i == 0 || i == n - 1 {
                [0.0; 2]
            } else {
                b.normal(i)
            }
        })
        .collect();
    // Bend rows: every interior point whose stencil holds a free point.
    let rows: Vec<usize> = (1..n - 1).filter(|&i| i + 1 >= lo && i <= hi + 1).collect();
    let q0: Vec<f64> = rows
        .iter()
        .map(|&i| {
            let (a, p, c) = (b.p[i - 1], b.p[i], b.p[i + 1]);
            let d2 = [a[0] - 2.0 * p[0] + c[0], a[1] - 2.0 * p[1] + c[1]];
            d2[0] * normals[i][0] + d2[1] * normals[i][1]
        })
        .collect();
    // Box of each free offset: the trust region and the navigable cross-section.
    let boxes: Vec<(f64, f64)> = (lo..=hi)
        .map(|i| bank_interval(b.p[i], normals[i], grid, cap, margin))
        .collect();
    // D: rows x free columns, stencil (1, -2, 1) on the free points only.
    let stencil = |r: usize, j: usize| -> f64 {
        let i = rows[r];
        let col = lo + j;
        if col + 1 == i || col == i + 1 {
            1.0
        } else if col == i {
            -2.0
        } else {
            0.0
        }
    };
    let rho = params.admm_rho;
    let w = params.length_weight;
    // M = ρ(I + DᵀD) + w L over the free points.
    let mut mat: Band5 = vec![[0.0; 5]; m];
    for row in mat.iter_mut() {
        row[2] += rho;
    }
    for r in 0..rows.len() {
        let i = rows[r];
        let cols: Vec<usize> = (i.saturating_sub(1)..=i + 1)
            .filter(|&c| c >= lo && c <= hi)
            .map(|c| c - lo)
            .collect();
        for &a in &cols {
            for &c in &cols {
                mat[a][c + 2 - a] += rho * stencil(r, a) * stencil(r, c);
            }
        }
    }
    for i in lo.saturating_sub(1)..=hi {
        // Tension between i and i + 1 (fixed neighbours hold u = 0).
        let (a, c) = (i, i + 1);
        let fa = a >= lo && a <= hi;
        let fc = c >= lo && c <= hi;
        if fa {
            mat[a - lo][2] += w;
        }
        if fc {
            mat[c - lo][2] += w;
        }
        if fa && fc {
            mat[a - lo][3] -= w;
            mat[c - lo][1] -= w;
        }
    }
    let du = |u: &[f64], r: usize| -> f64 {
        let i = rows[r];
        let mut s = 0.0;
        for col in i - 1..=i + 1 {
            if col >= lo && col <= hi {
                s += stencil(r, col - lo) * u[col - lo];
            }
        }
        s
    };
    let alpha = 1.0; // weight of the bending energy in the q block
    let beta = params.pref_weight;
    let (mut z1, mut y1) = (vec![0.0; m], vec![0.0; m]);
    let mut z2: Vec<f64> = q0.clone();
    let mut y2 = vec![0.0; rows.len()];
    for _ in 0..params.admm_iters {
        // u-update: M u = ρ(z1 - y1) + ρ Dᵀ(z2 - y2 - q0).
        let mut rhs: Vec<f64> = (0..m).map(|j| rho * (z1[j] - y1[j])).collect();
        for r in 0..rows.len() {
            let v = rho * (z2[r] - y2[r] - q0[r]);
            let i = rows[r];
            for col in i - 1..=i + 1 {
                if col >= lo && col <= hi {
                    rhs[col - lo] += stencil(r, col - lo) * v;
                }
            }
        }
        let u = solve_band(mat.clone(), rhs);
        // z1: clip to the box.
        for j in 0..m {
            let v = u[j] + y1[j];
            z1[j] = v.clamp(boxes[j].0, boxes[j].1);
            y1[j] = v - z1[j];
        }
        // z2: bending energy (and the preferred-radius penalty) in closed form, then the
        // curvature limit.
        for r in 0..rows.len() {
            let v = q0[r] + du(&u, r) + y2[r];
            let mut z = rho * v / (alpha + rho);
            if let Some(qp) = q_pref
                && z.abs() > qp
            {
                z = (rho * v + beta * qp * v.signum()) / (alpha + beta + rho);
                if z.abs() < qp {
                    z = qp * v.signum();
                }
            }
            z2[r] = z.clamp(-q_max, q_max);
            y2[r] = v - z2[r];
        }
    }
    // The z1 projection is the point actually used: inside the box, the bank and the cap.
    for j in 0..m {
        let i = lo + j;
        let s = z1[j];
        b.p[i] = [b.p[i][0] + s * normals[i][0], b.p[i][1] + s * normals[i][1]];
    }
}

/// The offsets `(lo, hi)` along `n` from `p` that keep `p` inside navigable water, within
/// `±cap`, each `margin` inside the edge. A point inside takes the stretch of water it is in; a
/// point outside the nearest stretch within reach; with none in reach, just `±cap`.
fn bank_interval(p: Pt, n: Pt, grid: &EdgeGrid, cap: f64, margin: f64) -> (f64, f64) {
    let q0 = [p[0] - cap * n[0], p[1] - cap * n[1]];
    let q1 = [p[0] + cap * n[0], p[1] + cap * n[1]];
    let mut ts: Vec<f64> = grid
        .hits(q0, q1)
        .into_iter()
        .map(|t| (t - 0.5) * 2.0 * cap)
        .collect();
    ts.sort_by(f64::total_cmp);
    ts.dedup();
    // Crossing points split [-cap, cap] into spans; test each span's middle.
    let mut cuts = vec![-cap];
    cuts.extend(ts.iter().copied().filter(|t| t.abs() < cap));
    cuts.push(cap);
    let mut best: Option<(f64, f64, f64)> = None; // (distance from 0, lo, hi)
    for w in cuts.windows(2) {
        let (a, c) = (w[0], w[1]);
        let mid = (a + c) / 2.0;
        if !grid.contains([p[0] + mid * n[0], p[1] + mid * n[1]]) {
            continue;
        }
        let (a2, c2) = if c - a > 2.0 * margin {
            (
                a + if a > -cap { margin } else { 0.0 },
                c - if c < cap { margin } else { 0.0 },
            )
        } else {
            (mid, mid)
        };
        let d = if a2 <= 0.0 && 0.0 <= c2 {
            0.0
        } else {
            a2.abs().min(c2.abs())
        };
        if best.is_none_or(|(bd, _, _)| d < bd) {
            best = Some((d, a2, c2));
        }
    }
    best.map_or((-cap, cap), |(_, a, c)| (a, c))
}

/// Fit one free piece between its (optional) poses.
fn fit_piece(
    piece: &[Pt],
    start: Option<Pose>,
    end: Option<Pose>,
    grid: &EdgeGrid,
    params: &FitParams,
    notes: &mut Vec<String>,
) -> Vec<Pt> {
    let seed_len = linear::length(piece);
    if seed_len <= 0.0 {
        return piece.to_vec();
    }
    let mut line = with_dubins_ends(piece, start, end, params, notes);
    // The limit aims a little above R_min so the chord reading clears it.
    let r_clamp = params.r_min * (1.0 + RADIUS_TOL / 2.0);
    let last = params.levels.len() - 1;
    for (li, level) in params.levels.iter().enumerate() {
        let h = level.spacing_m;
        if (seed_len / h).ceil() < 3.0 && li < last {
            continue;
        }
        let before = line.clone();
        let cap = params.max_jump_spacings * h;
        let margin = if li == last {
            params.contact_margin_m
        } else {
            0.0
        };
        let q_max = h * h / r_clamp;
        let q_pref = params.r_pref_m.map(|r| h * h / r);
        for _ in 0..level.steps {
            let mut b = Band::new(&Chained::new(&line).resample(h, 3), start, end, h);
            offset_step(&mut b, grid, cap, margin, q_max, q_pref, params);
            line = b.line(start, end);
        }
        if linear::length(&line) > params.max_growth * seed_len {
            notes.push(format!(
                "the {h} m level grew the line past {} times its seed length; it was dropped",
                params.max_growth
            ));
            line = before;
        }
    }
    line
}

/// The stretches of a fitted line outside navigable water or tighter than R_min, at stations
/// every `check_step_m`, ends found to `EDGE_TOL_M`.
fn check_line(line: &[Pt], grid: &EdgeGrid, params: &FitParams) -> Vec<Stretch> {
    let c = Chained::new(line);
    let len = c.length();
    let status = |s: f64| -> Option<StretchReason> {
        if !grid.covers(c.at(s), params.inside_tol_m) {
            Some(StretchReason::TooNarrow)
        } else if !radius_ok(c.local_radius(s, params.chord_m), params.r_min) {
            Some(StretchReason::RMinNotPossible)
        } else {
            None
        }
    };
    stretches(&c, len, params.check_step_m, &status)
}

/// Runs of stations with the same failing status, as stretches with refined ends.
pub(crate) fn stretches(
    c: &Chained,
    len: f64,
    step: f64,
    status: &dyn Fn(f64) -> Option<StretchReason>,
) -> Vec<Stretch> {
    let n = (len / step).ceil() as usize;
    let edge = |mut a: f64, mut b: f64| {
        let sa = status(a);
        while b - a > EDGE_TOL_M {
            let m = (a + b) / 2.0;
            if status(m) == sa { a = m } else { b = m }
        }
        (a + b) / 2.0
    };
    let mk = |a: f64, b: f64, reason| Stretch {
        start_chainage_m: a,
        end_chainage_m: b,
        start: c.at(a),
        end: c.at(b),
        reason,
    };
    let mut out = Vec::new();
    let mut open: Option<(f64, StretchReason)> = None;
    let mut prev: Option<(f64, Option<StretchReason>)> = None;
    for i in 0..=n {
        let s = (i as f64 * step).min(len);
        let st = status(s);
        match prev {
            None => {
                if let Some(r) = st {
                    open = Some((0.0, r));
                }
            }
            Some((ps, pst)) if pst != st => {
                let at = edge(ps, s);
                if let Some((a, r)) = open.take() {
                    out.push(mk(a, at, r));
                }
                if let Some(r) = st {
                    open = Some((at, r));
                }
            }
            _ => {}
        }
        prev = Some((s, st));
    }
    if let Some((a, r)) = open {
        out.push(mk(a, len, r));
    }
    out
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (fixture answers in `synthetic::fixtures`; navigable
    //! water of the narrowing reach derived in `narrowing_reach_navigable_ends`)
    use super::*;
    use crate::alignment::navigable::navigable_water;
    use crate::synthetic::{fixed_tail, narrowing_reach, straight_canal};

    const R_MIN: f64 = 255.0;
    const HALF: f64 = 15.2;

    fn nav(f: &crate::synthetic::WaterFixture) -> Vec<Polygon> {
        navigable_water(core::slice::from_ref(&f.water), HALF, 0.0, 0.0).unwrap()
    }

    #[test]
    fn straight_canal_fits_to_the_straight_line() {
        let f = straight_canal(3000.0, 100.0, 25.0, 11);
        let r = fairway_fit(
            &f.legs[0],
            &nav(&f),
            &Fixed::default(),
            &FitParams::new(R_MIN),
        )
        .unwrap();
        let off = r.line.iter().map(|p| p[1].abs()).fold(0.0, f64::max);
        assert!(off < 0.5, "max offset from y = 0: {off}");
        assert!(r.infeasible.is_empty(), "{:?}", r.infeasible);
        assert!(r.radius_ok);
    }

    /// The narrowing reach's navigable water ends where the half-width, less the shrink measured
    /// at right angles to the sloping bank (`b·√(1 + 0.2²)`), reaches zero: half-width
    /// `30 - 0.2 (x - 900)` on the way in, so `x = 900 + (30 - b√1.04) / 0.2`, and symmetric.
    fn narrowing_reach_navigable_ends() -> (f64, f64) {
        let a = 900.0 + (30.0 - HALF * 1.04_f64.sqrt()) / 0.2;
        (a, 2500.0 - a)
    }

    #[test]
    fn narrowing_reach_reports_exactly_the_narrow_stretch() {
        let f = narrowing_reach(60.0, 20.0);
        let n = nav(&f);
        let r = fairway_fit(&f.legs[0], &n, &Fixed::default(), &FitParams::new(R_MIN)).unwrap();
        let (a, b) = narrowing_reach_navigable_ends();
        assert_eq!(r.infeasible.len(), 1, "{:?}", r.infeasible);
        let st = &r.infeasible[0];
        assert_eq!(st.reason, StretchReason::TooNarrow);
        assert!((st.start[0] - a).abs() < 0.1, "{st:?} vs {a}");
        assert!((st.end[0] - b).abs() < 0.1, "{st:?} vs {b}");
        assert!(super::super::navigable::inside_outside_stretches(
            &r.line,
            &n,
            &r.infeasible,
            5.0
        ));
    }

    #[test]
    fn fixed_tail_meets_the_segment_tangentially_and_leaves_it_unchanged() {
        let f = fixed_tail();
        let fixed = Fixed {
            must_follow: f.fixed.clone(),
            ..Fixed::default()
        };
        let r = fairway_fit(&f.legs[0], &nav(&f), &fixed, &FitParams::new(R_MIN)).unwrap();
        let tail = &f.fixed[0];
        let n = r.line.len();
        assert_eq!(&r.line[n - 2..], &tail[..], "fixed segment changed");
        // Heading of the fitted line arriving at the segment's start.
        let k = r.line.iter().position(|p| *p == tail[0]).unwrap();
        let arrive = heading([
            r.line[k][0] - r.line[k - 1][0],
            r.line[k][1] - r.line[k - 1][1],
        ]);
        let err = heading_error(arrive, 0.0);
        assert!(
            err < 1f64.to_radians(),
            "heading error {} deg",
            err.to_degrees()
        );
        assert!(r.infeasible.is_empty(), "{:?}", r.infeasible);
        assert!(r.radius_ok, "{}", r.measured_min_radius_m);
    }
}

#[cfg(test)]
mod property_tests {
    //! Label: property
    use super::*;
    use crate::synthetic::fixtures::SplitMix;
    use crate::synthetic::{
        RiverParams, WaterFixture, fixed_tail, narrowing_reach, straight_canal, synthetic_river,
    };

    const R_MIN: f64 = 255.0;
    const CHORD: f64 = 85.0;

    fn nav(f: &WaterFixture) -> Vec<Polygon> {
        navigable_water(core::slice::from_ref(&f.water), 15.2, 0.0, 0.0).unwrap()
    }

    fn fit(f: &WaterFixture, fixed: &Fixed) -> (FitResult, Vec<Polygon>) {
        let n = nav(f);
        let r = fairway_fit(&f.legs[0], &n, fixed, &FitParams::new(R_MIN)).unwrap();
        (r, n)
    }

    /// Random turns and moves of a fixture, from a seed.
    fn variants(seed: u64, f: &WaterFixture, count: usize) -> Vec<WaterFixture> {
        let mut rng = SplitMix::new(seed);
        (0..count)
            .map(|_| {
                let a = core::f64::consts::PI * rng.signed_unit();
                let off = [5e5 * rng.signed_unit(), 5e6 + 1e5 * rng.signed_unit()];
                f.transformed(a, off)
            })
            .collect()
    }

    #[test]
    fn canals_from_random_seeds_fit_inside_navigable_water_at_r_min() {
        for seed in 0..12u64 {
            let base = straight_canal(2500.0, 120.0, 30.0, seed);
            for f in variants(seed, &base, 2) {
                let (r, n) = fit(&f, &Fixed::default());
                assert!(r.infeasible.is_empty(), "seed {seed}: {:?}", r.infeasible);
                check_fit_properties(&r, &n, R_MIN, CHORD, 5.0).unwrap();
                let grow = linear::length(&r.line) / linear::length(&f.legs[0]);
                assert!(
                    grow <= FitParams::new(R_MIN).max_growth,
                    "seed {seed}: {grow}"
                );
            }
        }
    }

    #[test]
    fn narrowing_reaches_from_random_seeds_report_only_the_narrow_stretch() {
        let base = narrowing_reach(60.0, 20.0);
        for (k, f) in variants(42, &base, 8).into_iter().enumerate() {
            let (r, n) = fit(&f, &Fixed::default());
            assert_eq!(r.infeasible.len(), 1, "variant {k}: {:?}", r.infeasible);
            let len = r.infeasible[0].end_chainage_m - r.infeasible[0].start_chainage_m;
            // 2500 - 2 x 972.49 = 555.0 m (see the known-answer test).
            assert!((len - 555.0).abs() < 0.3, "variant {k}: {len}");
            check_fit_properties(&r, &n, R_MIN, CHORD, 5.0).unwrap();
        }
    }

    #[test]
    fn fixed_tails_from_random_seeds_merge_tangentially_and_keep_the_segment() {
        let base = fixed_tail();
        for (k, f) in variants(7, &base, 8).into_iter().enumerate() {
            let fixed = Fixed {
                must_follow: f.fixed.clone(),
                ..Fixed::default()
            };
            let (r, n) = fit(&f, &fixed);
            let tail = &f.fixed[0];
            let m = r.line.len();
            assert_eq!(&r.line[m - 2..], &tail[..], "variant {k}");
            let want = heading([tail[1][0] - tail[0][0], tail[1][1] - tail[0][1]]);
            let got = heading([
                r.line[m - 2][0] - r.line[m - 3][0],
                r.line[m - 2][1] - r.line[m - 3][1],
            ]);
            assert!(heading_error(got, want) < 1f64.to_radians(), "variant {k}");
            assert!(r.infeasible.is_empty(), "variant {k}: {:?}", r.infeasible);
            check_fit_properties(&r, &n, R_MIN, CHORD, 5.0).unwrap();
        }
    }

    #[test]
    fn each_property_check_fails_on_a_corrupted_result() {
        let f = straight_canal(2500.0, 120.0, 30.0, 3);
        let (r, n) = fit(&f, &Fixed::default());
        check_fit_properties(&r, &n, R_MIN, CHORD, 5.0).unwrap();
        // A point pushed onto land.
        let mut land = r.clone();
        let i = land.line.len() / 2;
        land.line[i][1] += 200.0;
        let e = check_fit_properties(&land, &n, R_MIN, CHORD, 5.0).unwrap_err();
        assert!(e.contains("leaves navigable water"), "{e}");
        // A corner left in, inside the water: a 20 m dog-leg.
        let mut corner = r.clone();
        corner.line = vec![[0.0, 0.0], [1200.0, 0.0], [1240.0, 20.0], [2500.0, 20.0]];
        let corner_nav = nav(&f.transformed(0.0, [0.0, 0.0]));
        let e = check_fit_properties(&corner, &corner_nav, R_MIN, CHORD, 5.0).unwrap_err();
        assert!(e.contains("under R_min"), "{e}");
    }

    #[test]
    fn the_synthetic_river_fits_at_300_m_and_is_too_narrow_at_every_apex_at_40_m() {
        let shape = crate::rules::rulesets::channel_shape(
            &crate::rules::rulesets::RulesetRegistry::new(),
            "PIANC-B",
            "CEMT_IV",
            Some(2.5),
        )
        .unwrap();
        let p = FitParams::new(R_MIN);
        let wide = synthetic_river(RiverParams {
            width_m: 300.0,
            ..RiverParams::default()
        })
        .unwrap();
        let w = core::slice::from_ref(&wide.polygon);
        let r = fairway_fit_in_water(
            &wide.centreline,
            w,
            &shape,
            0.0,
            25.0,
            &Fixed::default(),
            &p,
        )
        .unwrap();
        assert!(r.radius_ok, "{}", r.measured_min_radius_m);
        // The river polygon is cut square at both ends of the centreline, so only the first and
        // last few metres (inside half the bottom width of the cut) are flagged.
        let len = linear::length(&r.line);
        for s in &r.infeasible {
            assert!(
                s.end_chainage_m < 20.0 || s.start_chainage_m > len - 20.0,
                "{s:?}"
            );
        }
        let narrow = synthetic_river(RiverParams::default()).unwrap();
        let w = core::slice::from_ref(&narrow.polygon);
        let r = fairway_fit_in_water(
            &narrow.centreline,
            w,
            &shape,
            0.0,
            2.0,
            &Fixed::default(),
            &p,
        )
        .unwrap();
        assert!(r.line.len() > 2);
        for k in 0..8 {
            let x = 375.0 + 750.0 * k as f64;
            let y = if k % 2 == 0 { 260.0 } else { -260.0 };
            let at = linear::project(&r.line, [x, y]);
            assert!(
                r.infeasible
                    .iter()
                    .any(|s| s.reason == StretchReason::TooNarrow
                        && s.start_chainage_m < at
                        && at < s.end_chainage_m),
                "apex {k} at chainage {at}: {:?}",
                r.infeasible
            );
        }
    }
}
