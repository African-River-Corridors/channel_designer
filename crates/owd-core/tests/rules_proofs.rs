//! Spec 0007 T7: proofed checks for OWD's Rules.
//!
//! - Property tests on the formula shapes, on random vessels and radii (`Label: property`).
//! - Boundary cases: each evaluated Rule of `pianc-wg141` and `gb50139` tested just inside and just
//!   outside its limit, keyed by rule id. The limit comes from the Rule's own data through the engine,
//!   never typed by hand (`Label: property`).
//! - The WG 141 worked example (5.3.3, p104) as a golden case, with its known divergence
//!   (`Label: sourced - PIANC WG 141 (2019) 5.3.3 text box, p104`).
//!
//! The sizing theorem (the governing limit meets every sizing Rule of its Requirement; adding a Rule
//! never lowers it) is in the Lean model of `arc-rules` (OVCD #482). OWD runs no Lean.

use arc_rules::domain::Domain;
use arc_rules::eval::{MeasureSource, MeasureValue, Outcome, evaluate};
use arc_rules::model::{Kind, Limit, MeasureRef, Rule, RuleSet};
use owd_core::rules::engine::{self, ChannelDomain, SHAPES, Selection, Vessel};

/// splitmix64: a small deterministic generator, so the properties need no random-number crate and
/// a failure always reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

const CLASSES: [&str; 10] = [
    "I", "II", "III", "IV", "Va", "Vb", "VIa", "VIb", "VIc", "VII",
];

fn vessel(rng: &mut Rng) -> Vessel {
    Vessel {
        loa_m: rng.range(30.0, 300.0),
        beam_m: rng.range(5.0, 35.0),
        draught_m: rng.range(0.8, 5.0),
        formation: rng.pick(&["motor_vessel", "pushed_convoy"]).into(),
        class_name: rng.pick(&CLASSES).into(),
    }
}

fn pianc(q: &str) -> Selection {
    Selection::built_in(&["pianc-wg141"], &[(engine::SE_QUALITY, q)]).unwrap()
}

fn gb(end: &str) -> Selection {
    Selection::built_in(&["gb50139"], &[(engine::RANGE_END, end)]).unwrap()
}

const CASES: usize = 400;

/// The bottom width at a radius never rises as the radius rises, for every set and input; every
/// width is above zero; the `capped-widening` widening never exceeds L.
#[test]
fn widths_never_rise_with_the_radius_and_stay_above_zero() {
    let mut rng = Rng(7);
    let selections = [
        pianc("A"),
        pianc("B"),
        pianc("C"),
        gb("permissive"),
        gb("strict"),
    ];
    for _ in 0..CASES {
        let v = vessel(&mut rng);
        let (r1, r2) = {
            let a = rng.range(1.0, 5000.0);
            let b = rng.range(1.0, 5000.0);
            if a < b { (a, b) } else { (b, a) }
        };
        for (i, s) in selections.iter().enumerate() {
            let sized = engine::size(s, &v).unwrap();
            let (w1, w2) = (
                engine::widening(s, &v, r1).unwrap(),
                engine::widening(s, &v, r2).unwrap(),
            );
            assert!(
                w2 <= w1,
                "set {i}, {v:?}: widening {w2} at R {r2} > {w1} at R {r1}"
            );
            assert!(sized.bottom_width_straight_m > 0.0, "set {i}, {v:?}");
            assert!(sized.bottom_width_straight_m + w2 > 0.0, "set {i}, {v:?}");
            if i < 3 {
                assert!(w1 <= v.loa_m, "capped-widening above L: {w1} > {}", v.loa_m);
            }
        }
    }
}

/// Quality A ≥ B ≥ C on every quantity.
#[test]
fn quality_a_is_at_least_b_and_b_at_least_c_on_every_quantity() {
    let mut rng = Rng(11);
    for _ in 0..CASES {
        let v = vessel(&mut rng);
        let r = rng.range(1.0, 5000.0);
        let q: Vec<(engine::Sized, f64)> = ["A", "B", "C"]
            .iter()
            .map(|x| {
                let s = pianc(x);
                (
                    engine::size(&s, &v).unwrap(),
                    engine::widening(&s, &v, r).unwrap(),
                )
            })
            .collect();
        for pair in q.windows(2) {
            let ((hi, whi), (lo, wlo)) = (&pair[0], &pair[1]);
            assert!(
                hi.bottom_width_straight_m >= lo.bottom_width_straight_m,
                "{v:?}"
            );
            assert!(hi.min_bend_radius_m >= lo.min_bend_radius_m, "{v:?}");
            assert!(hi.design_depth_m >= lo.design_depth_m, "{v:?}");
            assert!(whi >= wlo, "{v:?} at R {r}");
        }
    }
}

