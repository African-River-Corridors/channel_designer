//! Spec 0007 T1 and T2: PIANC WG 141 and GB 50139 sizing go through the shared rules engine
//! (`arc-rules`).
//!
//! Label: property. The parity test compares the engine path with the old `Ruleset` code (kept for
//! this check only) bit for bit; no fixed number is pinned here.

use arc_rules::load::{content_hash, load};
use owd_core::rules::engine::{self, BUILT_IN, ChannelDomain};
use owd_core::rules::rulesets::{RulesetRegistry, channel_shape, channel_shape_legacy};
use owd_core::rules::vessel_catalogue;

const ENGINE: [&str; 5] = ["PIANC-A", "PIANC-B", "PIANC-C", "CN", "CN-conservative"];

/// Every PIANC and GB ruleset (CN-reduced waits for reaches, T4) × every built-in vessel class × a spread of draughts (the nominal one and
/// four overrides from shallow to deep): the engine path gives the old shape bit for bit, and the
/// same bend widening at every radius the water checks read, a straight included.
#[test]
fn the_engine_gives_the_old_channel_shape_bit_for_bit() {
    let reg = RulesetRegistry::new();
    let radii = [
        -1.0,
        0.0,
        1e-9,
        1.0,
        37.5,
        150.0,
        333.3,
        1000.0,
        12345.678,
        f64::INFINITY,
    ];
    let mut n = 0;
    for ruleset in ENGINE {
        for (vessel, _) in vessel_catalogue() {
            for draught in [None, Some(0.8), Some(1.9), Some(2.5), Some(4.75)] {
                let new = channel_shape(&reg, ruleset, &vessel, draught).unwrap();
                let old = channel_shape_legacy(&reg, ruleset, &vessel, draught).unwrap();
                let bits = |s: &owd_core::rules::rulesets::ChannelShape| {
                    [
                        s.loa_m,
                        s.beam_m,
                        s.draught_m,
                        s.bottom_width_straight_m,
                        s.min_bend_radius_m,
                        s.design_depth_m,
                        s.bend_widening_at_rmin_m,
                        s.bottom_width_at_rmin_m,
                    ]
                    .map(f64::to_bits)
                };
                let case = format!("{ruleset} {vessel} {draught:?}");
                assert_eq!(bits(&new), bits(&old), "{case}");
                assert_eq!(new.width_basis, old.width_basis, "{case}");
                assert_eq!(new.source, old.source, "{case}");
                assert_eq!(new.warnings, old.warnings, "{case}");
                for r in radii.into_iter().chain([new.min_bend_radius_m]) {
                    assert_eq!(
                        new.bend_widening_m(r).to_bits(),
                        old.bend_widening_m(r).to_bits(),
                        "{case} at R {r}"
                    );
                    assert_eq!(
                        new.bottom_width_at_m(r).to_bits(),
                        old.bottom_width_at_m(r).to_bits(),
                        "{case} at R {r}"
                    );
                }
                assert!(new.bend_widening_m(f64::NAN).is_nan());
                n += 1;
            }
        }
    }
    assert!(n >= 5 * 5 * 10, "only {n} cases");
}

/// The check above can fail: a set whose quality B width is one bit off is caught.
#[test]
fn the_parity_check_catches_a_one_bit_change() {
    let (_, text) = BUILT_IN[0];
    let moved = text.replace("\"k\": 3.2,", "\"k\": 3.2000000000000006,");
    assert_ne!(moved, text);
    let set = load(&moved, &ChannelDomain).unwrap().set;
    let selection = engine::Selection {
        sets: vec![std::sync::Arc::new(set)],
        inputs: vec![(engine::SE_QUALITY.into(), "B".into())],
    };
    let reg = RulesetRegistry::new();
    let old = channel_shape_legacy(&reg, "PIANC-B", "CEMT_IV", None).unwrap();
    let vessel = engine::Vessel {
        loa_m: old.loa_m,
        beam_m: old.beam_m,
        draught_m: old.draught_m,
        formation: old.formation.clone(),
        class_name: "IV".into(),
    };
    let sized = engine::size(&selection, &vessel).unwrap();
    assert_ne!(
        sized.bottom_width_straight_m.to_bits(),
        old.bottom_width_straight_m.to_bits()
    );
}

