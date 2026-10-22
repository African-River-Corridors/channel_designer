//! Joined legs (spec 0005 R2-Q4, L7; ADR 0007) and the join guard (R2-Q2, L8).
//!
//! `join_legs` joins ordered seed legs into one design line with **one corridor per route**: the
//! legs, end to end, are the seed of a single fairway fit (a minimum-curvature corridor solve)
//! over the whole route, with its fixed pieces (a start pose, an end pose, must-follow segments).
//! A join is then just a bend inside the corridor: there are no fillets to splice and no corners
//! where pieces meet. Each join is read, by position (L7), on the final line: it holds R_min when
//! the radius on the chord holds through its window and no infeasible stretch touches it.
//!
//! **The join guard** (`guard_joins`) is kept for lines changed after a fit: each join that held
//! R_min is read again; if it fails, that window is refit with more weight on a gentle bend; if it
//! still fails, the join is named (the option's failed `join_guard` check). `join_legs` itself
//! needs no guard, since nothing fits the line after its one corridor solve.
//!
//! The same machinery closes the straight steps between optimised pools tangentially
//! (`tangent_joins`, `optimise_pools_tangent`).

use super::PyError;
use super::fairway_fit::{
    FitParams, Fixed, Pose, fairway_fit, fairway_fit_in_water, merge_stretches,
};
use super::navigable::{Chained, Stretch};
use super::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::{Polygon, Pt, linear};
use crate::rules::rulesets::ChannelShape;

/// Settings of `join_legs`. `JoinParams::new(r_min)` gives the defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinParams {
    pub r_min: f64,
    /// The chord radius is read on (default R_min/3).
    pub chord_m: f64,
    pub clearance_m: f64,
    pub smooth_m: f64,
    /// Half length of a join's window, per radian of turn, as a multiple of R_min (L5: 2 gives
    /// about 2.6 km for a 90° join at R_min 420 m). A join is read over half of it.
    pub blend_per_rad: f64,
    /// Turns smaller than this (radians) are not joins.
    pub min_turn_rad: f64,
    /// The weight on a gentle bend when the guard refits a window (the preferred-radius
    /// weight of the fairway fit).
    pub guard_weight: f64,
}

