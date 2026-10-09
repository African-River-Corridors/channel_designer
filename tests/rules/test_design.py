"""Channel design — golden, property and feasibility checks.

Golden values follow this worked example:
    total_loa = 160 m, beam = 28 m, draft = 1.75 m  -> design depth 2.25 m
    Params: width_multiple 2.8, radius_multiple 3, side_slope 4,
                    extra_width_coefficient 0.6, max_extra_width_radius 2000.
"""
import math

import numpy as np

from channel_designer.rules.design import (
    ChannelDesignInputs,
    ChannelDesignParams,
    compute,
    compute_scenario_grid,
    extra_width,
    min_bend_radius,
    resolve_design_depth,
    required_depth,
)


def _params(**kw):
    base = dict(width_multiple=2.8, radius_multiple=3.0, side_slope=4.0,
                draft_m=1.75, extra_width_coefficient=0.6, max_extra_width_radius=2000.0,
                radius_sample_length=10.0)
    base.update(kw)
    return ChannelDesignParams(**base)


def _arc(radius, sweep_deg=90.0, spacing=10.0):
    """A circular arc of given radius — a synthetic centreline for curvature tests."""
    sweep = math.radians(sweep_deg)
    n = max(int(radius * sweep / spacing) + 1, 5)
    theta = np.linspace(0.0, sweep, n)
    return np.column_stack((radius * np.cos(theta), radius * np.sin(theta)))


def _line(length=4000.0, spacing=10.0):
    n = int(length / spacing) + 1
    x = np.linspace(0.0, length, n)
    return np.column_stack((x, np.zeros_like(x)))


# ---------- golden: exact worked-example values ----------

def test_golden_design_depth():
    p = _params()
    # UKC = max(0.5, 0.15 x 1.75) = 0.5 -> depth = 2.25
    assert abs(resolve_design_depth(p) - 2.25) < 1e-9
    assert abs(required_depth(1.75) - 2.25) < 1e-9


def test_golden_straight_section():
    # Straight bottom width = width_multiple x beam = 2.8 x 28 = 78.4 m.
    p = _params()
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, params=p))
    assert abs(r.base_width_m - 78.4) < 1e-9
    assert abs(float(r.top_width_m[0]) - (78.4 + 2.0 * 4.0 * 2.25)) < 1e-9


def test_golden_bend_r600():
    p = _params()
    # band edges [480, 860, 1240, 1620, 2000]; R=600 -> band 0 -> 0.6 x 160^2 / 480
    e = extra_width(600.0, 160.0, p)
    assert abs(e - 32.0) < 1e-9
    bottom = p.width_multiple * 28.0 + e
    top = bottom + 2.0 * p.side_slope * resolve_design_depth(p)
    assert abs(bottom - 110.4) < 1e-9
    assert abs(top - 128.4) < 1e-9


def test_golden_tight_bend_below_rmin():
    p = _params()
    assert abs(min_bend_radius(160.0, p) - 480.0) < 1e-9
    # R = 400 < R_min = 480 -> not navigable
    assert 400.0 < min_bend_radius(160.0, p)


def test_golden_compute_straight_channel():
    # No centreline -> straight channel; widths match width_multiple x beam.
    p = _params()
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, params=p))
    base = p.width_multiple * 28.0
    top = base + 2.0 * p.side_slope * resolve_design_depth(p)
    assert abs(r.base_width_m - base) < 1e-6
    assert abs(float(r.bottom_width_m[0]) - base) < 1e-6
    assert abs(float(r.top_width_m[0]) - top) < 1e-6
    assert abs(float(r.half_offset_m[0]) - top / 2.0) < 1e-6
    assert r.tight_bend_count == 0
    assert r.ok


# ---------- property: invariants that must always hold ----------

def test_wider_beam_wider_channel():
    p = _params()
    narrow = compute(ChannelDesignInputs(total_loa=160.0, beam=14.0, centreline_points=_arc(1500.0), params=p))
    wide = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, centreline_points=_arc(1500.0), params=p))
    assert wide.base_width_m > narrow.base_width_m
    assert float(wide.bottom_width_m.mean()) > float(narrow.bottom_width_m.mean())
    assert float(wide.top_width_m.max()) > float(narrow.top_width_m.max())


