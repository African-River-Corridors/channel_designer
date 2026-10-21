"""Radius compliance is reported, never assumed (issues #9 and #10).

Both straightening steps can return a design line tighter than R_min. Each result says so:
``radius_ok``, the measured minimum radius, and the chord it was read on.
"""
from __future__ import annotations

import math

import numpy as np
from shapely.geometry import LineString

from owd.alignment import route
from owd.alignment.radius_check import min_radius_on_chord, min_radius_on_vertices
from owd.alignment.rolling_circle import rolling_circle
from owd.synthetic import sine_generated_centreline


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
    from owd.alignment.limits import resample_line
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


def test_golden_chord_reads_the_whole_line_at_the_exact_chord():
    """A kink in the last stride is read (only the backward pass reaches it)."""
    line = LineString([(0.0, 0.0), (1008.0, 0.0), (1009.0, 0.5)])
    assert min_radius_on_chord(line, chord_m=100.0) < math.inf
    # a line too short for the chord has no reading
    assert min_radius_on_chord(LineString([(0, 0), (40, 0), (60, 30)]), chord_m=100.0) == math.inf


def test_feasibility_chord_must_be_finite_and_positive():
    import pytest
    for bad in (0.0, -1.0, math.nan, math.inf):
        with pytest.raises(ValueError):
            min_radius_on_chord(LineString([(0, 0), (1000, 0)]), chord_m=bad)


def test_golden_vertex_radius_of_a_polyline_on_a_circle_is_its_radius():
    """A polyline with every vertex on an R circle reads R, whatever its vertex spacing."""
    for R, ds in ((150.0, 100.0), (200.0, 100.0), (100.0, 50.0)):
        th = np.arange(0.0, 5.0, ds / R)
        pts = np.column_stack([R * np.sin(th), R * (1 - np.cos(th))])
        assert abs(min_radius_on_vertices(pts, 1) - R) < 1e-6
        assert abs(min_radius_on_vertices(pts, 2) - R) < 1e-6


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
    assert res.radius_check_chord_m == 200.0             # read on the chord, R_min / 3 (#95)
    assert res.measured_min_radius_m >= 0.98 * 600.0
    assert res.min_radius_m >= 0.98 * 600.0              # the stations hold it too (enforced)


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



def test_property_route_prefers_a_compliant_corridor():
    """A wider corridor that saves more but fails R_min must not beat one that holds it.
    Straight baseline, so the baseline is not the cause: the 200 m corridor's excursion is."""
    line = LineString([(0.0, 0.0), (4000.0, 0.0)])
    river = line.buffer(20.0, cap_style="flat")
    grid = route.build_cost_grid(line, _hill, river, 0.0, bottom_width=30.0, depth=3.0,
                                 side_slope=3.0, corridor_half=200.0, station_step=50.0)
    kw = dict(bottom_width=30.0, depth=3.0, side_slope=3.0, r_min=900.0, grid=grid)
    narrow = route.optimise_route(line, _hill, river, 0.0, corridor_sweep=(120.0,), **kw)
    wide = route.optimise_route(line, _hill, river, 0.0, corridor_sweep=(200.0,), **kw)
    # The choice reads the stations (the reading the smoothing enforces): the wide corridor's
    # excursion turns tighter than R_min there. Its reported reading, on a 300 m chord, passes.
    from owd.alignment.radius_check import min_radius_on_vertices
    def stations_ok(r):
        return min_radius_on_vertices(np.asarray(r.line.coords), 1) >= 0.98 * 900.0
    assert stations_ok(narrow) and not stations_ok(wide)
    assert narrow.radius_ok and wide.radius_ok
    assert wide.approx_saving_pct > narrow.approx_saving_pct
    res = route.optimise_route(line, _hill, river, 0.0, corridor_sweep=(120.0, 200.0), **kw)
    assert res.radius_ok and res.corridor_half_m == 120.0


def test_property_unmeasured_result_is_not_ok():
    from owd.alignment.rolling_circle import RollingCircleResult
    res = RollingCircleResult(LineString([(0, 0), (1, 0)]), 300.0)
    assert not res.radius_ok and math.isnan(res.measured_min_radius_m)
    assert math.isnan(res.radius_check_chord_m)


def test_feasibility_route_check_chord_is_validated_and_rounds_half_up():
    import pytest
    line = LineString([(0.0, 0.0), (2000.0, 0.0)])
    river = line.buffer(20.0, cap_style="flat")
    kw = dict(bottom_width=30.0, depth=3.0, side_slope=3.0, r_min=600.0,
              corridor_sweep=(40.0,), station_step=50.0)
    for bad in (0.0, -100.0, math.nan, math.inf):
        with pytest.raises(ValueError):
            route.optimise_route(line, _hill, river, 0.0, check_chord_m=bad, **kw)
    res = route.optimise_route(line, _hill, river, 0.0, check_chord_m=250.0, **kw)
    assert res.radius_check_chord_m == 250.0             # reported on the chord asked for (#95)
