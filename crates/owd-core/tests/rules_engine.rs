//! Spec 0007: PIANC WG 141 and GB 50139 sizing on the shared rules engine (`arc-rules`).
//!
//! Label: snapshot. `tests/data/old-ruleset-snapshot.json` is what the old ruleset code printed
//! (at spec 0007 T5, before T8 removed it) for every old ruleset × every built-in vessel class ×
//! five draughts. The engine path must give the same numbers bit for bit, and the same warnings,
//! for the PIANC rulesets. The CN rows are history: #250 moved `gb50139` to GB 50139-2014 itself
//! (`gb50139_2014.rs`). Never hand-edit it. The other tests here are `Label: property`.

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

/// Every old PIANC ruleset × every built-in vessel class × five draughts: the engine path gives the
/// old numbers bit for bit, the same bend widening at five radii, the same width basis and the same
/// warnings. The CN rows moved on purpose with #250 and are not read.
#[test]
fn the_engine_gives_the_old_pianc_rulesets_numbers_and_warnings_bit_for_bit() {
    let rows = snapshot();
    assert_eq!(rows.len(), 6 * 12 * 5);
    let pianc: Vec<&Value> = rows
        .iter()
        .filter(|r| r["ruleset"].as_str().unwrap().starts_with("PIANC"))
        .collect();
    assert_eq!(pianc.len(), 3 * 12 * 5);
    for row in pianc {
        let rs = row["ruleset"].as_str().unwrap();
        let vessel = row["vessel"].as_str().unwrap();
        let draught = row["draught_m"].as_f64();
        // Spec 0015 T0: the shared catalogue (ovcd-core v0.2.4) gives CEMT-Va a draught range of
        // 2.5-2.8 m (v0.2.2: 2.5-4.5 m), so its default draught moved from 3.5 m to 2.65 m. The
        // engine is checked here on the inputs the snapshot was made with; the move is design impact.
        let draught = draught.or((vessel == "CEMT_Va").then_some(3.5));
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
            family: "CEMT".into(),
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
            family: "CEMT".into(),
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

/// Every GB Rule is national practice read in GB 50139-2014 itself, but the bend widening: the
/// 2014 text gives no formula, so that Rule cites WG 141's summary of the 2004 edition, as a
/// guideline. Every Rule is unverified.
#[test]
fn every_gb_rule_cites_gb50139_2014_but_the_widening_which_cites_wg141() {
    let set = engine::built_in().lookup("gb50139").unwrap();
    assert_eq!(set.key(), "gb50139@0.3.0");
    assert_eq!(set.rules.len(), 24);
    for r in &set.rules {
        assert!(r.verified.is_none(), "{}", r.id);
        if r.measure.id == engine::BEND_WIDENING {
            assert_eq!(r.origin, arc_rules::model::Origin::Guideline, "{}", r.id);
            assert!(r.source.document.contains("WG 141"), "{}", r.id);
            assert!(
                r.source
                    .also
                    .iter()
                    .any(|a| a.contains("GB 50139-2014 3.0.5(3)"))
            );
        } else {
            assert_eq!(
                r.origin,
                arc_rules::model::Origin::NationalPractice,
                "{}",
                r.id
            );
            assert!(r.source.document.starts_with("GB 50139-2014"), "{}", r.id);
            assert!(!r.source.locator.is_empty(), "{}", r.id);
        }
    }
}

/// The range end changes the depth of every class with a Table A.0.1 range (I to V) and the width
/// of a special design vessel, never a Table 3.0.2-1 width or a radius.
#[test]
fn the_range_end_changes_the_depth_and_the_special_vessels_width_only() {
    for (vessel, v) in vessel_catalogue() {
        if v.family != "CN" {
            continue;
        }
        let shape = |rs| channel_shape_for(&selection_for_old_ruleset(rs).unwrap(), &vessel, None);
        let a = shape("CN").unwrap();
        let b = shape("CN-conservative").unwrap();
        assert_eq!(a.min_bend_radius_m, b.min_bend_radius_m, "{vessel}");
        assert_eq!(
            a.bend_widening_at_rmin_m, b.bend_widening_at_rmin_m,
            "{vessel}"
        );
        assert!(a.design_depth_m < b.design_depth_m, "{vessel}");
        let table = a.width_basis == "table";
        assert_eq!(
            a.bottom_width_straight_m == b.bottom_width_straight_m,
            table,
            "{vessel}"
        );
        // GB 50139-2014 has no reduced radius: CN-reduced is CN-conservative.
        let c = shape("CN-reduced").unwrap();
        assert_eq!(
            (
                c.bottom_width_straight_m,
                c.min_bend_radius_m,
                c.design_depth_m
            ),
            (
                b.bottom_width_straight_m,
                b.min_bend_radius_m,
                b.design_depth_m
            ),
            "{vessel}"
        );
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
        class_name: "III".into(),
        family: engine::CN_FAMILY.into(),
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
        family: "CEMT".into(),
    };
    let i = [(engine::SE_QUALITY, "C")];
    let once = engine::Selection::built_in(&["pianc-wg141", "owd"], &i).unwrap();
    let twice = engine::Selection::built_in(&["pianc-wg141", "pianc-wg141", "owd"], &i).unwrap();
    assert_eq!(
        engine::size(&once, &vessel).unwrap(),
        engine::size(&twice, &vessel).unwrap()
    );
}

/// A user's Rule Set with a reduced radius on claimed canal reaches only (the 2004 edition of GB
/// 50139 had one; 2014 removed it, so no built-in Rule reads the claim).
const CANAL_SET: &str = r##"{
  "id": "canal-test", "family": "canal-test", "title": "A reduced radius on claimed canal reaches",
  "version": "0.1.0", "edition": {"document": "test", "edition": "test"}, "sources": [],
  "status": "draft", "applies_when": {}, "includes": [], "known_conflicts": [], "tables": [],
  "rules": [
    {"id": "canal-test/1/min-bend-radius-motor", "title": "4 x L", "origin": "guideline",
     "source": {"document": "test", "edition": "test", "locator": "1", "page": "1", "access": "free"},
     "codified": {"by": "agent", "date": "2026-10-22", "review": "#250"}, "verified": false,
     "applies_when": {"formation": ["motor_vessel"], "reduced_radius_claim": ["no"]},
     "kind": "formula", "measure": {"id": "station.radius"}, "compare": "ge",
     "limit": {"shape": "multiple", "unit": "m", "k": 4.0, "x": "vessel.loa"},
     "tolerance": 0, "consequence": "block", "sizes": true, "evaluated": true, "since": "0.1.0"},
    {"id": "canal-test/1/min-bend-radius-reduced-motor", "title": "3 x L on a claimed canal",
     "origin": "guideline",
     "source": {"document": "test", "edition": "test", "locator": "1", "page": "1", "access": "free"},
     "codified": {"by": "agent", "date": "2026-10-22", "review": "#250"}, "verified": false,
     "applies_when": {"formation": ["motor_vessel"], "reduced_radius_claim": ["yes"], "waterway": ["canal"]},
     "kind": "formula", "measure": {"id": "station.radius"}, "compare": "ge",
     "limit": {"shape": "multiple", "unit": "m", "k": 3.0, "x": "vessel.loa"},
     "tolerance": 0, "consequence": "block", "sizes": true, "evaluated": true, "since": "0.1.0"}
  ]
}"##;

/// A line with one canal reach (claim made) and one river reach sizes its alignment to the river
/// reach's R_min. At the line's stations the reduced Rule is `not applicable` on river stations and
/// tested on canal ones; the claim shows in the trace as a typed input. The reduced Rule comes from
/// a user's set.
#[test]
fn a_canal_and_a_river_reach_size_to_the_river_rmin_and_the_reduced_rule_applies_on_canal_only() {
    let vessel = engine::Vessel {
        loa_m: 110.0,
        beam_m: 11.4,
        draught_m: 2.5,
        formation: "motor_vessel".into(),
        class_name: "IV".into(),
        family: "CEMT".into(),
    };
    let river = engine::WaterwayReach {
        label: "2000 m to the end".into(),
        ..engine::WaterwayReach::whole_river()
    };
    let set = std::sync::Arc::new(load(CANAL_SET, &ChannelDomain).unwrap().set);
    let pianc = std::sync::Arc::new(engine::built_in().lookup("pianc-wg141").unwrap().clone());
    let sel = |reaches| engine::Selection {
        sets: vec![pianc.clone(), set.clone()],
        inputs: vec![(engine::SE_QUALITY.into(), "C".into())],
        reaches,
    };
    let both = sel(vec![canal(true), river.clone()]);
    let sized = engine::size(&both, &vessel).unwrap();
    // Motor vessel: 4L on the river, 3L (reduced) on the claimed canal. The line takes 4L.
    assert_eq!(sized.min_bend_radius_m, 4.0 * 110.0);
    let only_canal = engine::size(&sel(vec![canal(true)]), &vessel).unwrap();
    assert_eq!(only_canal.min_bend_radius_m, 3.0 * 110.0);
    // Without the claim the canal takes the normal rule.
    let unclaimed = engine::size(&sel(vec![canal(false)]), &vessel).unwrap();
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
        .filter(|r| r.rule.as_deref() == Some("canal-test/1/min-bend-radius-reduced-motor"))
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
        .filter(|r| r.rule.as_deref() == Some("canal-test/1/min-bend-radius-motor"))
        .map(|r| r.outcome.key())
        .collect();
    assert_eq!(normal, ["not applicable", "not applicable", "pass", "pass"]);
    // The sizing trace names each reach with its claim, marked typed.
    let sizing = both.check(&vessel, &sized);
    assert!(sizing.rows.iter().any(|r| {
        r.rule.as_deref() == Some("canal-test/1/min-bend-radius-reduced-motor")
            && r.object
                .as_deref()
                .is_some_and(|o| o.contains("canal, reduced_radius_claim yes typed"))
    }));
}

#[test]
fn the_size_form_of_a_choice_names_the_same_old_ruleset() {
    // Label: property. `old_name_for` undoes `old_name_choice` for every name that has the form (#362).
    for name in engine::OLD_RULESETS {
        if name == "CN-reduced" {
            continue;
        }
        let (set, input, value) = engine::old_name_choice(name).unwrap();
        assert_eq!(engine::old_name_for(set, &[(input, value)]).unwrap(), name);
        let versioned = format!("{set}@1.0");
        assert_eq!(
            engine::old_name_for(&versioned, &[("other", "x"), (input, value)]).unwrap(),
            name
        );
    }
    assert!(engine::old_name_for("pianc-wg141", &[]).is_err());
    assert!(engine::old_name_for("pianc-wg141", &[("se_quality", "Z")]).is_err());
    assert!(engine::old_name_for("my-set", &[("se_quality", "A")]).is_err());
}
