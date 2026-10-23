//! "What fits": the plan-only answer over every vessel class (spec 0012, ticket T8).
//!
//! For each vessel class of the pinned `ovcd-core` catalogue, in size order, and each Rule Set choice (an old
//! ruleset name, such as PIANC-B), the
//! river centreline is checked as it is: the water against the bend-widened bottom width at the
//! local radius (the width check of spec 0005, which also fails a station that is not in the
//! water), and the local radius against R_min. Every class stays in the answer. A class that does
//! not fit lists its stretches: kind, chainage and position of each end, what the rule requires,
//! what the river has, and the shortfall at the worst station.
//!
//! The answer reads width and radius only. It has no depth: [`PLAN_ONLY_LABEL`] says so, and every
//! output carries it. No file, clock or network access.

use crate::alignment::PyError;
use crate::alignment::navigable::{
    Chained, STATION_STEP_M, StretchReason, kind_checks, water_width_at,
};
use crate::alignment::radius_check::default_check_chord;
use crate::geom::{Polygon, Pt};
use crate::rules::engine::selection_for_old_ruleset;
use crate::rules::rulesets::{ChannelShape, channel_shape_for};
use crate::rules::vessel_catalogue;

/// Said on every output of the plan-only answer.
pub const PLAN_ONLY_LABEL: &str = "width and radius only, no depth";

/// What stops a class on a stretch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitKind {
    /// The water is narrower than the bend-widened bottom width.
    Width,
    /// The river turns tighter than the class's R_min.
    Radius,
}

impl FitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FitKind::Width => "width",
            FitKind::Radius => "radius",
        }
    }
}

/// One stretch of the river that stops a class. `required`, `found` and `deficit` (all metres) are
/// read at the worst station of the stretch.
#[derive(Debug, Clone, PartialEq)]
pub struct FitStretch {
    pub kind: FitKind,
    pub start_chainage_m: f64,
    pub end_chainage_m: f64,
    pub start: Pt,
    pub end: Pt,
    pub required_m: f64,
    pub found_m: f64,
    pub deficit_m: f64,
}

/// One row of the answer: a vessel class under a Rule Set choice.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassFit {
    /// The class's catalogue id, or a custom vessel's label.
    pub vessel_standard_id: String,
    /// True for the study's custom vessel, the row after the classes (spec 0015 T1).
    pub custom: bool,
    pub class_name: String,
    pub family: String,
    pub formation: String,
    pub loa_m: f64,
    pub beam_m: f64,
    /// R_min of the class under the Rule Set choice (m).
    pub r_min_m: f64,
    /// The bottom width of a straight channel (m).
    pub bottom_width_straight_m: f64,
    /// True when no stretch stops the class.
    pub fits: bool,
    /// The largest width shortfall over the stretches (m); 0 when there is none.
    pub worst_width_deficit_m: f64,
    /// The largest radius shortfall over the stretches (m); 0 when there is none.
    pub worst_radius_deficit_m: f64,
    pub stretches: Vec<FitStretch>,
    /// What the Rule Sets say about the class (for example a clause that does not cover it).
    pub warnings: Vec<String>,
}

/// The answer under one Rule Set choice.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleSetFits {
    /// The choice, by its old name (such as PIANC-B): see [`old_name_choice`].
    ///
    /// [`old_name_choice`]: crate::rules::engine::old_name_choice
    pub rule_set: String,
    /// One row per class, in size order.
    pub rows: Vec<ClassFit>,
    /// The largest class that fits as it is; `None` when no class does.
    pub largest_fitting: Option<String>,
}

/// The plan-only answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Fits {
    /// Always [`PLAN_ONLY_LABEL`].
    pub label: &'static str,
    pub length_m: f64,
    pub rule_sets: Vec<RuleSetFits>,
}

/// The vessel class ids in size order: by beam, then overall length, then id. The same order for
/// every Rule Set choice, so "a larger class" means one thing. The 12 default classes.
pub fn classes_in_size_order() -> Vec<String> {
    classes_in_size_order_of(false)
}

