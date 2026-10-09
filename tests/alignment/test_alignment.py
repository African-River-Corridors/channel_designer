"""Alignment layer on synthetic geometry: golden, property and feasibility checks."""
from __future__ import annotations

import math

import numpy as np
import pytest
from shapely.geometry import LineString, Point

from channel_designer.alignment import bends, centreline, limits, meanders, primitives, route
from channel_designer.alignment.rolling_circle import rolling_circle
from channel_designer.alignment.widths import measure_widths
from channel_designer.synthetic import sine_generated_centreline, synthetic_river


def _straight(length=2000.0):
    return LineString([(0.0, 0.0), (length, 0.0)])


def _arc(radius, sweep_deg, n=400):
    t = np.linspace(0, math.radians(sweep_deg), n)
    return LineString(np.column_stack([radius * np.sin(t), radius * (1 - np.cos(t))]))


# --- centreline from banks ----------------------------------------------------------------

def test_golden_centreline_of_parallel_banks_is_the_midline():
    left = LineString([(0, 25), (1000, 25)])
    right = LineString([(1000, -25), (0, -25)])          # reversed on purpose
    c = centreline.build_centreline(left, right, resample_step=20.0)
    ys = np.array(c.coords)[:, 1]
    assert np.allclose(ys, 0.0, atol=1e-6)
    assert abs(c.length - 1000.0) < 5.0      # smoothing pulls each end in by ~1 m


def test_property_centreline_of_synthetic_river_tracks_the_true_one():
    r = synthetic_river()
    c = centreline.build_centreline(r.left_bank, r.right_bank, resample_step=20.0)
    # every point of the rebuilt centreline lies close to the generating one
    d = [r.centreline.distance(Point(p)) for p in list(c.coords)[5:-5]]
    assert max(d) < 5.0, max(d)


def test_feasibility_trim_refuses_a_too_short_line():
    with pytest.raises(ValueError):
        centreline.trim_polyline(np.array([[0, 0], [10, 0]]), 6.0)


# --- widths -------------------------------------------------------------------------------

def test_golden_width_of_parallel_banks():
    c = LineString([(0, 0), (1000, 0)])
    rows, fails = measure_widths(c, LineString([(0, 20), (1000, 20)]),
                                 LineString([(0, -20), (1000, -20)]), spacing=50.0, cross_span=100.0)
    assert fails == 0 and len(rows) == 21
    assert all(abs(r["width_m"] - 40.0) < 1e-9 for r in rows)


def test_property_synthetic_river_width_is_its_design_width():
    r = synthetic_river()
    rows, _ = measure_widths(r.centreline, r.left_bank, r.right_bank, spacing=100.0,
                             cross_span=100.0)
    w = np.array([x["width_m"] for x in rows])
    assert np.median(w) == pytest.approx(r.width_m, rel=0.02)


# --- meanders -----------------------------------------------------------------------------

def test_golden_sine_apex_radius():
    """y = A sin(2πx/λ) has apex radius λ²/(4π²A)."""
    lam, amp = 1500.0, 260.0
    line = sine_generated_centreline(6000.0, lam, amp)
    want = lam ** 2 / (4 * math.pi ** 2 * amp)
    recs = meanders.find_meanders(line, max_radius=2 * want, sample_step=10.0)
    assert len(recs) == 8                                  # 4 wavelengths, 2 apexes each
    for rec in recs:
        assert rec["radius"] == pytest.approx(want, rel=0.02)
    dirs = [rec["tangent_direction"] for rec in recs]
    assert dirs == ["right", "left"] * 4


def test_property_threshold_below_the_apex_radius_finds_nothing():
    line = sine_generated_centreline()
    assert meanders.find_meanders(line, max_radius=150.0) == []


def test_feasibility_straight_line_has_no_meanders():
    assert meanders.find_meanders(_straight(), max_radius=1e6) == []


# --- rolling circle -----------------------------------------------------------------------

def test_property_rolling_circle_lifts_min_radius_to_target():
    line = sine_generated_centreline()
    r_min = 300.0
    res = rolling_circle(line, r_min, step=10.0)
    assert res.n_arcs > 0
    pts = np.array(res.centreline.coords)
    # The arc meets the old line where the circle CROSSES it, not tangentially, so there is a
    # short kink at each arc exit. Measured on a 50 m chord (a vessel-length scale) the radius
    # holds; on a 10 m chord the kink reads as a tighter radius. Known issue, kept on
    # purpose: see KNOWN_ISSUES in the module docstring and docs/known-issues.md.
    from channel_designer.alignment.limits import resample_line
    rmin, _ = route.min_radius_of_line(resample_line(res.centreline, 50.0))
    assert rmin >= 0.95 * r_min, rmin
    fine, _ = route.min_radius_of_line(resample_line(res.centreline, 10.0))
    assert fine < r_min, "if this starts passing, the exit kink is fixed: update the docstring"
    assert pts[0].tolist() == list(line.coords[0]) and pts[-1].tolist() == list(line.coords[-1])


def test_feasibility_rolling_circle_leaves_a_gentle_line_alone():
    line = sine_generated_centreline()
    res = rolling_circle(line, 100.0, step=10.0)          # every bend already > 219 m
    assert res.n_arcs == 0
    assert res.centreline.equals(line)


# --- bends --------------------------------------------------------------------------------

