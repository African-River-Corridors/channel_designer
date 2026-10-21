//! The water-aware rolling circle (spec 0005 R2-Q1): the four-rung ladder.
//!
//! Without water this is today's rolling circle, unchanged. With water, today's rolling circle
//! runs first, and then each bend it spliced (each circle, found again by position, L7) takes the
//! first rung that works:
//!
//! 1. **Inside arc** at R_min, as today, if the bend-widened channel fits in the water there and
//!    the arc leaves no kink at its ends (the radius read on the chord holds there).
//! 2. **Push out**: a fillet on the line's own tangents either side of the bend, at the smallest
//!    radius from R_min up (a fixed ladder) at which the channel, widened for that radius, fits.
//! 3. **Fairway fit** over a window around the bend, between the line's poses at the window ends.
//! 4. **Infeasible**: the fairway fit's line, which breaks the rules least, and its stretches.
//!
//! "Fits" means: every station of the window lies in the water shrunk by half the bottom width
//! widened for the bend's radius, plus the clearance (navigable water for that radius), and the
//! per-station width check finds nothing.

use super::{RollingCircleResult, rolling_circle};
use crate::alignment::PyError;
use crate::alignment::fairway_fit::{
    FitParams, Fixed, Pose, fairway_fit_in_water, merge_stretches,
};
use crate::alignment::fillet::fillet;
use crate::alignment::navigable::{Chained, Stretch, StretchReason, navigable_water, width_check};
use crate::alignment::radius_check::{min_radius_on_chord, radius_ok};
use crate::geom::grid::EdgeGrid;
use crate::geom::{Geometry, Polygon, Pt, linear};
use crate::rules::rulesets::ChannelShape;

/// The water the ladder works in, and its settings.
#[derive(Debug, Clone)]
pub struct WaterInput<'a> {
    pub water: &'a [Polygon],
    pub shape: &'a ChannelShape,
    /// Clearance from the bank (m), default 0.
    pub clearance_m: f64,
    /// Opening and closing of navigable water (m).
    pub smooth_m: f64,
    /// A preferred radius for the fairway fit (m), off by default.
    pub r_pref_m: Option<f64>,
}

/// The rung a bend took.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rung {
    InsideArc,
    /// The fillet's radius (m).
    PushOut(f64),
    FairwayFit,
    Infeasible,
}

impl Rung {
    pub fn as_str(&self) -> &'static str {
        match self {
            Rung::InsideArc => "inside_arc",
            Rung::PushOut(_) => "push_out",
            Rung::FairwayFit => "fairway_fit",
            Rung::Infeasible => "infeasible",
        }
    }
}

/// One bend of the ladder: where it is (the circle's tangent and contact points) and its rung.
#[derive(Debug, Clone, PartialEq)]
pub struct BendRung {
    pub tangent_point: Pt,
    pub contact_point: Pt,
    pub rung: Rung,
}

/// The rolling circle's result, with the ladder's record when water was given.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterAwareResult {
    pub result: RollingCircleResult,
    /// One per bend, in order; empty without water.
    pub rungs: Vec<BendRung>,
    /// Infeasible stretches on the final line; empty without water.
    pub infeasible: Vec<Stretch>,
}

/// Push-out radii, as multiples of R_min.
const PUSH_OUT_LADDER: [f64; 9] = [1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0];

/// Station spacing of the ladder's checks (m).
const CHECK_STEP_M: f64 = 5.0;

