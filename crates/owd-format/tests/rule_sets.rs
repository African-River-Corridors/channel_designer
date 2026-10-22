//! Spec 0007 T3: the Rule Set selection in the document (`format_version` 3).
//!
//! Label: property. No fixed number; the sizing numbers are compared with the old path.

use owd_core::rules::engine::{self, BUILT_IN};
use owd_core::rules::engine::{old_name_choice, selection_for_old_ruleset};
use owd_core::rules::rulesets::channel_shape_for;
use owd_format::design::{DesignError, Session, Stage, StageStatus};
use serde_json::{Value, json};

fn settings(s: &Session) -> Value {
    s.document()["stages"]["sizing"]["settings"].clone()
}

#[test]
fn a_new_design_opens_with_pianc_wg141_quality_c_all_marked_default() {
    let s = Session::new("t", "2026-10-21");
    let st = settings(&s);
    let sets: Vec<(&str, &str)> = st["rule_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["id"].as_str().unwrap(), x["source"].as_str().unwrap()))
        .collect();
    assert_eq!(sets, [("pianc-wg141", "default"), ("owd", "default")]);
    assert_eq!(
        st["set_inputs"],
        json!([{"name": "se_quality", "value": "C", "source": "default"}])
    );
    for x in st["rule_sets"].as_array().unwrap() {
        assert_eq!(x["version"], "0.3.0");
        let set = engine::built_in()
            .lookup(x["id"].as_str().unwrap())
            .unwrap();
        assert_eq!(x["hash"], set.hash.as_str());
    }
    assert_eq!(s.document()["format_version"], 3);
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    // Nothing to size until a vessel is chosen, and `run` does not try.
    let mut s = s;
    assert_eq!(s.run_stale().unwrap(), Vec::<Stage>::new());
    // Choosing only a vessel keeps the default selection and its marks.
    s.set_vessel("CEMT_IV", None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let st = settings(&s);
    assert_eq!(st["rule_sets"][0]["source"], "default");
    assert_eq!(st["set_inputs"][0]["source"], "default");
    let r = &s.document()["stages"]["sizing"]["results"];
    let old = channel_shape_for(
        &selection_for_old_ruleset("PIANC-C").unwrap(),
        "CEMT_IV",
        None,
    )
    .unwrap();
    assert_eq!(
        r["bottom_width_straight_m"],
        json!(old.bottom_width_straight_m)
    );
    assert_eq!(r["rule_sets"], "pianc-wg141@0.3.0 (se_quality C)");
}

#[test]
fn typed_choices_are_marked_typed_and_owd_stays_selected() {
    let mut s = Session::new("t", "2026-10-21");
    s.set_rule_sets(&["gb50139"], &[("range_end", "strict")])
        .unwrap();
    s.set_vessel("CN_III_motor", Some(2.0)).unwrap();
    let st = settings(&s);
    assert_eq!(st["rule_sets"][0]["id"], "gb50139");
    assert_eq!(st["rule_sets"][0]["source"], "typed");
    assert_eq!(st["rule_sets"][1]["id"], "owd");
    // The PIANC quality goes with the PIANC set.
    assert_eq!(
        st["set_inputs"],
        json!([{"name": "range_end", "value": "strict", "source": "typed"}])
    );
    // Adding PIANC keeps the typed range end and gives the quality its default.
    s.set_rule_sets(&["pianc-wg141", "gb50139"], &[]).unwrap();
    assert_eq!(
        settings(&s)["set_inputs"],
        json!([
            {"name": "range_end", "value": "strict", "source": "typed"},
            {"name": "se_quality", "value": "C", "source": "default"}
        ])
    );
    // A GB selection with no range end is refused, naming the input.
    let mut t = Session::new("t", "2026-10-21");
    let e = t.set_rule_sets(&["gb50139"], &[]).unwrap_err();
    assert!(e.to_string().contains("range_end"), "{e}");
    // An unknown set or input value is refused.
    assert!(t.set_rule_sets(&["nope"], &[]).is_err());
    assert!(t.set_rule_sets(&[], &[("se_quality", "D")]).is_err());
}

#[test]
fn every_old_ruleset_maps_to_its_selection_with_the_same_numbers_and_warnings() {
    for (old, vessel) in [
        ("PIANC-A", "CEMT_Va"),
        ("PIANC-B", "CEMT_IV"),
        ("PIANC-C", "CEMT_VIa"),
        ("CN", "CN_IV_motor"),
        ("CN-conservative", "CN_I_convoy_2ab"),
    ] {
        let mut s = Session::new("t", "2026-10-21");
        let (set, input, value) = old_name_choice(old).unwrap();
        s.set_rule_sets(&[set], &[(input, value)]).unwrap();
        s.set_vessel(vessel, Some(2.6)).unwrap();
        s.run_stage(Stage::Sizing).unwrap();
        let r = &s.document()["stages"]["sizing"]["results"];
        let o =
            channel_shape_for(&selection_for_old_ruleset(old).unwrap(), vessel, Some(2.6)).unwrap();
        for (k, v) in [
            ("bottom_width_straight_m", o.bottom_width_straight_m),
            ("min_bend_radius_m", o.min_bend_radius_m),
            ("design_depth_m", o.design_depth_m),
            ("bend_widening_at_rmin_m", o.bend_widening_at_rmin_m),
            ("bottom_width_at_rmin_m", o.bottom_width_at_rmin_m),
        ] {
            assert_eq!(r[k].as_f64().unwrap().to_bits(), v.to_bits(), "{old} {k}");
        }
        assert_eq!(r["warnings"], json!(o.warnings), "{old}");
        assert_eq!(r["width_basis"], o.width_basis.as_str(), "{old}");
    }
}

#[test]
fn a_version_2_file_is_refused_with_a_message_to_rebuild() {
    let mut doc: Value = serde_json::from_str(&Session::new("t", "2026-10-21").to_text()).unwrap();
    for v in [1, 2] {
        doc["format_version"] = json!(v);
        let e = Session::open(&doc.to_string()).unwrap_err();
        let m = e.to_string();
        assert!(matches!(e, DesignError::Input(_)), "{m}");
        assert!(m.contains(&format!("version {v}")), "{m}");
        assert!(m.contains("Rebuild the design"), "{m}");
    }
}

/// A Rule Set file with the id and version of a built-in set but other content.
fn changed_pianc() -> String {
    BUILT_IN[0].1.replace("\"k\": 3.2,", "\"k\": 3.3,")
}

#[test]
fn a_run_time_file_with_a_built_in_id_and_version_but_another_hash_is_refused() {
    let mut s = Session::new("t", "2026-10-21");
    let e = s.load_rule_set(&changed_pianc()).unwrap_err();
    assert!(e.to_string().contains("pianc-wg141@0.3.0"), "{e}");
    // The same file as the built-in one is the same set: nothing changes.
    assert_eq!(s.load_rule_set(BUILT_IN[0].1).unwrap(), "pianc-wg141@0.3.0");
}

#[test]
fn a_set_loaded_at_run_time_can_be_selected_and_a_change_to_it_marks_every_later_stage_stale() {
    let mine = changed_pianc().replace("\"version\": \"0.3.0\"", "\"version\": \"0.3.1\"");
    let mut s = Session::new("t", "2026-10-21");
    assert_eq!(s.load_rule_set(&mine).unwrap(), "pianc-wg141@0.3.1");
    s.set_rule_sets(&["pianc-wg141@0.3.1"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(
        r["bottom_width_straight_m"],
        json!(3.3 * r["beam_m"].as_f64().unwrap())
    );
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    // The same document opened with the file changed (another hash under the same version): the
    // sizing stage, and so every stage after it, is stale, and sizing refuses to run on it.
    let text = s.to_text();
    let mut again = Session::open(&text).unwrap();
    let changed = mine.replace("\"k\": 3.3,", "\"k\": 3.4,");
    again.load_rule_set(&changed).unwrap();
    assert_eq!(again.stage_status(Stage::Sizing), StageStatus::Stale);
    let e = again.run_stage(Stage::Sizing).unwrap_err();
    assert!(e.to_string().contains("content hash"), "{e}");
    // Opened without the file at all: stale too, and sizing names the missing set.
    let mut bare = Session::open(&text).unwrap();
    assert_eq!(bare.stage_status(Stage::Sizing), StageStatus::Stale);
    let e = bare.run_stage(Stage::Sizing).unwrap_err();
    assert!(e.to_string().contains("not loaded"), "{e}");
}

#[test]
fn changing_the_selection_marks_sizing_and_every_later_stage_stale() {
    let mut s = Session::new("t", "2026-10-21");
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    s.set_rule_sets(&["pianc-wg141", "gb50139"], &[("range_end", "strict")])
        .unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(
        r["rule_sets"],
        "pianc-wg141@0.3.0 + gb50139@0.3.0 (se_quality B, range_end strict)"
    );
}

/// Spec 0007 T4: one canal reach over the whole line with the claim, `gb50139` strict, gives the
/// old CN-reduced result bit for bit; marking a reach makes sizing stale.
#[test]
fn a_claimed_canal_reach_over_the_whole_line_gives_cn_reduced() {
    let mut s = Session::new("t", "2026-10-21");
    s.set_rule_sets(&["gb50139"], &[("range_end", "strict")])
        .unwrap();
    s.set_vessel("CN_III_convoy_2ab", None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    s.add_waterway_reach(0.0, None, "canal", true).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    let o = channel_shape_for(
        &selection_for_old_ruleset("CN-reduced").unwrap(),
        "CN_III_convoy_2ab",
        None,
    )
    .unwrap();
    for (k, v) in [
        ("bottom_width_straight_m", o.bottom_width_straight_m),
        ("min_bend_radius_m", o.min_bend_radius_m),
        ("design_depth_m", o.design_depth_m),
        ("bend_widening_at_rmin_m", o.bend_widening_at_rmin_m),
    ] {
        assert_eq!(r[k].as_f64().unwrap().to_bits(), v.to_bits(), "{k}");
    }
    assert_eq!(r["warnings"], json!(o.warnings));
    // The document keeps the reach, and it validates and reopens.
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.waterway_reaches().len(), 1);
    assert_eq!(back.stage_status(Stage::Sizing), StageStatus::Fresh);
}

#[test]
fn a_canal_reach_and_the_rest_of_the_river_size_to_the_river_rmin() {
    let mut s = Session::new("t", "2026-10-21");
    s.set_rule_sets(&["gb50139"], &[("range_end", "strict")])
        .unwrap();
    s.set_vessel("CN_III_motor", None).unwrap();
    s.add_waterway_reach(0.0, Some(2000.0), "canal", true)
        .unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    let loa = r["loa_m"].as_f64().unwrap();
    // The river after 2,000 m keeps the 4L motor-vessel radius; the line takes it.
    assert_eq!(r["min_bend_radius_m"], json!(4.0 * loa));
}

#[test]
fn reaches_are_checked_and_removed() {
    let mut s = Session::new("t", "2026-10-21");
    let e = s
        .add_waterway_reach(0.0, None, "free-flowing-river", true)
        .unwrap_err();
    assert!(e.to_string().contains("canal"), "{e}");
    assert!(s.add_waterway_reach(0.0, None, "lake", false).is_err());
    assert!(
        s.add_waterway_reach(500.0, Some(100.0), "canal", false)
            .is_err()
    );
    s.add_waterway_reach(1000.0, Some(2000.0), "impounded-river", false)
        .unwrap();
    s.add_waterway_reach(0.0, Some(1000.0), "canal", true)
        .unwrap();
    let e = s
        .add_waterway_reach(1500.0, None, "canal", false)
        .unwrap_err();
    assert!(e.to_string().contains("overlaps"), "{e}");
    let starts: Vec<f64> = s
        .waterway_reaches()
        .iter()
        .map(|r| r["start_m"].as_f64().unwrap())
        .collect();
    assert_eq!(starts, [0.0, 1000.0]);
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    s.remove_waterway_reach(0.0).unwrap();
    s.remove_waterway_reach(1000.0).unwrap();
    assert!(s.waterway_reaches().is_empty());
    assert!(s.document()["inputs"].get("waterway_reaches").is_none());
    assert!(s.remove_waterway_reach(5.0).is_err());
}

/// Issue #250: `gb50139` finds a CEMT vessel's GB 50139 channel class by tonnage, and a typed
/// `gb_class` overrides it and is kept as a set input. The picker does not offer `gb_class`: the
/// vessel gives it.
#[test]
fn gb50139_finds_a_cemt_class_by_tonnage_and_a_typed_gb_class_overrides_it() {
    let mut s = Session::new("t", "2026-10-22");
    s.set_rule_sets(&["gb50139"], &[("range_end", "strict")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(r["width_basis"], "appendix-a");
    // Class II by tonnage: draught + 0.4 m at the strict end.
    assert_eq!(r["design_depth_m"], json!(2.5 + 0.4));
    s.set_rule_sets(&[], &[("gb_class", "IV")]).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(r["design_depth_m"], json!(2.5 + 0.3));
    assert!(
        settings(&s)["set_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["name"] == "gb_class" && i["value"] == "IV" && i["source"] == "typed")
    );
    assert!(s.set_rule_sets(&[], &[("gb_class", "VIII")]).is_err());
    let (pick, _) = owd_format::design::rule_sets::rule_set_choices();
    let gb = pick.iter().find(|p| p["id"] == "gb50139").unwrap();
    let names: Vec<&str> = gb["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["range_end"]);
}
