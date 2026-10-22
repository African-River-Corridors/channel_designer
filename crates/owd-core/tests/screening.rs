//! Spec 0007 T5: constraint stretches and the design basis.
//!
//! Label: property. The synthetic river is built in code; its stretches follow from where the
//! fixture puts the throat, the bar and the bend, not from a printed number.

use owd_core::rules::engine::{self, Selection, Vessel};
use owd_core::rules::screening::{
    StationIn, StretchKind, constraint_stretches, design_basis, falls_short,
};

fn cemt_iv() -> Vessel {
    Vessel {
        loa_m: 110.0,
        beam_m: 9.5,
        draught_m: 2.5,
        formation: "motor_vessel".into(),
        class_name: "IV".into(),
        family: "CEMT".into(),
    }
}

fn pianc_b() -> Selection {
    Selection::built_in(&["pianc-wg141", "owd"], &[(engine::SE_QUALITY, "B")]).unwrap()
}

/// A straight trapezoid river, 3 km, 200 m wide and 6 m deep, with one narrow throat (25 m wide,
/// 1,000 to 1,100 m), one shallow bar (1.5 m deep, 1,800 to 1,900 m) and one tight bend (radius
/// 150 m, 2,400 to 2,500 m). The design line is straight; the bar's cross-sections dredge 10 m².
fn synthetic_river() -> Vec<StationIn> {
    (0..=300)
        .map(|i| {
            let s = i as f64 * 10.0;
            let inside = |a: f64, b: f64| s >= a && s <= b;
            StationIn {
                chainage_m: s,
                xy: [s, 0.0],
                river_width_m: if inside(1000.0, 1100.0) { 25.0 } else { 200.0 },
                river_depth_m: if inside(1800.0, 1900.0) { 1.5 } else { 6.0 },
                river_radius_m: if inside(2400.0, 2500.0) {
                    150.0
                } else {
                    f64::INFINITY
                },
                line_radius_m: f64::INFINITY,
                reach: 0,
                dredge_area_m2: if inside(1800.0, 1900.0) { 10.0 } else { 0.0 },
                dry_cut_area_m2: 0.0,
            }
        })
        .collect()
}

#[test]
fn a_throat_a_bar_and_a_bend_give_three_stretches_with_their_works() {
    let s = pianc_b();
    let v = cemt_iv();
    let sized = engine::size(&s, &v).unwrap();
    // The throat is narrower than 3.2 B, the bar shallower than 1.3 T, the bend tighter than 3 L.
    assert!(25.0 < sized.bottom_width_straight_m);
    let stretches = constraint_stretches(&s, &v, &sized, &synthetic_river(), 10.0).unwrap();
    let got: Vec<(&str, f64, f64, &str)> = stretches
        .iter()
        .map(|x| (x.kind.key(), x.start_chainage_m, x.end_chainage_m, x.works))
        .collect();
    assert_eq!(
        got,
        [
            ("width", 1000.0, 1100.0, "bank-cut"),
            ("depth", 1800.0, 1900.0, "dredge"),
            ("radius", 2400.0, 2500.0, "cut-through"),
        ]
    );
    let [w, d, r] = &stretches[..] else { panic!() };
    assert_eq!(w.governing_rule, "pianc-wg141/Table5.7/bottom-width-b");
    assert_eq!(w.required, sized.bottom_width_straight_m);
    assert_eq!(w.deficit, sized.bottom_width_straight_m - 25.0);
    assert_eq!(d.governing_rule, "pianc-wg141/5.3.4.3/design-depth-b");
    assert_eq!(d.deficit, sized.design_depth_m - 1.5);
    assert_eq!(d.dredge_m3, 11.0 * 10.0 * 10.0);
    assert_eq!(r.governing_rule, "pianc-wg141/Table5.7/min-bend-radius-b");
    assert_eq!(r.required, 3.0 * 110.0);
    assert_eq!(r.river, 150.0);
}

