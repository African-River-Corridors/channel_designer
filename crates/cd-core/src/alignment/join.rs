//! Joined legs and the join guard (spec 0005 R2-Q2, R2-Q4, L5, L7, L8).
//!
//! `join_legs` joins ordered seed legs into one design line. At each join it puts the largest
//! fillet, from a ceiling down to R_min, whose arc keeps the channel (widened for that radius)
//! inside the water. Where no fillet fits, a fairway-fit window replaces the corner; its half
//! length grows with the turn angle (L5). Fixed pieces (a start pose, an end pose, must-follow
//! segments) are then held by one fairway fit over the whole line.
//!
//! Joins are found by position, never by chainage, after any step that changes the line's length
//! (L7). **The join guard** (R2-Q2, L8): after any later fit, each join that held R_min is read
//! again; if it fails, that window is refit with more weight on a gentle bend; if it still fails,
//! the result names the join in `guard_failed` (the option's failed `join_guard` check).
//!
//! The same machinery closes the straight steps between optimised pools tangentially
//! (`tangent_joins`, `optimise_pools_tangent`).

use super::PyError;
use super::fairway_fit::{
    FitParams, Fixed, Pose, fairway_fit, fairway_fit_in_water, merge_stretches,
};
use super::fillet::fillet;
use super::navigable::{Chained, Stretch, navigable_water, width_check};
use super::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::grid::EdgeGrid;
use crate::geom::{Polygon, Pt, linear};
use crate::rules::rulesets::ChannelShape;

/// Settings of `join_legs`. `JoinParams::new(r_min)` gives the defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinParams {
    pub r_min: f64,
    /// The largest fillet tried (m); default 8 R_min.
    pub r_ceiling_m: f64,
    /// The chord radius is read on (default R_min/3).
    pub chord_m: f64,
    pub clearance_m: f64,
    pub smooth_m: f64,
    /// Half length of a fairway-fit window, per radian of turn, as a multiple of R_min (L5:
    /// 2 gives about 2.6 km for a 90° join at R_min 420 m).
    pub blend_per_rad: f64,
    /// How closely the largest fillet radius is found (m).
    pub radius_step_m: f64,
    /// Turns smaller than this (radians) are not joins.
    pub min_turn_rad: f64,
    /// The weight on a gentle bend when the guard refits a window (the preferred-radius
    /// penalty of the fairway fit).
    pub guard_weight: f64,
}

impl JoinParams {
    pub fn new(r_min: f64) -> Self {
        JoinParams {
            r_min,
            r_ceiling_m: 8.0 * r_min,
            chord_m: default_check_chord(r_min),
            clearance_m: 0.0,
            smooth_m: 0.0,
            blend_per_rad: 2.0,
            radius_step_m: 0.5,
            min_turn_rad: 3f64.to_radians(),
            guard_weight: 100.0,
        }
    }
}

/// How a join was made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JoinMethod {
    /// Under the minimum turn: left as it is.
    Straight,
    /// A fillet of this radius (m).
    Fillet(f64),
    /// No fillet fits: a fairway-fit window.
    FairwayFit,
}

/// One join: where it is (by position), its turn, how it was made, and whether it held R_min.
#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub at: Pt,
    pub turn_rad: f64,
    pub method: JoinMethod,
    pub held_r_min: bool,
}

/// The joined line and what it achieved.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinResult {
    pub line: Vec<Pt>,
    pub joins: Vec<Join>,
    pub infeasible: Vec<Stretch>,
    /// Joins that held R_min, failed after a later fit, and stayed failed after the guard's
    /// refit: the failed `join_guard` check.
    pub guard_failed: Vec<Pt>,
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    pub radius_ok: bool,
}