/// Adding a set never makes a design basis value smaller: the combined channel is at least as wide,
/// as deep and as gently bent as each set alone, and widens at least as much at any radius.
#[test]
fn adding_a_set_never_makes_a_design_basis_value_smaller() {
    let mut rng = Rng(13);
    for _ in 0..CASES {
        let v = vessel(&mut rng);
        let q = rng.pick(&["A", "B", "C"]);
        let end = rng.pick(&["permissive", "strict"]);
        let one = engine::size(&pianc(q), &v).unwrap();
        let both = Selection::built_in(
            &["pianc-wg141", "gb50139"],
            &[(engine::SE_QUALITY, q), (engine::RANGE_END, end)],
        )
        .unwrap();
        let two = engine::size(&both, &v).unwrap();
        assert!(
            two.bottom_width_straight_m >= one.bottom_width_straight_m,
            "{v:?}"
        );
        assert!(two.min_bend_radius_m >= one.min_bend_radius_m, "{v:?}");
        assert!(two.design_depth_m >= one.design_depth_m, "{v:?}");
        let r = rng.range(1.0, 5000.0);
        assert!(
            engine::widening(&both, &v, r).unwrap() >= engine::widening(&pianc(q), &v, r).unwrap(),
            "{v:?} at R {r}"
        );
    }
}

/// One design for one boundary case: the Rule's own Measure takes `value`; every other Measure is
/// the vessel's or the radius; the inputs meet the Rule's applicability.
struct Case<'a> {
    rule: &'a Rule,
    vessel: &'a Vessel,
    inputs: Vec<(String, String)>,
    value: f64,
}

