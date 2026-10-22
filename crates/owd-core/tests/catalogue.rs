//! The full catalogue (spec 0015 T0): the 12 classes OWD offered before keep their GB 50139 class,
//! and every pickable class sizes under PIANC WG 141.
//!
//! Label: property

use owd_core::rules::classes::{class, default_class_ids};
use owd_core::rules::engine::{Vessel, gb_class_found, selection_for_old_ruleset};
use owd_core::rules::full_catalogue;
use owd_core::rules::rulesets::{channel_shape_for, vessel_for};

/// The 12 classes as v0.2.2 named them: (old id, family, class).
const OLD_NAMES: [(&str, &str, &str); 12] = [
    ("CEMT_IV", "CEMT", "IV"),
    ("CEMT_Va", "CEMT", "Va"),
    ("CEMT_Vb", "CEMT", "Vb"),
    ("CEMT_VIa", "CEMT", "VIa"),
    ("CEMT_VIa_motor_NL2011", "CEMT", "VIa"),
    ("CN_I_motor", "CN", "I"),
    ("CN_II_motor", "CN", "II"),
    ("CN_III_motor", "CN", "III"),
    ("CN_IV_motor", "CN", "IV"),
    ("CN_III_convoy_2ab", "CN", "III"),
    ("CN_II_convoy_2ab", "CN", "II"),
    ("CN_I_convoy_2ab", "CN", "I"),
];

#[test]
fn the_twelve_keep_their_gb_50139_class() {
    for (old, family, class_name) in OLD_NAMES {
        let now = vessel_for(old, None).unwrap();
        let before = Vessel {
            family: family.into(),
            class_name: class_name.into(),
            ..now.clone()
        };
        let (a, b) = (gb_class_found(&before, &[]), gb_class_found(&now, &[]));
        assert_eq!(
            (a.class.as_str(), a.above_3000_t),
            (b.class.as_str(), b.above_3000_t),
            "{old}: {} / {}",
            a.how,
            b.how
        );
    }
}

#[test]
fn the_default_list_is_the_twelve_by_catalogue_id() {
    let ids: Vec<String> = OLD_NAMES
        .iter()
        .map(|(old, _, _)| class(old).unwrap().standard_id)
        .collect();
    assert_eq!(ids, default_class_ids());
}

#[test]
fn every_pickable_class_sizes_under_pianc() {
    let sel = selection_for_old_ruleset("PIANC-B").unwrap();
    let all = full_catalogue();
    assert!(all.len() > 100, "{}", all.len());
    for (id, _) in &all {
        let s = channel_shape_for(&sel, id, None).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert!(
            s.bottom_width_straight_m > s.beam_m
                && s.min_bend_radius_m > 0.0
                && s.design_depth_m > s.draught_m,
            "{id}: {s:?}"
        );
    }
    // A class from beyond the old 12 sizes: the maintainer's example.
    let s = channel_shape_for(&sel, "CEMT-VIb-motor", None).unwrap();
    assert_eq!((s.loa_m, s.beam_m), (140.0, 15.0));
}