/// The built-in set loads through the loader a user's file takes, and its content hash is the one
/// the loader records.
#[test]
fn pianc_wg141_loads_through_the_user_loader_with_its_content_hash() {
    let (path, text) = BUILT_IN[0];
    assert_eq!(path, "rules/pianc-wg141/0.3.0.json");
    assert_eq!(BUILT_IN[1].0, "rules/gb50139/0.3.0.json");
    let loaded = load(text, &ChannelDomain).unwrap();
    let set = &loaded.set;
    assert_eq!(set.key(), "pianc-wg141@0.3.0");
    assert_eq!(set.hash.len(), 64);
    assert_eq!(set.hash, content_hash(set));
    let lib = engine::built_in();
    assert_eq!(lib.lookup("pianc-wg141").unwrap().hash, set.hash);
    // Every blocking Rule is unverified, and the loader says so.
    assert_eq!(loaded.warnings.len(), set.rules.len());
}

/// Each Rule has origin `guideline`, cites WG 141 with a clause or table and a page, is unverified,
/// sizes the channel and is a lower bound.
#[test]
fn every_pianc_rule_cites_wg141_and_is_an_unverified_sizing_lower_bound() {
    let set = &engine::built_in().lookup("pianc-wg141").unwrap();
    assert_eq!(set.rules.len(), 12);
    for r in &set.rules {
        assert_eq!(r.origin, arc_rules::model::Origin::Guideline, "{}", r.id);
        assert!(r.source.document.contains("WG 141"), "{}", r.id);
        assert!(!r.source.locator.is_empty(), "{}", r.id);
        assert!(!r.source.page.is_empty(), "{}", r.id);
        assert!(r.verified.is_none(), "{}", r.id);
        assert!(r.sizes && r.evaluated, "{}", r.id);
        assert_eq!(r.compare, Some(arc_rules::model::Compare::Ge), "{}", r.id);
        assert_eq!(r.since, "0.3.0", "{}", r.id);
    }
}

/// The loader refuses a WG 141 Rule with no locator.
#[test]
fn the_loader_refuses_a_rule_with_no_locator() {
    let (_, text) = BUILT_IN[0];
    let bare = text.replacen("\"locator\": \"Table 5.7\",", "\"locator\": \"\",", 1);
    assert_ne!(bare, text);
    let e = load(&bare, &ChannelDomain).unwrap_err();
    assert!(e.contains("must cite its paragraph"), "{e}");
}

/// The selection's input picks the quality: the engine trace has one governing row per sized
/// Measure, from the chosen quality's Rule.
#[test]
fn the_quality_input_picks_the_governing_rule() {
    for q in ["A", "B", "C"] {
        let s = engine::Selection::built_in(&["pianc-wg141"], &[(engine::SE_QUALITY, q)]).unwrap();
        let vessel = engine::Vessel {
            loa_m: 110.0,
            beam_m: 11.4,
            draught_m: 2.5,
            formation: "motor_vessel".into(),
            class_name: "IV".into(),
        };
        let trace = s.trace(&vessel, 300.0);
        let governing: Vec<&str> = trace
            .rows
            .iter()
            .filter(|r| r.governing)
            .filter_map(|r| r.rule.as_deref())
            .collect();
        assert_eq!(governing.len(), 4, "{governing:?}");
        let suffix = format!("-{}", q.to_lowercase());
        assert!(
            governing.iter().all(|id| id.ends_with(&suffix)),
            "{governing:?}"
        );
    }
}

/// With one widening Rule selected, `widening` reads its formula without a full trace. It gives
/// the engine's governing limit bit for bit.
#[test]
fn the_one_rule_widening_equals_the_engines_governing_limit() {
    for q in ["A", "B", "C"] {
        let s = engine::Selection::built_in(&["pianc-wg141"], &[(engine::SE_QUALITY, q)]).unwrap();
        let vessel = engine::Vessel {
            loa_m: 185.0,
            beam_m: 11.4,
            draught_m: 3.5,
            formation: "pushed_convoy".into(),
            class_name: "Vb".into(),
        };
        let narrow = s.narrowed_to(engine::BEND_WIDENING, &vessel);
        assert_eq!(narrow.sets[0].rules.len(), 1);
        for r in [1e-6, 3.0, 92.5, 370.0, 555.5, 1e7] {
            let traced =
                engine::governing_limit(&s.trace(&vessel, r), engine::BEND_WIDENING).unwrap();
            let fast = engine::widening(&narrow, &vessel, r).unwrap();
            assert_eq!(fast.to_bits(), traced.to_bits(), "{q} at R {r}");
        }
    }
}

