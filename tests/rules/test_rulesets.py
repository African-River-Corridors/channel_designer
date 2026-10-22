"""Test triad for the channel standards layer: golden, property, feasibility.

Covers ``channel_shape`` by Rule Set selection (spec 0007): ``pianc-wg141`` with the Safety and
Ease quality, and ``gb50139`` (GB 50139-2014, #250) with the range end. GB 50139-2014 has no reduced
radius, so a claimed canal reach changes nothing.
The coefficients live in the Rule Set files; these tests read them back through the sized channel.
The size_channel path is covered at the end of this file.
"""
import math

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
    """Label: sourced - GB 50139-2014 3.0.5(4), p8: 3L pushed convoy / 4L cargo ship. The 2004
    reduced radius is gone (explanation of 3.0.5, p43): a claimed canal reach takes the same."""
    for vid, k in (("CN_III_convoy_2ab", 3.0), ("CN_I_motor", 4.0)):
        L = get_standard(vid).loa_m[1]
        assert shape("CN", vid).min_bend_radius_m == k * L
        assert shape("CN-reduced", vid).min_bend_radius_m == k * L


def test_golden_shape_cemt_via_pianc_b():
    """PIANC B: 1.3 x 4.4 = 5.72 m; GB 50139-2014 class II permissive: 4.4 + 0.3 = 4.70 m."""
    d44_cn = shape("CN", "CN_II_motor", draught_m=4.4).design_depth_m
    d44_pi = shape("PIANC-B", "CN_II_motor", draught_m=4.4).design_depth_m
    assert abs((d44_pi - d44_cn) - 1.02) < 1e-6, (d44_cn, d44_pi)


def test_property_two_sets_take_the_larger_of_each_quantity():
    """Every selected Rule applies and the strictest governs (spec 0007)."""
    both = channel_shape("CN_II_motor", 2.6, rule_sets=["pianc-wg141", "gb50139"],
                         set_inputs={"se_quality": "B", "range_end": "strict"})
    p, c = shape("PIANC-B", "CN_II_motor", 2.6), shape("CN-conservative", "CN_II_motor", 2.6)
    for k in ("bottom_width_straight_m", "min_bend_radius_m", "design_depth_m"):
        assert getattr(both, k) == max(getattr(p, k), getattr(c, k)), k


def test_golden_cn_depth_takes_the_range_end_of_table_a_0_1():
    """Label: sourced - GB 50139-2014 A.0.1 and Table A.0.1, p26: class II 0.3-0.4 m. At T 4.4 the
    permissive end gives 4.70 m and the strict end 4.80 m."""
    c = shape("CN", "CN_II_motor", draught_m=4.4)
    assert abs(c.design_depth_m - 4.70) < 1e-9
    assert any("permissive end of GB 50139-2014 Table A.0.1's class-II range" in w for w in c.warnings)
    c = shape("CN-conservative", "CN_II_motor", draught_m=4.4)
    assert abs(c.design_depth_m - 4.80) < 1e-9


def test_property_cn_depth_clearance_is_per_class_not_per_set():
    """Label: sourced - GB 50139-2014 Table A.0.1: I 0.4-0.5, II-III 0.3-0.4, IV 0.2-0.3 m. Class I
    never gets less clearance than class II."""
    for vid, lo, hi in (("CN_I_motor", 0.4, 0.5), ("CN_II_motor", 0.3, 0.4),
                        ("CN_III_motor", 0.3, 0.4), ("CN_IV_motor", 0.2, 0.3)):
        for name, expect in (("CN", lo), ("CN-conservative", hi)):
            c = shape(name, vid)
            assert abs((c.design_depth_m - c.draught_m) - expect) < 1e-9, (vid, name)
    ci, cii = shape("CN", "CN_I_motor"), shape("CN", "CN_II_motor")
    assert (ci.design_depth_m - ci.draught_m) > (cii.design_depth_m - cii.draught_m)