/// [`classes_in_size_order`], over every class a channel can be sized for when `all` is true
/// (`owd fits --all`, spec 0015 T0).
pub fn classes_in_size_order_of(all: bool) -> Vec<String> {
    let mut all = if all {
        crate::rules::full_catalogue()
    } else {
        vessel_catalogue()
    };
    let dims = |v: &crate::rules::VesselStandard| crate::rules::classes::design_figures(v, None);
    all.sort_by(|(ka, a), (kb, b)| {
        dims(a)
            .beam_m
            .total_cmp(&dims(b).beam_m)
            .then(dims(a).loa_m.total_cmp(&dims(b).loa_m))
            .then(ka.cmp(kb))
    });
    all.into_iter().map(|(k, _)| k).collect()
}

/// The answer for `centreline` in `water`, under each of `choices` (old ruleset names), for every
/// class.
pub fn plan_only_fits(
    centreline: &[Pt],
    water: &[Polygon],
    choices: &[&str],
) -> Result<Fits, PyError> {
    plan_only_fits_of(centreline, water, choices, false)
}

/// [`plan_only_fits`] over every class a channel can be sized for when `all` is true.
pub fn plan_only_fits_of(
    centreline: &[Pt],
    water: &[Polygon],
    choices: &[&str],
    all: bool,
) -> Result<Fits, PyError> {
    plan_only_fits_with(centreline, water, choices, all, None)
}

/// [`plan_only_fits_of`] with the study's custom vessel as one more row after the classes (spec
/// 0015 T1). The largest fitting class is read over the classes only.
pub fn plan_only_fits_with(
    centreline: &[Pt],
    water: &[Polygon],
    choices: &[&str],
    all: bool,
    custom: Option<&crate::rules::custom::CustomVessel>,
) -> Result<Fits, PyError> {
    plan_only_fits_at(centreline, water, choices, all, custom, None)
}

/// [`plan_only_fits_with`], with every class sized at `draught_m` instead of its own (#364). The
/// custom vessel keeps its own draught.
pub fn plan_only_fits_at(
    centreline: &[Pt],
    water: &[Polygon],
    choices: &[&str],
    all: bool,
    custom: Option<&crate::rules::custom::CustomVessel>,
    draught_m: Option<f64>,
) -> Result<Fits, PyError> {
    if centreline.len() < 2 {
        return Err(PyError::value(
            "the river centreline needs at least two points.",
        ));
    }
    let chained = Chained::new(centreline);
    let classes = classes_in_size_order_of(all);
    let mut rule_sets = Vec::new();
    for &rs in choices {
        // An old ruleset name, sized through its Rule Set selection (spec 0007).
        let selection = selection_for_old_ruleset(rs)
            .map_err(|e| PyError::value(e.raw_message().to_string()))?;
        let mut rows = Vec::new();
        for class in &classes {
            let shape = channel_shape_for(&selection, class, draught_m)
                .map_err(|e| PyError::value(e.raw_message().to_string()))?;
            let std = crate::rules::classes::class(class)
                .map_err(|_| PyError::value(format!("unknown vessel class {class}")))?;
            rows.push(class_fit(
                &chained,
                water,
                &shape,
                (std.class_name, std.family, std.formation),
            )?);
        }
        let largest_fitting = rows
            .iter()
            .rev()
            .find(|r| r.fits)
            .map(|r| r.vessel_standard_id.clone());
        if let Some(c) = custom {
            let shape = crate::rules::rulesets::channel_shape_for_custom(&selection, c)
                .map_err(|e| PyError::value(e.raw_message().to_string()))?;
            let v = c.vessel();
            rows.push(class_fit(
                &chained,
                water,
                &shape,
                (v.class_name, v.family, v.formation),
            )?);
        }
        rule_sets.push(RuleSetFits {
            rule_set: rs.to_string(),
            rows,
            largest_fitting,
        });
    }
    Ok(Fits {
        label: PLAN_ONLY_LABEL,
        length_m: chained.length(),
        rule_sets,
    })
}

