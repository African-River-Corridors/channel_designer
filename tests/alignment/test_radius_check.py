"""Radius compliance is reported, never assumed (issues #9 and #10).

Both straightening steps can return a design line tighter than R_min. Each result says so:
``radius_ok``, the measured minimum radius, and the chord it was read on.
"""
from __future__ import annotations

import math

import numpy as np
from shapely.geometry import LineString

from channel_designer.alignment import route
from channel_designer.alignment.radius_check import min_radius_on_chord
from channel_designer.alignment.rolling_circle import rolling_circle
from channel_designer.synthetic import sine_generated_centreline


def _arc(radius, sweep_deg, n=400):
    t = np.linspace(0, math.radians(sweep_deg), n)
    return LineString(np.column_stack([radius * np.sin(t), radius * (1 - np.cos(t))]))


def _jagged_trace(jitter_m=8.0, vertex_m=50.0):
    """A plan trace digitised at ~50 m vertices with a zigzag on top of the meander (#10)."""
    x = np.arange(0.0, 6000.0 + vertex_m, vertex_m)
    y = 260.0 * np.sin(2.0 * math.pi * x / 1500.0)
    y[1:-1] += jitter_m * np.where(np.arange(1, x.size - 1) % 2 == 0, 1.0, -1.0)
    return LineString(np.column_stack([x, y]))


def _independent_min_radius(line, chord_m):
    """Fixed-phase three-point radius with points half a chord apart (the issues' check)."""
    from channel_designer.alignment.limits import resample_line
    return float(np.min(route.radii(resample_line(line, chord_m / 2.0))))


# --- the measurement ----------------------------------------------------------------------

def test_golden_chord_radius_of_an_arc_is_its_radius():
    r = min_radius_on_chord(_arc(500.0, 90.0), chord_m=100.0)
    assert abs(r - 500.0) < 0.5, r


def test_golden_chord_radius_of_a_straight_is_infinite():
    assert min_radius_on_chord(LineString([(0, 0), (1000, 0)]), chord_m=100.0) == math.inf


def test_property_chord_radius_finds_the_sine_apex():
    line = sine_generated_centreline()        # apex radius λ²/(4π²A) ≈ 219 m
    r = min_radius_on_chord(line, chord_m=60.0)
    assert 210.0 < r < 225.0, r


# --- rolling circle (#10) -----------------------------------------------------------------

def test_property_rolling_circle_reports_a_compliant_line():
    res = rolling_circle(sine_generated_centreline(), 300.0, step=10.0)
    assert res.radius_ok
    assert res.radius_check_chord_m == 100.0             # one LOA at R_min = 3 × LOA
    assert res.measured_min_radius_m >= 0.98 * 300.0
    assert res.min_radius_m == 300.0                     # unchanged: the radius requested


def test_property_rolling_circle_flags_a_jagged_input():
    """#10: on a zigzag trace the circle does not resolve the jitter. Say so."""
    line = _jagged_trace()
    res = rolling_circle(line, 360.0, step=10.0, passes=6, check_chord_m=90.0)
    assert res.radius_check_chord_m == 90.0
    assert res.measured_min_radius_m < 360.0
    assert not res.radius_ok
    # the reported figure is the real one, not the target: it agrees with the issue's check
    independent = _independent_min_radius(res.centreline, 90.0)
    assert abs(res.measured_min_radius_m - independent) <= 0.03 * independent


# --- least-cost route (#9) ----------------------------------------------------------------

def _hill(xs, ys):
    xs = np.asarray(xs); ys = np.asarray(ys)
    return 2.0 + 20.0 * np.exp(-((xs - 2000) ** 2 + (ys - 40) ** 2) / (2 * 120.0 ** 2))


def test_property_route_reports_a_compliant_line():
    line = LineString([(0.0, 0.0), (4000.0, 0.0)])
    river = line.buffer(20.0, cap_style="flat")
    res = route.optimise_route(line, _hill, river, 0.0, bottom_width=30.0, depth=3.0,
                               side_slope=3.0, r_min=600.0, corridor_sweep=(40.0, 80.0))
    assert res.radius_ok
    assert res.radius_check_chord_m == 200.0             # 2 × station step beats R_min/3
    assert res.measured_min_radius_m >= 0.98 * 600.0


def test_property_route_does_not_relax_rmin_to_a_tight_baseline():
    """#9: the baseline turns at ~219 m, R_min is 300 m. The old code relaxed the target to the
    baseline's radius, cleared that, and reported 219 m as if it passed."""
    line = sine_generated_centreline()
    river = line.buffer(20.0, cap_style="flat")
    res = route.optimise_route(line, _hill, river, 0.0, bottom_width=30.0, depth=3.0,
                               side_slope=3.0, r_min=300.0, corridor_sweep=(40.0,),
                               station_step=50.0)
    assert res.measured_min_radius_m < 300.0
    assert not res.radius_ok
    # it tried for R_min (every smoothing pass), rather than stopping at the baseline's radius
    assert res.smooth_passes == 600

