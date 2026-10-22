//! The reference line (spec 0016 T1): one smooth line in the water that belongs to no vessel.
//!
//! **The seed** is a smoothed mid-line of the two banks: both banks resampled every
//! [`SEED_STEP_M`], each point paired with a point on the other bank moving forward only (at most
//! [`SEED_MAX_STEPS_AHEAD`] steps), the mid-points of both directions blended by share of length,
//! then [`CHAIKIN_PASSES`] of Chaikin corner cutting and Taubin smoothing ([`taubin_smoothing`]).
//! Taubin alternates a shrinking and a slightly larger growing step, so it takes out the wiggle of
//! the banks without the shrink of a moving average.
//!
//! **The refit** runs the fairway fit (ADR 0007) on the seed in the water, with a bank clearance of
//! the larger of [`CLEARANCE_MIN_M`] and [`CLEARANCE_SHARE`] of the local width, no minimum radius
//! (the line only has to be smooth), and one pull towards the seed ([`SEED_PULL_WEIGHT`]), so the
//! line stays central and does not hug the inside of each bend. Each end is then cut back by
//! [`END_TRIM_M`], so the ends lie inside the water, not on the polygon's end edges.
//!
//! The line takes no vessel input: one line serves every vessel. No file, clock or network access.

use super::PyError;
use super::centreline::{
    compute_paired_centreline, ensure_orientation, resample_polyline, trim_polyline,
};
use super::fairway_fit::{Clearance, FitParams, Fixed, fairway_fit};
use super::navigable::{Chained, Stretch, StretchReason};
use crate::geom::{Polygon, Pt, dist, linear};

/// The resampling step of the banks and of the seed (m).
pub const SEED_STEP_M: f64 = 20.0;
/// How many steps ahead a bank point may pair on the other bank.
pub const SEED_MAX_STEPS_AHEAD: f64 = 5.0;
/// Chaikin corner-cutting passes on the blended mid-line.
pub const CHAIKIN_PASSES: i64 = 2;
/// Taubin smoothing: the shrinking step, the growing step and the iterations.
pub const TAUBIN_LAMBDA: f64 = 0.5;
pub const TAUBIN_MU: f64 = -0.53;
pub const TAUBIN_ITERATIONS: usize = 60;
/// The bank clearance: the larger of this (m) ...
pub const CLEARANCE_MIN_M: f64 = 5.0;
/// ... and this share of the local width.
pub const CLEARANCE_SHARE: f64 = 0.10;
/// The weight of the pull towards the seed in the refit: an offset of one chord
/// ([`REFIT_CHORD_M`]) from the seed costs as much as this, per 10 m of line.
///
/// Label: tuned (spec 0016 T1, 2026-10-22) with `tests/reference_line_tuning.rs` on two windows
/// of a 179 km West African river: 20 km of narrow, winding river (60 to 200 m wide) and 12 km
/// of wider river (300 to 400 m). With no pull the line hugs the banks (off-centre share 0.37
/// and 0.40); at 100 it keeps the seed's wiggle (7.0 and 6.2 curvature sign changes per km,
/// against the seed's 7.1 and 6.4). At 0.3 the sign changes halve (3.6 and 3.1 per km) and the
/// line stays central (off-centre share 0.07 and 0.05; 0 is the mid-line, 1 a bank).
pub const SEED_PULL_WEIGHT: f64 = 0.3;
/// How much is cut from each end of the refitted line (m). The banks end on the river polygon's
/// end edges, so the mid-line's ends lie on the edge of the water, where nothing reads as water.
pub const END_TRIM_M: f64 = CLEARANCE_MIN_M;
/// The length that scales the curvature cost in the refit (m). The refit has no minimum radius:
/// this only sets the balance with the pull towards the seed.
pub const REFIT_SCALE_M: f64 = 100.0;
/// The chord of the refit (m): the narrowest box is taken within half of it, and the pull is
/// measured in it.
pub const REFIT_CHORD_M: f64 = 20.0;

