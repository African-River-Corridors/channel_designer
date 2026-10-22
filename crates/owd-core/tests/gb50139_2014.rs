//! Spec 0007, issue #250: the `gb50139` Rule Set against GB 50139-2014 itself.
//!
//! Label: sourced - GB 50139-2014. Each test names the clause or table its numbers come from: Table
//! 3.0.2-1 (pp3-4), 3.0.5 (pp7-8), Appendix A.0.1 and A.0.2 (pp26-27), the explanation of 3.0.5
//! and 3.0.7 (pp43-44), and the CEMT tonnage of ECMT Resolution 92/2. The tests that only say what
//! the set does with a missing input are `Label: property`.

use owd_core::rules::engine::{self, Selection, Vessel};
use owd_core::rules::rulesets::channel_shape_for;
use owd_core::rules::screening::blocking_not_evaluated;

fn gb(end: &str) -> Selection {
    Selection::built_in(&["gb50139"], &[(engine::RANGE_END, end)]).unwrap()
}

fn vessel(class: &str, loa: f64, beam: f64, t: f64, formation: &str) -> Vessel {
    Vessel {
        loa_m: loa,
        beam_m: beam,
        draught_m: t,
        formation: formation.into(),
        class_name: class.into(),
        family: engine::CN_FAMILY.into(),
    }
}

/// Table 3.0.2-1, pp3-4: class, LOA x beam, design draught, single-line straight width, bend radius,
/// and whether the row is a cargo ship (the others are convoys or towed trains).
const TABLE_3_0_2_1: [(&str, f64, f64, f64, f64, f64, bool); 20] = [
    ("I", 406.0, 64.8, 3.5, 125.0, 1200.0, false),
    ("I", 316.0, 48.6, 3.5, 100.0, 950.0, false),
    ("I", 223.0, 32.4, 3.5, 70.0, 670.0, false),
    ("II", 270.0, 48.6, 2.6, 100.0, 810.0, false),
    ("II", 186.0, 32.4, 2.6, 70.0, 560.0, false),
    ("II", 182.0, 16.2, 2.6, 40.0, 550.0, false),
    ("III", 238.0, 21.6, 2.0, 55.0, 720.0, false),
    ("III", 167.0, 21.6, 2.0, 45.0, 500.0, false),
    ("III", 160.0, 10.8, 2.0, 30.0, 480.0, false),
    ("IV", 167.0, 21.6, 1.6, 45.0, 500.0, false),
    ("IV", 112.0, 21.6, 1.6, 40.0, 340.0, false),
    ("IV", 111.0, 10.8, 1.6, 30.0, 330.0, false),
    ("IV", 67.5, 10.8, 1.6, 30.0, 330.0, true),
    ("V", 94.0, 18.4, 1.3, 35.0, 280.0, false),
    ("V", 91.0, 9.2, 1.3, 22.0, 270.0, false),
    ("V", 55.0, 8.6, 1.3, 22.0, 270.0, true),
    ("VI", 188.0, 7.0, 1.0, 15.0, 180.0, false),
    ("VI", 45.0, 5.5, 1.0, 15.0, 180.0, true),
    ("VII", 145.0, 5.5, 0.7, 12.0, 130.0, false),
    ("VII", 32.5, 5.5, 0.7, 12.0, 130.0, true),
];

/// Table A.0.1, p26: the clearance by class, (permissive, strict). VI and VII give one value.
const TABLE_A_0_1: [(&str, f64, f64); 7] = [
    ("I", 0.4, 0.5),
    ("II", 0.3, 0.4),
    ("III", 0.3, 0.4),
    ("IV", 0.2, 0.3),
    ("V", 0.2, 0.3),
    ("VI", 0.2, 0.2),
    ("VII", 0.2, 0.2),
];

/// Label: sourced - GB 50139-2014 Table 3.0.2-1 and 3.0.5(4). Every typical vessel takes its row's
/// single-line width at both range ends, and a radius of at least its row's: the larger of the row
/// and 3 L (convoy) or 4 L (cargo ship).
#[test]
fn every_table_3_0_2_1_row_gives_its_width_and_radius() {
    for (class, loa, beam, t, width, radius, cargo) in TABLE_3_0_2_1 {
        let formation = if cargo {
            "motor_vessel"
        } else {
            "pushed_convoy"
        };
        let k = if cargo { 4.0 } else { 3.0 };
        for end in ["permissive", "strict"] {
            let s = engine::size(&gb(end), &vessel(class, loa, beam, t, formation)).unwrap();
            let case = format!("{class} {loa} x {beam} {end}");
            assert_eq!(s.bottom_width_straight_m, width, "{case}");
            assert_eq!(s.min_bend_radius_m, radius.max(k * loa), "{case}");
        }
    }
}