/// Join ordered seed legs. See the module notes.
pub fn join_legs(
    legs: &[Vec<Pt>],
    water: &[Polygon],
    shape: &ChannelShape,
    fixed: &Fixed,
    params: &JoinParams,
) -> Result<JoinResult, PyError> {
    if legs.is_empty() || legs.iter().any(|l| l.len() < 2) {
        return Err(PyError::value("each seed leg needs at least two points."));
    }
    shape
        .ruleset()
        .validate()
        .map_err(|e| PyError::value(e.raw_message().to_string()))?;
    let mut line: Vec<Pt> = Vec::new();
    let mut at: Vec<Pt> = Vec::new();
    for (k, leg) in legs.iter().enumerate() {
        for &p in leg {
            if line.last() != Some(&p) {
                line.push(p);
            }
        }
        if k + 1 < legs.len() {
            at.push(leg[leg.len() - 1]);
        }
    }
    let mut ctx = Ctx::new(water, shape, params);
    let mut joins = Vec::new();
    for (k, &p) in at.iter().enumerate() {
        // The room either side: up to the neighbouring joins (by position) or the line's ends.
        let s = linear::project(&line, p);
        // A neighbouring join shares the room between them; a line end gives all of it.
        let room_in = match k {
            0 => s,
            _ => (s - linear::project(&line, at[k - 1])) / 2.0,
        };
        let room_out = match at.get(k + 1) {
            None => linear::length(&line) - s,
            Some(q) => (linear::project(&line, *q) - s) / 2.0,
        };
        let (j, new_line) = ctx.join(&line, s, room_in, room_out, p)?;
        line = new_line;
        joins.push(j);
    }
    let mut infeasible = Vec::new();
    let mut guard_failed = Vec::new();
    let has_fixed =
        fixed.start_pose.is_some() || fixed.end_pose.is_some() || !fixed.must_follow.is_empty();
    if has_fixed {
        let fit = fairway_fit_in_water(
            &line,
            water,
            shape,
            params.clearance_m,
            params.smooth_m,
            fixed,
            &ctx.fit_params(),
        )?;
        let (guarded, failed) = guard_joins(&fit.line, &joins, water, shape, params)?;
        line = guarded;
        guard_failed = failed;
        infeasible = fit.infeasible;
    }
    // The checks on the final line, inside each join's window, added to the fit's.
    let fin = Chained::new(&line);
    let mut found = width_check(&line, water, shape, params.r_min, params.chord_m)?;
    let straight = ctx.nav_for(f64::INFINITY)?.clone();
    let status = |s: f64| {
        (!straight.covers(fin.at(s), 0.05)).then_some(super::navigable::StretchReason::TooNarrow)
    };
    found.extend(super::fairway_fit::stretches(
        &fin,
        fin.length(),
        5.0,
        &status,
    ));
    for j in &joins {
        let s = linear::project(&line, j.at);
        let h = ctx.window_half(j.turn_rad);
        found
            .iter()
            .filter(|st| st.end_chainage_m > s - h && st.start_chainage_m < s + h)
            .for_each(|st| infeasible.push(st.clone()));
    }
    let infeasible = merge_stretches(&line, infeasible);
    let measured = min_radius_on_chord(&line, params.chord_m)?;
    Ok(JoinResult {
        line,
        joins,
        infeasible,
        guard_failed,
        measured_min_radius_m: measured,
        radius_check_chord_m: params.chord_m,
        radius_ok: radius_ok(measured, params.r_min),
    })
}

/// The join guard: re-read, by position, each join of `joins` that held R_min on `line` (the
/// line after a later fit). A join that now reads tighter than R_min on the chord is refit over
/// its window with a heavy preferred-radius weight; one that still fails is returned in the
/// second list. Returns the guarded line.
pub fn guard_joins(
    line: &[Pt],
    joins: &[Join],
    water: &[Polygon],
    shape: &ChannelShape,
    params: &JoinParams,
) -> Result<(Vec<Pt>, Vec<Pt>), PyError> {
    let ctx = Ctx::new(water, shape, params);
    let mut line = line.to_vec();
    let mut failed = Vec::new();
    for j in joins.iter().filter(|j| j.held_r_min) {
        if ctx.holds(&line, j.at, j.turn_rad) {
            continue;
        }
        let c = Chained::new(&line);
        let s = linear::project(&line, j.at);
        let h = ctx.window_half(j.turn_rad);
        let (s0, s1) = ((s - h).max(0.0), (s + h).min(c.length()));
        let mut fp = ctx.fit_params();
        fp.r_pref_m = Some(match j.method {
            JoinMethod::Fillet(r) => r,
            _ => 2.0 * params.r_min,
        });
        fp.pref_weight = params.guard_weight;
        let fit = fairway_fit_in_water(
            &c.slice(s0, s1),
            water,
            shape,
            params.clearance_m,
            params.smooth_m,
            &ctx.poses(&c, s0, s1),
            &fp,
        )?;
        let refit = splice(&c, s0, s1, &fit.line);
        if ctx.holds(&refit, j.at, j.turn_rad) {
            line = refit;
        } else {
            failed.push(j.at);
        }
    }
    Ok((line, failed))
}

