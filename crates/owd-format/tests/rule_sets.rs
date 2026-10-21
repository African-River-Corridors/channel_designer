//! Spec 0007 T3: the Rule Set selection in the document (`format_version` 3).
//!
//! Label: property. No fixed number; the sizing numbers are compared with the old path.

use owd_core::rules::engine::{self, BUILT_IN};
use owd_core::rules::rulesets::{RulesetRegistry, channel_shape};
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
    let old = channel_shape(&RulesetRegistry::new(), "PIANC-C", "CEMT_IV", None).unwrap();
    assert_eq!(
        r["bottom_width_straight_m"],
        json!(old.bottom_width_straight_m)
    );
    assert_eq!(r["ruleset_id"], "pianc-wg141@0.3.0 (se_quality C)");
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
        s.set_sizing(old, vessel, Some(2.6)).unwrap();
        s.run_stage(Stage::Sizing).unwrap();
        let r = &s.document()["stages"]["sizing"]["results"];
        let o = channel_shape(&RulesetRegistry::new(), old, vessel, Some(2.6)).unwrap();
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
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    s.set_rule_sets(&["pianc-wg141", "gb50139"], &[("range_end", "strict")])
        .unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
    s.run_stage(Stage::Sizing).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(
        r["ruleset_id"],
        "pianc-wg141@0.3.0 + gb50139@0.3.0 (se_quality B, range_end strict)"
    );
}
