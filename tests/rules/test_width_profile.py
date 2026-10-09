"""Parity tests for the width_profile bridge.

Proves the bridge adds zero arithmetic: make_width_fn(...)(R) must equal
channel_rules.width_at_radius(...) bitwise for every case, not merely approximately.
"""
from __future__ import annotations

import math

from channel_designer.rules.width_profile import make_width_fn, width_params_dict
from channel_designer.rules.rulesets import ChannelRuleInputs, width_at_radius

RULE_SETS = ("pianc",)
LOAS = (60.0, 80.0, 100.0)
BEAMS = (12.0, 16.0)
RADII = (180.0, 240.0, 300.0, 720.0, 2000.0, float("inf"))
DRAFT_M = 3.0
MIN_WL_M = 0.0


def test_bitwise_parity():
    for rule_set in RULE_SETS:
        for loa_m in LOAS:
            for beam_m in BEAMS:
                width = make_width_fn(loa_m, beam_m, DRAFT_M, rule_set=rule_set)
                reference_inputs = ChannelRuleInputs(
                    beam_m=beam_m, loa_m=loa_m, draft_m=DRAFT_M,
                    min_water_level_m=MIN_WL_M, rule_set=rule_set,
                )
                for radius_m in RADII:
                    got = width(radius_m)
                    want = width_at_radius(reference_inputs, radius_m)
                    assert got == want, (
                        f"bridge != authority for rule_set={rule_set} loa={loa_m} "
                        f"beam={beam_m} R={radius_m}: {got!r} != {want!r}"
                    )


def test_monotone_non_increasing_in_radius():
    for rule_set in RULE_SETS:
        for loa_m in LOAS:
            for beam_m in BEAMS:
                width = make_width_fn(loa_m, beam_m, DRAFT_M, rule_set=rule_set)
                finite_radii = sorted(r for r in RADII if not math.isinf(r))
                values = [width(r) for r in finite_radii]
                for a, b in zip(values, values[1:]):
                    assert a >= b - 1e-9, (
                        f"width not monotone non-increasing in R for rule_set={rule_set} "
                        f"loa={loa_m} beam={beam_m}: {finite_radii} -> {values}"
                    )


def test_width_at_infinity_equals_straight_width_from_size_channel():
    from channel_designer.rules.rulesets import size_channel

    for rule_set in RULE_SETS:
        for loa_m in LOAS:
            for beam_m in BEAMS:
                width = make_width_fn(loa_m, beam_m, DRAFT_M, rule_set=rule_set)
                straight_inputs = ChannelRuleInputs(
                    beam_m=beam_m, loa_m=loa_m, draft_m=DRAFT_M,
                    min_water_level_m=MIN_WL_M, rule_set=rule_set,
                )
                straight = size_channel(straight_inputs).straight_bottom_width_m
                assert width(float("inf")) == straight


def test_half_width_convenience():
    width = make_width_fn(80.0, 12.0, DRAFT_M, rule_set="pianc")
    for radius_m in RADII:
        assert width.half_width(radius_m) == width(radius_m) / 2.0


def test_width_params_dict_provenance():
    params = width_params_dict(80.0, 12.0, DRAFT_M, rule_set="pianc")
    assert params["rule_set"] == "pianc"
    assert params["loa_m"] == 80.0
    assert params["beam_m"] == 12.0
    assert params["draft_m"] == DRAFT_M
    assert "straight_bottom_m" in params
    assert "widening_coeff_note" in params


def test_demo_pianc_loa80_beam12_radius240():
    # Default PIANC path: fairway = 2.8 x beam (the Report-121 build-up, 3.5 x beam = 42.0,
    # is behind pianc_use_buildup=True).
    width = make_width_fn(80.0, 12.0, DRAFT_M, rule_set="pianc")
    value = width(240.0)
    straight = width(float("inf"))
    assert round(straight, 6) == 33.6          # 2.8 x 12
    expected = straight + 80.0 ** 2 / (8.0 * 240.0)
    assert value == expected
    print(f"width(240) for PIANC LOA 80 beam 12 -> {value:.2f} "
          f"(33.6 + 80**2/(8*240) = {expected:.2f})")


def test_demo_pianc_loa60_beam10():
    # Convoy LOA 60 (incl tug), beam 10 -> 28.0 m straight,
    # R_min 180, widening 60^2/(8R) = +2.5 m at R_min.
    width = make_width_fn(60.0, 10.0, 2.5, rule_set="pianc")
    assert round(width(float("inf")), 6) == 28.0
    assert abs(width(180.0) - 30.5) < 1e-9
