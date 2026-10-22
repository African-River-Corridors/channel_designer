//! The `ovcd-core` dependency is pinned to a tag, never a branch (ADR 0001), and it is the same OVCD
//! release `arc-rules` comes from, so OWD links one OVCD version (spec 0015 T0).
//!
//! Label: snapshot. The class table below is what `ovcd-core` v0.2.4 gives for the 12 classes OWD
//! listed by hand before the shared catalogue, found by their old ids. Never hand-edit it. Against
//! v0.2.2 two classes changed: CEMT-Va's draught range is 2.5-2.8 m (was 2.5-4.5 m), and the
//! old `CEMT_VIa_motor_NL2011` is the catalogue's `RWS-M12` (RWS 2010 family, length 135 m; the
//! design length, the long end, is unchanged).

use ovcd_core::vessel_standards::get_standard;

/// The git tag a manifest line pins (`tag = "vX.Y.Z"`).
fn tag_of(manifest: &str, dependency: &str) -> String {
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with(dependency))
        .unwrap_or_else(|| panic!("no {dependency} line"));
    assert!(
        line.contains("tag = \"v"),
        "{dependency} must be pinned by tag: {line}"
    );
    assert!(
        !line.contains("branch"),
        "{dependency} must not follow a branch: {line}"
    );
    line.split("tag = \"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap()
        .to_string()
}

#[test]
fn ovcd_core_is_pinned_to_the_tag_arc_rules_comes_from() {
    let core = tag_of(include_str!("../Cargo.toml"), "ovcd-core");
    let rules = tag_of(include_str!("../../../Cargo.toml"), "arc-rules");
    assert_eq!(
        core, rules,
        "ovcd-core and arc-rules must come from one OVCD release"
    );
}

/// (old id, catalogue id, family, formation, LOA min, LOA max, beam, draught min, draught max)
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    f64,
    f64,
    f64,
    f64,
    f64,
);

const OVCD_V0_2_4: [Row; 12] = [
    (
        "CEMT_IV",
        "CEMT-IV",
        "CEMT-1992",
        "motor_vessel",
        80.0,
        85.0,
        9.5,
        2.5,
        2.5,
    ),
    (
        "CEMT_Va",
        "CEMT-Va",
        "CEMT-1992",
        "motor_vessel",
        95.0,
        110.0,
        11.4,
        2.5,
        2.8,
    ),
    (
        "CEMT_Vb",
        "CEMT-Vb",
        "CEMT-1992",
        "pushed_convoy",
        172.0,
        185.0,
        11.4,
        2.5,
        4.5,
    ),
    (
        "CEMT_VIa",
        "CEMT-VIa",
        "CEMT-1992",
        "pushed_convoy",
        95.0,
        110.0,
        22.8,
        2.5,
        4.5,
    ),
    (
        "CEMT_VIa_motor_NL2011",
        "RWS-M12",
        "RWS-2010",
        "motor_vessel",
        135.0,
        135.0,
        17.0,
        4.0,
        4.0,
    ),
    (
        "CN_I_motor",
        "CN-I-MOTOR",
        "CN-GB50139",
        "motor_vessel",
        95.0,
        95.0,
        16.2,
        3.2,
        3.2,
    ),
    (
        "CN_II_motor",
        "CN-II-MOTOR",
        "CN-GB50139",
        "motor_vessel",
        90.0,
        90.0,
        14.8,
        2.6,
        2.6,
    ),
    (
        "CN_III_motor",
        "CN-III-MOTOR",
        "CN-GB50139",
        "motor_vessel",
        85.0,
        85.0,
        10.8,
        2.0,
        2.0,
    ),
    (
        "CN_IV_motor",
        "CN-IV-MOTOR",
        "CN-GB50139",
        "motor_vessel",
        67.5,
        67.5,
        10.8,
        1.6,
        1.6,
    ),
    (
        "CN_III_convoy_2ab",
        "CN-III-CONVOY-2x2",
        "CN-GB50139",
        "pushed_convoy",
        167.0,
        167.0,
        21.6,
        2.0,
        2.0,
    ),
    (
        "CN_II_convoy_2ab",
        "CN-II-CONVOY-2x2",
        "CN-GB50139",
        "pushed_convoy",
        186.0,
        186.0,
        32.4,
        2.6,
        2.6,
    ),
    (
        "CN_I_convoy_2ab",
        "CN-I-CONVOY-2x2",
        "CN-GB50139",
        "pushed_convoy",
        223.0,
        223.0,
        32.4,
        3.5,
        3.5,
    ),
];

#[test]
fn the_twelve_old_ids_find_their_catalogue_classes_dimension_for_dimension() {
    for (old, id, family, formation, loa_lo, loa_hi, beam, t_lo, t_hi) in OVCD_V0_2_4 {
        let v = get_standard(old).unwrap_or_else(|e| panic!("{old}: {e}"));
        assert_eq!(v.standard_id, id, "{old} id");
        assert_eq!(v.family, family, "{old} family");
        assert_eq!(v.formation, formation, "{old} formation");
        assert_eq!(v.loa_m, Some((loa_lo, loa_hi)), "{old} LOA");
        assert_eq!(v.beam_m, Some(beam), "{old} beam");
        assert_eq!(v.draught_m, Some((t_lo, t_hi)), "{old} draught");
        assert_eq!(get_standard(id).unwrap(), v, "{id} and {old} are one class");
    }
}