/// Every GB Rule has origin `national-practice` and says it was read in WG 141 Appendix A.3, is
/// unverified, and the Rules the range end changes link the finding.
#[test]
fn every_gb_rule_is_national_practice_read_in_wg141_appendix_a3() {
    let set = engine::built_in().lookup("gb50139").unwrap();
    assert_eq!(set.key(), "gb50139@0.3.0");
    assert_eq!(set.rules.len(), 9);
    for r in &set.rules {
        assert_eq!(
            r.origin,
            arc_rules::model::Origin::NationalPractice,
            "{}",
            r.id
        );
        assert!(
            r.source.edition.contains("read in WG 141 Appendix A.3"),
            "{}",
            r.id
        );
        assert!(r.verified.is_none(), "{}", r.id);
        let ranged = r
            .applies_when
            .conditions
            .iter()
            .any(|(k, _)| k == engine::RANGE_END);
        if ranged || r.id.ends_with("/swept-path") {
            assert_eq!(r.finding.as_deref(), Some("#250"), "{}", r.id);
        }
    }
}

/// The range end changes only the class IV-and-up depth and the swept path note (finding #250):
/// CN and CN-conservative give the same width, radius and widening, and for classes I to III the
/// same depth.
#[test]
fn the_range_end_changes_only_the_class_iv_and_up_depth() {
    let reg = RulesetRegistry::new();
    for (vessel, v) in vessel_catalogue() {
        let a = channel_shape(&reg, "CN", &vessel, None).unwrap();
        let b = channel_shape(&reg, "CN-conservative", &vessel, None).unwrap();
        assert_eq!(
            a.bottom_width_straight_m, b.bottom_width_straight_m,
            "{vessel}"
        );
        assert_eq!(a.min_bend_radius_m, b.min_bend_radius_m, "{vessel}");
        assert_eq!(
            a.bend_widening_at_rmin_m, b.bend_widening_at_rmin_m,
            "{vessel}"
        );
        let low = ["I", "II", "III"].contains(&v.class_name.as_str());
        assert_eq!(a.design_depth_m == b.design_depth_m, low, "{vessel}");
    }
}

/// The swept path note shows its value in the trace, with no test, at both range ends.
#[test]
fn the_swept_path_is_a_note_with_a_value() {
    for end in ["permissive", "strict"] {
        let s = engine::Selection::built_in(&["gb50139"], &[(engine::RANGE_END, end)]).unwrap();
        let vessel = engine::Vessel {
            loa_m: 110.0,
            beam_m: 11.4,
            draught_m: 2.5,
            formation: "motor_vessel".into(),
            class_name: "IV".into(),
        };
        let trace = s.trace(&vessel, 400.0);
        let row = trace
            .rows
            .iter()
            .find(|r| r.rule.as_deref() == Some("gb50139/A.3.2/swept-path"))
            .unwrap();
        assert_eq!(row.outcome, arc_rules::eval::Outcome::Note);
        assert!(matches!(row.value, Some(arc_rules::eval::MeasureValue::Number(x)) if x > 11.4));
    }
}

/// The selection an old ruleset maps to, with the `owd` checks (spec 0007, "Rule Sets at 0.3.0").
fn mapped(ruleset: &str) -> engine::Selection {
    let (set, input, value) = match ruleset {
        "PIANC-A" => ("pianc-wg141", engine::SE_QUALITY, "A"),
        "PIANC-B" => ("pianc-wg141", engine::SE_QUALITY, "B"),
        "PIANC-C" => ("pianc-wg141", engine::SE_QUALITY, "C"),
        "CN" => ("gb50139", engine::RANGE_END, "permissive"),
        _ => ("gb50139", engine::RANGE_END, "strict"),
    };
    engine::Selection::built_in(&[set, engine::OWD_SET], &[(input, value)]).unwrap()
}