impl JoinParams {
    pub fn new(r_min: f64) -> Self {
        JoinParams {
            r_min,
            chord_m: default_check_chord(r_min),
            clearance_m: 0.0,
            smooth_m: 0.0,
            blend_per_rad: 2.0,
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
    /// A fillet of this radius (m). `join_legs` no longer makes fillets (ADR 0007); the variant
    /// stays so a recorded join still reads.
    Fillet(f64),
    /// A bend inside the route's corridor (the fairway fit).
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
    /// refit: the failed `join_guard` check. Always empty from `join_legs`, whose one corridor
    /// solve is the last fit.
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
    let mut seed: Vec<Pt> = Vec::new();
    let mut at: Vec<Pt> = Vec::new();
    for (k, leg) in legs.iter().enumerate() {
        for &p in leg {
            if seed.last() != Some(&p) {
                seed.push(p);
            }
        }
        if k + 1 < legs.len() {
            at.push(leg[leg.len() - 1]);
        }
    }
    // The turn at each join, read on the seed across the room either side (up to the
    // neighbouring joins, by position, or the line's ends).
    let c = Chained::new(&seed);
    let turns: Vec<f64> = at
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let s = linear::project(&seed, p);
            let room_in = match k {
                0 => s,
                _ => (s - linear::project(&seed, at[k - 1])) / 2.0,
            };
            let room_out = match at.get(k + 1) {
                None => c.length() - s,
                Some(q) => (linear::project(&seed, *q) - s) / 2.0,
            };
            let arm = 25.0_f64.min(room_in / 2.0).min(room_out / 2.0).max(1.0);
            super::fillet::turn_angle(c.direction(s - arm, arm), c.direction(s + arm, arm))
        })
        .collect();
    let ctx = Ctx::new(params);
    // One corridor for the whole route.
    let fit = fairway_fit_in_water(
        &seed,
        water,
        shape,
        params.clearance_m,
        params.smooth_m,
        fixed,
        &ctx.fit_params(),
    )?;
    let line = fit.line;
    let infeasible = merge_stretches(&line, fit.infeasible);
    let joins = at
        .iter()
        .zip(&turns)
        .map(|(&p, &turn)| {
            let s = linear::project(&line, p);
            let h = ctx.window_half(turn) / 2.0;
            let clear = !infeasible
                .iter()
                .any(|st| st.end_chainage_m > s - h && st.start_chainage_m < s + h);
            Join {
                at: p,
                turn_rad: turn,
                method: if turn.abs() < params.min_turn_rad {
                    JoinMethod::Straight
                } else {
                    JoinMethod::FairwayFit
                },
                held_r_min: clear && ctx.holds(&line, p, turn),
            }
        })
        .collect();
    let measured = min_radius_on_chord(&line, params.chord_m)?;
    Ok(JoinResult {
        line,
        joins,
        infeasible,
        guard_failed: vec![],
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
    let ctx = Ctx::new(params);
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

/// The settings, and the readings made with them.
struct Ctx<'a> {
    p: &'a JoinParams,
}

impl<'a> Ctx<'a> {
    fn new(p: &'a JoinParams) -> Self {
        Ctx { p }
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
        let fp = FitParams::new(r_min);
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
    use crate::synthetic::{fixed_tail, open_basin_90};

    const R_MIN: f64 = 255.0;

    fn shape() -> ChannelShape {
        crate::rules::rulesets::channel_shape_for(
            &crate::rules::engine::selection_for_old_ruleset("PIANC-B").unwrap(),
            "CEMT_IV",
            Some(2.5),
        )
        .unwrap()
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

    /// The largest step of curvature (1/m) between stations 10 m apart, read through points 10 m
    /// either side, on a line with continuous curvature (as in the fairway fit's tests). A
    /// fillet's ends read about 1/R across two stations.
    const KINK_STEP: f64 = 0.05 / R_MIN;

    /// The basin's seed with the corner replaced by a fillet of radius `r` (tangent to both legs).
    fn filleted(r: f64) -> Vec<Pt> {
        let seed = vec![[0.0, -1500.0], [0.0, 0.0], [1500.0, 0.0]];
        let c = Chained::new(&seed);
        let f = fillet_between([0.0, -r], [0.0, 1.0], [r, 0.0], [1.0, 0.0], r, 5.0).unwrap();
        splice(&c, 1500.0 - r, 1500.0 + r, &f)
    }

    #[test]
    fn the_open_basin_is_one_smooth_corridor_at_least_as_wide_as_its_largest_fillet() {
        let (r, _) = basin_join(200.0);
        let fillet_r = basin_radius(200.0);
        assert!((fillet_r - 639.3).abs() < 0.1, "{fillet_r}");
        assert_eq!(r.joins.len(), 1);
        assert_eq!(r.joins[0].method, JoinMethod::FairwayFit);
        assert!(r.joins[0].held_r_min && r.guard_failed.is_empty());
        assert!(r.radius_ok && r.infeasible.is_empty(), "{:?}", r.infeasible);
        // Free to use the open water around the join, the corridor's tightest bend is at least
        // the largest fillet's.
        assert!(
            radius_ok(r.measured_min_radius_m, fillet_r),
            "{} vs {fillet_r}",
            r.measured_min_radius_m
        );
        // One smooth line through the join: no step in curvature.
        let step = crate::alignment::fairway_fit::max_curvature_step(&r.line, 10.0, 10.0);
        assert!(step <= KINK_STEP, "{step:.2e}");
        // The check fails on the largest fillet itself: a corner in curvature at each end.
        let step =
            crate::alignment::fairway_fit::max_curvature_step(&filleted(fillet_r), 10.0, 10.0);
        assert!(step > KINK_STEP, "{step:.2e}");
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
        let ctx = Ctx::new(&p);
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
        let ctx = Ctx::new(&p);
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