def test_longer_loa_wider_bends_monotonic():
    p = _params()
    # Fixed in-band radius: more LOA -> more bend widening (~linear in LOA).
    widths = [extra_width(1000.0, loa, p) for loa in (100.0, 140.0, 180.0, 220.0)]
    assert all(b > a for a, b in zip(widths, widths[1:]))


def test_extra_width_zero_above_threshold():
    p = _params()
    assert extra_width(2000.0, 160.0, p) == 0.0
    assert extra_width(5000.0, 160.0, p) == 0.0
    assert extra_width(float("inf"), 160.0, p) == 0.0


def test_top_width_never_below_bottom():
    p = _params()
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, centreline_points=_arc(800.0), params=p))
    assert np.all(np.asarray(r.top_width_m) >= np.asarray(r.bottom_width_m) - 1e-9)


def test_curvature_recovers_arc_radius():
    # The smoothed radius along a clean arc should be ~ its true radius.
    p = _params(edge_margin_m=0.0)
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, centreline_points=_arc(1500.0, sweep_deg=120.0), params=p))
    mid = np.asarray(r.smoothed_radius_m)
    finite = mid[np.isfinite(mid)]
    assert abs(float(np.median(finite)) - 1500.0) < 50.0


def test_grid_reuses_curvature_consistently():
    p = _params()
    grid = compute_scenario_grid(_arc(1000.0), [120.0, 160.0], [16.0, 28.0], params=p)
    assert set(grid["scenarios"]) == {"loa120_beam16", "loa120_beam28",
                                      "loa160_beam16", "loa160_beam28"}
    # Same LOA profile + bigger beam -> wider base width.
    s_small = grid["scenarios"]["loa160_beam16"]
    s_big = grid["scenarios"]["loa160_beam28"]
    assert s_big["base_width_m"] > s_small["base_width_m"]
    assert abs(grid["design_depth_m"] - 2.25) < 1e-9


# ---------- feasibility: checks fire when they should ----------

def test_tight_bend_check_fires():
    p = _params()
    # R_min for 160 m = 480 m; a 300 m arc is far below it.
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, centreline_points=_arc(300.0), params=p))
    assert r.tight_bend_count > 0
    assert not r.ok
    bends = next(c for c in r.checks if c.name == "bends_navigable")
    assert not bends.ok


def test_banded_extra_width_applied_along_arc():
    # A 600 m arc for a 160 m convoy sits in band 0: 0.6 x 160^2 / 480 = 32 m extra, away from
    # the ends (edge margin).
    p = _params()
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0,
                                    centreline_points=_arc(600.0, sweep_deg=120.0), params=p))
    extra = np.asarray(r.extra_width_m)
    assert abs(float(extra.max()) - 32.0) < 1e-9
    assert float(extra[0]) == 0.0                     # inside the edge margin


def test_cut_through_indicated_for_sharp_bend():
    # A very tight bend vs a long convoy under a width method whose bend width grows as
    # LOA^2/R: the width exceeds the LOA, so a cut-through is indicated (the
    # bend_width_within_loa check fires).
    p = _params(width_method="pianc")
    r = compute(ChannelDesignInputs(total_loa=200.0, beam=6.0, centreline_points=_arc(20.0), params=p))
    within = next(c for c in r.checks if c.name == "bend_width_within_loa")
    assert float(np.max(np.asarray(r.bottom_width_m))) > 200.0
    assert not within.ok


def test_depth_check_fires_when_override_too_shallow():
    p = _params(design_depth_m=1.5)   # below required 2.25 for 1.75 m draft
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, params=p))
    depth_chk = next(c for c in r.checks if c.name == "depth_sufficient")
    assert not depth_chk.ok


def test_clean_arc_passes_all_checks():
    p = _params()
    r = compute(ChannelDesignInputs(total_loa=160.0, beam=28.0, centreline_points=_arc(1500.0), params=p))
    assert r.ok
    assert r.tight_bend_count == 0
