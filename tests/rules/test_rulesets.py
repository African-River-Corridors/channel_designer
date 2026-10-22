"""Test triad for the channel standards layer: golden, property, feasibility.

Covers ``channel_shape`` by Rule Set selection (spec 0007): ``pianc-wg141`` with the Safety and
Ease quality, ``gb50139`` with the range end, and the reduced GB radius on a claimed canal reach.
The coefficients live in the Rule Set files; these tests read them back through the sized channel.
The size_channel path is covered at the end of this file.
"""
import pytest

from owd.rules.rulesets import channel_shape, design_basis
from vessel_designer.core.vessel_standards import get_standard

CANAL = [{"waterway": "canal", "reduced_radius_claim": "yes"}]

# What each old ruleset name now is (spec 0007, "Rule Sets at 0.3.0").
SETS = {
    "PIANC-A": dict(rule_sets=["pianc-wg141"], set_inputs={"se_quality": "A"}),
    "PIANC-B": dict(rule_sets=["pianc-wg141"], set_inputs={"se_quality": "B"}),
    "PIANC-C": dict(rule_sets=["pianc-wg141"], set_inputs={"se_quality": "C"}),
    "CN": dict(rule_sets=["gb50139"], set_inputs={"range_end": "permissive"}),
    "CN-conservative": dict(rule_sets=["gb50139"], set_inputs={"range_end": "strict"}),
    "CN-reduced": dict(rule_sets=["gb50139"], set_inputs={"range_end": "strict"}, reaches=CANAL),
}


def shape(name, vessel, draught_m=None):
    return channel_shape(vessel, draught_m, **SETS[name])


# --- golden: values straight from WG 141 ---------------------------------------------

def test_golden_pianc_table_5_7():
    """Table 5.7 (p109): width 3.0/3.2/3.4 x B, R_min 2/3/4 x L, depth 1.2/1.3/1.3 x T."""
    v = get_standard("CEMT_IV")
    L, B = v.loa_m[1], v.beam_m
    for q, (kw, kr, kd) in zip("CBA", ((3.0, 2.0, 1.2), (3.2, 3.0, 1.3), (3.4, 4.0, 1.3))):
        s = shape(f"PIANC-{q}", "CEMT_IV", draught_m=2.5)
        assert s.bottom_width_straight_m == kw * B, q
        assert s.min_bend_radius_m == kr * L, q
        assert s.design_depth_m == max(kd * 2.5, 1.2 * 2.5), q


def test_golden_pianc_c_c_values():
    """Table 5.7 fn.3 + Table E.5 (p261). A = 0.6 (Class VIa laden downstream); B and C 0.5.
    Far from the cap, the widening is c_C x L^2 / R."""
    L, R = get_standard("CEMT_IV").loa_m[1], 5000.0
    for q, c in (("A", 0.6), ("B", 0.5), ("C", 0.5)):
        w = shape(f"PIANC-{q}", "CEMT_IV").bend_widening_m(R)
        assert abs(w - c * L * L / R) < 1e-12, q


def test_golden_chinese_radius_rule():
    """A.3.2 (p164): 3L pushed fleet / 4L cargo vessel, reducible to 2L / 3L on a claimed canal."""
    for vid, normal, reduced in (("CN_I_convoy_2ab", 3.0, 2.0), ("CN_I_motor", 4.0, 3.0)):
        L = get_standard(vid).loa_m[1]
        assert shape("CN", vid).min_bend_radius_m == normal * L
        assert shape("CN-reduced", vid).min_bend_radius_m == reduced * L


def test_golden_shape_cemt_via_pianc_b():
    """CEMT VIa under PIANC-B."""
    c = shape("PIANC-B", "CEMT_VIa")
    assert c.loa_m == 110.0 and c.beam_m == 22.8          # sized on the LONG end
    assert abs(c.bottom_width_straight_m - 3.2 * 22.8) < 1e-9
    assert c.min_bend_radius_m == 3.0 * 110.0
    assert abs(c.design_depth_m - 1.3 * c.draught_m) < 1e-9
    assert c.rule_sets == "pianc-wg141@0.3.0 + owd@0.3.0"


# --- property: invariants ------------------------------------------------------------

def test_property_bend_widening_monotone_and_capped():
    """Non-increasing in R, zero on a straight, and sane at the set's own minimum radius."""
    for name in SETS:
        s = shape(name, "CEMT_VIa")
        prev = None
        for R in (150.0, 300.0, 600.0, 1200.0, 5000.0):
            w = s.bend_widening_m(R)
            assert w >= 0.0
            if prev is not None:
                assert w <= prev + 1e-9, name
            prev = w
        assert s.bend_widening_m(float("inf")) == 0.0
        assert 0.0 < s.bend_widening_m(s.min_bend_radius_m) < s.loa_m, name


