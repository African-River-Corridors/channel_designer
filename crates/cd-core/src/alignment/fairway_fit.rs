//! The fairway fit (spec 0005 Q3, Q5, L1, L3; ADR 0007): a smooth design line inside navigable
//! water between fixed pieces, never tighter than R_min where the water allows it.
//!
//! **A minimum-curvature corridor solve** (the racing line inside known bounds; Heilmeier et al.
//! 2020). The line is a reference line `r(s)` moved sideways along its normals `n(s)` by offsets
//! `a(s)`: `p = r + a·n`. Each pass:
//! - casts a ray along each free point's normal to the edge of navigable water, giving a box
//!   `lo <= a <= hi`, made conservative by taking the narrowest box within half a chord either
//!   side (half an LOA at the default chord);
//! - minimises the summed squared curvature of `p`, as a convex QP in the offsets
//!   ([`corridor::min_sum_squares_in_box`], Clarabel); rows where the curvature is over 1/R_min
//!   weigh more, which pushes towards the line whose tightest bend is widest. The input's own
//!   tightest radius inside the water is a floor on the preferred radius, so a bend the water
//!   already holds is never traded for less total bending (#132);
//! - re-linearises: the solution, resampled, is the next pass's reference line.
//!
//! Nothing is ever snapped to an edge: the boxes are limits of the QP, and a point with no
//! navigable width (`lo > hi`, or no water in reach) is left free, so the line passes through
//! that stretch with continuous curvature, and the stretch is reported.
//!
//! The first reference is the seed, smoothed. A fixed pose holds its point and the next one along
//! its heading, with a fixed ghost point one spacing behind it, so the line leaves the pose
//! tangentially with its curvature in the objective. Must-follow segments are points with zero
//! offset inside the same corridor, so the line meets them with continuous curvature, and they
//! are copied exactly into the result. A Dubins piece (`dubins_paths`) of radius R_min to each
//! pose shapes the seed only (L1).

pub mod corridor;

use super::PyError;
use super::navigable::{
    Chained, EDGE_TOL_M, Stretch, StretchReason, inside_outside_stretches, navigable_water,
    width_check,
};
use super::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::grid::EdgeGrid;
use crate::geom::{Polygon, Pt, dist, linear};
use crate::rules::rulesets::ChannelShape;
use corridor::{Row, min_sum_squares_in_box};
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

/// Settings of the fit. `FitParams::new(r_min)` gives the defaults. The bounds window (half a
/// chord either side), the seed smoothing (half a chord) and the reach of a point outside the
/// water (one chord) follow `chord_m`.
#[derive(Debug, Clone, PartialEq)]
pub struct FitParams {
    pub r_min: f64,
    /// The chord the reported radius is read on (default R_min/3, one LOA when R_min = 3 LOA).
    pub chord_m: f64,
    /// A preferred radius: rows tighter than it weigh `1 + pref_weight` times more. Off by
    /// default; the fit raises it to the input's tightest radius inside navigable water.
    pub r_pref_m: Option<f64>,
    /// The extra weight on rows tighter than the preferred radius.
    pub pref_weight: f64,
    /// Point spacing of the corridor (m).
    pub spacing_m: f64,
    /// Re-linearisation passes, at most.
    pub passes: usize,
    /// The passes stop when no point moves further than this (m).
    pub converged_m: f64,
    /// How far a ray looks for the edge of navigable water (m); default 4 R_min.
    pub reach_m: f64,
    /// A point on the inside of a reference bend moves at most this share of the bend's radius
    /// in one pass, so the offset line never folds.
    pub fold_share: f64,
    /// The largest weight a tight row gets: `1 + min((κ R_min)⁴, reweight_cap)`.
    pub reweight_cap: f64,
    /// The box of a point with no navigable width: `±free_cap_m` (default R_min).
    pub free_cap_m: f64,
    /// The weight of the soft pull on a point outside the water towards the nearest water within
    /// one chord (a pull of one chord costs as much as this).
    pub pull_weight: f64,
    /// How far inside the edge of navigable water the boxes end (m). The default, 1 m, covers
    /// the sagitta of one 10 m spacing past an edge corner of radius 12.5 m or more: navigable
    /// water's corners at the land are rounded by the shrink, half the bottom width.
    pub contact_margin_m: f64,
    /// A line that grows past this multiple of the seed's length gets a note.
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
            pref_weight: 10.0,
            spacing_m: 10.0,
            passes: 16,
            converged_m: 0.01,
            reach_m: 4.0 * r_min,
            fold_share: 0.9,
            reweight_cap: 1e4,
            free_cap_m: r_min,
            pull_weight: 1.0,
            contact_margin_m: 1.0,
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
    if !(params.spacing_m.is_finite() && params.spacing_m > 0.0) {
        return Err(PyError::value("the spacing must be finite and positive."));
    }
    let grid = EdgeGrid::new(navigable);
    let seed = Chained::new(line);
    let mut notes = Vec::new();
    let h = params.spacing_m;