/// A shape sized from a selection (with the `owd` checks) equals the old ruleset's, numbers,
/// width basis and warnings: the warnings now come from the Rule Trace of the sized channel.
#[test]
fn a_selection_gives_the_old_shape_and_its_warnings() {
    let reg = RulesetRegistry::new();
    let mut warned = 0;
    for ruleset in ENGINE {
        for (vessel, _) in vessel_catalogue() {
            for draught in [None, Some(0.8), Some(1.9), Some(2.5), Some(4.75)] {
                let new = owd_core::rules::rulesets::channel_shape_for(
                    &mapped(ruleset),
                    &vessel,
                    draught,
                )
                .unwrap();
                let old = channel_shape_legacy(&reg, ruleset, &vessel, draught).unwrap();
                let case = format!("{ruleset} {vessel} {draught:?}");
                for (a, b) in [
                    (new.bottom_width_straight_m, old.bottom_width_straight_m),
                    (new.min_bend_radius_m, old.min_bend_radius_m),
                    (new.design_depth_m, old.design_depth_m),
                    (new.bend_widening_at_rmin_m, old.bend_widening_at_rmin_m),
                    (new.bottom_width_at_rmin_m, old.bottom_width_at_rmin_m),
                    (new.bend_widening_m(250.0), old.bend_widening_m(250.0)),
                ] {
                    assert_eq!(a.to_bits(), b.to_bits(), "{case}");
                }
                assert_eq!(new.width_basis, old.width_basis, "{case}");
                assert_eq!(new.warnings, old.warnings, "{case}");
                warned += new.warnings.len();
            }
        }
    }
    assert!(warned > 100, "the warnings were exercised: {warned}");
}

/// Two sets at once: every Rule applies and the governing limit of each Requirement sizes the
/// channel, so each quantity is the larger of the two single-set values. A synthetic vessel.
#[test]
fn two_sets_size_each_quantity_to_the_larger_single_set_value() {
    let vessel = engine::Vessel {
        loa_m: 135.0,
        beam_m: 14.2,
        draught_m: 3.1,
        formation: "pushed_convoy".into(),
        class_name: "Vb".into(),
    };
    let one = |ids: &[&str]| {
        let s = engine::Selection::built_in(
            ids,
            &[(engine::SE_QUALITY, "B"), (engine::RANGE_END, "strict")],
        )
        .unwrap();
        engine::size(&s, &vessel).unwrap()
    };
    let p = one(&["pianc-wg141", "owd"]);
    let g = one(&["gb50139", "owd"]);
    let both = one(&["pianc-wg141", "gb50139", "owd"]);
    let max = |a: f64, b: f64| if b > a { b } else { a };
    assert_eq!(
        both.bottom_width_straight_m,
        max(p.bottom_width_straight_m, g.bottom_width_straight_m)
    );
    assert_eq!(
        both.min_bend_radius_m,
        max(p.min_bend_radius_m, g.min_bend_radius_m)
    );
    assert_eq!(both.design_depth_m, max(p.design_depth_m, g.design_depth_m));
    // The two sets disagree on something, so the test says more than "the same set twice".
    assert!(
        p.bottom_width_straight_m != g.bottom_width_straight_m
            || p.min_bend_radius_m != g.min_bend_radius_m
    );
    // The widening at the combined R_min is the larger of the two formulas there.
    let r = both.min_bend_radius_m;
    let wid = |ids: &[&str]| {
        let s = engine::Selection::built_in(ids, &[(engine::SE_QUALITY, "B")]).unwrap();
        engine::widening(&s, &vessel, r).unwrap()
    };
    assert_eq!(
        both.bend_widening_at_rmin_m,
        max(wid(&["pianc-wg141"]), {
            let s = engine::Selection::built_in(&["gb50139"], &[(engine::RANGE_END, "strict")])
                .unwrap();
            engine::widening(&s, &vessel, r).unwrap()
        })
    );
}

/// Selecting one set twice changes nothing.
#[test]
fn one_set_selected_twice_changes_nothing() {
    let vessel = engine::Vessel {
        loa_m: 110.0,
        beam_m: 11.4,
        draught_m: 2.5,
        formation: "motor_vessel".into(),
        class_name: "IV".into(),
    };
    let i = [(engine::SE_QUALITY, "C")];
    let once = engine::Selection::built_in(&["pianc-wg141", "owd"], &i).unwrap();
    let twice = engine::Selection::built_in(&["pianc-wg141", "pianc-wg141", "owd"], &i).unwrap();
    assert_eq!(
        engine::size(&once, &vessel).unwrap(),
        engine::size(&twice, &vessel).unwrap()
    );
}
