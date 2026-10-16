//! A ruleset and a width method registered from OUTSIDE the crate (this file is an integration
//! test, so it sees only the public API) are used by `channel_shape`, `size_channel`,
//! `width_at_radius`, `design::compute` and the width profile.
//!
//! Label: property. The hand checks below are worked out from the formulas in the rules, not
//! printed by code.

use cd_core::rules::design::{ChannelDesignInputs, ChannelDesignParams, compute};
use cd_core::rules::methods::{
    MethodRegistry, OptValue, Options, WidthMethod, global_methods, register_width_method,
};
use cd_core::rules::rulesets::{
    ChannelRuleInputs, Ruleset, RulesetRegistry, channel_shape, global_rulesets, size_channel,
    width_at_radius,
};
use cd_core::rules::simple::{PiancInputs, size_channel as simple_size};
use cd_core::rules::width_profile::make_width_fn;
use cd_core::rules::{Check, RuleError};
use std::sync::Arc;

/// A consultant's method: 2.5 x beam, LOA^2/(4R) bend widening, R_min = 4 x LOA. It reads one
/// option of its own, `extra_m`, and reports a breakdown.
fn consultant_method() -> WidthMethod {
    fn extra(o: &Options) -> f64 {
        o.get("extra_m").and_then(OptValue::as_f64).unwrap_or(0.0)
    }
    WidthMethod {
        name: "consultant".into(),
        straight_width: Arc::new(|_loa, beam, o| Ok(2.5 * beam + extra(o))),
        bend_width: Arc::new(|loa, beam, o, r, _rd, _defl| {
            Ok(2.5 * beam + extra(o) + loa * loa / (4.0 * r))
        }),
        min_radius: Arc::new(|loa, _o| Ok(4.0 * loa)),
        breakdown: Some(Arc::new(|_loa, beam, o| {
            Ok(Options::new().with("fairway", OptValue::Float(2.5 * beam + extra(o))))
        })),
        description: "consultant: 2.5 x beam, LOA^2/(4R)".into(),
    }
}

fn inputs() -> ChannelRuleInputs {
    let mut inp = ChannelRuleInputs::new(10.0, 100.0, 2.0, 5.0);
    inp.rule_set = "consultant".into();
    inp
}

#[test]
fn a_ruleset_registered_from_outside_is_used_by_channel_shape() {
    let mut reg = RulesetRegistry::new();
    let mut mine = Ruleset::new("MINE", "pianc", "my standard", "my source", 2.5, 2.5);
    mine.width_mult = 3.1;
    mine.c_c = 0.55;
    mine.depth_mult = 1.25;
    reg.register(mine, false).unwrap();

    // CEMT_Va: LOA 95..110 (long end 110), beam 11.4, nominal draught 3.5.
    let shape = channel_shape(&reg, "MINE", "CEMT_Va", None).unwrap();
    assert_eq!(shape.ruleset_id, "MINE");
    assert!((shape.bottom_width_straight_m - 3.1 * 11.4).abs() < 1e-12);
    assert!((shape.min_bend_radius_m - 2.5 * 110.0).abs() < 1e-12);
    assert!((shape.design_depth_m - 1.25 * 3.5).abs() < 1e-12);
    // dF_C = c_C x L^2 / R at R_min
    let widening = 0.55 * 110.0 * 110.0 / 275.0;
    assert!((shape.bend_widening_at_rmin_m - widening).abs() < 1e-12);
    assert!(shape.source.starts_with("my source; vessel "));

    // The built-in registry does not know it: registration is per registry.
    let builtin = RulesetRegistry::new();
    assert!(matches!(
        channel_shape(&builtin, "MINE", "CEMT_Va", None),
        Err(RuleError::Key(_))
    ));
}

#[test]
fn a_ruleset_can_be_registered_on_the_process_wide_registry() {
    let mine = Ruleset::new("GLOBAL-ONE", "china", "g", "g source", 3.0, 3.0);
    global_rulesets()
        .lock()
        .unwrap()
        .register(mine, false)
        .unwrap();
    let reg = global_rulesets().lock().unwrap();
    let shape = channel_shape(&reg, "GLOBAL-ONE", "CN_IV_motor", None).unwrap();
    assert_eq!(shape.ruleset_id, "GLOBAL-ONE");
}

#[test]
fn a_family_other_than_pianc_or_china_is_refused() {
    let mut reg = RulesetRegistry::new();
    let bad = Ruleset::new("X", "martian", "x", "x", 1.0, 1.0);
    let err = reg.register(bad, false).unwrap_err();
    assert_eq!(
        err.raw_message(),
        "ruleset family must be 'pianc' or 'china'; got 'martian'"
    );
}