    // The corridor's points: free pieces (seeded with Dubins ends) between must-follow segments.
    let mut nodes: Vec<Node> = Vec::new();
    let mut from_s = 0.0;
    let mut from_pt = line[0];
    let mut from_pose = fixed.start_pose;
    let mut from_user_pose = fixed.start_pose.is_some();
    if let Some(p) = from_pose {
        from_pt = p.at;
    }
    for (k, mf) in fixed.must_follow.iter().enumerate() {
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
        // A must-follow segment that starts where the free piece would start leaves nothing to fit
        // between them: building a piece there made a Dubins loop from a point back to itself.
        let gap = libm::hypot(mf[0][0] - from_pt[0], mf[0][1] - from_pt[1]);
        if gap > EDGE_TOL_M {
            let piece = piece_seed(&seed, from_s, to_s, from_pt, mf[0]);
            let piece = with_dubins_ends(&piece, from_pose, Some(to_pose), params, &mut notes);
            push_piece(
                &mut nodes,
                &piece,
                (from_pose, from_user_pose),
                (Some(to_pose), false),
                h,
            );
        }
        for p in Chained::new(mf).resample(h, 1) {
            push_node(&mut nodes, p, Kind::Fixed, Some(k));
        }
        from_s = linear::project(line, mf[n - 1]);
        from_pt = mf[n - 1];
        from_pose = Some(Pose {
            at: mf[n - 1],
            heading_rad: heading([mf[n - 1][0] - mf[n - 2][0], mf[n - 1][1] - mf[n - 2][1]]),
        });
        from_user_pose = false;
    }
    let end_pt = fixed.end_pose.map_or(line[line.len() - 1], |p| p.at);
    // After a must-follow segment that runs to (or past) the seed's end, there is nothing left
    // to fit unless an end pose asks for more.
    let tail_left =
        nodes.is_empty() || from_s < seed.length() - EDGE_TOL_M || fixed.end_pose.is_some();
    if tail_left && from_pt != end_pt {
        let piece = piece_seed(&seed, from_s, seed.length(), from_pt, end_pt);
        let piece = with_dubins_ends(&piece, from_pose, fixed.end_pose, params, &mut notes);
        push_piece(
            &mut nodes,
            &piece,
            (from_pose, from_user_pose),
            (fixed.end_pose, fixed.end_pose.is_some()),
            h,
        );
    }

    let seed_len = linear::length(&emit(&nodes, &fixed.must_follow));
    smooth_free(&mut nodes, params.chord_m / 2.0, h);
    // The input's tightest bend inside the water is a floor on the preferred radius. Above R_min
    // the tightness weight is all but flat, so the QP would trade a sharper peak for less total
    // bending; the ramp makes rows tighter than a line the water already holds weigh more.
    let mut solve = params.clone();
    let r_in = input_min_radius(line, &grid, params);
    if r_in.is_finite() {
        solve.r_pref_m = Some(params.r_pref_m.map_or(r_in, |r| r.max(r_in)));
    }
    solve_corridor(&mut nodes, &grid, &solve, &mut notes);
    let out = emit(&nodes, &fixed.must_follow);
    if linear::length(&out) > params.max_growth * seed_len {
        notes.push(format!(
            "the fitted line is more than {} times its seed's length",
            params.max_growth
        ));
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

/// The input's tightest radius on the chord, at stations every `check_step_m` inside navigable
/// water. Infinity for a straight input or one with no station in the water.
fn input_min_radius(line: &[Pt], grid: &EdgeGrid, params: &FitParams) -> f64 {
    let c = Chained::new(line);
    let step = params.check_step_m;
    (0..=(c.length() / step) as usize)
        .map(|i| i as f64 * step)
        .filter(|&s| grid.covers(c.at(s), params.inside_tol_m))
        .map(|s| c.local_radius(s, params.chord_m))
        .fold(f64::INFINITY, f64::min)
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

/// The signed curvature (1/m, positive turning left) at chainage `s` of `c`, through the points
/// `half_m` before and after it, kept on the line.
pub(crate) fn signed_curvature(c: &Chained, s: f64, half_m: f64) -> f64 {
    let len = c.length();
    if len < 2.0 * half_m {
        return 0.0;
    }
    let s = s.clamp(half_m, len - half_m);
    let (a, b, d) = (c.at(s - half_m), c.at(s), c.at(s + half_m));
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [d[0] - b[0], d[1] - b[1]]);
    let den = dist(a, b) * dist(b, d) * dist(a, d);
    if den > 0.0 {
        2.0 * (u[0] * v[1] - u[1] * v[0]) / den
    } else {
        0.0
    }
}

/// The largest change of signed curvature (1/m) between neighbouring stations `step_m` apart,
/// curvature read through points `half_m` either side. A kink or a fillet's end reads as a jump;
/// a line with continuous curvature does not.
pub fn max_curvature_step(line: &[Pt], half_m: f64, step_m: f64) -> f64 {
    let c = Chained::new(line);
    let n = (c.length() / step_m).floor() as usize;
    let k: Vec<f64> = (0..=n)
        .map(|i| signed_curvature(&c, i as f64 * step_m, half_m))
        .collect();
    k.windows(2).fold(0.0, |m, w| m.max((w[1] - w[0]).abs()))
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
            None => {
                notes.push("no Dubins piece to the start pose; the corridor alone meets it".into())
            }
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
            None => {
                notes.push("no Dubins piece to the end pose; the corridor alone meets it".into())
            }
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

/// What a corridor point is: free to move sideways, fixed, or a fixed ghost (behind a pose, in
/// the objective only, never in the result).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Free,
    Fixed,
    Ghost,
}

/// A corridor point. `mf` names the must-follow segment it samples, if any.
#[derive(Debug, Clone, Copy)]
struct Node {
    p: Pt,
    kind: Kind,
    mf: Option<usize>,
}

/// Append a point; one within 1e-6 m of the last point merges with it (fixed wins, the
/// must-follow tag is kept).
fn push_node(nodes: &mut Vec<Node>, p: Pt, kind: Kind, mf: Option<usize>) {
    if let Some(l) = nodes.last_mut()
        && dist(l.p, p) < 1e-6
    {
        if kind != Kind::Free {
            l.p = p;
            l.kind = kind;
        }
        l.mf = l.mf.or(mf);
        return;
    }
    nodes.push(Node { p, kind, mf });
}

/// The points of one free piece, resampled at about `h`: its end points fixed; at a posed end,
/// the next point along the pose's heading fixed too, and, at a user's pose (not the end of a
/// must-follow segment), a ghost one spacing behind it.
fn push_piece(
    nodes: &mut Vec<Node>,
    piece: &[Pt],
    start: (Option<Pose>, bool),
    end: (Option<Pose>, bool),
    h: f64,
) {
    let c = Chained::new(piece);
    let mut pts = c.resample(h, 4);
    let n = pts.len() - 1;
    let hh = c.length() / n as f64;
    if let (Some(p), ghost) = start {
        let d = p.dir();
        if ghost {
            push_node(
                nodes,
                [p.at[0] - hh * d[0], p.at[1] - hh * d[1]],
                Kind::Ghost,
                None,
            );
        }
        pts[0] = p.at;
        pts[1] = [p.at[0] + hh * d[0], p.at[1] + hh * d[1]];
    }
    if let (Some(p), _) = end {
        let d = p.dir();
        pts[n] = p.at;
        pts[n - 1] = [p.at[0] - hh * d[0], p.at[1] - hh * d[1]];
    }
    for (i, &p) in pts.iter().enumerate() {
        let fixed =
            i == 0 || i == n || (i == 1 && start.0.is_some()) || (i == n - 1 && end.0.is_some());
        push_node(nodes, p, if fixed { Kind::Fixed } else { Kind::Free }, None);
    }
    if let (Some(p), true) = end {
        let d = p.dir();
        push_node(
            nodes,
            [p.at[0] + hh * d[0], p.at[1] + hh * d[1]],
            Kind::Ghost,
            None,
        );
    }
}

/// The result line: the points in order, ghosts left out, each must-follow segment copied
/// exactly in place of its samples.
fn emit(nodes: &[Node], must_follow: &[Vec<Pt>]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::new();
    let mut done = vec![false; must_follow.len()];
    for nd in nodes {
        match (nd.kind, nd.mf) {
            (Kind::Ghost, _) => {}
            (_, Some(k)) => {
                if !done[k] {
                    done[k] = true;
                    append(&mut out, &must_follow[k]);
                }
            }
            _ => append(&mut out, &[nd.p]),
        }
    }
    out
}

/// Smooth the free points of the seed (a binomial filter, about a Gaussian of `sigma_m`), fixed
/// points held: the first reference line.
fn smooth_free(nodes: &mut [Node], sigma_m: f64, h: f64) {
    let passes = libm::ceil(2.0 * (sigma_m / h) * (sigma_m / h)) as usize;
    let n = nodes.len();
    for _ in 0..passes {
        let prev: Vec<Pt> = nodes.iter().map(|nd| nd.p).collect();
        for i in 1..n.saturating_sub(1) {
            if nodes[i].kind == Kind::Free {
                let (a, p, c) = (prev[i - 1], prev[i], prev[i + 1]);
                nodes[i].p = [
                    (a[0] + 2.0 * p[0] + c[0]) / 4.0,
                    (a[1] + 2.0 * p[1] + c[1]) / 4.0,
                ];
            }
        }
    }
}

/// The unit left normal at each point: from its neighbours' chord, or its one segment at an end.
fn normals(p: &[Pt]) -> Vec<Pt> {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, c) = (p[i.saturating_sub(1)], p[(i + 1).min(n - 1)]);
            let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt();
            if l > 0.0 {
                [-dy / l, dx / l]
            } else {
                [0.0, 1.0]
            }
        })
        .collect()
}