/// Label: sourced - GB 50139-2014 Table 3.0.2-1. The class IV cargo ship's row (67.5 x 10.8 m)
/// shares the 330 m radius with the 111 m convoy, above its own 4 L = 270 m: the table governs.
#[test]
fn the_table_radius_governs_where_it_is_above_4l() {
    let s = engine::size(
        &gb("strict"),
        &vessel("IV", 67.5, 10.8, 1.6, "motor_vessel"),
    )
    .unwrap();
    assert_eq!(s.min_bend_radius_m, 330.0);
}

/// Label: sourced - GB 50139-2014 A.0.2 and 3.0.7 (with its explanation, p44). A vessel whose
/// dimensions differ from every row is a special design vessel: B1 = BF + 2d, BF = B + L sin beta,
/// d = 0.25-0.30 BF for a convoy and 0.34-0.40 BF for a cargo ship, beta 3 deg (I-V) or 2 deg
/// (VI-VII). The class I cargo ship (95 x 16.2 m) has no row: classes I-III are set by convoys.
#[test]
fn a_special_design_vessel_takes_the_a_0_2_width() {
    let cases = [
        // class, LOA, beam, formation, beta, d (permissive), d (strict)
        ("I", 95.0, 16.2, "motor_vessel", 3.0, 0.34, 0.40),
        ("II", 90.0, 14.8, "motor_vessel", 3.0, 0.34, 0.40),
        ("III", 85.0, 10.8, "motor_vessel", 3.0, 0.34, 0.40),
        ("IV", 100.0, 10.8, "pushed_convoy", 3.0, 0.25, 0.30),
        ("VI", 50.0, 6.0, "motor_vessel", 2.0, 0.34, 0.40),
        ("VII", 120.0, 5.5, "pushed_convoy", 2.0, 0.25, 0.30),
    ];
    for (class, loa, beam, formation, beta, lo, hi) in cases {
        let bf = beam + loa * libm::sin(beta * std::f64::consts::PI / 180.0);
        for (end, d) in [("permissive", lo), ("strict", hi)] {
            let v = vessel(class, loa, beam, 1.0, formation);
            let s = engine::size(&gb(end), &v).unwrap();
            let want = bf + 2.0 * d * bf;
            assert!(
                (s.bottom_width_straight_m - want).abs() < 1e-9,
                "{class} {end}: {} vs {want}",
                s.bottom_width_straight_m
            );
            let k = if formation == "motor_vessel" {
                4.0
            } else {
                3.0
            };
            assert_eq!(s.min_bend_radius_m, k * loa, "{class} {end}");
        }
    }
}

/// Label: sourced - GB 50139-2014 Table 3.0.2-1, A.0.2. The A.0.2 width with d on both sides agrees
/// with the table: 223 x 32.4 m at beta 3 deg and d = 0.30 BF gives 70.5 m, the table 70 m. One
/// side only, (1 + d), would give 57 m.
#[test]
fn the_a_0_2_clearance_is_on_both_sides() {
    let v = vessel("I", 223.1, 32.4, 3.5, "pushed_convoy");
    let w = engine::size(&gb("strict"), &v)
        .unwrap()
        .bottom_width_straight_m;
    assert!((w - 70.0).abs() < 1.0, "{w}");
}

/// Label: sourced - GB 50139-2014 A.0.1 and Table A.0.1. Depth = T + clearance, and `range_end`
/// picks the end of each range.
#[test]
fn the_depth_is_draught_plus_the_table_a_0_1_clearance() {
    for (class, lo, hi) in TABLE_A_0_1 {
        for (end, c) in [("permissive", lo), ("strict", hi)] {
            let t = 1.7;
            let s = engine::size(&gb(end), &vessel(class, 80.0, 9.0, t, "motor_vessel")).unwrap();
            assert_eq!(s.design_depth_m, t + c, "{class} {end}");
        }
    }
}

/// Label: sourced - GB 50139-2014 3.0.5 and its explanation (p43). The 2004 edition's reduced
/// radius is gone: a claimed canal reach takes the same radius as a river.
#[test]
fn a_claimed_canal_reach_takes_no_reduced_radius() {
    let canal = engine::WaterwayReach {
        label: "the whole river".into(),
        waterway: "canal".into(),
        claim: "yes".into(),
        claim_typed: true,
    };
    for formation in ["motor_vessel", "pushed_convoy"] {
        let v = vessel("III", 85.0, 10.8, 2.0, formation);
        let river = engine::size(&gb("strict"), &v).unwrap();
        let claimed = engine::size(&gb("strict").with_reaches(vec![canal.clone()]), &v).unwrap();
        assert_eq!(river.min_bend_radius_m, claimed.min_bend_radius_m);
    }
    let set = engine::built_in().lookup("gb50139").unwrap();
    assert!(set.rules.iter().all(|r| !r.id.contains("reduced")));
}

