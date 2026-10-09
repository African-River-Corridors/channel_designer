"""Test triad for the channel_rules standards layer: golden, property, feasibility.

Covers RULESETS + channel_shape — the "ruleset x vessel class -> channel shape" front
door. The size_channel path is covered at the end of this file.
"""


from channel_designer.rules.rulesets import (
    CN_TABLE_A2,
    RULESETS,
    _bend_widening,
    channel_shape,
    get_ruleset,
)
from vessel_designer.core.vessel_standards import get_standard


# --- golden: values straight from WG 141 ---------------------------------------------

def test_golden_pianc_table_5_7():
    """Table 5.7 (p109): width 3.0/3.2/3.4 x B, R_min 2/3/4 x L, depth 1.2/1.3/1.3 x T."""
    assert [RULESETS[f"PIANC-{q}"].width_mult for q in "CBA"] == [3.0, 3.2, 3.4]
    assert [RULESETS[f"PIANC-{q}"].r_mult_convoy for q in "CBA"] == [2.0, 3.0, 4.0]
    assert [RULESETS[f"PIANC-{q}"].depth_mult for q in "CBA"] == [1.2, 1.3, 1.3]


def test_golden_pianc_c_c_values():
    """Table 5.7 fn.3 + Table E.5 (p261). A = 0.6 (Class VIa laden downstream)."""
    assert RULESETS["PIANC-A"].c_c == 0.6
    assert RULESETS["PIANC-B"].c_c == 0.5
    assert RULESETS["PIANC-C"].c_c == 0.5


def test_golden_chinese_radius_rule():
    """A.3.2 (p164): 3L pushed fleet / 4L cargo vessel, reducible to 2L / 3L."""
    cn, red = get_ruleset("CN"), get_ruleset("CN-reduced")
    assert (cn.r_mult_convoy, cn.r_mult_motor) == (3.0, 4.0)
    assert (red.r_mult_convoy, red.r_mult_motor) == (2.0, 3.0)


def test_golden_shape_cemt_via_pianc_b():
    """CEMT VIa under PIANC-B."""
    c = channel_shape("PIANC-B", "CEMT_VIa")
    assert c.loa_m == 110.0 and c.beam_m == 22.8          # sized on the LONG end
    assert abs(c.bottom_width_straight_m - 3.2 * 22.8) < 1e-9
    assert c.min_bend_radius_m == 3.0 * 110.0
    assert abs(c.design_depth_m - 1.3 * c.draught_m) < 1e-9


# --- property: invariants ------------------------------------------------------------

def test_property_bend_widening_monotone_and_capped():
    """Non-increasing in R, zero on a straight, and PIANC's dF_C never exceeds L."""
    for rid, rs in RULESETS.items():
        L, B = 110.0, 22.8
        prev = None
        for R in (150.0, 300.0, 600.0, 1200.0, 5000.0):
            w = _bend_widening(rs, L, B, R)
            assert w >= 0.0
            if prev is not None:
                assert w <= prev + 1e-9, rid
            prev = w
        assert _bend_widening(rs, L, B, float("inf")) == 0.0
        # at the ruleset's own minimum radius, every rule must give a sane widening
        r_min = rs.r_mult_convoy * L
        assert 0.0 < _bend_widening(rs, L, B, r_min) < L, rid


def test_property_only_pianc_states_a_cap():
    """PIANC caps dF_C at L (5.3.3 p101). The Chinese summary states no cap, so ours must
    not invent one — the formula is only meaningful at R >= R_min."""
    L, B = 110.0, 22.8
    assert _bend_widening(get_ruleset("PIANC-A"), L, B, 1.0) == L      # capped
    assert _bend_widening(get_ruleset("CN"), L, B, 1.0) > L            # uncapped, faithful


def test_property_china_bend_formula_approximates_c_c_half():
    """China's L^2/(2R+B) tends to PIANC c_C = 0.5 as R >> B. The two traditions agree."""
    L, B, R = 110.0, 22.8, 5000.0
    cn = _bend_widening(get_ruleset("CN"), L, B, R)
    pianc_half = 0.5 * L * L / R
    assert abs(cn - pianc_half) / pianc_half < 0.01