fn dot(a: Pt, b: Pt) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The second derivative at interior point `i` on uneven spacing (1/m; its normal component is
/// the curvature, positive turning left): as a constant vector, and the coefficients on the
/// offsets of `i - 1`, `i` and `i + 1` (vectors: each offset moves its point along its own normal).
/// It is zero on a straight line however the points are spaced.
fn second_derivative(p: &[Pt], nrm: &[Pt], i: usize) -> (Pt, [Pt; 3]) {
    let hm = dist(p[i - 1], p[i]);
    let hp = dist(p[i], p[i + 1]);
    let f = 2.0 / (hm + hp);
    let (cm, cp) = (f / hm, f / hp);
    let c0 = -(cm + cp);
    let d2 = [
        cm * p[i - 1][0] + c0 * p[i][0] + cp * p[i + 1][0],
        cm * p[i - 1][1] + c0 * p[i][1] + cp * p[i + 1][1],
    ];
    let sc = |c: f64, n: Pt| [c * n[0], c * n[1]];
    (d2, [sc(cm, nrm[i - 1]), sc(c0, nrm[i]), sc(cp, nrm[i + 1])])
}

/// The spans of navigable water along the line `p + t·n`, `|t| <= reach`: the ray's crossings with
/// the edge split it, and a span whose middle is inside is water.
fn water_spans(p: Pt, n: Pt, grid: &EdgeGrid, reach: f64) -> Vec<(f64, f64)> {
    let q0 = [p[0] - reach * n[0], p[1] - reach * n[1]];
    let q1 = [p[0] + reach * n[0], p[1] + reach * n[1]];
    let mut ts: Vec<f64> = grid
        .hits(q0, q1)
        .into_iter()
        .map(|t| (t - 0.5) * 2.0 * reach)
        .collect();
    ts.sort_by(f64::total_cmp);
    ts.dedup();
    let mut cuts = vec![-reach];
    cuts.extend(ts.iter().copied().filter(|t| t.abs() < reach));
    cuts.push(reach);
    cuts.windows(2)
        .filter(|w| {
            let mid = (w[0] + w[1]) / 2.0;
            grid.contains([p[0] + mid * n[0], p[1] + mid * n[1]])
        })
        .map(|w| (w[0], w[1]))
        .collect()
}

/// Where a point stands in the water, from its spans (each end `margin` inside the edge, not at
/// the ray's reach).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    /// Inside a span: its box.
    Inside(f64, f64),
    /// Outside the water with a span within `outside_reach`: the offset of that span's near
    /// edge (a soft pull, never a box).
    Outside(f64),
    /// No navigable width in reach.
    Nowhere,
}

fn place(spans: &[(f64, f64)], reach: f64, outside_reach: f64, margin: f64) -> Place {
    let inner = |&(a, c): &(f64, f64)| {
        (
            if a > -reach { a + margin } else { a },
            if c < reach { c - margin } else { c },
        )
    };
    if let Some((a, c)) = spans
        .iter()
        .filter(|(a, c)| *a <= 0.0 && 0.0 <= *c)
        .map(inner)
        .next()
    {
        return if a < c {
            Place::Inside(a, c)
        } else {
            Place::Nowhere
        };
    }
    spans
        .iter()
        .map(inner)
        .filter(|(a, c)| a < c)
        .map(|(a, c)| if a > 0.0 { a } else { c })
        .filter(|t| t.abs() <= outside_reach)
        .min_by(|x, y| x.abs().total_cmp(&y.abs()))
        .map_or(Place::Nowhere, Place::Outside)
}