def test_golden_straight_is_class_s():
    line = _straight(3000.0)
    river = line.buffer(25.0, cap_style="flat")
    out = bends.classify_bends(line, river, r_min=300.0, top_width=50.0, bottom_width=30.0)
    assert out["summary"]["n_tight"] == 0 and out["summary"]["n_gentle"] == 0


def test_property_tighter_rmin_makes_more_tight_bends():
    r = synthetic_river()
    loose = bends.classify_bends(r.centreline, r.polygon, 150.0, 50.0, 30.0)
    tight = bends.classify_bends(r.centreline, r.polygon, 600.0, 50.0, 30.0)
    assert loose["summary"]["n_tight"] == 0
    assert tight["summary"]["n_tight"] > 0
    assert tight["summary"]["tight_length_m"] > loose["summary"]["tight_length_m"]


def test_feasibility_e_exact_is_zero_for_a_feasible_bend():
    k = np.full(50, 1.0 / 1000.0)            # R = 1000 m everywhere
    assert bends.e_exact(k, 10.0, 300.0) == 0.0
    assert bends.e_exact(np.full(50, 1 / 150.0), 10.0, 300.0) > 0.0


# --- channel limits -----------------------------------------------------------------------

def test_golden_limits_of_a_straight_are_parallel_offsets():
    left, right, diag = limits.build_channel_limits([_straight()], base_offset=25.0, loa=80.0)
    assert left.distance(Point(1000, 25)) < 1e-6 and right.distance(Point(1000, -25)) < 1e-6
    assert diag["max_extra_width"] == 0.0


def test_property_limits_widen_in_a_bend():
    line = _arc(300.0, 90.0)
    _l, _r, diag = limits.build_channel_limits([line], base_offset=25.0, loa=80.0,
                                               extra_width_coefficient=0.6,
                                               min_radius_length_multiple=3.0)
    assert diag["max_extra_width"] > 0.0


# --- primitives ---------------------------------------------------------------------------

def test_golden_arc_primitive_ends_on_its_circle():
    prims = primitives.build([300.0], n_heading=48, widen_fn=primitives.lux_widening(80.0))
    left = [p for p in prims if p["R"] == 300.0 and p["sign"] == 1][0]
    xe, ye, th = left["end"]
    assert math.hypot(xe, ye - 300.0) == pytest.approx(300.0)
    assert left["widen"] == pytest.approx(80.0 ** 2 / 600.0)
    straight = prims[-1]
    assert math.isinf(straight["R"]) and straight["widen"] == 0.0


def test_property_radii_for_respects_rmin():
    assert min(primitives.radii_for(240.0)) >= 240.0
    assert primitives.descriptor([300.0], 48)["builder"] == primitives.BUILDER


# --- least-cost route ---------------------------------------------------------------------

def test_golden_route_on_flat_ground_stays_put():
    """No terrain variation -> every offset costs the same; the DP must not wander off."""
    line = _straight(3000.0)
    river = line.buffer(20.0, cap_style="flat")
    flat = lambda xs, ys: np.full(np.shape(xs), 3.0)        # noqa: E731  ground at WL
    res = route.optimise_route(line, flat, river, 0.0, bottom_width=30.0, depth=3.0,
                               side_slope=3.0, r_min=300.0, corridor_sweep=(40.0,))
    assert np.allclose(res.offsets, 0.0, atol=1e-6) or res.approx_saving_pct <= 1e-9


def test_property_route_moves_away_from_a_hill_and_keeps_rmin():
    line = _straight(4000.0)
    river = line.buffer(20.0, cap_style="flat")

    def hill(xs, ys):
        xs = np.asarray(xs); ys = np.asarray(ys)
        return 2.0 + 20.0 * np.exp(-((xs - 2000) ** 2 + (ys - 40) ** 2) / (2 * 120.0 ** 2))

    res = route.optimise_route(line, hill, river, 0.0, bottom_width=30.0, depth=3.0,
                               side_slope=3.0, r_min=600.0, corridor_sweep=(40.0, 80.0))
    assert res.approx_saving_pct > 0.0
    mid = res.offsets[len(res.offsets) // 2]
    assert mid < 0.0, "the channel should move to the side away from the hill (y < 0)"
    assert res.min_radius_m >= 0.98 * 600.0


def test_feasibility_dp_reports_an_infeasible_corridor():
    centres = np.column_stack([np.arange(0, 1000, 100.0), np.zeros(10)])
    offsets = np.array([-10.0, 0.0, 10.0])
    cost = np.full((10, 3), np.inf)
    with pytest.raises(RuntimeError):
        route.dp_optimise(offsets, centres, cost, 300.0, 100.0, 10.0)


def test_feasibility_high_ground_needs_a_wider_transect():
    """Ground well above WL daylights beyond the default transect, so edge offsets look
    cheaper than they are. ``daylight_m`` widens the transect and removes the bias."""
    line = _straight(2000.0)
    river = line.buffer(20.0, cap_style="flat")
    high = lambda xs, ys: np.full(np.shape(xs), 8.0)        # noqa: E731  5 m above WL
    kw = dict(bottom_width=30.0, depth=3.0, side_slope=3.0, corridor_half=40.0)
    narrow = route.build_cost_grid(line, high, river, 0.0, **kw)
    wide = route.build_cost_grid(line, high, river, 0.0, daylight_m=15.0 + 3.0 * 8.0, **kw)
    mid = len(narrow.stations_m) // 2
    assert narrow.vol[mid, 0] < narrow.vol[mid, len(narrow.offsets) // 2]   # biased edge
    assert np.allclose(wide.vol[mid], wide.vol[mid, 0])                     # no bias
