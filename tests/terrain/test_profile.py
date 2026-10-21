"""terrain.profile: long profiles over a terrain sampler. Synthetic terrain only — no DEM."""
from __future__ import annotations

import math

import numpy as np
import pytest
from shapely.geometry import LineString

from owd.terrain import line_profile


def plane(a=2.0, bx=0.01, by=0.0):
    return lambda xs, ys: a + bx * np.asarray(xs) + by * np.asarray(ys)


# --- golden -------------------------------------------------------------------------------

def test_plane_profile_is_exact():
    # z = 2 + 0.01 x along y = 0: at chainage s the level is 2 + 0.01 s (worked by hand).
    p = line_profile(LineString([(0, 0), (1000, 0)]), plane(), step_m=100.0)
    assert np.allclose(p.chainage_m, np.arange(0, 1001, 100.0))
    assert np.allclose(p.z, 2.0 + 0.01 * p.chainage_m)
    assert p.min_z == pytest.approx(2.0) and p.max_z == pytest.approx(12.0)
    assert p.ascent_m == pytest.approx(10.0) and p.descent_m == pytest.approx(0.0)


def test_valley_ascent_and_descent():
    # V-shaped valley z = |x - 500| / 50: down 10 m then up 10 m.
    p = line_profile(LineString([(0, 0), (1000, 0)]), lambda xs, ys: np.abs(np.asarray(xs) - 500.0) / 50.0, 50.0)
    assert p.descent_m == pytest.approx(10.0) and p.ascent_m == pytest.approx(10.0)
    assert p.min_z == pytest.approx(0.0) and p.at(500.0) == pytest.approx(0.0)


# --- properties -----------------------------------------------------------------------------

def test_chainage_includes_both_ends_and_is_increasing():
    line = LineString([(0, 0), (300, 400), (300, 1037)])          # length 500 + 637 = 1137 m
    p = line_profile(line, plane(), step_m=100.0)
    assert p.chainage_m[0] == 0.0 and p.length_m == pytest.approx(line.length)
    assert np.all(np.diff(p.chainage_m) > 0)
    assert math.hypot(p.x[-1] - 300, p.y[-1] - 1037) < 1e-9


def test_profile_follows_a_bend():
    # On a sampler that depends on y only, the profile reads y along the line.
    line = LineString([(0, 0), (0, 100), (100, 100)])
    p = line_profile(line, lambda xs, ys: np.asarray(ys, dtype=float), step_m=10.0)
    assert p.at(50.0) == pytest.approx(50.0) and p.at(150.0) == pytest.approx(100.0)


# --- feasibility: the checks fire ------------------------------------------------------------

def test_nodata_is_counted_and_skipped():
    def holes(xs, ys):
        z = np.asarray(xs, dtype=float) / 100.0
        z[(np.asarray(xs) > 300) & (np.asarray(xs) < 600)] = np.nan
        return z
    p = line_profile(LineString([(0, 0), (1000, 0)]), holes, step_m=100.0)
    assert p.n_nodata == 2
    assert p.ascent_m == pytest.approx(10.0)       # rises across the gap still count, NaN skipped


@pytest.mark.parametrize("step", [0.0, -5.0])
def test_bad_step_is_refused(step):
    with pytest.raises(ValueError):
        line_profile(LineString([(0, 0), (10, 0)]), plane(), step_m=step)


def test_wrong_sampler_shape_is_refused():
    with pytest.raises(ValueError):
        line_profile(LineString([(0, 0), (100, 0)]), lambda xs, ys: np.zeros(3), step_m=10.0)