/// The minimum-curvature corridor solve on `nodes`, in place. See the module notes.
fn solve_corridor(
    nodes: &mut Vec<Node>,
    grid: &EdgeGrid,
    params: &FitParams,
    notes: &mut Vec<String>,
) {
    let h = params.spacing_m;
    let half_window = params.chord_m / 2.0;
    // Row weights by position along the line (share of its length), carried between passes.
    let mut wprev: Vec<(f64, f64)> = Vec::new();
    let mut best: Option<((usize, f64), Vec<Node>)> = None;
    for pass in 0..params.passes {
        let pts: Vec<Pt> = nodes.iter().map(|nd| nd.p).collect();
        let n = pts.len();
        let free: Vec<usize> = (0..n).filter(|&i| nodes[i].kind == Kind::Free).collect();
        if free.is_empty() || n < 3 {
            return;
        }
        let mut col = vec![usize::MAX; n];
        for (j, &i) in free.iter().enumerate() {
            col[i] = j;
        }
        let nrm = normals(&pts);
        let rowsk: Vec<(usize, Pt, [Pt; 3])> = (1..n - 1)
            .filter(|&i| (i - 1..=i + 1).any(|k| col[k] != usize::MAX))
            .map(|i| {
                let (d2, c) = second_derivative(&pts, &nrm, i);
                (i, d2, c)
            })
            .collect();
        let mut kappa = vec![0.0; n];
        for &(i, d2, _) in &rowsk {
            kappa[i] = dot(nrm[i], d2);
        }
        // Boxes: water spans, the narrowest within half a chord, then the fold limit.
        let reach = params.reach_m.max(params.chord_m);
        let own: Vec<Place> = free
            .iter()
            .map(|&i| {
                let spans = water_spans(pts[i], nrm[i], grid, reach);
                place(&spans, reach, params.chord_m, params.contact_margin_m)
            })
            .collect();
        let chain = Chained::new(&pts);
        let mut lo = vec![0.0; free.len()];
        let mut hi = vec![0.0; free.len()];
        let mut a = 0;
        for (j, &i) in free.iter().enumerate() {
            let s = chain.cum[i];
            while chain.cum[free[a]] < s - half_window {
                a += 1;
            }
            let (mut l, mut u) = (-params.free_cap_m, params.free_cap_m);
            if let Place::Inside(ol, ou) = own[j] {
                // The narrowest box within half a chord, of the neighbours inside the water.
                let (mut nl, mut nu) = (ol, ou);
                for (k, &fi) in free.iter().enumerate().skip(a) {
                    if chain.cum[fi] > s + half_window {
                        break;
                    }
                    if let Place::Inside(bl, bu) = own[k] {
                        nl = nl.max(bl);
                        nu = nu.min(bu);
                    }
                }
                (l, u) = if nl <= nu { (nl, nu) } else { (ol, ou) };
            }
            // The fold limit on the inside of the reference's bend.
            let k = kappa[i];
            let fold = |v: f64| params.fold_share / v.abs();
            if k > 0.0 {
                u = u.min(fold(k));
                l = l.min(u);
            } else if k < 0.0 {
                l = l.max(-fold(k));
                u = u.max(l);
            }
            lo[j] = l;
            hi[j] = u;
        }
        // Rows: the second derivative × R_min, x and y, weighted by length and by tightness. The
        // tightness is the curvature averaged over a chord, so the weights change smoothly from
        // pass to pass (a step in the weights makes the passes oscillate).
        // A weight rises at once and falls slowly (the geometric mean with the last pass's), so
        // the reweighting settles instead of moving the tight spot back and forth.
        let kbar = chord_mean_abs(&chain.cum, &kappa, half_window);
        let total = chain.length();
        let mut wnow: Vec<(f64, f64)> = Vec::with_capacity(rowsk.len());
        let mut rows: Vec<Row> = Vec::with_capacity(2 * rowsk.len());
        for &(i, d2, c) in &rowsk {
            let len = (dist(pts[i - 1], pts[i]) + dist(pts[i], pts[i + 1])) / 2.0;
            let f = chain.cum[i] / total;
            let wt = tightness_weight(kbar[i], params);
            let w = match interp(&wprev, f) {
                Some(wp) if wp > wt => (wp * wt).sqrt(),
                _ => wt,
            };
            wnow.push((f, w));
            let s = params.r_min * (w * len / h).sqrt();
            for x in 0..2 {
                let terms = (0..3)
                    .filter(|&t| col[i + t - 1] != usize::MAX)
                    .map(|t| (col[i + t - 1], s * c[t][x]))
                    .collect();
                rows.push(Row {
                    c: s * d2[x],
                    terms,
                });
            }
        }
        // A point outside the water is pulled softly towards the nearest water.
        let pull = params.pull_weight.sqrt() / params.chord_m;
        for (j, pl) in own.iter().enumerate() {
            if let Place::Outside(t) = *pl {
                rows.push(Row {
                    c: -pull * t,
                    terms: vec![(j, pull)],
                });
            }
        }
        let Some(off) = min_sum_squares_in_box(&rows, &lo, &hi) else {
            notes.push(format!(
                "the corridor solve did not converge on pass {}; the line from the pass before is kept",
                pass + 1
            ));
            return;
        };
        let moved = off.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        for (j, &i) in free.iter().enumerate() {
            nodes[i].p = [
                pts[i][0] + off[j] * nrm[i][0],
                pts[i][1] + off[j] * nrm[i][1],
            ];
        }
        resample_free(nodes, h);
        wprev = wnow;
        // Keep the best pass: fewest free points outside the water, then the widest tightest
        // bend on the chord. A late pass never makes the result worse.
        let line: Vec<Pt> = nodes
            .iter()
            .filter(|nd| nd.kind != Kind::Ghost)
            .map(|nd| nd.p)
            .collect();
        let outside = nodes
            .iter()
            .filter(|nd| nd.kind == Kind::Free && !grid.covers(nd.p, params.inside_tol_m))
            .count();
        let radius = min_radius_on_chord(&line, params.chord_m).unwrap_or(0.0);
        if best
            .as_ref()
            .is_none_or(|((o, r), _)| outside < *o || (outside == *o && radius > *r))
        {
            best = Some(((outside, radius), nodes.clone()));
        }
        if moved < params.converged_m {
            break;
        }
    }
    if let Some((_, b)) = best {
        *nodes = b;
    }
}