/// One row: `who` is the vessel's class name, family and formation.
pub(crate) fn class_fit(
    c: &Chained,
    water: &[Polygon],
    shape: &ChannelShape,
    who: (String, String, String),
) -> Result<ClassFit, PyError> {
    let r_min = shape.min_bend_radius_m;
    let chord = default_check_chord(r_min);
    // Each kind in its own right: a radius stretch is not hidden under a width stretch (#365).
    let found = kind_checks(c.line, water, shape, r_min, chord)?;
    let stretches: Vec<FitStretch> = found
        .iter()
        .map(|st| {
            let kind = match st.reason {
                StretchReason::TooNarrow => FitKind::Width,
                StretchReason::RMinNotPossible => FitKind::Radius,
            };
            worst_station(c, water, shape, r_min, chord, kind, st)
        })
        .collect();
    let worst = |k: FitKind| {
        stretches
            .iter()
            .filter(|s| s.kind == k)
            .map(|s| s.deficit_m)
            .fold(0.0, f64::max)
    };
    let (class_name, family, formation) = who;
    Ok(ClassFit {
        vessel_standard_id: shape.vessel_standard_id.clone(),
        custom: shape.custom,
        class_name,
        family,
        formation,
        loa_m: shape.loa_m,
        beam_m: shape.beam_m,
        r_min_m: r_min,
        bottom_width_straight_m: shape.bottom_width_straight_m,
        fits: stretches.is_empty(),
        worst_width_deficit_m: worst(FitKind::Width),
        worst_radius_deficit_m: worst(FitKind::Radius),
        stretches,
        warnings: shape.warnings.clone(),
    })
}