#[test]
fn a_width_method_registered_from_outside_is_used_by_size_channel_and_width_at_radius() {
    let mut reg = MethodRegistry::new();
    reg.register(consultant_method(), false).unwrap();
    assert_eq!(reg.names(), vec!["consultant", "pianc"]);

    let mut inp = inputs();
    inp.options = Options::new().with("extra_m", OptValue::Float(1.0));
    let res = size_channel(&reg, &inp).unwrap();
    // straight = 2.5 x 10 + 1 = 26; R_min = 400; reference radius 3 x LOA = 300;
    // bend = 26 + 100^2 / (4 x 300) = 34.333...
    assert!((res.straight_bottom_width_m - 26.0).abs() < 1e-12);
    assert!((res.min_turn_radius_m - 400.0).abs() < 1e-12);
    assert!((res.reference_bend_radius_m - 300.0).abs() < 1e-12);
    assert!((res.bend_bottom_width_m - (26.0 + 10_000.0 / 1200.0)).abs() < 1e-12);
    assert_eq!(res.rule_set, "consultant");
    // The method's own breakdown comes through, with the widening added.
    assert!(res.width_breakdown.get("fairway").is_some());
    assert!(
        res.width_breakdown
            .get("bend_widening_at_reference_radius")
            .is_some()
    );
    // 300 m is below the rule minimum of 400 m: the warning-severity check fails, the result
    // is still ok because warnings do not gate it.
    let warn = res
        .checks
        .iter()
        .find(|c: &&Check| c.name == "ref_radius_at_or_above_rule_min")
        .unwrap();
    assert!(!warn.ok);
    assert!(res.ok());

    let w = width_at_radius(&reg, &inp, 200.0).unwrap();
    assert!((w - (26.0 + 10_000.0 / 800.0)).abs() < 1e-12);
    assert_eq!(width_at_radius(&reg, &inp, f64::INFINITY).unwrap(), 26.0);
    assert_eq!(width_at_radius(&reg, &inp, 0.0).unwrap(), 26.0);
}

#[test]
fn an_unregistered_method_is_refused_with_the_registered_names() {
    let reg = MethodRegistry::new();
    let err = size_channel(&reg, &inputs()).unwrap_err();
    assert_eq!(
        err.raw_message(),
        "rule_set must be one of ('pianc',) (register more with \
         channel_designer.rules.methods.register_width_method)"
    );
}

#[test]
fn a_registered_method_drives_the_design_sweep_the_simple_sizing_and_the_width_profile() {
    let mut reg = MethodRegistry::new();
    reg.register(consultant_method(), false).unwrap();

    let params = ChannelDesignParams {
        width_method: Some("consultant".into()),
        ..ChannelDesignParams::default()
    };
    let res = compute(
        &reg,
        &ChannelDesignInputs {
            total_loa: 100.0,
            beam: 10.0,
            centreline: None,
            params,
        },
    )
    .unwrap();
    assert!((res.base_width_m - 25.0).abs() < 1e-12);

    let mut simple = PiancInputs::new(10.0, 100.0);
    simple.width_method = "consultant".into();
    let out = simple_size(&reg, &simple, None).unwrap();
    // R = 4 x 100 = 400 (simple's own radius multiple); bend = 25 + 10_000 / 1600
    assert!((out.bottom_width_bend_m - (25.0 + 10_000.0 / 1600.0)).abs() < 1e-12);

    let f = make_width_fn(&reg, 100.0, 10.0, 2.0, "consultant").unwrap();
    assert!((f.width(250.0).unwrap() - (25.0 + 10.0)).abs() < 1e-12);
    assert!((f.half_width(250.0).unwrap() - 17.5).abs() < 1e-12);
}

#[test]
fn registering_a_name_twice_is_refused_unless_replace_is_set() {
    let mut reg = MethodRegistry::new();
    reg.register(consultant_method(), false).unwrap();
    let err = reg.register(consultant_method(), false).unwrap_err();
    assert_eq!(
        err.raw_message(),
        "width method 'consultant' is already registered; pass replace=True to override it"
    );
    reg.register(consultant_method(), true).unwrap();
    let err = reg.unregister("pianc").unwrap_err();
    assert_eq!(
        err.raw_message(),
        "the built-in 'pianc' method cannot be unregistered"
    );
    reg.unregister("consultant").unwrap();
    assert_eq!(reg.names(), vec!["pianc"]);
}

#[test]
fn a_method_can_be_registered_on_the_process_wide_registry() {
    let mut m = consultant_method();
    m.name = "consultant-global".into();
    register_width_method(m, true).unwrap();
    let reg = global_methods().lock().unwrap();
    assert!(reg.names().contains(&"consultant-global".to_string()));
    let mut inp = inputs();
    inp.rule_set = "consultant-global".into();
    assert!(size_channel(&reg, &inp).is_ok());
}

#[test]
fn every_registered_method_meets_the_contract() {
    // straight > 0, bend >= straight at any positive radius (callers clamp, but the shipped
    // methods never go below), min_radius > 0.
    let mut reg = MethodRegistry::new();
    reg.register(consultant_method(), false).unwrap();
    let opts = Options::new();
    for name in reg.names() {
        let m = reg.get(&name).unwrap();
        let straight = (m.straight_width)(100.0, 10.0, &opts).unwrap();
        assert!(straight > 0.0, "{name}: straight width must be > 0");
        for r in [50.0, 300.0, 5000.0] {
            let bend = (m.bend_width)(100.0, 10.0, &opts, r, r, 45.0).unwrap();
            assert!(
                bend >= straight,
                "{name}: bend width below straight at R {r}"
            );
        }
        assert!(
            (m.min_radius)(100.0, &opts).unwrap() > 0.0,
            "{name}: min radius"
        );
    }
}
