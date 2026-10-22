//! The custom vessel through the shell (spec 0015 T2): the one path the desktop and the web share.
//! The fields come as the form has them (typed text, or numbers); a refusal names its field, so
//! the form shows it there; a warning does not block.
//!
//! Label: property (the web example, sized for a custom vessel with CEMT-IV's figures, against the
//! class).

use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd")
}

fn opened() -> AppState {
    let text = std::fs::read_to_string(example()).unwrap();
    AppState::open(&example(), &text, None).unwrap()
}

/// CEMT-IV's figures, as the form sends them: typed text.
fn fields() -> Value {
    json!({"name": "My barge", "loa_m": "85", "beam_m": "9.5", "draught_m": "2.5",
           "formation": "motor", "n_abreast": "", "n_in_line": "", "gb_class": "",
           "air_draught_m": "", "deadweight_t": ""})
}

fn sizing(s: &AppState) -> Value {
    s.snapshot()["document"]["stages"]["sizing"].clone()
}

#[test]
fn a_custom_vessel_sizes_at_once_with_its_label_as_the_class_does() {
    let mut s = opened();
    s.set_custom_vessel(&fields()).unwrap();
    let st = sizing(&s);
    assert_eq!(st["settings"]["custom_vessel"]["name"], "My barge");
    assert!(
        st["settings"]["custom_vessel"]
            .get("air_draught_m")
            .is_none()
    );
    assert_eq!(
        st["results"]["vessel_label"],
        "Custom: My barge, 85 × 9.5 × 2.5 m"
    );
    assert!(s.is_dirty());
    let mut c = opened();
    c.set_vessel("CEMT-IV", Some(2.5)).unwrap();
    for k in [
        "bottom_width_straight_m",
        "min_bend_radius_m",
        "design_depth_m",
    ] {
        assert_eq!(st["results"][k], sizing(&c)["results"][k], "{k}");
    }
}

#[test]
fn the_check_names_the_field_a_refusal_is_on() {
    let s = opened();
    let ok = s.check_custom_vessel(&fields());
    assert!(ok["refusal"].is_null(), "{ok}");
    for (k, v, field) in [
        ("beam_m", json!("90"), "beam_m"),
        ("loa_m", json!(""), "loa_m"),
        ("draught_m", json!("two"), "draught_m"),
        ("name", json!(""), "name"),
        ("formation", json!("raft"), "formation"),
        ("gb_class", json!("IX"), "gb_class"),
        ("deadweight_t", json!("-1"), "deadweight_t"),
    ] {
        let mut f = fields();
        f[k] = v;
        let r = s.check_custom_vessel(&f);
        assert_eq!(r["refusal"]["field"], field, "{k}: {r}");
        assert!(
            r["refusal"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
        );
    }
    let mut convoy = fields();
    convoy["formation"] = json!("convoy");
    assert_eq!(
        s.check_custom_vessel(&convoy)["refusal"]["field"],
        "n_abreast"
    );
    convoy["n_abreast"] = json!("2");
    convoy["n_in_line"] = json!(2);
    assert!(s.check_custom_vessel(&convoy)["refusal"].is_null());
}

#[test]
fn a_warning_shows_and_does_not_block() {
    let mut s = opened();
    let mut f = fields();
    f["loa_m"] = json!("60");
    f["beam_m"] = json!("29");
    let r = s.check_custom_vessel(&f);
    assert!(r["refusal"].is_null());
    assert!(
        r["warnings"][0]
            .as_str()
            .unwrap()
            .contains("length-to-beam ratio"),
        "{r}"
    );
    s.set_custom_vessel(&f).unwrap();
    assert!(sizing(&s)["results"]["vessel_label"].is_string());
}

#[test]
fn a_refused_custom_vessel_changes_nothing() {
    let mut s = opened();
    let before = s.snapshot()["document"].clone();
    let mut f = fields();
    f["beam_m"] = json!("90");
    let e = s.set_custom_vessel(&f).unwrap_err();
    assert!(e.contains("beam_m"), "{e}");
    assert_eq!(s.snapshot()["document"], before);
    assert!(!s.is_dirty());
}

#[test]
fn a_class_replaces_the_custom_vessel_again() {
    let mut s = opened();
    s.set_custom_vessel(&fields()).unwrap();
    s.set_vessel("CEMT-Va", None).unwrap();
    let st = sizing(&s);
    assert_eq!(st["settings"]["vessel_standard_id"], "CEMT-Va");
    assert!(st["settings"].get("custom_vessel").is_none());
    assert_eq!(st["results"]["vessel_standard_id"], "CEMT-Va");
}
