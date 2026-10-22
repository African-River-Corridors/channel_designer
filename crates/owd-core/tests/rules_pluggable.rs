//! A width method registered from OUTSIDE the crate (this file is an integration test, so it sees
//! only the public API) is used by `size_channel`, `width_at_radius`, `design::compute` and the
//! width profile. (A Rule Set is added as a file at run time; see `rules_engine.rs`.)
//!
//! Label: property. The hand checks below are worked out from the formulas in the rules, not
//! printed by code.

use owd_core::rules::Check;
use owd_core::rules::design::{ChannelDesignInputs, ChannelDesignParams, compute};
use owd_core::rules::methods::{
    MethodRegistry, OptValue, Options, WidthMethod, global_methods, register_width_method,
};
use owd_core::rules::rulesets::{ChannelRuleInputs, size_channel, width_at_radius};
use owd_core::rules::simple::{PiancInputs, size_channel as simple_size};
use owd_core::rules::width_profile::make_width_fn;
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
         owd.rules.methods.register_width_method)"
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