/// Taubin λ|μ smoothing, the end points pinned: each iteration moves every inner point `λ` of the
/// way to its neighbours' mean, then `μ` (negative: back out). Wants even spacing.
pub fn taubin_smoothing(points: &[Pt], iterations: usize, lambda: f64, mu: f64) -> Vec<Pt> {
    let mut pts = points.to_vec();
    let n = pts.len();
    if n < 3 {
        return pts;
    }
    for _ in 0..iterations {
        for f in [lambda, mu] {
            let prev = pts.clone();
            for i in 1..n - 1 {
                for k in 0..2 {
                    let lap = 0.5 * (prev[i - 1][k] + prev[i + 1][k]) - prev[i][k];
                    pts[i][k] = prev[i][k] + f * lap;
                }
            }
        }
    }
    pts
}

/// The seed: the smoothed mid-line of the banks (see the module notes), every [`SEED_STEP_M`].
pub fn seed_mid_line(left: &[Pt], right: &[Pt]) -> Result<Vec<Pt>, PyError> {
    let l = resample_polyline(left, SEED_STEP_M).map_err(PyError::value)?;
    let r = resample_polyline(right, SEED_STEP_M).map_err(PyError::value)?;
    let (l, r) = ensure_orientation(&l, &r);
    // The pairing looks `5 × step` ahead, so `SEED_MAX_STEPS_AHEAD` steps.
    debug_assert_eq!(SEED_MAX_STEPS_AHEAD, 5.0);
    let blended = compute_paired_centreline(&l, &r, SEED_STEP_M, 1, CHAIKIN_PASSES)
        .map_err(PyError::value)?;
    let smooth = taubin_smoothing(&blended, TAUBIN_ITERATIONS, TAUBIN_LAMBDA, TAUBIN_MU);
    resample_polyline(&smooth, SEED_STEP_M).map_err(PyError::value)
}

/// The reference line and how it was made.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceLine {
    pub line: Vec<Pt>,
    /// The smoothed mid-line it was refitted from.
    pub seed: Vec<Pt>,
    /// Stretches of the line outside the water (where the water is too narrow to hold it).
    pub outside_water: Vec<Stretch>,
    pub notes: Vec<String>,
}

/// The reference line between `left` and `right` in `water`. See the module notes.
pub fn reference_line(
    left: &[Pt],
    right: &[Pt],
    water: &[Polygon],
) -> Result<ReferenceLine, PyError> {
    let seed = seed_mid_line(left, right)?;
    let params = refit_params(&seed, left, right);
    let fit = fairway_fit(&seed, water, &Fixed::default(), &params)?;
    let line = trim_polyline(&fit.line, END_TRIM_M).map_err(PyError::value)?;
    Ok(ReferenceLine {
        // The stretches are by chainage on the line before the trim.
        outside_water: fit
            .infeasible
            .into_iter()
            .filter(|s| s.reason == StretchReason::TooNarrow)
            .map(|mut s| {
                s.start_chainage_m = (s.start_chainage_m - END_TRIM_M).max(0.0);
                s.end_chainage_m = (s.end_chainage_m - END_TRIM_M).max(0.0);
                s
            })
            .collect(),
        line,
        seed,
        notes: fit.notes,
    })
}

/// [`refit_params`], for the tuning harness (`tests/reference_line_tuning.rs`).
#[doc(hidden)]
pub fn refit_params_for_tuning(seed: &[Pt], left: &[Pt], right: &[Pt]) -> FitParams {
    refit_params(seed, left, right)
}

/// The settings of the refit of `seed` between `left` and `right`.
pub(crate) fn refit_params(seed: &[Pt], left: &[Pt], right: &[Pt]) -> FitParams {
    let mut params = FitParams::new(REFIT_SCALE_M);
    params.chord_m = REFIT_CHORD_M;
    // No minimum radius: no row weighs more for being tight.
    params.reweight_cap = 0.0;
    params.clearance = Some(Clearance {
        min_m: CLEARANCE_MIN_M,
        share: CLEARANCE_SHARE,
    });
    params.seed_pull_weight = SEED_PULL_WEIGHT;
    // A ray must reach across the widest water, or the width (and so the clearance) reads short.
    params.reach_m = params.reach_m.max(widest(seed, left, right) * 1.5 + 10.0);
    params
}

