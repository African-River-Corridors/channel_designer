//! Spec 0007: PIANC WG 141 and GB 50139 sizing on the shared rules engine (`arc-rules`).
//!
//! Label: snapshot. `tests/data/old-ruleset-snapshot.json` is what the old ruleset code printed
//! (at spec 0007 T5, before T8 removed it) for every old ruleset × every built-in vessel class ×
//! five draughts. The engine path must give the same numbers bit for bit, and the same warnings.
//! Never hand-edit it. The other tests here are `Label: property`.

use arc_rules::load::{content_hash, load};
use owd_core::rules::engine::{self, BUILT_IN, ChannelDomain, selection_for_old_ruleset};
use owd_core::rules::rulesets::channel_shape_for;
use owd_core::rules::vessel_catalogue;
use serde_json::Value;

fn snapshot() -> Vec<Value> {
    serde_json::from_str(include_str!("data/old-ruleset-snapshot.json")).unwrap()
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

/// Every old ruleset (CN-reduced: one claimed canal reach over the whole line) × every built-in
/// vessel class × five draughts: the engine path gives the old numbers bit for bit, the same bend
/// widening at five radii, the same width basis and the same warnings.
#[test]
fn the_engine_gives_the_old_rulesets_numbers_and_warnings_bit_for_bit() {
    let rows = snapshot();
    assert_eq!(rows.len(), 6 * 12 * 5);
    for row in &rows {
        let rs = row["ruleset"].as_str().unwrap();
        let vessel = row["vessel"].as_str().unwrap();
        let draught = row["draught_m"].as_f64();
        let s =
            channel_shape_for(&selection_for_old_ruleset(rs).unwrap(), vessel, draught).unwrap();
        let case = format!("{rs} {vessel} {draught:?}");
        for (k, v) in [
            ("bottom_width_straight_m", s.bottom_width_straight_m),
            ("min_bend_radius_m", s.min_bend_radius_m),
            ("design_depth_m", s.design_depth_m),
            ("bend_widening_at_rmin_m", s.bend_widening_at_rmin_m),
            ("bottom_width_at_rmin_m", s.bottom_width_at_rmin_m),
        ] {
            assert_eq!(v.to_bits(), f(&row[k]).to_bits(), "{case} {k}");
        }
        for (r, w) in row["radii"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["widening_at"].as_array().unwrap())
        {
            assert_eq!(
                s.bend_widening_m(f(r)).to_bits(),
                f(w).to_bits(),
                "{case} at R {r}"
            );
        }
        assert_eq!(
            s.width_basis,
            row["width_basis"].as_str().unwrap(),
            "{case}"
        );
        let warnings: Vec<&str> = row["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap())
            .collect();
        assert_eq!(s.warnings, warnings, "{case}");
        // A straight and a radius that is not a number, as the water checks may read them.
        assert_eq!(s.bend_widening_m(f64::INFINITY), 0.0);
        assert_eq!(s.bend_widening_m(0.0), 0.0);
        assert!(s.bend_widening_m(f64::NAN).is_nan());
    }
}

/// The check above can fail: a set whose quality B width is one bit off is caught.
#[test]
fn the_snapshot_check_catches_a_one_bit_change() {
    let (_, text) = BUILT_IN[0];
    let moved = text.replace("\"k\": 3.2,", "\"k\": 3.2000000000000006,");
    assert_ne!(moved, text);
    let set = load(&moved, &ChannelDomain).unwrap().set;
    let selection = engine::Selection {
        sets: vec![std::sync::Arc::new(set)],
        inputs: vec![(engine::SE_QUALITY.into(), "B".into())],
        reaches: vec![engine::WaterwayReach::whole_river()],
    };
    let row = snapshot()
        .into_iter()
        .find(|r| r["ruleset"] == "PIANC-B" && r["vessel"] == "CEMT_IV" && r["draught_m"].is_null())
        .unwrap();
    let vessel = owd_core::rules::rulesets::vessel_for("CEMT_IV", None).unwrap();
    let sized = engine::size(&selection, &vessel).unwrap();
    assert_ne!(
        sized.bottom_width_straight_m.to_bits(),
        f(&row["bottom_width_straight_m"]).to_bits()
    );
}

/// A selection with no bend-widening Rule cannot size a channel: the water checks need one.
#[test]
fn a_selection_with_no_widening_rule_is_refused() {
    let (_, text) = BUILT_IN[0];
    let mut set = load(text, &ChannelDomain).unwrap().set;
    set.rules.retain(|r| r.measure.id != engine::BEND_WIDENING);
    let selection = engine::Selection {
        sets: vec![std::sync::Arc::new(set)],
        inputs: vec![(engine::SE_QUALITY.into(), "B".into())],
        reaches: vec![engine::WaterwayReach::whole_river()],
    };
    let e = channel_shape_for(&selection, "CEMT_IV", None).unwrap_err();
    assert!(e.raw_message().contains(engine::BEND_WIDENING), "{e:?}");
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
    assert_eq!(set.rules.len(), 11);
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
    for (vessel, v) in vessel_catalogue() {
        let shape = |rs| channel_shape_for(&selection_for_old_ruleset(rs).unwrap(), &vessel, None);
        let a = shape("CN").unwrap();
        let b = shape("CN-conservative").unwrap();
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

fn canal(claim: bool) -> engine::WaterwayReach {
    engine::WaterwayReach {
        label: "0 to 2000 m".into(),
        waterway: "canal".into(),
        claim: if claim { "yes" } else { "no" }.into(),
        claim_typed: claim,
    }
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

/// A line with one canal reach (claim made) and one river reach sizes its alignment to the river
/// reach's R_min. At the line's stations the reduced Rule is `not applicable` on river stations and
/// tested on canal ones; the claim shows in the trace as a typed input.
#[test]
fn a_canal_and_a_river_reach_size_to_the_river_rmin_and_the_reduced_rule_applies_on_canal_only() {
    let vessel = engine::Vessel {
        loa_m: 110.0,
        beam_m: 11.4,
        draught_m: 2.5,
        formation: "motor_vessel".into(),
        class_name: "IV".into(),
    };
    let river = engine::WaterwayReach {
        label: "2000 m to the end".into(),
        ..engine::WaterwayReach::whole_river()
    };
    let gb = |reaches| {
        engine::Selection::built_in(&["gb50139", "owd"], &[(engine::RANGE_END, "strict")])
            .unwrap()
            .with_reaches(reaches)
    };
    let both = gb(vec![canal(true), river.clone()]);
    let sized = engine::size(&both, &vessel).unwrap();
    // Motor vessel: 4L on the river, 3L (reduced) on the claimed canal. The line takes 4L.
    assert_eq!(sized.min_bend_radius_m, 4.0 * 110.0);
    let only_canal = engine::size(&gb(vec![canal(true)]), &vessel).unwrap();
    assert_eq!(only_canal.min_bend_radius_m, 3.0 * 110.0);
    // Without the claim the canal takes the normal rule.
    let unclaimed = engine::size(&gb(vec![canal(false)]), &vessel).unwrap();
    assert_eq!(unclaimed.min_bend_radius_m, 4.0 * 110.0);
    // Stations of a line: two on the canal, two on the river.
    let stations: Vec<engine::Station> = [(0, 330.0), (0, 500.0), (1, 450.0), (1, 900.0)]
        .iter()
        .enumerate()
        .map(|(i, (reach, r))| engine::Station {
            label: format!("station {i}"),
            reach: *reach,
            radius_m: *r,
        })
        .collect();
    let trace = both.check_stations(&vessel, &sized, &stations);
    let reduced: Vec<(&str, &str)> = trace
        .rows
        .iter()
        .filter(|r| r.rule.as_deref() == Some("gb50139/A.3.2/min-bend-radius-reduced-motor"))
        .map(|r| (r.object.as_deref().unwrap(), r.outcome.key()))
        .collect();
    assert_eq!(
        reduced,
        [
            ("station 0", "pass"),
            ("station 1", "pass"),
            ("station 2", "not applicable"),
            ("station 3", "not applicable")
        ]
    );
    // Each station checks its own reach's Rule: 450 m on the river passes 4L = 440 m.
    let normal: Vec<&str> = trace
        .rows
        .iter()
        .filter(|r| r.rule.as_deref() == Some("gb50139/A.3.2/min-bend-radius-motor"))
        .map(|r| r.outcome.key())
        .collect();
    assert_eq!(normal, ["not applicable", "not applicable", "pass", "pass"]);
    // The sizing trace names each reach with its claim, marked typed.
    let sizing = both.check(&vessel, &sized);
    assert!(sizing.rows.iter().any(|r| {
        r.rule.as_deref() == Some("gb50139/A.3.2/min-bend-radius-reduced-motor")
            && r.object
                .as_deref()
                .is_some_and(|o| o.contains("canal, reduced_radius_claim yes typed"))
    }));
}