impl MeasureSource for Case<'_> {
    fn def(&self, id: &str) -> Option<arc_rules::domain::MeasureInfo> {
        ChannelDomain.measure(id)
    }
    fn objects(&self, object: &str) -> Vec<String> {
        vec![object.to_string()]
    }
    fn value(&self, m: &MeasureRef, _: usize) -> Result<MeasureValue, String> {
        let v = if m.id == self.rule.measure.id {
            self.value
        } else {
            match m.id.as_str() {
                engine::LOA => self.vessel.loa_m,
                engine::BEAM => self.vessel.beam_m,
                engine::DRAUGHT => self.vessel.draught_m,
                engine::BEND_RADIUS => 400.0,
                _ => 0.0,
            }
        };
        Ok(MeasureValue::Number(v))
    }
    fn input(&self, name: &str) -> Option<String> {
        self.inputs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

/// The inputs that make a Rule apply: the first value of each of its conditions.
fn inputs_for(rule: &Rule) -> Vec<(String, String)> {
    rule.applies_when
        .conditions
        .iter()
        .map(|(k, vs)| (k.clone(), vs[0].clone()))
        .collect()
}

/// The outcome of one Rule alone at `value`, and its limit.
fn outcome(set: &RuleSet, rule: &Rule, vessel: &Vessel, value: f64) -> (Outcome, f64) {
    let one = RuleSet {
        rules: vec![rule.clone()],
        ..set.clone()
    };
    let case = Case {
        rule,
        vessel,
        inputs: inputs_for(rule),
        value,
    };
    let trace = evaluate(&[&one], SHAPES, &case);
    let row = trace
        .rows
        .iter()
        .find(|r| r.rule.as_deref() == Some(rule.id.as_str()) && r.object.is_some())
        .unwrap_or_else(|| panic!("{}: no row", rule.id));
    (row.outcome, row.limit.unwrap_or(f64::NAN))
}

/// The vessels the boundary cases use: a class IV motor vessel of typical size, and one at each
/// WG 141 Table A.2 entry, so both branches of the GB width are tested. The formation and class come
/// from each Rule's applicability where it names them.
fn boundary_vessels(rule: &Rule) -> Vec<Vessel> {
    let input = |name: &str, default: &str| {
        inputs_for(rule)
            .into_iter()
            .find(|(k, _)| k == name)
            .map_or(default.to_string(), |(_, v)| v)
    };
    [(110.0, 11.4), (223.0, 32.4), (186.0, 32.4), (167.0, 21.6)]
        .into_iter()
        .map(|(loa_m, beam_m)| Vessel {
            loa_m,
            beam_m,
            draught_m: 2.5,
            formation: input(engine::FORMATION, "motor_vessel"),
            class_name: input(engine::VESSEL_CLASS, "IV"),
        })
        .collect()
}

/// The evaluated limit Rules of the two published sets.
fn limit_rules() -> Vec<(&'static RuleSet, &'static Rule)> {
    let lib = engine::built_in();
    let mut out = Vec::new();
    for id in ["pianc-wg141", "gb50139"] {
        let set = lib.lookup(id).unwrap();
        for r in &set.rules {
            if r.evaluated && r.kind != Kind::Note && r.kind != Kind::Check {
                out.push((set, r));
            }
        }
    }
    out
}

/// Every evaluated Rule passes at its limit and fails one step below it (each is a lower bound with
/// no tolerance), keyed by rule id.
#[test]
fn every_evaluated_rule_passes_just_inside_and_fails_just_outside_its_limit() {
    let rules = limit_rules();
    assert!(rules.len() >= 20, "only {} Rules", rules.len());
    for (set, rule) in rules {
        for v in boundary_vessels(rule) {
            let (_, limit) = outcome(set, rule, &v, 0.0);
            assert!(
                limit.is_finite() && limit > 0.0,
                "{}: limit {limit}",
                rule.id
            );
            let (inside, _) = outcome(set, rule, &v, limit);
            let (outside, _) = outcome(set, rule, &v, limit.next_down());
            assert_eq!(inside, Outcome::Pass, "{} at {limit}", rule.id);
            assert_eq!(outside, Outcome::Fail, "{} below {limit}", rule.id);
        }
    }
}

/// The coefficients of a Rule's formula limit, by name.
fn coefficients(rule: &Rule) -> Vec<(String, f64)> {
    match &rule.limit {
        Some(Limit::Formula { coefficients, .. }) => coefficients.clone(),
        Some(Limit::Constant { value, .. }) => vec![("value".into(), *value)],
        _ => Vec::new(),
    }
}

fn with_coefficient(rule: &Rule, name: &str, x: f64) -> Rule {
    let mut r = rule.clone();
    match &mut r.limit {
        Some(Limit::Formula { coefficients, .. }) => {
            for (k, v) in coefficients.iter_mut() {
                if k == name {
                    *v = x;
                }
            }
        }
        Some(Limit::Constant { value, .. }) => *value = x,
        _ => {}
    }
    r
}

/// Move each limit coefficient of each Rule one step up and down, and see whether a boundary case
/// changes outcome. A step is the ADR 0005 tolerance, 1e-9 relative: one unit in the last place is
/// lost in rounding (2.5 + 0.5 and 2.5 + 0.5000000000000001 are the same number). Checked once and
/// reported in the pull request; not a gate. Run with
/// `cargo test -p owd-core --test rules_proofs -- --ignored --nocapture`.
#[test]
#[ignore = "a report, not a gate: run on purpose"]
fn report_which_one_step_moves_a_boundary_case_notices() {
    let (mut noticed, mut missed) = (0, Vec::new());
    for (set, rule) in limit_rules() {
        for (name, x) in coefficients(rule) {
            let step = if x == 0.0 { 1e-9 } else { x.abs() * 1e-9 };
            let mut seen = false;
            for v in boundary_vessels(rule) {
                let (_, limit) = outcome(set, rule, &v, 0.0);
                for moved in [x + step, x - step] {
                    let r = with_coefficient(rule, &name, moved);
                    let at = outcome(set, &r, &v, limit).0;
                    let below = outcome(set, &r, &v, limit.next_down()).0;
                    if at != Outcome::Pass || below != Outcome::Fail {
                        seen = true;
                    }
                }
            }
            if seen {
                noticed += 1;
            } else {
                missed.push(format!("{} `{name}`", rule.id));
            }
        }
    }
    println!("one-step moves noticed by a boundary case: {noticed}");
    println!("not noticed ({}):", missed.len());
    for m in &missed {
        println!("  {m}");
    }
}

/// WG 141's Middle Rhine worked example (5.3.3 text box, p104): class VIb pushed convoy, L 185 m,
/// B 22.8 m, Safety and Ease quality B, at the minimum radius R = 3 L = 555 m.
///
/// Label: sourced - PIANC WG 141 (2019) 5.3.3 text box, p104: basic width 68.4 m, bend widening
/// 30.8 m at c_C = 0.5, minimum fairway width 99.2 m.
///
/// The known divergence: the example's totals add up against 3.0 B = 68.4 m (the Table 5.7 floor
/// "for security reasons"), not against the 3.2 B = 72.96 m that Table 5.7 gives for quality B and
/// that the example quotes. OWD sizes quality B to 3.2 B, as the table says, so its total is 103.8 m,
/// not 99.2 m. The difference is recorded here, not smoothed over.
#[test]
fn the_wg141_middle_rhine_example_and_its_known_divergence() {
    let v = Vessel {
        loa_m: 185.0,
        beam_m: 22.8,
        draught_m: 2.8,
        formation: "pushed_convoy".into(),
        class_name: "VIb".into(),
    };
    let s = pianc("B");
    let sized = engine::size(&s, &v).unwrap();
    // R_min = 3 L = 555 m, as the example sets it.
    assert_eq!(sized.min_bend_radius_m, 555.0);
    // dF_C = 0.5 L^2 / R = 30.83 m; the example prints 30.8.
    let widening = engine::widening(&s, &v, 555.0).unwrap();
    assert!((widening - 30.8).abs() < 0.05, "{widening}");
    assert_eq!(widening, sized.bend_widening_at_rmin_m);
    // The example's total, 99.2 m, is 3.0 B + 30.8 ...
    assert!((3.0 * 22.8 + widening - 99.2).abs() < 0.05);
    // ... but OWD's quality B basic width is 3.2 B, as Table 5.7 tabulates it.
    assert_eq!(sized.bottom_width_straight_m, 3.2 * 22.8);
    let owd_total = sized.bottom_width_straight_m + widening;
    assert!((owd_total - 103.8).abs() < 0.05, "{owd_total}");
    // The whole difference is the basic width: 3.2 B against the example's 3.0 B = 68.4 m.
    assert!((sized.bottom_width_straight_m - 68.4 - 0.2 * 22.8).abs() < 1e-9);
}