def test_property_only_pianc_states_a_cap():
    """PIANC caps dF_C at L (5.3.3 p101). The Chinese summary states no cap, so ours must
    not invent one: the formula is only meaningful at R >= R_min."""
    a, cn = shape("PIANC-A", "CEMT_VIa"), shape("CN", "CEMT_VIa")
    assert a.bend_widening_m(1.0) == a.loa_m          # capped
    assert cn.bend_widening_m(1.0) > cn.loa_m         # uncapped, faithful


def test_property_china_bend_formula_approximates_c_c_half():
    """China's L^2/(2R+B) tends to PIANC c_C = 0.5 as R >> B. The two traditions agree."""
    cn = shape("CN", "CEMT_VIa")
    L, R = cn.loa_m, 5000.0
    assert abs(cn.bend_widening_m(R) - 0.5 * L * L / R) / (0.5 * L * L / R) < 0.01


def test_property_depth_rules_diverge_with_draught():
    """PIANC is a RATIO (1.3*T), China an ABSOLUTE add-on (class II: +0.4 m), so the two
    cross. Below the crossover China is the DEEPER rule; above it China falls away and keeps
    falling. At 4.4 m they are 0.92 m apart.

        T + 0.4 > 1.3*T   <=>   T < 1.333 m
    """
    c = shape("CN", "CN_II_motor", draught_m=1.0).design_depth_m
    p = shape("PIANC-B", "CN_II_motor", draught_m=1.0).design_depth_m
    assert c > p, (c, p)
    prev = None
    for T in (2.0, 2.6, 3.0, 4.0, 4.4, 5.0):
        p = shape("PIANC-B", "CN_II_motor", draught_m=T).design_depth_m
        c = shape("CN", "CN_II_motor", draught_m=T).design_depth_m
        assert c < p, T
        if prev is not None:
            assert c / p < prev, "the gap should widen with draught"
        prev = c / p
    d44_cn = shape("CN", "CN_II_motor", draught_m=4.4).design_depth_m
    d44_pi = shape("PIANC-B", "CN_II_motor", draught_m=4.4).design_depth_m
    assert abs((d44_pi - d44_cn) - 0.92) < 1e-6, (d44_cn, d44_pi)


def test_property_two_sets_take_the_larger_of_each_quantity():
    """Every selected Rule applies and the strictest governs (spec 0007)."""
    both = channel_shape("CN_II_motor", 2.6, rule_sets=["pianc-wg141", "gb50139"],
                         set_inputs={"se_quality": "B", "range_end": "strict"})
    p, c = shape("PIANC-B", "CN_II_motor", 2.6), shape("CN-conservative", "CN_II_motor", 2.6)
    for k in ("bottom_width_straight_m", "min_bend_radius_m", "design_depth_m"):
        assert getattr(both, k) == max(getattr(p, k), getattr(c, k)), k


def test_golden_cn_depth_is_the_strict_end_of_the_class_range():
    """The strict end, inside GB 50139's range. At T 4.4 the class II strict end is 0.4 m, so
    4.80 m."""
    c = shape("CN", "CN_II_motor", draught_m=4.4)
    assert abs(c.design_depth_m - 4.80) < 1e-9
    assert any("strict end of GB 50139's class-II range" in w for w in c.warnings)


def test_property_cn_depth_clearance_is_per_class_not_per_set():
    """The clause gives class I 0.4-0.5 m and classes II-III 0.3-0.4 m; class I must never get
    less clearance than class II."""
    for vid, expect in (("CN_I_motor", 0.5), ("CN_II_motor", 0.4), ("CN_III_motor", 0.4)):
        c = shape("CN", vid)
        assert abs((c.design_depth_m - c.draught_m) - expect) < 1e-9, (vid, c.design_depth_m)
    ci, cii = shape("CN", "CN_I_motor"), shape("CN", "CN_II_motor")
    assert (ci.design_depth_m - ci.draught_m) > (cii.design_depth_m - cii.draught_m)


def test_feasibility_class_outside_the_clause_is_flagged_not_invented():
    """A.3.2 covers classes I-III. Class IV must say the number is a user choice, not the standard."""
    c = shape("CN", "CN_IV_motor")
    assert any("covers classes I-III only" in w and "user choice" in w for w in c.warnings)


def test_feasibility_depth_below_pianc_ratio_is_reported():
    """4.80 m at T 4.4 is 0.48 m under PIANC's 1.2*T river floor. Never let that go unsaid."""
    c = shape("CN", "CN_II_motor", draught_m=4.4)
    assert c.design_depth_m < 1.2 * 4.4
    assert any("BELOW PIANC" in w for w in c.warnings)