def test_feasibility_a_cemt_vessel_takes_its_gb_class_by_tonnage():
    """Label: sourced - GB 50139-2014 Table 3.0.1 and ECMT Resolution 92/2. CEMT IV (up to 1500 t)
    is GB class II; a typed ``gb_class`` overrides it."""
    c = shape("CN", "CEMT_IV", draught_m=2.5)
    assert c.width_basis == "appendix-a"
    assert abs(c.design_depth_m - 2.8) < 1e-9
    assert any("by tonnage: CEMT IV up to 1500 t" in w for w in c.warnings)
    c = channel_shape("CEMT_IV", 2.5, rule_sets=["gb50139"],
                      set_inputs={"range_end": "permissive", "gb_class": "IV"})
    assert abs(c.design_depth_m - 2.7) < 1e-9


def test_feasibility_depth_below_pianc_ratio_is_reported():
    """4.70 m at T 4.4 is 0.58 m under PIANC's 1.2*T river floor. Never let that go unsaid."""
    c = shape("CN", "CN_II_motor", draught_m=4.4)
    assert c.design_depth_m < 1.2 * 4.4
    assert any("BELOW PIANC" in w for w in c.warnings)


def test_property_unknown_sets_and_vessels_raise():
    with pytest.raises(ValueError, match="unknown Rule Set"):
        channel_shape("CEMT_VIa", rule_sets=["NOPE"])
    with pytest.raises(KeyError):
        channel_shape("NOPE")


# --- feasibility: the honesty flags fire ---------------------------------------------

def test_feasibility_a_typical_vessel_takes_its_table_row():
    """Label: sourced - GB 50139-2014 Table 3.0.2-1, p4: 186 x 32.4 m, class II, 70 m."""
    c = shape("CN", "CN_II_convoy_2ab")
    assert c.width_basis == "table"
    assert abs(c.bottom_width_straight_m - 70.0) < 1e-9


def test_feasibility_a_special_design_vessel_takes_the_a_0_2_formula():
    """Label: sourced - GB 50139-2014 3.0.7 and A.0.2: the class II cargo ship has no row, so its
    width is (B + L sin 3 deg)(1 + 2 x 0.34) at the permissive end."""
    c = shape("CN", "CN_II_motor")
    assert c.width_basis == "appendix-a"
    assert any("special design vessel" in w for w in c.warnings)


def test_feasibility_pianc_carries_no_calibration_warning():
    c = shape("PIANC-B", "CEMT_VIa")
    assert c.width_basis == "rule"
    assert c.warnings == []


# GB 50139-2014 Table 3.0.2-1 (pp3-4): the catalogue's typical vessels and their single-line widths.
_TABLE_WIDTHS = {"CN_I_convoy_2ab": 70.0, "CN_II_convoy_2ab": 70.0, "CN_III_convoy_2ab": 45.0,
                 "CN_IV_motor": 30.0}


def test_cn_widths_are_the_2014_table_or_the_a_0_2_formula():
    """Label: sourced - GB 50139-2014 Table 3.0.2-1 and A.0.2. Every Chinese vessel class under every
    Chinese selection: a typical vessel takes its row, the class I-III cargo ships the formula with
    beta 3 deg and d = 0.34 / 0.40 BF."""
    for name, d in (("CN", 0.34), ("CN-conservative", 0.40), ("CN-reduced", 0.40)):
        for sid in ("CN_I_convoy_2ab", "CN_II_convoy_2ab", "CN_III_convoy_2ab", "CN_IV_motor",
                    "CN_I_motor", "CN_II_motor", "CN_III_motor"):
            v = get_standard(sid)
            s = shape(name, sid)
            if sid in _TABLE_WIDTHS:
                assert s.bottom_width_straight_m == _TABLE_WIDTHS[sid], (name, sid)
                assert s.width_basis == "table", (name, sid)
            else:
                bf = v.beam_m + v.loa_m[1] * math.sin(math.radians(3.0))
                assert abs(s.bottom_width_straight_m - bf * (1 + 2 * d)) < 1e-9, (name, sid)
                assert s.width_basis == "appendix-a", (name, sid)


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