/// Shared state: the water, the settings, and navigable water per radius.
struct Ctx<'a> {
    water: &'a [Polygon],
    shape: &'a ChannelShape,
    p: &'a JoinParams,
    nav: Vec<(f64, EdgeGrid)>,
}

impl<'a> Ctx<'a> {
    fn new(water: &'a [Polygon], shape: &'a ChannelShape, p: &'a JoinParams) -> Self {
        Ctx {
            water,
            shape,
            p,
            nav: Vec::new(),
        }
    }

    fn fit_params(&self) -> FitParams {
        let mut fp = FitParams::new(self.p.r_min);
        fp.chord_m = self.p.chord_m;
        fp
    }

    /// Half length of a join's fairway-fit window (L5).
    fn window_half(&self, turn: f64) -> f64 {
        (self.p.blend_per_rad * self.p.r_min * turn.abs()).max(self.p.r_min)
    }

    fn nav_for(&mut self, r: f64) -> Result<&EdgeGrid, PyError> {
        if let Some(i) = self.nav.iter().position(|(rr, _)| *rr == r) {
            return Ok(&self.nav[i].1);
        }
        let half = self.shape.bottom_width_at_m(r) / 2.0;
        let polys = navigable_water(self.water, half, self.p.clearance_m, self.p.smooth_m)?;
        self.nav.push((r, EdgeGrid::new(&polys)));
        Ok(&self.nav.last().unwrap().1)
    }

    /// Whether a fillet piece keeps a channel widened for `r` in the water.
    fn fits(&mut self, piece: &[Pt], r: f64) -> Result<bool, PyError> {
        let grid = self.nav_for(r)?;
        let c = Chained::new(piece);
        let n = (c.length() / 5.0).ceil() as usize;
        let inside = (0..=n).all(|i| grid.covers(c.at((i as f64 * 5.0).min(c.length())), 0.05));
        Ok(inside
            && width_check(piece, self.water, self.shape, self.p.r_min, self.p.chord_m)?.is_empty())
    }

    /// Whether the radius read on the chord holds through a join's window on `line`.
    fn holds(&self, line: &[Pt], at: Pt, turn: f64) -> bool {
        let c = Chained::new(line);
        let s = linear::project(line, at);
        let h = (self.window_half(turn) / 2.0).max(self.p.chord_m);
        let (a, b) = ((s - h).max(0.0), (s + h).min(c.length()));
        let n = ((b - a) / 5.0).ceil() as usize;
        (0..=n).all(|i| {
            let x = (a + i as f64 * 5.0).min(b);
            radius_ok(c.local_radius(x, self.p.chord_m), self.p.r_min)
        })
    }

    /// The poses of `c` at `s0` and `s1`, as fixed ends of a window (none at the line's ends).
    fn poses(&self, c: &Chained, s0: f64, s1: f64) -> Fixed {
        let pose = |s: f64| {
            let d = c.direction(s, 5.0);
            Pose {
                at: c.at(s),
                heading_rad: libm::atan2(d[1], d[0]),
            }
        };
        Fixed {
            start_pose: (s0 > 0.0).then(|| pose(s0)),
            end_pose: (s1 < c.length()).then(|| pose(s1)),
            must_follow: vec![],
        }
    }

