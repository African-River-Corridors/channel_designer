//! The `ovcd-core` dependency is pinned to a tag, never a branch (ADR 0001), and the vessel
//! classes it holds are the ones the Python reference gets from `vessel_designer` at the same tag.
//!
//! Label: snapshot. The class table below is what `vessel_designer` v0.2.2 printed
//! (`VESSEL_STANDARDS`, the Python pinned in `pyproject.toml`). Never hand-edit it.

use ovcd_core::vessel_standards::get_standard;

#[test]
fn ovcd_core_is_pinned_to_a_tag_not_a_branch() {
    let manifest = include_str!("../Cargo.toml");
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("ovcd-core"))
        .expect("cd-core depends on ovcd-core");
    assert!(
        line.contains("tag = \"v"),
        "ovcd-core must be pinned by tag: {line}"
    );
    assert!(
        !line.contains("branch"),
        "ovcd-core must not follow a branch: {line}"
    );
    let pyproject = include_str!("../../../pyproject.toml");
    let tag = line
        .split("tag = \"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap();
    assert!(
        pyproject.contains(&format!("open-vessel-concept-drawings@{tag}")),
        "the Python reference must pin the same tag ({tag}) as the Rust dependency"
    );
}

/// (id, class, formation, LOA min, LOA max, beam, draught min, draught max)
type Row = (
    &'static str,
    &'static str,
    &'static str,
    f64,
    f64,
    f64,
    f64,
    f64,
);

const PYTHON_V0_2_2: [Row; 12] = [
    ("CEMT_IV", "IV", "motor_vessel", 80.0, 85.0, 9.5, 2.5, 2.5),
    ("CEMT_Va", "Va", "motor_vessel", 95.0, 110.0, 11.4, 2.5, 4.5),
    (
        "CEMT_Vb",
        "Vb",
        "pushed_convoy",
        172.0,
        185.0,
        11.4,
        2.5,
        4.5,
    ),
    (
        "CEMT_VIa",
        "VIa",
        "pushed_convoy",
        95.0,
        110.0,
        22.8,
        2.5,
        4.5,
    ),
    (
        "CEMT_VIa_motor_NL2011",
        "VIa",
        "motor_vessel",
        110.0,
        135.0,
        17.0,
        4.0,
        4.0,
    ),
    (
        "CN_I_motor",
        "I",
        "motor_vessel",
        95.0,
        95.0,
        16.2,
        3.2,
        3.2,
    ),
    (
        "CN_II_motor",
        "II",
        "motor_vessel",
        90.0,
        90.0,
        14.8,
        2.6,
        2.6,
    ),
    (
        "CN_III_motor",
        "III",
        "motor_vessel",
        85.0,
        85.0,
        10.8,
        2.0,
        2.0,
    ),
    (
        "CN_III_convoy_2ab",
        "III",
        "pushed_convoy",
        167.0,
        167.0,
        21.6,
        2.0,
        2.0,
    ),
    (
        "CN_IV_motor",
        "IV",
        "motor_vessel",
        67.5,
        67.5,
        10.8,
        1.6,
        1.6,
    ),
    (
        "CN_II_convoy_2ab",
        "II",
        "pushed_convoy",
        186.0,
        186.0,
        32.4,
        2.6,
        2.6,
    ),
    (
        "CN_I_convoy_2ab",
        "I",
        "pushed_convoy",
        223.0,
        223.0,
        32.4,
        3.5,
        3.5,
    ),
];

#[test]
fn the_rust_vessel_classes_match_the_python_reference_dimension_for_dimension() {
    for (id, class, formation, loa_lo, loa_hi, beam, t_lo, t_hi) in PYTHON_V0_2_2 {
        let v = get_standard(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(v.class_name, class, "{id} class");
        assert_eq!(v.formation, formation, "{id} formation");
        assert_eq!(v.loa_m, (loa_lo, loa_hi), "{id} LOA");
        assert_eq!(v.beam_m, beam, "{id} beam");
        assert_eq!(v.draught_m, (t_lo, t_hi), "{id} draught");
    }
}