/// `rolling_circle` with an optional water polygon. Without it, exactly today's result.
#[allow(clippy::too_many_arguments)]
pub fn rolling_circle_with_water(
    centreline: &Geometry,
    min_radius: f64,
    step: f64,
    passes: i64,
    sides: &[String],
    min_chainage_gap: Option<f64>,
    check_chord_m: Option<f64>,
    water: Option<&WaterInput>,
) -> Result<WaterAwareResult, PyError> {
    let base = rolling_circle(
        centreline,
        min_radius,
        step,
        passes,
        sides,
        min_chainage_gap,
        check_chord_m,
    )?;
    let Some(w) = water else {
        return Ok(WaterAwareResult {
            result: base,
            rungs: vec![],
            infeasible: vec![],
        });
    };
    w.shape
        .ruleset()
        .validate()
        .map_err(|e| PyError::value(e.raw_message().to_string()))?;
    let chord = base.radius_check_chord_m;
    let mut ladder = Ladder::new(w, min_radius, chord);
    // Bends: the circles' spans on today's line, overlapping spans merged (a second pass or
    // side often splices again inside a bend already done).
    let mut spans: Vec<(f64, f64)> = base
        .circles
        .iter()
        .map(|c| {
            let a = linear::project(&base.centreline, c.tangent_point);
            let b = linear::project(&base.centreline, c.contact_point);
            (a.min(b), a.max(b))
        })
        .collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut bends: Vec<(f64, f64)> = Vec::new();
    for sp in spans {
        match bends.last_mut() {
            Some(last) if sp.0 <= last.1 => last.1 = last.1.max(sp.1),
            _ => bends.push(sp),
        }
    }
    let base_c = Chained::new(&base.centreline);
    let mut line = base.centreline.clone();
    let mut rungs = Vec::new();
    // Each bend's window, kept by position and read back on the final line (L7).
    let mut windows: Vec<(Pt, Pt)> = Vec::new();
    for (a, b) in bends {
        let (tangent, contact) = (base_c.at(a), base_c.at(b));
        let (rung, new_line, window) = ladder.bend(&line, tangent, contact)?;
        line = new_line;
        windows.push(window);
        rungs.push(BendRung {
            tangent_point: tangent,
            contact_point: contact,
            rung,
        });
    }
    // Infeasible stretches: the checks on the final line, inside the bend windows.
    let fin = Chained::new(&line);
    let straight = ladder.nav_for(f64::INFINITY)?.clone();
    let status = |s: f64| {
        if !straight.covers(fin.at(s), 0.05) {
            Some(StretchReason::TooNarrow)
        } else {
            None
        }
    };
    let mut all =
        crate::alignment::fairway_fit::stretches(&fin, fin.length(), CHECK_STEP_M, &status);
    all.extend(width_check(&line, w.water, w.shape, min_radius, chord)?);
    let spans_on_final: Vec<(f64, f64)> = windows
        .iter()
        .map(|(a, b)| {
            let (x, y) = (linear::project(&line, *a), linear::project(&line, *b));
            (x.min(y), x.max(y))
        })
        .collect();
    let infeasible = merge_stretches(
        &line,
        all.into_iter()
            .filter(|s| {
                spans_on_final
                    .iter()
                    .any(|(a, b)| s.end_chainage_m > *a && s.start_chainage_m < *b)
            })
            .collect(),
    );
    let measured = min_radius_on_chord(&line, chord)?;
    Ok(WaterAwareResult {
        result: RollingCircleResult {
            centreline: line,
            min_radius_m: base.min_radius_m,
            circles: base.circles,
            measured_min_radius_m: measured,
            radius_check_chord_m: chord,
            radius_ok: radius_ok(measured, min_radius),
        },
        rungs,
        infeasible,
    })
}

/// A bend's rung, the line after it, and the bend's window (its two ends, by position).
type BendOutcome = (Rung, Vec<Pt>, (Pt, Pt));

/// The ladder's state: the water, and navigable water per radius (built once each).
struct Ladder<'a> {
    w: &'a WaterInput<'a>,
    r_min: f64,
    chord: f64,
    nav: Vec<(f64, EdgeGrid)>,
}

impl<'a> Ladder<'a> {
    fn new(w: &'a WaterInput<'a>, r_min: f64, chord: f64) -> Self {
        Ladder {
            w,
            r_min,
            chord,
            nav: Vec::new(),
        }
    }

    /// Navigable water for a channel widened for radius `r`.
    fn nav_for(&mut self, r: f64) -> Result<&EdgeGrid, PyError> {
        if let Some(i) = self.nav.iter().position(|(rr, _)| *rr == r) {
            return Ok(&self.nav[i].1);
        }
        let half = self.w.shape.bottom_width_at_m(r) / 2.0;
        let polys = navigable_water(self.w.water, half, self.w.clearance_m, self.w.smooth_m)?;
        self.nav.push((r, EdgeGrid::new(&polys)));
        Ok(&self.nav.last().unwrap().1)
    }

    /// Whether `window` (a line) holds a channel widened for radius `r` inside the water.
    fn fits(&mut self, window: &[Pt], r: f64) -> Result<bool, PyError> {
        let grid = self.nav_for(r)?;
        let c = Chained::new(window);
        let n = (c.length() / CHECK_STEP_M).ceil() as usize;
        let inside = (0..=n).all(|i| {
            let s = (i as f64 * CHECK_STEP_M).min(c.length());
            grid.covers(c.at(s), 0.05)
        });
        Ok(inside
            && width_check(window, self.w.water, self.w.shape, self.r_min, self.chord)?.is_empty())
    }

    /// Whether the radius read on the chord holds at chainages `s` of `line`.
    fn no_kink(&self, line: &Chained, at: &[f64]) -> bool {
        at.iter()
            .all(|&s| radius_ok(line.local_radius(s, self.chord), self.r_min))
    }