    /// One join at chainage `s` of `line`, with `room_in` and `room_out` metres either side.
    fn join(
        &mut self,
        line: &[Pt],
        s: f64,
        room_in: f64,
        room_out: f64,
        at: Pt,
    ) -> Result<(Join, Vec<Pt>), PyError> {
        let c = Chained::new(line);
        let arm = 25.0_f64.min(room_in / 2.0).min(room_out / 2.0).max(1.0);
        let d_in = c.direction(s - arm, arm);
        let d_out = c.direction(s + arm, arm);
        let turn = super::fillet::turn_angle(d_in, d_out);
        if turn.abs() < self.p.min_turn_rad {
            let held = self.holds(line, at, turn);
            return Ok((
                Join {
                    at,
                    turn_rad: turn,
                    method: JoinMethod::Straight,
                    held_r_min: held,
                },
                line.to_vec(),
            ));
        }
        let (s0, s1) = ((s - room_in).max(0.0), (s + room_out).min(c.length()));
        let (r_min, r_ceiling, step) = (self.p.r_min, self.p.r_ceiling_m, self.p.radius_step_m);
        let mut try_r = |r: f64| -> Result<Option<Vec<Pt>>, PyError> {
            let Some(f) = fillet(&c, s0, s1, r, 10.0, 5.0) else {
                return Ok(None);
            };
            Ok(self.fits(&f, r)?.then_some(f))
        };
        let best = if let Some(f) = try_r(r_ceiling)? {
            Some((r_ceiling, f))
        } else if let Some(f) = try_r(r_min)? {
            // Largest radius that fits: bisection between R_min (fits) and the ceiling (not).
            let (mut lo, mut hi, mut best) = (r_min, r_ceiling, f);
            while hi - lo > step {
                let mid = (lo + hi) / 2.0;
                match try_r(mid)? {
                    Some(f) => {
                        lo = mid;
                        best = f;
                    }
                    None => hi = mid,
                }
            }
            Some((lo, best))
        } else {
            None
        };
        if let Some((r, f)) = best {
            return Ok((
                Join {
                    at,
                    turn_rad: turn,
                    method: JoinMethod::Fillet(r),
                    held_r_min: true,
                },
                splice(&c, s0, s1, &f),
            ));
        }
        // No fillet fits: a fairway-fit window, its length growing with the turn (L5).
        let h = self.window_half(turn);
        let (w0, w1) = ((s - h).max(0.0), (s + h).min(c.length()));
        let fit = fairway_fit_in_water(
            &c.slice(w0, w1),
            self.water,
            self.shape,
            self.p.clearance_m,
            self.p.smooth_m,
            &self.poses(&c, w0, w1),
            &self.fit_params(),
        )?;
        let out = splice(&c, w0, w1, &fit.line);
        let held = fit.infeasible.is_empty() && self.holds(&out, at, turn);
        Ok((
            Join {
                at,
                turn_rad: turn,
                method: JoinMethod::FairwayFit,
                held_r_min: held,
            },
            out,
        ))
    }
}

/// `line` with chainages `s0..s1` replaced by `piece` (which runs from the point at `s0` to the
/// point at `s1`).
pub(crate) fn splice(c: &Chained, s0: f64, s1: f64, piece: &[Pt]) -> Vec<Pt> {
    let mut out = if s0 > 0.0 { c.slice(0.0, s0) } else { vec![] };
    out.pop();
    out.extend_from_slice(piece);
    if s1 < c.length() {
        let tail = c.slice(s1, c.length());
        out.extend_from_slice(&tail[1..]);
    }
    out.dedup();
    out
}