def test_property_unknown_sets_and_vessels_raise():
    with pytest.raises(ValueError, match="unknown Rule Set"):
        channel_shape("CEMT_VIa", rule_sets=["NOPE"])
    with pytest.raises(KeyError):
        channel_shape("NOPE")


# --- feasibility: the honesty flags fire ---------------------------------------------

def test_feasibility_table_a2_beats_the_formula():
    """A vessel matching a Table A.2 entry must use the TABLE, and say the formula differs.
    WG 141's A.3.2 formula omits wind, so using it alone under-sizes the Chinese channel."""
    c = shape("CN", "CN_II_convoy_2ab")       # 186 x 32.4 -> Table A.2 row
    assert c.width_basis == "table"
    assert abs(c.bottom_width_straight_m - 70.0) < 1e-9
    assert any("Table A.2" in w for w in c.warnings)


def test_feasibility_off_table_vessel_uses_the_table_5_1_ratio():
    """The width is WG 141 Table 5.1's own China-River ratio W_F/B = 2.3."""
    c = shape("CN", "CEMT_VIa")               # no Table A.2 row near 110 x 22.8
    assert c.width_basis == "table-5.1-ratio"
    assert abs(c.bottom_width_straight_m - 2.3 * get_standard("CEMT_VIa").beam_m) < 1e-9
    assert any("Table 5.1" in w and "WIND-INCLUDED" in w for w in c.warnings)


def test_feasibility_pianc_carries_no_calibration_warning():
    c = shape("PIANC-B", "CEMT_VIa")
    assert c.width_basis == "rule"
    assert c.warnings == []


# Chinese-family straight widths for every vessel class, pinned before Table A.2 was reduced
# to the entries the code reads. Identical values prove the dropped rows were never needed.
_CN_WIDTHS_BEFORE_A2_TRIM = {
    "CEMT_IV": (21.85, "table-5.1-ratio"), "CEMT_VIa": (52.44, "table-5.1-ratio"),
    "CEMT_VIa_motor_NL2011": (39.1, "table-5.1-ratio"), "CEMT_Va": (26.22, "table-5.1-ratio"),
    "CEMT_Vb": (26.22, "table-5.1-ratio"), "CN_III_convoy_2ab": (45.0, "table"),
    "CN_III_motor": (24.84, "table-5.1-ratio"), "CN_II_convoy_2ab": (70.0, "table"),
    "CN_II_motor": (34.04, "table-5.1-ratio"), "CN_IV_motor": (24.84, "table-5.1-ratio"),
    "CN_I_convoy_2ab": (70.0, "table"), "CN_I_motor": (37.26, "table-5.1-ratio"),
}


def test_cn_widths_are_what_the_whole_table_gave():
    """The widths of every class under every Chinese selection are what they were with the whole
    of Table A.2."""
    for name in ("CN", "CN-conservative", "CN-reduced"):
        for sid, (w, basis) in _CN_WIDTHS_BEFORE_A2_TRIM.items():
            s = shape(name, sid)
            assert abs(s.bottom_width_straight_m - w) < 1e-9, (name, sid)
            assert s.width_basis == basis, (name, sid)


def test_design_basis_names_the_governing_rule_of_each_quantity():
    rows = design_basis("CEMT_IV", 2.5, rule_sets=["pianc-wg141"], set_inputs={"se_quality": "B"})
    assert [r["quantity"] for r in rows] == [
        "bottom-width", "min-bend-radius", "design-depth", "widening-at-rmin"]
    assert rows[0]["rule"] == "pianc-wg141/Table5.7/bottom-width-b"
    assert all(r["status"] == "unverified" for r in rows)


# --- size_channel path ----------------------------------------------------------------

def test_feasibility_size_channel_rejects_china_instead_of_running_pianc():
    """rule_set='china' used to pass validation and silently run the PIANC path."""
    from owd.rules.rulesets import ChannelRuleInputs
    with pytest.raises(ValueError, match="channel_shape"):
        ChannelRuleInputs(beam_m=12.0, loa_m=80.0, draft_m=2.0, min_water_level_m=0.0,
                          rule_set="china")


def test_property_size_channel_every_rule_set_runs():
    from owd.rules.rulesets import ChannelRuleInputs, rule_sets, size_channel
    for rs in rule_sets():
        r = size_channel(ChannelRuleInputs(beam_m=12.0, loa_m=80.0, draft_m=2.0,
                                           min_water_level_m=7.0, rule_set=rs))
        assert r.ok and r.bend_bottom_width_m >= r.straight_bottom_width_m
        assert abs(r.bed_level_m - (7.0 - r.required_depth_m)) < 1e-12