    /// One bend: the rung it takes, the line after it, and any infeasible stretches.
    fn bend(&mut self, line: &[Pt], tangent: Pt, contact: Pt) -> Result<BendOutcome, PyError> {
        let c = Chained::new(line);
        let len = c.length();
        let (st, sc) = (
            linear::project(line, tangent),
            linear::project(line, contact),
        );
        let (st, sc) = (st.min(sc), st.max(sc));
        // Rung 1: the arc as it stands, a chord either side for the widened channel.
        let half = self.chord / 2.0;
        let w1 = c.slice((st - half).max(0.0), (sc + half).min(len));
        if self.no_kink(&c, &[st, sc]) && self.fits(&w1, self.r_min)? {
            return Ok((Rung::InsideArc, line.to_vec(), (w1[0], w1[w1.len() - 1])));
        }
        // Rung 2: a fillet on the line's tangents, R_min upwards.
        let (s0, s1) = ((st - self.r_min).max(0.0), (sc + self.r_min).min(len));
        for k in PUSH_OUT_LADDER {
            let r = k * self.r_min;
            let Some(f) = fillet(&c, s0, s1, r, half.min(25.0), 5.0) else {
                continue;
            };
            let spliced = splice(&c, s0, s1, &f);
            let sc2 = Chained::new(&spliced);
            let (a, b) = (
                linear::project(&spliced, f[0]),
                linear::project(&spliced, f[f.len() - 1]),
            );
            let w2 = sc2.slice((a - half).max(0.0), (b + half).min(sc2.length()));
            if self.no_kink(&sc2, &[a, b]) && self.fits(&w2, r)? {
                return Ok((Rung::PushOut(r), spliced, (w2[0], w2[w2.len() - 1])));
            }
        }
        // Rungs 3 and 4: the fairway fit over a wider window, between the line's poses.
        let (s0, s1) = (
            (st - 2.0 * self.r_min).max(0.0),
            (sc + 2.0 * self.r_min).min(len),
        );
        let pose = |s: f64| Pose {
            at: c.at(s),
            heading_rad: {
                let d = c.direction(s, 5.0);
                libm::atan2(d[1], d[0])
            },
        };
        let fixed = Fixed {
            start_pose: (s0 > 0.0).then(|| pose(s0)),
            end_pose: (s1 < len).then(|| pose(s1)),
            must_follow: vec![],
        };
        let mut params = FitParams::new(self.r_min);
        params.chord_m = self.chord;
        params.r_pref_m = self.w.r_pref_m;
        let window = c.slice(s0, s1);
        let fit = fairway_fit_in_water(
            &window,
            self.w.water,
            self.w.shape,
            self.w.clearance_m,
            self.w.smooth_m,
            &fixed,
            &params,
        )?;
        let spliced = splice(&c, s0, s1, &fit.line);
        // The bend itself: its span and half a chord either side, found again on the fitted
        // window by position. A stretch outside it (a window reaching a cut end of the water)
        // is not this bend's.
        let core = (c.at((st - half).max(0.0)), c.at((sc + half).min(len)));
        let (ca, cb) = (
            linear::project(&fit.line, core.0),
            linear::project(&fit.line, core.1),
        );
        let hit = fit
            .infeasible
            .iter()
            .any(|s| s.end_chainage_m > ca.min(cb) && s.start_chainage_m < ca.max(cb));
        Ok((
            if hit {
                Rung::Infeasible
            } else {
                Rung::FairwayFit
            },
            spliced,
            core,
        ))
    }
}