/// Label: sourced - GB 50139-2014 Table 3.0.2-1, 3.0.5(4), A.0.1. The catalogue's Chinese vessels
/// through `channel_shape_for`: the class IV cargo ship and the class I convoy are typical vessels.
#[test]
fn catalogue_vessels_take_the_2014_numbers() {
    let sel = |end| Selection::built_in(&["gb50139", "owd"], &[(engine::RANGE_END, end)]).unwrap();
    let s = channel_shape_for(&sel("permissive"), "CN_IV_motor", None).unwrap();
    assert_eq!(
        (s.bottom_width_straight_m, s.min_bend_radius_m),
        (30.0, 330.0)
    );
    assert_eq!(s.design_depth_m, 1.6 + 0.2);
    assert_eq!(s.width_basis, "table");
    let s = channel_shape_for(&sel("strict"), "CN_I_convoy_2ab", None).unwrap();
    assert_eq!(
        (s.bottom_width_straight_m, s.min_bend_radius_m),
        (70.0, 670.0)
    );
    assert_eq!(s.design_depth_m, 3.5 + 0.5);
    let s = channel_shape_for(&sel("strict"), "CN_I_motor", None).unwrap();
    assert_eq!(s.width_basis, "appendix-a");
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains("special design vessel"))
    );
}

/// Label: sourced - GB 50139-2014 3.0.5(2). Every gb50139 channel says that the standard expects a
/// double-line channel.
#[test]
fn the_double_line_note_shows_with_every_gb_channel() {
    let sel = Selection::built_in(&["gb50139", "owd"], &[(engine::RANGE_END, "strict")]).unwrap();
    for id in ["CN_IV_motor", "CN_I_motor"] {
        let s = channel_shape_for(&sel, id, None).unwrap();
        assert!(
            s.warnings.iter().any(|w| w.contains("double-line channel")),
            "{id}: {:?}",
            s.warnings
        );
    }
}

/// Label: sourced - GB 50139-2014 Table 3.0.1 (with note 2) and ECMT Resolution 92/2. A CEMT
/// vessel takes the smallest GB class whose design tonnage covers the top of its CEMT range: IV (up
/// to 1500 t) is class II (2000 t), Va (up to 3000 t) class I; Vb and above are over 3000 t, so class
/// I and a special design vessel (3.0.7).
#[test]
fn a_cemt_vessel_takes_its_gb_class_by_tonnage() {
    let cemt = |class: &str| Vessel {
        family: "CEMT".into(),
        ..vessel(class, 100.0, 10.0, 2.0, "motor_vessel")
    };
    let none: [(String, String); 0] = [];
    for (cemt_class, gb_class, above) in [
        ("I", "IV", false),
        ("II", "III", false),
        ("III", "II", false),
        ("IV", "II", false),
        ("Va", "I", false),
        ("Vb", "I", true),
        ("VIa", "I", true),
        ("VIb", "I", true),
        ("VIc", "I", true),
        ("VII", "I", true),
    ] {
        let f = engine::gb_class_found(&cemt(cemt_class), &none);
        assert_eq!(
            (f.class.as_str(), f.above_3000_t),
            (gb_class, above),
            "{cemt_class}"
        );
        assert!(f.how.starts_with("by tonnage"), "{}", f.how);
    }
    // The NL 2011 variant of VIa is a CEMT class too.
    let nl = Vessel {
        family: "CEMT-NL-2011".into(),
        ..cemt("VIa")
    };
    assert!(engine::gb_class_found(&nl, &none).above_3000_t);
}