def test_property_depth_rules_diverge_with_draught():
    """PIANC is a RATIO (1.3*T), China an ABSOLUTE add-on (class II: +0.4 m), so the two
    cross. Below the crossover China is the DEEPER rule; above it China falls away and keeps
    falling. At the class II design draught of 2.6 m the two rules are close; at 4.4 m they
    are 0.92 m apart.

        T + 0.4 > 1.3*T   <=>   T < 1.333 m
    """
    # below the crossover the Chinese absolute clearance is the more generous rule
    c = channel_shape("CN", "CN_II_motor", draught_m=1.0).design_depth_m
    p = channel_shape("PIANC-B", "CN_II_motor", draught_m=1.0).design_depth_m
    assert c > p, (c, p)
    # above it China is shallower, and the gap widens monotonically with draught
    prev = None
    for T in (2.0, 2.6, 3.0, 4.0, 4.4, 5.0):
        p = channel_shape("PIANC-B", "CN_II_motor", draught_m=T).design_depth_m
        c = channel_shape("CN", "CN_II_motor", draught_m=T).design_depth_m
        ratio = c / p
        assert c < p, T
        if prev is not None:
            assert ratio < prev, "the gap should widen with draught"
        prev = ratio
    # at T = 4.4 m the rules differ by 0.92 m
    d44_cn = channel_shape("CN", "CN_II_motor", draught_m=4.4).design_depth_m
    d44_pi = channel_shape("PIANC-B", "CN_II_motor", draught_m=4.4).design_depth_m
    assert abs((d44_pi - d44_cn) - 0.92) < 1e-6, (d44_cn, d44_pi)


def test_golden_cn_depth_is_the_strict_end_of_the_class_range():
    """Default: the strict end, inside GB 50139's range. At T 4.4 the class II strict end is
    0.4 m, so 4.80 m."""
    c = channel_shape("CN", "CN_II_motor", draught_m=4.4)
    assert abs(c.design_depth_m - 4.80) < 1e-9
    assert any("strict end of GB 50139's class-II range" in w for w in c.warnings)


def test_property_cn_depth_clearance_is_per_class_not_per_ruleset():
    """The clause gives class I 0.4-0.5 m and classes II-III 0.3-0.4 m. A single ruleset-wide
    add-on averaged those, which handed class I LESS than the standard's minimum. Guard it."""
    for vid, expect in (("CN_I_motor", 0.5), ("CN_II_motor", 0.4), ("CN_III_motor", 0.4)):
        c = channel_shape("CN", vid)
        assert abs((c.design_depth_m - c.draught_m) - expect) < 1e-9, (vid, c.design_depth_m)
    # class I must never be given less clearance than class II — the old bug
    ci = channel_shape("CN", "CN_I_motor")
    cii = channel_shape("CN", "CN_II_motor")
    assert (ci.design_depth_m - ci.draught_m) > (cii.design_depth_m - cii.draught_m)


def test_feasibility_class_outside_the_clause_is_flagged_not_invented():
    """A.3.2 covers classes I-III. Class IV must say the number is a user choice, not the standard."""
    c = channel_shape("CN", "CN_IV_motor")
    assert any("covers classes I-III only" in w and "user choice" in w for w in c.warnings)


def test_feasibility_depth_below_pianc_ratio_is_reported():
    """4.80 m at T 4.4 is 0.48 m under PIANC's 1.2*T river floor. Never let that go unsaid."""
    c = channel_shape("CN", "CN_II_motor", draught_m=4.4)
    assert c.design_depth_m < 1.2 * 4.4
    assert any("BELOW PIANC" in w for w in c.warnings)


def test_property_rmin_follows_length_and_formation():
    """R_min is a multiple of LOA, and China distinguishes convoy from motor vessel."""
    for rid in RULESETS:
        for vid in ("CEMT_VIa", "CN_I_motor"):
            c = channel_shape(rid, vid)
            v = get_standard(vid)
            rs = get_ruleset(rid)
            mult = rs.r_mult_motor if v.formation == "motor_vessel" else rs.r_mult_convoy
            assert abs(c.min_bend_radius_m - mult * v.loa_m[1]) < 1e-9
    # PIANC does not distinguish formation; China does
    assert get_ruleset("PIANC-B").r_mult_convoy == get_ruleset("PIANC-B").r_mult_motor
    assert get_ruleset("CN").r_mult_convoy != get_ruleset("CN").r_mult_motor