/// `line` with chainages `s0..s1` replaced by `piece` (which runs from the point at `s0` to the
/// point at `s1`).
fn splice(c: &Chained, s0: f64, s1: f64, piece: &[Pt]) -> Vec<Pt> {
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

#[cfg(test)]
mod tests {
    //! Label: snapshot (the rungs each synthetic-river bend takes, recorded 2026-10-16 when the
    //! ladder was built: "first water-aware run, spec 0005 #97"); property (no-water equality,
    //! no exit kink); sourced - hand calculation (the push-out corner: tangent length R).
    use super::*;
    use crate::alignment::radius_check::RADIUS_TOL;
    use crate::rules::rulesets::{RulesetRegistry, channel_shape};
    use crate::synthetic::{RiverParams, open_basin_90, synthetic_river};

    const R_MIN: f64 = 255.0;

    fn shape() -> ChannelShape {
        channel_shape(&RulesetRegistry::new(), "PIANC-B", "CEMT_IV", Some(2.5)).unwrap()
    }

    fn sides() -> Vec<String> {
        vec!["left".into(), "right".into()]
    }

    fn run(width: f64, smooth: f64) -> WaterAwareResult {
        let river = synthetic_river(RiverParams {
            width_m: width,
            ..RiverParams::default()
        })
        .unwrap();
        let s = shape();
        let wi = WaterInput {
            water: core::slice::from_ref(&river.polygon),
            shape: &s,
            clearance_m: 0.0,
            smooth_m: smooth,
            r_pref_m: None,
        };
        let g = Geometry::LineString(river.centreline);
        rolling_circle_with_water(&g, R_MIN, 10.0, 2, &sides(), None, None, Some(&wi)).unwrap()
    }

    /// The radius read on the chord at each bend's arc ends on the final line.
    fn assert_no_exit_kink(r: &WaterAwareResult) {
        let c = Chained::new(&r.result.centreline);
        for b in &r.rungs {
            for p in [b.tangent_point, b.contact_point] {
                let s = linear::project(&r.result.centreline, p);
                let rad = c.local_radius(s, r.result.radius_check_chord_m);
                assert!(
                    rad >= (1.0 - RADIUS_TOL) * R_MIN,
                    "{:?} at {p:?}: {rad}",
                    b.rung
                );
            }
        }
        assert!(r.result.radius_ok, "{}", r.result.measured_min_radius_m);
    }

    #[test]
    fn without_water_the_result_is_todays() {
        let river = synthetic_river(RiverParams::default()).unwrap();
        let g = Geometry::LineString(river.centreline);
        for (step, passes) in [(10.0, 1), (10.0, 2), (25.0, 3)] {
            let today = rolling_circle(&g, R_MIN, step, passes, &sides(), None, None).unwrap();
            let r = rolling_circle_with_water(&g, R_MIN, step, passes, &sides(), None, None, None)
                .unwrap();
            assert_eq!(r.result, today);
            assert!(r.rungs.is_empty() && r.infeasible.is_empty());
        }
    }

    #[test]
    fn at_300_m_every_bend_fits_and_the_rungs_are_recorded() {
        let r = run(300.0, 10.0);
        let rungs: Vec<&str> = r.rungs.iter().map(|b| b.rung.as_str()).collect();
        assert_eq!(rungs, vec!["inside_arc"; 8]);
        assert!(r.infeasible.is_empty(), "{:?}", r.infeasible);
        assert_no_exit_kink(&r);
    }

    #[test]
    fn at_46_m_the_fairway_fit_places_every_bend() {
        let r = run(46.0, 2.0);
        let rungs: Vec<&str> = r.rungs.iter().map(|b| b.rung.as_str()).collect();
        assert_eq!(rungs, vec!["fairway_fit"; 8]);
        assert!(r.infeasible.is_empty(), "{:?}", r.infeasible);
        assert_no_exit_kink(&r);
    }

    #[test]
    fn at_40_m_every_apex_is_too_narrow_and_the_option_keeps_a_line() {
        let r = run(40.0, 2.0);
        assert_eq!(r.rungs.len(), 8);
        assert!(r.rungs.iter().all(|b| b.rung == Rung::Infeasible));
        assert_eq!(r.infeasible.len(), 8, "{:?}", r.infeasible);
        for k in 0..8 {
            let x = 375.0 + 750.0 * k as f64;
            let y = if k % 2 == 0 { 260.0 } else { -260.0 };
            let at = linear::project(&r.result.centreline, [x, y]);
            assert!(
                r.infeasible
                    .iter()
                    .any(|s| s.reason == StretchReason::TooNarrow
                        && s.start_chainage_m < at
                        && at < s.end_chainage_m),
                "apex {k}"
            );
        }
        assert!(r.result.centreline.len() > 100);
    }

    #[test]
    fn a_kinked_corner_in_open_water_is_pushed_out_to_a_tangent_fillet() {
        // The open basin's two legs as one line, its land corner far away: the corner itself
        // (taken as an "arc" from (0, -300) to (300, 0)) kinks, so rung 1 fails; a fillet of
        // R_min on the legs fits, with its tangent points R_min from the corner.
        let f = open_basin_90(1500.0, 1000.0);
        let line = vec![[0.0, -1500.0], [0.0, 0.0], [1500.0, 0.0]];
        let s = shape();
        let wi = WaterInput {
            water: core::slice::from_ref(&f.water),
            shape: &s,
            clearance_m: 0.0,
            smooth_m: 0.0,
            r_pref_m: None,
        };
        let mut ladder = Ladder::new(&wi, R_MIN, R_MIN / 3.0);
        let (rung, out, _) = ladder.bend(&line, [0.0, -300.0], [300.0, 0.0]).unwrap();
        assert_eq!(rung, Rung::PushOut(R_MIN));
        assert!(
            out.iter()
                .any(|p| crate::geom::dist(*p, [0.0, -R_MIN]) < 1e-6)
        );
        assert!(
            out.iter()
                .any(|p| crate::geom::dist(*p, [R_MIN, 0.0]) < 1e-6)
        );
        let rad = min_radius_on_chord(&out, R_MIN / 3.0).unwrap();
        assert!(radius_ok(rad, R_MIN), "{rad}");
    }
}