/// The widest gap between the banks across `line` (m): at each point, its distance to the nearer
/// point of each bank, summed.
fn widest(line: &[Pt], left: &[Pt], right: &[Pt]) -> f64 {
    let (li, ri) = (
        linear::ProjectIndex::new(left),
        linear::ProjectIndex::new(right),
    );
    let (lc, rc) = (Chained::new(left), Chained::new(right));
    line.iter()
        .map(|&p| dist(p, lc.at(li.project(p))) + dist(p, rc.at(ri.project(p))))
        .fold(0.0, f64::max)
}

/// Chainage ranges on `from` carried onto `to`: each end is the point at that chainage of `from`,
/// projected onto `to`. Ranges come back in order, each with its start before its end.
pub fn carry_by_chainage(from: &[Pt], to: &[Pt], ranges: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if from.len() < 2 || to.len() < 2 {
        return vec![];
    }
    let (fc, ix) = (Chained::new(from), linear::ProjectIndex::new(to));
    let mut out: Vec<(f64, f64)> = ranges
        .iter()
        .map(|&(a, b)| {
            let (x, y) = (ix.project(fc.at(a)), ix.project(fc.at(b)));
            (x.min(y), x.max(y))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::alignment::fairway_fit::{fairway_fit_in_water, signed_curvature};
    use crate::geom::linear::point_line_distance;
    use crate::rules::custom::{CustomVessel, Formation};
    use crate::rules::engine::selection_for_old_ruleset;
    use crate::rules::rulesets::channel_shape_for_custom;

    /// A small, repeatable wiggle (m) at point `i`: a sum of sines with no common period.
    fn noise(i: usize, amp: f64) -> f64 {
        let t = i as f64;
        amp * (0.6 * libm::sin(1.7 * t) + 0.3 * libm::sin(4.1 * t + 1.0) + 0.1 * libm::sin(9.3 * t))
    }

    /// A channel along a centre curve (points with their left normals), `width` wide, each bank
    /// with a wiggle of `amp` metres. Returns (left bank, right bank, water, true centre).
    fn channel(c: &[(Pt, Pt)], width: f64, amp: f64) -> (Vec<Pt>, Vec<Pt>, Polygon, Vec<Pt>) {
        let h = width / 2.0;
        let left: Vec<Pt> = c
            .iter()
            .enumerate()
            .map(|(i, (p, n))| {
                let o = h + noise(i, amp);
                [p[0] + o * n[0], p[1] + o * n[1]]
            })
            .collect();
        let right: Vec<Pt> = c
            .iter()
            .enumerate()
            .map(|(i, (p, n))| {
                let o = h + noise(i + 7919, amp);
                [p[0] - o * n[0], p[1] - o * n[1]]
            })
            .collect();
        let mut ring = right.clone();
        ring.extend(left.iter().rev());
        ring.push(ring[0]);
        let water = Polygon {
            exterior: ring,
            holes: vec![],
        };
        (left, right, water, c.iter().map(|(p, _)| *p).collect())
    }

    fn straight(len: f64) -> Vec<(Pt, Pt)> {
        (0..=(len / 10.0) as usize)
            .map(|i| ([10.0 * i as f64, 0.0], [0.0, 1.0]))
            .collect()
    }

    /// A meander: y = a sin(2πx/λ), sampled every ~10 m along x.
    fn meander(len: f64, a: f64, lambda: f64) -> Vec<(Pt, Pt)> {
        let k = 2.0 * core::f64::consts::PI / lambda;
        (0..=(len / 10.0) as usize)
            .map(|i| {
                let x = 10.0 * i as f64;
                let d = a * k * libm::cos(k * x);
                let l = libm::hypot(1.0, d);
                ([x, a * libm::sin(k * x)], [-d / l, 1.0 / l])
            })
            .collect()
    }

    /// A 90° bend of centre radius `rc` between two straights of 1000 m.
    fn bend(rc: f64) -> Vec<(Pt, Pt)> {
        let mut c: Vec<(Pt, Pt)> = (0..100)
            .map(|i| ([-1000.0 + 10.0 * i as f64, 0.0], [0.0, 1.0]))
            .collect();
        let n = (rc * core::f64::consts::FRAC_PI_2 / 10.0).ceil() as usize;
        for i in 0..n {
            let t =
                -core::f64::consts::FRAC_PI_2 + core::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
            let (s, co) = (libm::sin(t), libm::cos(t));
            c.push(([rc * co, rc + rc * s], [-co, -s]));
        }
        for i in 0..=100 {
            c.push(([rc, rc + 10.0 * i as f64], [-1.0, 0.0]));
        }
        c
    }

    /// Signed curvature every 10 m, read through points 20 m either side.
    fn curvatures(line: &[Pt]) -> Vec<f64> {
        let c = Chained::new(line);
        let n = (c.length() / 10.0) as usize;
        (2..n.saturating_sub(2))
            .map(|i| signed_curvature(&c, 10.0 * i as f64, 20.0))
            .collect()
    }

    fn sign_changes_per_km(line: &[Pt]) -> f64 {
        let k = curvatures(line);
        let n = k.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        n as f64 / (linear::length(line) / 1000.0)
    }

    fn peak(line: &[Pt]) -> f64 {
        curvatures(line).iter().fold(0.0, |m, k| m.max(k.abs()))
    }

    #[test]
    fn on_a_straight_channel_it_is_the_centreline() {
        let (l, r, w, _) = channel(&straight(2000.0), 100.0, 0.0);
        let rl = reference_line(&l, &r, &[w]).unwrap();
        let off = rl.line.iter().fold(0.0f64, |m, p| m.max(p[1].abs()));
        assert!(off < 0.5, "the line strays {off} m from the centreline");
        assert!(rl.outside_water.is_empty());
    }

    #[test]
    fn its_ends_lie_inside_the_water_not_on_its_edge() {
        // The banks end on the polygon's end edges, so the mid-line's ends lie on them.
        let (l, r, w, _) = channel(&meander(3000.0, 100.0, 1200.0), 100.0, 0.0);
        let rl = reference_line(&l, &r, std::slice::from_ref(&w)).unwrap();
        let grid = crate::geom::grid::EdgeGrid::new(std::slice::from_ref(&w));
        for p in [rl.line[0], rl.line[rl.line.len() - 1]] {
            assert!(grid.contains(p), "an end {p:?} is not inside the water");
            let d = point_line_distance(p, &w.exterior);
            assert!(
                d > END_TRIM_M - 0.5,
                "an end {p:?} is {d:.2} m from the edge"
            );
        }
    }

    #[test]
    fn it_lies_inside_the_water_less_the_clearance() {
        for (c, width) in [(meander(4000.0, 150.0, 1200.0), 80.0), (bend(300.0), 200.0)] {
            let (l, r, w, _) = channel(&c, width, 3.0);
            let rl = reference_line(&l, &r, &[w]).unwrap();
            // The clearance at the narrowest the noisy banks get.
            let need = CLEARANCE_MIN_M.max(CLEARANCE_SHARE * (width - 6.0));
            let ch = Chained::new(&rl.line);
            let n = (ch.length() / 10.0) as usize;
            for i in 0..=n {
                let p = ch.at(10.0 * i as f64);
                let d = point_line_distance(p, &l).min(point_line_distance(p, &r));
                assert!(
                    d >= need - 1.5,
                    "{width} m channel: at {} m the line is {d:.1} m from a bank, under {need:.1} m",
                    10 * i
                );
            }
        }
    }

    #[test]
    fn the_refit_keeps_the_clearance_without_the_pull() {
        // With no pull towards the seed, the smoothest line hugs the inside of the bend: only the
        // clearance holds it off the bank.
        let (l, r, w, _) = channel(&bend(250.0), 200.0, 0.0);
        let seed = seed_mid_line(&l, &r).unwrap();
        let mut params = refit_params(&seed, &l, &r);
        params.seed_pull_weight = 0.0;
        let fit = fairway_fit(&seed, &[w], &Fixed::default(), &params).unwrap();
        let need = CLEARANCE_SHARE * 200.0;
        let near = fit
            .line
            .iter()
            .map(|&p| point_line_distance(p, &l).min(point_line_distance(p, &r)))
            .fold(f64::MAX, f64::min);
        assert!(
            near >= need - 1.5,
            "the line comes {near:.1} m from a bank, under {need} m"
        );
        assert!(
            near < need + 5.0,
            "the clearance does not bind ({near:.1} m): a weak test"
        );
    }

    #[test]
    fn it_is_smoother_than_the_mid_line() {
        let (l, r, w, _) = channel(&meander(6000.0, 200.0, 1500.0), 120.0, 4.0);
        let l20 = resample_polyline(&l, SEED_STEP_M).unwrap();
        let r20 = resample_polyline(&r, SEED_STEP_M).unwrap();
        let mid = compute_paired_centreline(&l20, &r20, SEED_STEP_M, 1, 0).unwrap();
        let rl = reference_line(&l, &r, &[w]).unwrap();
        let (a, b) = (sign_changes_per_km(&rl.line), sign_changes_per_km(&mid));
        assert!(a < b, "sign changes per km: reference {a}, mid-line {b}");
        let (pa, pb) = (peak(&rl.line), peak(&mid));
        assert!(pa < pb, "peak curvature: reference {pa}, mid-line {pb}");
    }

    #[test]
    fn on_a_bend_it_stays_nearer_the_centre_than_a_small_vessel_s_fairway() {
        let (l, r, w, centre) = channel(&bend(250.0), 200.0, 0.0);
        let rl = reference_line(&l, &r, std::slice::from_ref(&w)).unwrap();
        let v = CustomVessel {
            name: "10 x 2".into(),
            loa_m: 10.0,
            beam_m: 2.0,
            draught_m: 2.0,
            formation: Formation::Motor,
            n_abreast: None,
            n_in_line: None,
            gb_class: None,
            air_draught_m: None,
            deadweight_t: None,
        };
        let shape =
            channel_shape_for_custom(&selection_for_old_ruleset("PIANC-C").unwrap(), &v).unwrap();
        let fit = fairway_fit_in_water(
            &centre,
            &[w],
            &shape,
            0.0,
            0.0,
            &Fixed::default(),
            &FitParams::new(shape.min_bend_radius_m),
        )
        .unwrap();
        let mean_off = |line: &[Pt]| {
            let c = Chained::new(line);
            let n = (c.length() / 10.0) as usize;
            (0..=n)
                .map(|i| point_line_distance(c.at(10.0 * i as f64), &centre))
                .sum::<f64>()
                / (n + 1) as f64
        };
        let (a, b) = (mean_off(&rl.line), mean_off(&fit.line));
        assert!(
            a < b,
            "mean offset from the centre: reference {a:.1} m, fairway {b:.1} m"
        );
        assert!(a < 10.0, "the reference line strays {a:.1} m on average");
    }

    #[test]
    fn taubin_keeps_a_straight_line_and_pins_the_ends() {
        let line: Vec<Pt> = (0..20).map(|i| [i as f64, 2.0 * i as f64]).collect();
        let t = taubin_smoothing(&line, 60, TAUBIN_LAMBDA, TAUBIN_MU);
        for (a, b) in line.iter().zip(&t) {
            assert!(dist(*a, *b) < 1e-9);
        }
        let zig: Vec<Pt> = (0..20)
            .map(|i| [i as f64, if i % 2 == 0 { 0.0 } else { 1.0 }])
            .collect();
        let t = taubin_smoothing(&zig, 60, TAUBIN_LAMBDA, TAUBIN_MU);
        assert_eq!((t[0], t[19]), (zig[0], zig[19]));
        let spread = t[5..15]
            .iter()
            .fold(0.0f64, |m, p| m.max((p[1] - 0.5).abs()));
        assert!(spread < 0.1, "the zigzag keeps {spread} m");
    }

    #[test]
    fn ranges_carry_onto_a_line_by_position() {
        let from: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
        let to: Vec<Pt> = vec![[100.0, 5.0], [1100.0, 5.0]];
        let got = carry_by_chainage(&from, &to, &[(600.0, 500.0), (200.0, 300.0)]);
        assert_eq!(got, vec![(100.0, 200.0), (400.0, 500.0)]);
    }
}