/// Linear interpolation in `(x, y)` pairs sorted by `x`, clamped at the ends; `None` when empty.
fn interp(xy: &[(f64, f64)], x: f64) -> Option<f64> {
    let (first, last) = (xy.first()?, xy.last()?);
    if x <= first.0 {
        return Some(first.1);
    }
    if x >= last.0 {
        return Some(last.1);
    }
    let k = xy.partition_point(|p| p.0 <= x);
    let ((x0, y0), (x1, y1)) = (xy[k - 1], xy[k]);
    Some(if x1 > x0 {
        y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    } else {
        y1
    })
}

/// The mean of `|k|` within `half` metres either side of each point.
fn chord_mean_abs(cum: &[f64], k: &[f64], half: f64) -> Vec<f64> {
    let n = k.len();
    let mut pre = vec![0.0; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + k[i].abs();
    }
    let (mut a, mut b) = (0, 0);
    (0..n)
        .map(|i| {
            while cum[a] < cum[i] - half {
                a += 1;
            }
            while b + 1 < n && cum[b + 1] <= cum[i] + half {
                b += 1;
            }
            (pre[b + 1] - pre[a]) / (b + 1 - a) as f64
        })
        .collect()
}

/// The weight of a row whose curvature is `k`: `1 + min((k R_min)⁴, cap)`, times
/// `1 + pref_weight · x⁴/(1 + x⁴)` with `x = k R_pref` when a preferred radius is set (a smooth
/// ramp through the preferred radius, never a step).
fn tightness_weight(k: f64, params: &FitParams) -> f64 {
    let x = k * params.r_min;
    let mut w = 1.0 + (x * x * x * x).min(params.reweight_cap);
    if let Some(rp) = params.r_pref_m {
        let y = k * rp;
        let y4 = y * y * y * y;
        w *= 1.0 + params.pref_weight * y4 / (1.0 + y4);
    }
    w
}