def test_property_unknown_ids_raise():
    for bad, fn in (("NOPE", lambda: get_ruleset("NOPE")),
                    ("NOPE", lambda: channel_shape("NOPE", "CEMT_VIa"))):
        try:
            fn()
        except KeyError:
            pass
        else:
            raise AssertionError(f"expected KeyError for {bad}")


# --- feasibility: the honesty flags fire ---------------------------------------------

def test_feasibility_table_a2_beats_the_formula():
    """A vessel matching a Table A.2 entry must use the TABLE, and say the formula differs.

    This is the bug this layer exists to prevent: WG 141's A.3.2 formula omits wind, so
    using it alone under-sizes the Chinese channel by ~31%.
    """
    c = channel_shape("CN", "CN_II_convoy_2ab")       # 186 x 32.4 -> Table A.2 row
    assert c.width_basis == "table"
    assert abs(c.bottom_width_straight_m - 70.0) < 1e-9
    assert any("Table A.2" in w for w in c.warnings)


def test_feasibility_off_table_vessel_uses_the_table_5_1_ratio():
    """SUPERSEDED BASIS 2026-09-19. This used to assert the width was scaled by OUR 1.31
    calibration. The basis is now WG 141 Table 5.1's own China-River ratio W_F/B = 2.3, so
    the width is the standard's number, not ours — and the test must say so, or it goes on
    certifying a basis we no longer use."""
    c = channel_shape("CN", "CEMT_VIa")               # no Table A.2 row near 110 x 22.8
    assert c.width_basis == "table-5.1-ratio"
    v = get_standard("CEMT_VIa")
    assert abs(c.bottom_width_straight_m - 2.3 * v.beam_m) < 1e-9
    assert any("Table 5.1" in w and "WIND-INCLUDED" in w for w in c.warnings)


def test_feasibility_pianc_carries_no_calibration_warning():
    c = channel_shape("PIANC-B", "CEMT_VIa")
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


def test_cn_table_a2_holds_only_what_is_read():
    """Every kept Table A.2 entry is matched by a vessel class, and the widths of every
    class under every Chinese ruleset are what they were with the whole table."""
    from vessel_designer.core.vessel_standards import VESSEL_STANDARDS
    from channel_designer.rules import rulesets as R
    used = set()
    original = R._cn_table_width

    def spy(loa, beam):
        w = original(loa, beam)
        if w is not None:
            used.add(min(CN_TABLE_A2, key=lambda k: abs(k[0] - loa) / k[0] + abs(k[1] - beam) / k[1]))
        return w

    R._cn_table_width = spy
    try:
        for sid in VESSEL_STANDARDS:
            for rs in ("CN", "CN-conservative", "CN-reduced"):
                s = channel_shape(rs, sid)
                if sid in _CN_WIDTHS_BEFORE_A2_TRIM:
                    w, basis = _CN_WIDTHS_BEFORE_A2_TRIM[sid]
                    assert abs(s.bottom_width_straight_m - w) < 1e-9, (rs, sid)
                    assert s.width_basis == basis, (rs, sid)
    finally:
        R._cn_table_width = original
    assert used == set(CN_TABLE_A2), set(CN_TABLE_A2) - used
    # values only: (LOA, beam) -> width, no other columns
    assert all(len(k) == 2 and isinstance(v, float) for k, v in CN_TABLE_A2.items())


# --- size_channel path ----------------------------------------------------------------

def test_feasibility_size_channel_rejects_china_instead_of_running_pianc():
    """rule_set='china' used to pass validation and silently run the PIANC path."""
    import pytest
    from channel_designer.rules.rulesets import ChannelRuleInputs
    with pytest.raises(ValueError, match="channel_shape"):
        ChannelRuleInputs(beam_m=12.0, loa_m=80.0, draft_m=2.0, min_water_level_m=0.0,
                          rule_set="china")


def test_property_size_channel_every_rule_set_runs():
    from channel_designer.rules.rulesets import ChannelRuleInputs, rule_sets, size_channel
    for rs in rule_sets():
        r = size_channel(ChannelRuleInputs(beam_m=12.0, loa_m=80.0, draft_m=2.0,
                                           min_water_level_m=7.0, rule_set=rs))
        assert r.ok and r.bend_bottom_width_m >= r.straight_bottom_width_m
        assert abs(r.bed_level_m - (7.0 - r.required_depth_m)) < 1e-12
