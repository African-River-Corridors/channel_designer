//! The custom vessel in the core (spec 0015 T1): sized like a class, and GB 50139 reads its typed
//! class.
//!
//! Label: property (a custom vessel with a class's design figures against the class itself; the
//! GB 50139 width and depth with and without a typed class).

use owd_core::rules::classes::{class, default_class_ids, design_figures};
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_core::rules::engine::{self, Selection, selection_for_old_ruleset};
use owd_core::rules::rulesets::{channel_shape_for, channel_shape_for_custom, custom_selection};
use owd_core::rules::screening::blocking_not_evaluated;

fn like(id: &str) -> CustomVessel {
    let v = class(id).unwrap();
    let f = design_figures(&v, None);
    CustomVessel {
        name: format!("{id} (custom)"),
        loa_m: f.loa_m,
        beam_m: f.beam_m,
        draught_m: f.draught_m,
        formation: if v.formation == "pushed_convoy" {
            Formation::Convoy
        } else {
            Formation::Motor
        },
        n_abreast: (v.formation == "pushed_convoy").then_some(v.n_abreast.max(1) as u32),
        n_in_line: (v.formation == "pushed_convoy").then_some(v.n_in_line.max(1) as u32),
        gb_class: None,
        air_draught_m: None,
        deadweight_t: None,
    }
}

#[test]
fn a_custom_vessel_with_a_class_figures_sizes_as_that_class_under_pianc() {
    for rs in ["PIANC-A", "PIANC-B", "PIANC-C"] {
        let sel = selection_for_old_ruleset(rs).unwrap();
        for id in default_class_ids() {
            let a = channel_shape_for(&sel, id, None).unwrap();
            let b = channel_shape_for_custom(&sel, &like(id)).unwrap();
            assert_eq!(
                (
                    a.bottom_width_straight_m,
                    a.min_bend_radius_m,
                    a.design_depth_m,
                    a.bend_widening_at_rmin_m
                ),
                (
                    b.bottom_width_straight_m,
                    b.min_bend_radius_m,
                    b.design_depth_m,
                    b.bend_widening_at_rmin_m
                ),
                "{rs} {id}"
            );
            assert!(b.custom && !a.custom);
            assert!(
                b.vessel_standard_id.starts_with("Custom: "),
                "{}",
                b.vessel_standard_id
            );
            assert!(b.source.contains("user-entered dimensions"), "{}", b.source);
        }
    }
}

#[test]
fn a_refused_custom_vessel_does_not_size() {
    let sel = selection_for_old_ruleset("PIANC-B").unwrap();
    let bad = CustomVessel {
        beam_m: 200.0,
        ..like("CEMT-IV")
    };
    let e = channel_shape_for_custom(&sel, &bad).unwrap_err();
    assert!(e.raw_message().contains("beam_m"), "{e:?}");
}

#[test]
fn an_unusual_custom_vessel_sizes_with_its_warning_on_the_shape() {
    let sel = selection_for_old_ruleset("PIANC-B").unwrap();
    let wide = CustomVessel {
        name: "Wide barge".into(),
        loa_m: 60.0,
        beam_m: 29.0,
        ..like("CEMT-IV")
    };
    let s = channel_shape_for_custom(&sel, &wide).unwrap();
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains("length-to-beam ratio")),
        "{:?}",
        s.warnings
    );
}

#[test]
fn gb_50139_evaluates_a_custom_vessel_only_with_a_typed_class() {
    let both = Selection::built_in(
        &["pianc-wg141", "gb50139"],
        &[(engine::SE_QUALITY, "C"), (engine::RANGE_END, "strict")],
    )
    .unwrap();
    let untyped = like("CEMT-IV");
    let typed = CustomVessel {
        gb_class: Some("III".into()),
        ..untyped.clone()
    };
    let blocked = |c: &CustomVessel| {
        let sel = custom_selection(&both, c);
        let v = c.vessel();
        let sized = engine::size(&sel, &v).unwrap();
        blocking_not_evaluated(&sel, &v, &sized)
    };
    assert_eq!(
        blocked(&untyped),
        [
            "gb50139/3.0.1/single-line-width-needs-class",
            "gb50139/3.0.1/design-depth-needs-class"
        ]
    );
    assert_eq!(blocked(&typed), Vec::<String>::new());
    // Alone, GB 50139 cannot size an untyped custom vessel, and says to give the class.
    let alone = Selection::built_in(&["gb50139"], &[(engine::RANGE_END, "strict")]).unwrap();
    let e = channel_shape_for_custom(&alone, &untyped).unwrap_err();
    assert!(e.raw_message().contains("gb_class"), "{e:?}");
    assert!(channel_shape_for_custom(&alone, &typed).is_ok());
}

#[test]
fn a_custom_vessel_is_a_special_design_vessel_under_gb_50139() {
    // A custom vessel with a typical vessel's figures and class is still special (Appendix A).
    let alone = Selection::built_in(&["gb50139"], &[(engine::RANGE_END, "strict")]).unwrap();
    let c = CustomVessel {
        gb_class: Some("III".into()),
        ..like("CN-III-MOTOR")
    };
    let s = channel_shape_for_custom(&alone, &c).unwrap();
    assert_eq!(s.width_basis, "appendix-a", "{}", s.width_basis);
}
