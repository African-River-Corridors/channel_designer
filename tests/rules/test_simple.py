"""PIANC sizing sanity checks on the one-call path."""
from channel_designer.rules.simple import (
    PIANCInputs, size_channel, required_depth, max_draft_for_depth,
)


def test_width_scales_with_beam():
    # Straight width = width_multiple_straight x beam (default 2.5).
    base = size_channel(PIANCInputs(beam_m=12, loa_m=60)).bottom_width_straight_m
    wide = size_channel(PIANCInputs(beam_m=24, loa_m=60)).bottom_width_straight_m
    assert abs(base - 30.0) < 1e-9
    assert abs(wide - 60.0) < 1e-9


def test_bend_widening_and_radius():
    c = size_channel(PIANCInputs(beam_m=12, loa_m=120, radius_multiple=4.0))
    assert c.min_bend_radius_m == 480.0                       # design radius R2 = 4 × LOA
    assert abs(c.bend_widening_m - 120.0 ** 2 / (8.0 * 480.0)) < 1e-9   # LOA²/(8R)
    assert c.bend_widening_m >= 0.0
    assert abs(c.bend_widening_m - (c.bottom_width_bend_m - c.bottom_width_straight_m)) < 1e-9
    assert c.bottom_width_bend_m >= c.bottom_width_straight_m


def test_depth_and_draft_inverse():
    # 1.75 m draft → 2.25 m depth (the 0.5 m minimum UKC dominates)
    assert abs(required_depth(1.75) - 2.25) < 1e-9
    # given a 6 m channel, the allowed laden draft
    d = max_draft_for_depth(6.0)
    assert 5.0 < d < 6.0
    # round-trip within the fractional branch (deep draft)
    assert abs(max_draft_for_depth(required_depth(5.0)) - 5.0) < 1e-6


def test_size_from_depth_scenario():
    c = size_channel(PIANCInputs(beam_m=16, loa_m=100, side_slope_h_per_v=3.0), depth_m=4.0)
    assert c.design_depth_m == 4.0
    # top width = straight bottom (2.5 × 16) + 2 × slope × depth
    assert abs(c.top_width_straight_m - (2.5 * 16 + 2 * 3.0 * 4.0)) < 1e-9
    assert c.passing_place_length_m == 5.0 * 100