/// Tangent joins at the given positions of `line`, in open space (no water limit): each point
/// where two pieces meet with a step or a kink (the end of an optimised pool) gets a fairway-fit
/// window of `R_min` either side, between the line's poses there. Returns the joined line and the
/// heading change across each join after it (radians), read over `arm_m` either side.
pub fn tangent_joins(
    line: &[Pt],
    at: &[Pt],
    r_min: f64,
    arm_m: f64,
) -> Result<(Vec<Pt>, Vec<f64>), PyError> {
    let mut line = line.to_vec();
    for &p in at {
        let c = Chained::new(&line);
        let s = linear::project(&line, p);
        let (s0, s1) = ((s - r_min).max(0.0), (s + r_min).min(c.length()));
        if s1 - s0 <= 0.0 {
            continue;
        }
        let piece = c.slice(s0, s1);
        // Open space: a box well around the window.
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for q in &piece {
            for k in 0..2 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        let m = 4.0 * r_min;
        let open = Polygon {
            exterior: vec![
                [lo[0] - m, lo[1] - m],
                [hi[0] + m, lo[1] - m],
                [hi[0] + m, hi[1] + m],
                [lo[0] - m, hi[1] + m],
                [lo[0] - m, lo[1] - m],
            ],
            holes: vec![],
        };
        let d0 = c.direction(s0 - 5.0, 5.0);
        let d1 = c.direction(s1 + 5.0, 5.0);
        let fixed = Fixed {
            start_pose: (s0 > 0.0).then_some(Pose {
                at: piece[0],
                heading_rad: libm::atan2(d0[1], d0[0]),
            }),
            end_pose: (s1 < c.length()).then_some(Pose {
                at: piece[piece.len() - 1],
                heading_rad: libm::atan2(d1[1], d1[0]),
            }),
            must_follow: vec![],
        };
        let mut fp = FitParams::new(r_min);
        fp.levels.retain(|l| l.spacing_m <= 50.0);
        let fit = fairway_fit(&piece, &[open], &fixed, &fp)?;
        line = splice(&c, s0, s1, &fit.line);
    }
    let c = Chained::new(&line);
    let errors = at
        .iter()
        .map(|&p| {
            let s = linear::project(&line, p);
            let a = c.direction(s - arm_m, arm_m);
            let b = c.direction(s + arm_m, arm_m);
            super::fillet::turn_angle(a, b).abs()
        })
        .collect();
    Ok((line, errors))
}

/// `optimise_pools`, then [`tangent_joins`] at both ends of every pool it spliced in, so each
/// pool meets the line tangentially (closes the straight steps of up to `goal_pos_tol_m`).
/// Returns the pools result with the joined line, and the heading change across each join.
#[allow(clippy::too_many_arguments)]
pub fn optimise_pools_tangent(
    baseline: &[Pt],
    terrain: crate::volumes::sections::TerrainSampler,
    river_polygon: &[Polygon],
    pools: &[(f64, f64, f64)],
    depth_m: f64,
    section: &crate::optimise::lattice::SectionSpec,
    cfg: &crate::optimise::lattice::LatticeConfig,
    corridor_line: Option<&[Pt]>,
    opts: crate::optimise::lattice::PoolOptions,
    keep_if_dearer: bool,
) -> Result<(crate::optimise::lattice::PoolsResult, Vec<f64>), PyError> {
    let mut res = crate::optimise::lattice::optimise_pools(
        baseline,
        terrain,
        river_polygon,
        pools,
        depth_m,
        section,
        cfg,
        corridor_line,
        opts,
        keep_if_dearer,
    )
    .map_err(PyError::value)?;
    let mut ends = Vec::new();
    for pa in &res.pools {
        let used = pa.result.converged && !(keep_if_dearer && pa.result.cost > pa.baseline_cost);
        if let (true, Some(l)) = (used, pa.result.line.as_ref())
            && l.len() >= 2
        {
            ends.push(l[0]);
            ends.push(l[l.len() - 1]);
        }
    }
    let (line, errors) = tangent_joins(&res.line, &ends, cfg.r_min_m, 2.0)?;
    res.line = line;
    Ok((res, errors))
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (the open basin's largest fillet, with the channel
    //! widened for its own radius: `R = A - B/R`, derived below); property (guard, positions,
    //! pool tangency).
    use super::*;
    use crate::alignment::fillet::fillet_between;
    use crate::rules::rulesets::{RulesetRegistry, channel_shape};
    use crate::synthetic::{fixed_tail, open_basin_90};

    const R_MIN: f64 = 255.0;

    fn shape() -> ChannelShape {
        channel_shape(&RulesetRegistry::new(), "PIANC-B", "CEMT_IV", Some(2.5)).unwrap()
    }

    /// The open basin's largest fillet when the shrink is half the bottom width widened for the
    /// fillet's own radius: b(R) = 15.2 + 0.5·85²/(2R) (PIANC-B, c_C = 0.5, LOA 85 m). With
    /// R = (1 + √2)(√2·c − b(R)): R = A − B/R, A = (1 + √2)(√2·c − 15.2), B = (1 + √2)·1806.25.
    fn basin_radius(c: f64) -> f64 {
        let k = 1.0 + core::f64::consts::SQRT_2;
        let a = k * (core::f64::consts::SQRT_2 * c - 15.2);
        let b = k * 1806.25;
        (a + (a * a - 4.0 * b).sqrt()) / 2.0
    }

    fn basin_join(c: f64) -> (JoinResult, crate::synthetic::WaterFixture) {
        let f = open_basin_90(1500.0, c);
        let r = join_legs(
            &f.legs,
            core::slice::from_ref(&f.water),
            &shape(),
            &Fixed::default(),
            &JoinParams::new(R_MIN),
        )
        .unwrap();
        (r, f)
    }

    #[test]
    fn the_open_basin_gets_one_fillet_at_the_largest_radius_it_allows() {
        let (r, _) = basin_join(200.0);
        let want = basin_radius(200.0);
        assert!((want - 639.3).abs() < 0.1, "{want}");
        assert_eq!(r.joins.len(), 1);
        let JoinMethod::Fillet(got) = r.joins[0].method else {
            panic!("{:?}", r.joins[0]);
        };
        assert!((got - want).abs() < 1.0, "fillet {got} vs {want}");
        assert!(got >= R_MIN && r.joins[0].held_r_min);
        assert!(r.radius_ok && r.infeasible.is_empty(), "{:?}", r.infeasible);
    }

    #[test]
    fn a_join_with_no_room_for_a_fillet_gets_a_fairway_fit_window() {
        // Land corner at (60, -60): even R_min cuts it (R_max about 150 m).
        assert!(basin_radius(60.0).is_nan() || basin_radius(60.0) < R_MIN);
        let (r, _) = basin_join(60.0);
        assert_eq!(r.joins.len(), 1);
        assert_eq!(r.joins[0].method, JoinMethod::FairwayFit);
    }

    /// The basin join's line with its fillet replaced by a tighter one (R 150 m): what a bad later
    /// fit would leave.
    fn tightened(line: &[Pt]) -> Vec<Pt> {
        let c = Chained::new(line);
        let s = linear::project(line, [0.0, 0.0]);
        let (s0, s1) = (s - 900.0, s + 900.0);
        let f = fillet_between(c.at(s0), [0.0, 1.0], c.at(s1), [1.0, 0.0], 150.0, 5.0).unwrap();
        splice(&c, s0, s1, &f)
    }

    #[test]
    fn the_guard_restores_a_join_a_later_fit_tightened_or_flags_it() {
        let (r, f) = basin_join(200.0);
        let water = core::slice::from_ref(&f.water);
        let p = JoinParams::new(R_MIN);
        let sh = shape();
        let ctx = Ctx::new(water, &sh, &p);
        let bad = tightened(&r.line);
        // The tightening is real: without the guard the join no longer holds R_min.
        assert!(!ctx.holds(&bad, r.joins[0].at, r.joins[0].turn_rad));
        let (guarded, failed) = guard_joins(&bad, &r.joins, water, &shape(), &p).unwrap();
        assert!(
            ctx.holds(&guarded, r.joins[0].at, r.joins[0].turn_rad)
                || failed == vec![r.joins[0].at]
        );
        // In open water the refit restores it.
        assert!(failed.is_empty());
        let rad = min_radius_on_chord(&guarded, p.chord_m).unwrap();
        assert!(radius_ok(rad, R_MIN), "{rad}");
    }

    #[test]
    fn the_guard_finds_joins_by_position_after_the_line_is_shortened() {
        let (r, f) = basin_join(200.0);
        let water = core::slice::from_ref(&f.water);
        let p = JoinParams::new(R_MIN);
        let bad = tightened(&r.line);
        // Cut 500 m off the start: every chainage after it moves by 500 m.
        let c = Chained::new(&bad);
        let short = c.slice(500.0, c.length());
        let old_s = linear::project(&bad, r.joins[0].at);
        let new_s = linear::project(&short, r.joins[0].at);
        assert!((old_s - new_s - 500.0).abs() < 1.0);
        // Read at the old chainage, the shortened line is straight there: a chainage reading
        // would miss the tight join. By position the guard finds it and restores it.
        let cs = Chained::new(&short);
        assert!(cs.local_radius(old_s + 300.0, p.chord_m) > 1e6);
        let (guarded, failed) = guard_joins(&short, &r.joins, water, &shape(), &p).unwrap();
        assert!(failed.is_empty());
        let sh = shape();
        let ctx = Ctx::new(water, &sh, &p);
        assert!(ctx.holds(&guarded, r.joins[0].at, r.joins[0].turn_rad));
        assert!(!ctx.holds(&short, r.joins[0].at, r.joins[0].turn_rad));
    }

    #[test]
    fn fixed_pieces_are_held_through_the_join() {
        let f = fixed_tail();
        let fixed = Fixed {
            must_follow: f.fixed.clone(),
            ..Fixed::default()
        };
        let legs = vec![
            vec![[0.0, 0.0], [1000.0, 0.0]],
            vec![[1000.0, 0.0], [2000.0, 0.0]],
        ];
        let r = join_legs(
            &legs,
            core::slice::from_ref(&f.water),
            &shape(),
            &fixed,
            &JoinParams::new(R_MIN),
        )
        .unwrap();
        let n = r.line.len();
        assert_eq!(&r.line[n - 2..], &f.fixed[0][..]);
        assert_eq!(r.joins[0].method, JoinMethod::Straight);
        assert!(r.guard_failed.is_empty() && r.radius_ok);
    }

    #[test]
    fn tangent_joins_close_a_step_and_a_kink() {
        // A pool's lattice line ending 3 m off the baseline: a straight step, then the baseline.
        let line = vec![[0.0, 0.0], [1000.0, 0.0], [1003.0, 2.0], [2000.0, 2.0]];
        let before = Chained::new(&line);
        let s = linear::project(&line, [1000.0, 0.0]);
        let kink = super::super::fillet::turn_angle(
            before.direction(s - 2.0, 2.0),
            before.direction(s + 2.0, 2.0),
        );
        assert!(kink.abs() > 30f64.to_radians());
        let (out, errors) = tangent_joins(&line, &[[1000.0, 0.0]], R_MIN, 2.0).unwrap();
        assert!(errors[0] < 1f64.to_radians(), "{}", errors[0].to_degrees());
        let rad = min_radius_on_chord(&out, R_MIN / 3.0).unwrap();
        assert!(radius_ok(rad, R_MIN), "{rad}");
    }
}

#[cfg(test)]
mod pool_tests {
    //! Label: property (each spliced pool meets the line tangentially)
    use super::*;
    use crate::optimise::lattice::{HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec};
    use crate::synthetic::{RiverParams, synthetic_river};

    #[test]
    fn optimised_pools_meet_the_line_tangentially() {
        let params = RiverParams {
            length_m: 3000.0,
            wavelength_m: 2400.0,
            amplitude_m: 120.0,
            hill_at: [1500.0, -60.0],
            hill_height_m: 10.0,
            hill_radius_m: 80.0,
            ..RiverParams::default()
        };
        let river = synthetic_river(params).unwrap();
        let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
            xs.iter()
                .zip(ys)
                .map(|(&x, &y)| river.terrain(x, y))
                .collect()
        };
        let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(15.2));
        spec.side_slope_h_per_v = 3.0;
        spec.dry_weight = 1.5;
        spec.rake_half_m = 80.0;
        spec.rake_step_m = 5.0;
        spec.ds_step_m = 10.0;
        let mut lat = LatticeConfig::new(255.0);
        lat.pitch_m = 20.0;
        lat.n_heading = 24;
        lat.radius_multiples = vec![1.0, 2.0, 4.0];
        let lat = lat.validated().unwrap();
        let (res, errors) = optimise_pools_tangent(
            &river.centreline,
            &terrain,
            core::slice::from_ref(&river.polygon),
            &[(500.0, 2500.0, 1.0)],
            3.25,
            &spec,
            &lat,
            None,
            PoolOptions {
                corridor_half_m: 180.0,
                cell_m: 5.0,
            },
            false,
        )
        .unwrap();
        assert!(res.n_replaced() >= 1, "no pool was spliced in");
        // Without the tangent joins, the same reading finds the straight step's kink.
        let plain = crate::optimise::lattice::optimise_pools(
            &river.centreline,
            &terrain,
            core::slice::from_ref(&river.polygon),
            &[(500.0, 2500.0, 1.0)],
            3.25,
            &spec,
            &lat,
            None,
            PoolOptions {
                corridor_half_m: 180.0,
                cell_m: 5.0,
            },
            false,
        )
        .unwrap();
        let pl = plain.pools[0].result.line.as_ref().unwrap();
        let c = Chained::new(&plain.line);
        let worst = [pl[0], pl[pl.len() - 1]]
            .iter()
            .map(|&p| {
                let s = linear::project(&plain.line, p);
                crate::alignment::fillet::turn_angle(
                    c.direction(s - 2.0, 2.0),
                    c.direction(s + 2.0, 2.0),
                )
                .abs()
            })
            .fold(0.0, f64::max);
        assert!(
            worst > 1f64.to_radians(),
            "plain pools already tangent: {}",
            worst.to_degrees()
        );
        assert_eq!(errors.len(), 2 * res.n_replaced());
        for e in &errors {
            assert!(
                *e < 1f64.to_radians(),
                "heading change {} deg",
                e.to_degrees()
            );
        }
    }
}