/// splitmix64, for random stations with no random-number crate.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Every station that falls short of a governing limit lies in exactly one stretch of its kind,
/// and no other station lies in one.
#[test]
fn every_short_station_is_in_exactly_one_stretch_of_its_kind_and_no_other_is() {
    let s = pianc_b();
    let v = cemt_iv();
    let sized = engine::size(&s, &v).unwrap();
    let narrow = s.narrowed_to(engine::BEND_WIDENING, &v);
    let mut rng = Rng(5);
    for _ in 0..50 {
        let stations: Vec<StationIn> = (0..200)
            .map(|i| StationIn {
                chainage_m: i as f64 * 10.0,
                xy: [i as f64 * 10.0, 0.0],
                river_width_m: 20.0 + 40.0 * rng.next(),
                river_depth_m: 2.0 + 2.5 * rng.next(),
                river_radius_m: 200.0 + 300.0 * rng.next(),
                line_radius_m: 200.0 + 2000.0 * rng.next(),
                reach: 0,
                dredge_area_m2: rng.next(),
                dry_cut_area_m2: rng.next(),
            })
            .collect();
        let stretches = constraint_stretches(&s, &v, &sized, &stations, 10.0).unwrap();
        for kind in StretchKind::ALL {
            for (i, st) in stations.iter().enumerate() {
                let short = match kind {
                    StretchKind::Width => falls_short(
                        sized.bottom_width_straight_m
                            + engine::widening(&narrow, &v, st.line_radius_m).unwrap(),
                        st.river_width_m,
                    ),
                    StretchKind::Depth => falls_short(sized.design_depth_m, st.river_depth_m),
                    StretchKind::Radius => falls_short(sized.min_bend_radius_m, st.river_radius_m),
                };
                let holding = stretches
                    .iter()
                    .filter(|x| x.kind == kind && x.stations.contains(&i))
                    .count();
                assert_eq!(holding, usize::from(short), "{kind:?} station {i}");
            }
        }
        // A stretch is a run: its stations are consecutive, and its ends are their chainages.
        for x in &stretches {
            assert!(x.stations.windows(2).all(|w| w[1] == w[0] + 1));
            assert_eq!(x.start_chainage_m, stations[x.stations[0]].chainage_m);
            assert_eq!(
                x.end_chainage_m,
                stations[*x.stations.last().unwrap()].chainage_m
            );
        }
    }
}

#[test]
fn the_design_basis_names_the_governing_rule_of_each_quantity_per_reach_kind() {
    let v = cemt_iv();
    let s = Selection::built_in(
        &["pianc-wg141", "gb50139", "owd"],
        &[
            (engine::SE_QUALITY, "C"),
            (engine::RANGE_END, "strict"),
            (engine::GB_CLASS, "IV"),
        ],
    )
    .unwrap()
    .with_reaches(vec![
        engine::WaterwayReach {
            label: "0 to 2000 m".into(),
            waterway: "canal".into(),
            claim: "yes".into(),
            claim_typed: true,
        },
        engine::WaterwayReach {
            label: "2000 m to the end".into(),
            ..engine::WaterwayReach::whole_river()
        },
    ]);
    let sized = engine::size(&s, &v).unwrap();
    let basis = design_basis(&s, &v, &sized).unwrap();
    let rows: Vec<(&str, Option<&str>, &str)> = basis
        .iter()
        .map(|b| (b.quantity, b.waterway.as_deref(), b.rule.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("bottom-width", None, "pianc-wg141/Table5.7/bottom-width-c"),
            // GB 50139-2014 has no reduced radius: the claimed canal takes 4 L too.
            (
                "min-bend-radius",
                Some("canal"),
                "gb50139/3.0.5(4)/min-bend-radius-motor"
            ),
            (
                "min-bend-radius",
                Some("free-flowing-river"),
                "gb50139/3.0.5(4)/min-bend-radius-motor"
            ),
            // 1.2 T = 3.0 m is above T + 0.3 m (class IV, strict).
            ("design-depth", None, "pianc-wg141/5.3.4.3/design-depth-c"),
            (
                "widening-at-rmin",
                None,
                "pianc-wg141/5.3.3/bend-widening-c"
            ),
        ]
    );
    for b in &basis {
        assert_eq!(b.status, "unverified");
        assert!(!b.citation.is_empty(), "{}", b.rule);
        // Each row names the other Rules of its Requirement, all with a lower limit.
        for (_, limit) in &b.others {
            assert!(*limit <= b.value, "{}: {limit} > {}", b.rule, b.value);
        }
    }
    // The canal's radius (4 L) sits with the PIANC quality C radius (2 L) in its row.
    let canal = &basis[1];
    assert_eq!(canal.value, 4.0 * 110.0);
    assert!(
        canal
            .others
            .iter()
            .any(|(r, _)| r == "pianc-wg141/Table5.7/min-bend-radius-c")
    );
}