/// Read a stretch at its stations (every `STATION_STEP_M`, and both ends) and keep the worst.
fn worst_station(
    c: &Chained,
    water: &[Polygon],
    shape: &ChannelShape,
    r_min: f64,
    chord: f64,
    kind: FitKind,
    st: &crate::alignment::navigable::Stretch,
) -> FitStretch {
    let (a, b) = (st.start_chainage_m, st.end_chainage_m);
    let n = (((b - a) / STATION_STEP_M).ceil() as usize).max(1);
    let mut best = (f64::NEG_INFINITY, 0.0, 0.0);
    for i in 0..=n {
        let s = a + (b - a) * i as f64 / n as f64;
        let r = c.local_radius(s, chord);
        let (req, got) = match kind {
            FitKind::Width => (shape.bottom_width_at_m(r), water_width_at(c, s, water)),
            FitKind::Radius => (r_min, r),
        };
        let d = req - got;
        if d > best.0 {
            best = (d, req, got);
        }
    }
    FitStretch {
        kind,
        start_chainage_m: a,
        end_chainage_m: b,
        start: st.start,
        end: st.end,
        required_m: best.1,
        found_m: best.2,
        deficit_m: best.0.max(0.0),
    }
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation. The throat is 60 m wide between two land teeth; the
    //! straight bottom width of each class is its ruleset's, so a class is stopped by the throat
    //! exactly when its straight width is over 60 m. The bend has radius 400 m, so a class is
    //! stopped by it when its R_min is over 400 m (PIANC-B: 3 x LOA, CEMT IV 255 m).
    use super::*;

    #[test]
    fn a_radius_stretch_under_a_width_stretch_is_reported_too() {
        // Label: property (#365). A tight bend in narrow water: each kind in its own right.
        let (line, water) = crate::alignment::navigable::tests::tight_bend_in_narrow_water();
        let f = plan_only_fits_of(&line, &[water], &["PIANC-B"], false).unwrap();
        let row = f.rule_sets[0]
            .rows
            .iter()
            .find(|r| r.vessel_standard_id == "CEMT-IV")
            .unwrap();
        let kinds: Vec<FitKind> = row.stretches.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&FitKind::Width), "{kinds:?}");
        assert!(kinds.contains(&FitKind::Radius), "{kinds:?}");
        assert!(
            row.worst_radius_deficit_m > 100.0,
            "R_min 255 m on a 100 m bend"
        );
    }

    const THROAT_M: f64 = 60.0;
    const BEND_R_M: f64 = 400.0;
    const BEND_AT_M: f64 = 2000.0;

    /// A straight reach, a 90-degree left bend of radius 400 m, then a straight leg north. The
    /// water is a big open rectangle with two land teeth that leave a 60 m throat at x 900..1600.
    fn fixture() -> (Vec<Pt>, Vec<Polygon>) {
        let mut line: Vec<Pt> = vec![[0.0, 0.0], [BEND_AT_M, 0.0]];
        for i in 1..=90 {
            let th = core::f64::consts::FRAC_PI_2 * i as f64 / 90.0;
            line.push([
                BEND_AT_M + BEND_R_M * libm::sin(th),
                BEND_R_M * (1.0 - libm::cos(th)),
            ]);
        }
        line.push([BEND_AT_M + BEND_R_M, 1400.0]);
        let h = THROAT_M / 2.0;
        let ring = vec![
            [-500.0, -1500.0],
            [900.0, -1500.0],
            [900.0, -h],
            [1600.0, -h],
            [1600.0, -1500.0],
            [3500.0, -1500.0],
            [3500.0, 1500.0],
            [1600.0, 1500.0],
            [1600.0, h],
            [900.0, h],
            [900.0, 1500.0],
            [-500.0, 1500.0],
            [-500.0, -1500.0],
        ];
        (
            line,
            vec![Polygon {
                exterior: ring,
                holes: vec![],
            }],
        )
    }

    fn answer(rs: &str) -> RuleSetFits {
        let (line, water) = fixture();
        plan_only_fits(&line, &water, &[rs])
            .unwrap()
            .rule_sets
            .remove(0)
    }

    #[test]
    fn every_class_is_in_the_answer_in_size_order_with_the_label() {
        let (line, water) = fixture();
        let f = plan_only_fits(&line, &water, &["PIANC-B"]).unwrap();
        assert_eq!(f.label, "width and radius only, no depth");
        let ids: Vec<&str> = f.rule_sets[0]
            .rows
            .iter()
            .map(|r| r.vessel_standard_id.as_str())
            .collect();
        assert_eq!(ids.len(), vessel_catalogue().len());
        let beams: Vec<f64> = f.rule_sets[0].rows.iter().map(|r| r.beam_m).collect();
        assert!(beams.windows(2).all(|w| w[0] <= w[1]), "{beams:?}");
    }

    #[test]
    fn each_class_gets_the_stretches_the_hand_calculation_gives() {
        let a = answer("PIANC-B");
        let mut width_stopped = 0;
        let mut radius_stopped = 0;
        for row in &a.rows {
            let throat = row.bottom_width_straight_m > THROAT_M;
            // R_min is read 2 % low (the reading tolerance): a class within 1 % of the bend's
            // radius is a toss-up for the 90-segment arc, so it is not asserted.
            let needed = row.r_min_m * 0.98;
            if (needed - BEND_R_M).abs() < 0.01 * BEND_R_M {
                continue;
            }
            let bend = needed > BEND_R_M;
            let kinds: Vec<FitKind> = row.stretches.iter().map(|s| s.kind).collect();
            assert_eq!(
                kinds.contains(&FitKind::Width),
                throat,
                "{}: {:?}",
                row.vessel_standard_id,
                row.stretches
            );
            assert_eq!(
                kinds.contains(&FitKind::Radius),
                bend,
                "{}: {:?}",
                row.vessel_standard_id,
                row.stretches
            );
            assert_eq!(row.fits, !throat && !bend, "{}", row.vessel_standard_id);
            if throat {
                width_stopped += 1;
                let st = row
                    .stretches
                    .iter()
                    .find(|s| s.kind == FitKind::Width)
                    .unwrap();
                // The water is 60 m wide on x 900..1600 and wider everywhere else.
                assert!((st.start_chainage_m - 900.0).abs() < 0.05, "{st:?}");
                assert!((st.end_chainage_m - 1600.0).abs() < 0.05, "{st:?}");
                assert!((st.found_m - THROAT_M).abs() < 1e-6, "{st:?}");
                assert!((st.required_m - row.bottom_width_straight_m).abs() < 1e-6);
                assert!((st.deficit_m - (row.bottom_width_straight_m - THROAT_M)).abs() < 1e-6);
            }
            if bend {
                radius_stopped += 1;
                let st = row
                    .stretches
                    .iter()
                    .find(|s| s.kind == FitKind::Radius)
                    .unwrap();
                let arc_end = BEND_AT_M + core::f64::consts::FRAC_PI_2 * BEND_R_M;
                assert!(
                    st.start_chainage_m > BEND_AT_M - 300.0 && st.end_chainage_m < arc_end + 300.0
                );
                assert!((st.required_m - row.r_min_m).abs() < 1e-9);
                assert!(st.found_m < row.r_min_m && st.found_m > 0.0);
            }
        }
        assert!(width_stopped > 0 && radius_stopped > 0);
        assert!(
            a.rows.iter().any(|r| r.fits),
            "the fixture must let a class through"
        );
    }

    #[test]
    fn the_largest_fitting_class_is_the_last_row_that_fits() {
        let a = answer("PIANC-B");
        let last = a.rows.iter().rev().find(|r| r.fits).unwrap();
        assert_eq!(
            a.largest_fitting.as_deref(),
            Some(last.vessel_standard_id.as_str())
        );
    }

    #[test]
    fn a_class_that_cannot_pass_a_narrower_class_deficit_gets_no_smaller_deficit() {
        // Monotone: if class B is at least as big as class A in beam and length (same formation),
        // B fails whenever A does, and by at least as much.
        for rs in ["PIANC-A", "PIANC-B"] {
            let a = answer(rs);
            let mut pairs = 0;
            for (i, small) in a.rows.iter().enumerate() {
                for big in &a.rows[i + 1..] {
                    if big.formation != small.formation
                        || big.beam_m < small.beam_m
                        || big.loa_m < small.loa_m
                    {
                        continue;
                    }
                    pairs += 1;
                    assert!(
                        !big.fits || small.fits,
                        "{rs}: {} fits but {} does not",
                        big.vessel_standard_id,
                        small.vessel_standard_id
                    );
                    assert!(
                        big.worst_width_deficit_m + 1e-9 >= small.worst_width_deficit_m,
                        "{rs}: {} vs {}",
                        big.vessel_standard_id,
                        small.vessel_standard_id
                    );
                    assert!(
                        big.worst_radius_deficit_m + 1e-9 >= small.worst_radius_deficit_m,
                        "{rs}: {} vs {}",
                        big.vessel_standard_id,
                        small.vessel_standard_id
                    );
                }
            }
            assert!(pairs > 3, "{rs}: only {pairs} comparable pairs");
        }
    }

    #[test]
    fn a_river_with_no_stretches_lets_every_class_through_and_bad_input_is_refused() {
        let line = vec![[0.0, 0.0], [3000.0, 0.0]];
        let wide = Polygon {
            exterior: vec![
                [-500.0, -500.0],
                [3500.0, -500.0],
                [3500.0, 500.0],
                [-500.0, 500.0],
                [-500.0, -500.0],
            ],
            holes: vec![],
        };
        let f = plan_only_fits(&line, &[wide], &["PIANC-B"]).unwrap();
        assert!(
            f.rule_sets[0]
                .rows
                .iter()
                .all(|r| r.fits && r.stretches.is_empty())
        );
        assert_eq!(
            f.rule_sets[0].largest_fitting.as_deref(),
            f.rule_sets[0]
                .rows
                .last()
                .map(|r| r.vessel_standard_id.as_str())
        );
        assert!(plan_only_fits(&line[..1], &[], &["PIANC-B"]).is_err());
        assert!(plan_only_fits(&line, &[], &["NOPE"]).is_err());
    }
}