/// Resample each run of free points (between its fixed neighbours) at about `h`: the next
/// reference line.
fn resample_free(nodes: &mut Vec<Node>, h: f64) {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut i = 0;
    while i < nodes.len() {
        out.push(nodes[i]);
        if i + 1 < nodes.len() && nodes[i + 1].kind == Kind::Free {
            let mut j = i + 1;
            while nodes[j].kind == Kind::Free {
                j += 1;
            }
            let run: Vec<Pt> = nodes[i..=j].iter().map(|nd| nd.p).collect();
            let r = Chained::new(&run).resample(h, 2);
            for &p in &r[1..r.len() - 1] {
                out.push(Node {
                    p,
                    kind: Kind::Free,
                    mf: None,
                });
            }
            i = j;
        } else {
            i += 1;
        }
    }
    *nodes = out;
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

    #[test]
    fn a_must_follow_segment_at_the_start_is_held_without_a_loop() {
        // The seed starts on its must-follow segment (a designed reach from the berth). Before the
        // fix, a free piece from the start back to itself became a Dubins loop of about 700 m.
        let water = vec![Polygon {
            exterior: vec![
                [-300.0, -300.0],
                [4300.0, -300.0],
                [4300.0, 300.0],
                [-300.0, 300.0],
                [-300.0, -300.0],
            ],
            holes: vec![],
        }];
        let seed = [[0.0, 0.0], [1000.0, 0.0], [2000.0, 100.0], [4000.0, 0.0]];
        let fixed = Fixed {
            must_follow: vec![vec![[0.0, 0.0], [1000.0, 0.0]]],
            ..Fixed::default()
        };
        let r = fairway_fit(&seed, &water, &fixed, &FitParams::new(420.0)).unwrap();
        let len = linear::length(&r.line);
        assert!(len < 4100.0, "{len}");
        assert!(
            r.line.iter().all(|p| p[0] >= -1e-6),
            "the line runs behind its start"
        );
    }

    #[test]
    fn a_right_angle_seed_in_wide_water_is_fitted_at_the_default_spacing() {
        // An L-shaped channel 800 m wide and a seed with a 90 deg corner. A 420 m line fits with
        // room to spare. Before the QP was scaled, pass 1 stalled (InsufficientProgress) at the
        // default 10 m spacing and the smoothed seed came back at R 84 m.
        let water = vec![Polygon {
            exterior: vec![
                [-400.0, -400.0],
                [1900.0, -400.0],
                [1900.0, 1900.0],
                [1100.0, 1900.0],
                [1100.0, 400.0],
                [-400.0, 400.0],
                [-400.0, -400.0],
            ],
            holes: vec![],
        }];
        let seed = [[0.0, 0.0], [1500.0, 0.0], [1500.0, 1500.0]];
        let r = fairway_fit(&seed, &water, &Fixed::default(), &FitParams::new(420.0)).unwrap();
        assert!(r.radius_ok, "{} {:?}", r.measured_min_radius_m, r.notes);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
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

#[cfg(test)]
pub(crate) mod corridor_fixtures {
    //! Synthetic fixtures for the corridor solve's acceptance checks (#126).
    use crate::geom::{Polygon, Pt};
    use crate::synthetic::fixtures::SplitMix;

    /// **A wide 90° bend.** A river `width_m` wide: its centreline runs east 1500 m to the origin,
    /// turns left on an arc of radius 600 m about (0, 600), and runs north 1500 m from (600,
    /// 600). Returns the water and the centreline (the seed), sampled every 10 m.
    pub fn wide_bend(rc: f64, width_m: f64) -> (Polygon, Vec<Pt>) {
        let mut c: Vec<(Pt, Pt)> = Vec::new(); // (point, left normal)
        for i in 0..150 {
            c.push(([-1500.0 + 10.0 * i as f64, 0.0], [0.0, 1.0]));
        }
        let n = (rc * core::f64::consts::FRAC_PI_2 / 10.0).ceil() as usize; // about 10 m
        for i in 0..n {
            let t =
                -core::f64::consts::FRAC_PI_2 + core::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
            let (s, co) = (libm::sin(t), libm::cos(t));
            c.push(([rc * co, rc + rc * s], [-co, -s]));
        }
        for i in 0..=150 {
            c.push(([rc, rc + 10.0 * i as f64], [-1.0, 0.0]));
        }
        let h = width_m / 2.0;
        let mut ring: Vec<Pt> = c
            .iter()
            .map(|(p, n)| [p[0] - h * n[0], p[1] - h * n[1]])
            .collect();
        ring.extend(
            c.iter()
                .rev()
                .map(|(p, n)| [p[0] + h * n[0], p[1] + h * n[1]]),
        );
        ring.push(ring[0]);
        (
            Polygon {
                exterior: ring,
                holes: vec![],
            },
            c.iter().map(|(p, _)| *p).collect(),
        )
    }

    /// **A creek with narrow stretches.** A sine centreline (`synthetic::sine_generated_centreline`,
    /// two wavelengths) whose width steps every 150 m between 45 and 100 m, drawn from `seed`.
    /// Returns the water and the centreline (the seed).
    pub fn stepped_creek(seed: u64) -> (Polygon, Vec<Pt>) {
        let mut rng = SplitMix::new(seed);
        let lam = 1800.0 + 600.0 * rng.signed_unit();
        let amp = 200.0 + 60.0 * rng.signed_unit();
        let c = crate::synthetic::sine_generated_centreline(2.0 * lam, lam, amp, 10.0);
        let ws: Vec<f64> = (0..=(2.0 * lam / 150.0) as usize)
            .map(|_| 72.5 + 27.5 * rng.signed_unit())
            .collect();
        let nr = super::normals(&c);
        let half = |p: Pt| ws[((p[0] / 150.0).max(0.0) as usize).min(ws.len() - 1)] / 2.0;
        let mut ring: Vec<Pt> = c
            .iter()
            .zip(&nr)
            .map(|(p, n)| [p[0] - half(*p) * n[0], p[1] - half(*p) * n[1]])
            .collect();
        ring.extend(
            c.iter()
                .zip(&nr)
                .rev()
                .map(|(p, n)| [p[0] + half(*p) * n[0], p[1] + half(*p) * n[1]]),
        );
        ring.push(ring[0]);
        (
            Polygon {
                exterior: ring,
                holes: vec![],
            },
            c,
        )
    }

    /// **An offset narrow strip.** A reach 60 m wide along y = 0 (x 0 to 1000), a strip under
    /// 20 m wide rising to a second reach 60 m wide along y = 80 (x 1500 to 2500). Shrunk by
    /// half a 30.4 m bottom width, the strip has no navigable width. Returns the water and the
    /// seed (the three centrelines, with their corners).
    pub fn offset_strip() -> (Polygon, Vec<Pt>) {
        let ring = vec![
            [0.0, -30.0],
            [1000.0, -30.0],
            [1000.0, -10.0],
            [1500.0, 70.0],
            [1500.0, 50.0],
            [2500.0, 50.0],
            [2500.0, 110.0],
            [1500.0, 110.0],
            [1500.0, 90.0],
            [1000.0, 10.0],
            [1000.0, 30.0],
            [0.0, 30.0],
            [0.0, -30.0],
        ];
        (
            Polygon {
                exterior: ring,
                holes: vec![],
            },
            vec![[0.0, 0.0], [1000.0, 0.0], [1500.0, 80.0], [2500.0, 80.0]],
        )
    }
}

#[cfg(test)]
mod corridor_tests {
    //! Label: property (#126 acceptance: bend placement, no sharpening, fixed tail, no navigable
    //! width; each check shown failing on a corrupted input). Known answers: the centreline seed
    //! lies in the middle of the water (position 0.5, nothing in the outer third).
    use super::corridor_fixtures::{offset_strip, stepped_creek, wide_bend};
    use super::*;
    use crate::synthetic::fixtures::SplitMix;
    use crate::synthetic::{RiverParams, WaterFixture, fixed_tail, synthetic_river};

    const R_MIN: f64 = 255.0;
    const HALF: f64 = 15.2;
    /// The largest step of curvature (1/m) between stations 10 m apart, read through points 10 m
    /// either side, on a line with continuous curvature in a reach whose bends are R_min or
    /// gentler. A kink, a corner or a fillet's end reads far more (about 1/R across two
    /// stations). A creek tighter than R_min changes curvature faster than this legitimately,
    /// so the creek is held to "no sharper than its input" instead.
    const KINK_STEP: f64 = 0.05 / R_MIN;
    /// Bend placement: the share of a bend's length in the outer third of the navigable width
    /// must be over this. The issue measured 4-8 % for the band; the seed has 0.
    const OUTER_SHARE: f64 = 0.30;

    fn nav(w: &Polygon) -> Vec<Polygon> {
        navigable_water(core::slice::from_ref(w), HALF, 0.0, 0.0).unwrap()
    }

    fn turned(seed: u64, w: &Polygon, line: &[Pt]) -> (Polygon, Vec<Pt>) {
        let mut rng = SplitMix::new(seed);
        let f = WaterFixture {
            water: w.clone(),
            legs: vec![line.to_vec()],
            fixed: vec![],
        }
        .transformed(
            core::f64::consts::PI * rng.signed_unit(),
            [5e5 * rng.signed_unit(), 5e6 + 1e5 * rng.signed_unit()],
        );
        (f.water, f.legs[0].clone())
    }

    fn kink_free(line: &[Pt]) -> Result<(), String> {
        let k = max_curvature_step(line, 10.0, 10.0);
        if k <= KINK_STEP {
            Ok(())
        } else {
            Err(format!(
                "curvature steps by {k:.2e} /m in 10 m (limit {KINK_STEP:.2e})"
            ))
        }
    }

    /// The share of a bend's length in the outer third of the navigable width: stations every
    /// 5 m where the line's curvature (on the default chord) is at least half its peak, placed
    /// across the water by their offset from the river's centreline (0 inner edge, 1 outer).
    fn outer_share(line: &[Pt], centreline: &[Pt], nav_width: f64) -> f64 {
        let (c, cc) = (Chained::new(line), Chained::new(centreline));
        let half = default_check_chord(R_MIN) / 2.0;
        let ks: Vec<(f64, f64)> = (0..(c.length() / 5.0) as usize)
            .map(|i| {
                let s = i as f64 * 5.0;
                (s, signed_curvature(&c, s, half))
            })
            .collect();
        let peak = ks.iter().fold(0.0f64, |m, x| m.max(x.1.abs()));
        let (mut all, mut outer) = (0usize, 0usize);
        for &(s, k) in &ks {
            if k.abs() < peak / 2.0 {
                continue;
            }
            let q = c.at(s);
            let sc = linear::project(centreline, q);
            let (pc, t) = (cc.at(sc), cc.direction(sc, 5.0));
            // Offset to the left of the centreline; the bend turns left, so left is inner.
            let d = (q[0] - pc[0]) * -t[1] + (q[1] - pc[1]) * t[0];
            all += 1;
            if 0.5 - d / nav_width > 2.0 / 3.0 {
                outer += 1;
            }
        }
        outer as f64 / all as f64
    }

    fn swings_wide(line: &[Pt], centreline: &[Pt], nav_width: f64) -> Result<(), String> {
        let sh = outer_share(line, centreline, nav_width);
        if sh > OUTER_SHARE {
            Ok(())
        } else {
            Err(format!(
                "{sh:.3} of the bend in the outer third (needs over {OUTER_SHARE})"
            ))
        }
    }

    #[test]
    fn a_tight_bend_in_wide_water_is_taken_wide() {
        // Centreline radius 200 m (under R_min), water 200 m wide: navigable width 169.6 m.
        let (water, centre) = wide_bend(200.0, 200.0);
        let wn = 200.0 - 2.0 * HALF;
        for k in 0..3u64 {
            let (w, c) = if k == 0 {
                (water.clone(), centre.clone())
            } else {
                turned(k, &water, &centre)
            };
            for posed in [false, true] {
                let dir = |a: Pt, b: Pt| libm::atan2(b[1] - a[1], b[0] - a[0]);
                let n = c.len();
                let fixed = if posed {
                    Fixed {
                        start_pose: Some(Pose {
                            at: c[0],
                            heading_rad: dir(c[0], c[1]),
                        }),
                        end_pose: Some(Pose {
                            at: c[n - 1],
                            heading_rad: dir(c[n - 2], c[n - 1]),
                        }),
                        must_follow: vec![],
                    }
                } else {
                    Fixed::default()
                };
                let nv = nav(&w);
                let r = fairway_fit(&c, &nv, &fixed, &FitParams::new(R_MIN)).unwrap();
                swings_wide(&r.line, &c, wn).unwrap();
                // Only the cut ends of the river (inside half the bottom width) are flagged.
                let len = linear::length(&r.line);
                for st in &r.infeasible {
                    assert!(
                        st.end_chainage_m < 20.0 || st.start_chainage_m > len - 20.0,
                        "{st:?}"
                    );
                }
                assert!(r.radius_ok, "{}", r.measured_min_radius_m);
                kink_free(&r.line).unwrap();
            }
        }
        // The check fails on a line down the middle (the centreline seed, where the band
        // stayed): nothing in the outer third.
        let e = swings_wide(&centre, &centre, wn).unwrap_err();
        assert!(e.starts_with("0.000"), "{e}");
    }

    /// The smallest radius (on the chord) of `line` at stations every 10 m whose nearest point on
    /// `input` is inside navigable water, and the same of `input` itself.
    fn radii_where_input_inside(
        line: &[Pt],
        input: &[Pt],
        grid: &EdgeGrid,
        chord: f64,
    ) -> (f64, f64) {
        let (c, ci) = (Chained::new(line), Chained::new(input));
        let mut min_in = f64::INFINITY;
        for i in 0..=(ci.length() / 10.0) as usize {
            let s = i as f64 * 10.0;
            if grid.covers(ci.at(s), 0.05) {
                min_in = min_in.min(ci.local_radius(s, chord));
            }
        }
        let mut min_fit = f64::INFINITY;
        for i in 0..=(c.length() / 10.0) as usize {
            let s = i as f64 * 10.0;
            let si = linear::project(input, c.at(s));
            if grid.covers(ci.at(si), 0.05) {
                min_fit = min_fit.min(c.local_radius(s, chord));
            }
        }
        (min_fit, min_in)
    }

    fn not_sharper(line: &[Pt], input: &[Pt], grid: &EdgeGrid, chord: f64) -> Result<(), String> {
        let (fit, inp) = radii_where_input_inside(line, input, grid, chord);
        if fit >= inp {
            Ok(())
        } else {
            Err(format!(
                "fitted radius {fit:.1} m is sharper than the input's {inp:.1} m"
            ))
        }
    }

    #[test]
    fn a_narrow_creek_is_never_fitted_sharper_than_its_input() {
        let p = FitParams::new(R_MIN);
        // The synthetic river's apexes are 219 m on the centreline, under R_min: a creek whose
        // navigable strip is 9.6 to 29.6 m wide.
        for width in [40.0, 46.0, 60.0] {
            let river = synthetic_river(RiverParams {
                width_m: width,
                ..RiverParams::default()
            })
            .unwrap();
            for k in 0..3u64 {
                let (w, c) = if k == 0 {
                    (river.polygon.clone(), river.centreline.clone())
                } else {
                    turned(k + 10, &river.polygon, &river.centreline)
                };
                let nv = nav(&w);
                let r = fairway_fit(&c, &nv, &Fixed::default(), &p).unwrap();
                let grid = EdgeGrid::new(&nv);
                not_sharper(&r.line, &c, &grid, p.chord_m).unwrap();
            }
        }
        // The check fails on a line with a 20 m dog-leg left in, inside the water.
        let river = synthetic_river(RiverParams {
            width_m: 60.0,
            ..RiverParams::default()
        })
        .unwrap();
        let nv = nav(&river.polygon);
        let grid = EdgeGrid::new(&nv);
        let c = Chained::new(&river.centreline);
        let s = c.length() / 2.0;
        let mut bad = c.slice(0.0, s - 10.0);
        let d = c.direction(s, 5.0);
        let m = c.at(s);
        bad.push([m[0] - 8.0 * d[1], m[1] + 8.0 * d[0]]);
        bad.extend(c.slice(s + 10.0, c.length()));
        let e = not_sharper(&bad, &river.centreline, &grid, p.chord_m).unwrap_err();
        assert!(e.contains("sharper"), "{e}");
    }

    #[test]
    fn a_creek_with_narrow_stretches_is_never_fitted_sharper_than_its_input() {
        // Creeks whose inputs bend at 740-950 m, well above R_min, in water 45-100 m wide. With a
        // flat weight above R_min the QP traded the input's peak for less total bending: 880 m
        // against 949 m (seed 1008), 696 m against 738 m, 743 m against 776 m.
        let p = FitParams::new(R_MIN);
        for seed in [1008u64, 1002, 1026] {
            let (water, centre) = stepped_creek(seed);
            for k in 0..2u64 {
                let (w, c) = if k == 0 {
                    (water.clone(), centre.clone())
                } else {
                    turned(seed + k, &water, &centre)
                };
                let nv = nav(&w);
                let r = fairway_fit(&c, &nv, &Fixed::default(), &p).unwrap();
                let grid = EdgeGrid::new(&nv);
                not_sharper(&r.line, &c, &grid, p.chord_m)
                    .map_err(|e| format!("seed {seed} variant {k}: {e}"))
                    .unwrap();
            }
        }
    }

    #[test]
    fn a_fixed_tail_is_met_with_continuous_curvature() {
        let base = fixed_tail();
        for k in 0..4u64 {
            let f = if k == 0 {
                base.clone()
            } else {
                let mut rng = SplitMix::new(k + 20);
                base.transformed(
                    core::f64::consts::PI * rng.signed_unit(),
                    [5e5 * rng.signed_unit(), 5e6],
                )
            };
            let fixed = Fixed {
                must_follow: f.fixed.clone(),
                ..Fixed::default()
            };
            let r =
                fairway_fit(&f.legs[0], &nav(&f.water), &fixed, &FitParams::new(R_MIN)).unwrap();
            let tail = &f.fixed[0];
            let m = r.line.len();
            assert_eq!(
                &r.line[m - 2..],
                &tail[..],
                "variant {k}: the fixed segment changed"
            );
            // Curvature through the join: the 300 m either side of the tail's start.
            let c = Chained::new(&r.line);
            let sj = linear::project(&r.line, tail[0]);
            let near = c.slice(sj - 300.0, (sj + 300.0).min(c.length()));
            kink_free(&near).unwrap();
            // Approaching the straight tail, the line is all but straight too.
            let k_before = signed_curvature(&c, sj - 15.0, 10.0).abs();
            assert!(
                k_before <= KINK_STEP,
                "variant {k}: {k_before:.2e} /m before the tail"
            );
            assert!(r.radius_ok, "variant {k}: {}", r.measured_min_radius_m);
        }
        // The check fails on a line that meets the tail with a corner (straight from the start).
        let mut bad = vec![[0.0, 0.0], [2400.0, 100.0]];
        bad.extend_from_slice(&base.fixed[0][1..]);
        let c = Chained::new(&bad);
        let near = c.slice(2100.0, c.length());
        assert!(kink_free(&near).is_err());
    }

    #[test]
    fn a_stretch_with_no_navigable_width_is_reported_and_passed_smoothly() {
        let (water, seed) = offset_strip();
        for k in 0..3u64 {
            let (w, c) = if k == 0 {
                (water.clone(), seed.clone())
            } else {
                turned(k + 30, &water, &seed)
            };
            let nv = nav(&w);
            let r = fairway_fit(&c, &nv, &Fixed::default(), &FitParams::new(R_MIN)).unwrap();
            // One `too_narrow` stretch spans the strip (x 1000 to 1500, along about 507 m of
            // line), apart from the cut ends.
            let len = linear::length(&r.line);
            let mid: Vec<&Stretch> = r
                .infeasible
                .iter()
                .filter(|s| s.end_chainage_m > 20.0 && s.start_chainage_m < len - 20.0)
                .collect();
            assert_eq!(mid.len(), 1, "variant {k}: {:?}", r.infeasible);
            assert_eq!(mid[0].reason, StretchReason::TooNarrow);
            let span = mid[0].end_chainage_m - mid[0].start_chainage_m;
            assert!(span > 500.0 && span < 600.0, "variant {k}: {span}");
            // Through it, the line keeps continuous curvature, never tighter than R_min, and is
            // snapped to nothing.
            let through = Chained::new(&r.line).slice(
                mid[0].start_chainage_m - 100.0,
                mid[0].end_chainage_m + 100.0,
            );
            kink_free(&through).unwrap();
            assert!(r.radius_ok, "variant {k}: {}", r.measured_min_radius_m);
            check_fit_properties(&r, &nv, R_MIN, default_check_chord(R_MIN), 5.0).unwrap();
        }
        // The check fails where points are left where they were while their neighbours move
        // (the seed's corners through the strip).
        assert!(kink_free(&seed).is_err());
    }
}