/// Label: sourced - GB 50139-2014 A.0.1, A.0.2, Table A.0.1 and 3.0.7. CEMT IV (85 x 9.5 m, class
/// II by tonnage) has no row: the A.0.2 width at beta 3 deg, and the class II clearance. CEMT Vb (185
/// x 11.4 m convoy, above 3000 t) is class I and a special design vessel. The trace and the warnings
/// say how the class was found.
#[test]
fn cemt_vessels_are_sized_by_their_class_by_tonnage() {
    let sel = |end| Selection::built_in(&["gb50139", "owd"], &[(engine::RANGE_END, end)]).unwrap();
    let rad = |deg: f64| deg * std::f64::consts::PI / 180.0;
    let s = channel_shape_for(&sel("permissive"), "CEMT_IV", Some(2.5)).unwrap();
    let bf = 9.5 + 85.0 * libm::sin(rad(3.0));
    assert!((s.bottom_width_straight_m - (bf + 2.0 * 0.34 * bf)).abs() < 1e-9);
    assert_eq!(s.design_depth_m, 2.5 + 0.3);
    assert_eq!(s.width_basis, "appendix-a");
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains("II, by tonnage: CEMT IV up to 1500 t")),
        "{:?}",
        s.warnings
    );
    let s = channel_shape_for(&sel("strict"), "CEMT_Vb", Some(2.5)).unwrap();
    let bf = 11.4 + 185.0 * libm::sin(rad(3.0));
    assert!((s.bottom_width_straight_m - (bf + 2.0 * 0.30 * bf)).abs() < 1e-9);
    assert_eq!(s.design_depth_m, 2.5 + 0.5);
    assert_eq!(s.min_bend_radius_m, 3.0 * 185.0);
    assert!(s.warnings.iter().any(|w| w.contains("above 3000 t")));
    let v = owd_core::rules::rulesets::vessel_for("CEMT_Vb", None).unwrap();
    let trace = sel("strict").trace(&v, 0.0);
    let note = trace
        .rows
        .iter()
        .find(|r| r.rule.as_deref() == Some("gb50139/3.0.1/channel-class"))
        .unwrap();
    assert!(
        matches!(&note.value, Some(arc_rules::eval::MeasureValue::Text(t)) if t.starts_with("I, by tonnage"))
    );
}

/// Label: property. A vessel with no GB class and no tonnage: gb50139 alone cannot size its width,
/// and says to type the class. Beside PIANC, its width and depth Rules are blocking Rules not
/// evaluated, never a silent pass.
#[test]
fn a_vessel_with_no_class_and_no_tonnage_needs_a_typed_gb_class() {
    let v = Vessel {
        family: "other".into(),
        ..vessel("X", 100.0, 10.0, 2.0, "motor_vessel")
    };
    let alone = Selection::built_in(&["gb50139"], &[(engine::RANGE_END, "strict")]).unwrap();
    let e = engine::size(&alone, &v).unwrap_err();
    assert!(e.raw_message().contains("gb_class"), "{e:?}");

    let both = Selection::built_in(
        &["pianc-wg141", "gb50139"],
        &[(engine::SE_QUALITY, "C"), (engine::RANGE_END, "strict")],
    )
    .unwrap();
    let sized = engine::size(&both, &v).unwrap();
    assert_eq!(
        blocking_not_evaluated(&both, &v, &sized),
        [
            "gb50139/3.0.1/single-line-width-needs-class",
            "gb50139/3.0.1/design-depth-needs-class"
        ]
    );
}

/// Label: sourced - GB 50139-2014 A.0.2 and Table A.0.1. A typed `gb_class` overrides the class
/// found from the vessel, for a CEMT vessel and a Chinese one alike.
#[test]
fn a_typed_gb_class_overrides_the_class_found() {
    let typed = |class| {
        Selection::built_in(
            &["gb50139"],
            &[(engine::RANGE_END, "permissive"), (engine::GB_CLASS, class)],
        )
        .unwrap()
    };
    let s = channel_shape_for(&typed("IV"), "CEMT_IV", Some(2.5)).unwrap();
    assert_eq!(s.design_depth_m, 2.5 + 0.2);
    assert!(s.warnings.iter().any(|w| w.contains("IV, typed")));
    // CN_IV_motor typed VII: no class VII row is 67.5 x 10.8 m, so the A.0.2 width at 2 deg.
    let s = channel_shape_for(&typed("VII"), "CN_IV_motor", None).unwrap();
    let bf = 10.8 + 67.5 * libm::sin(2.0 * std::f64::consts::PI / 180.0);
    assert!((s.bottom_width_straight_m - (bf + 2.0 * 0.34 * bf)).abs() < 1e-9);
}

/// Label: property. A row matches to the 0.1 m the table prints: 67.54 m is the class IV cargo
/// ship's row, 67.6 m is a special design vessel.
#[test]
fn a_row_matches_to_the_printed_precision() {
    let at = |loa| {
        engine::size(&gb("strict"), &vessel("IV", loa, 10.8, 1.6, "motor_vessel"))
            .unwrap()
            .bottom_width_straight_m
    };
    assert_eq!(at(67.54), 30.0);
    assert_ne!(at(67.6), 30.0);
}
