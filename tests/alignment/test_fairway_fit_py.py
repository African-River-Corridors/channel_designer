"""The Python wrapper of the fairway fit (alignment.fairway): synthetic geometry only."""
import math

import numpy as np
import pytest
from shapely.geometry import LineString, Point, box

from channel_designer.alignment.fairway import Pose, fairway_fit
from channel_designer.alignment.radius_check import min_radius_on_chord


def test_property_a_straight_seed_in_open_water_stays_straight():
    r = fairway_fit(LineString([(0, 0), (3000, 0)]), box(-100, -300, 3100, 300), r_min=420.0)
    ys = np.asarray(r.line.coords)[:, 1]
    assert r.radius_ok and not r.infeasible and np.abs(ys).max() < 1.0


def test_property_a_dog_leg_seed_comes_out_smoother_inside_the_water():
    seed = LineString([(0, 0), (1500, 0), (1500, 1500), (3000, 1500)])
    water = seed.buffer(400)
    r = fairway_fit(seed, water, r_min=420.0)
    assert min_radius_on_chord(r.line, 140.0) > min_radius_on_chord(seed, 140.0)
    assert r.measured_min_radius_m >= 420.0 * 0.98 and r.radius_ok
    assert water.buffer(1.0).contains(r.line)


def test_feasibility_a_gap_in_navigable_water_is_reported_not_hidden():
    nav = box(-100, -200, 1000, 200).union(box(1200, -200, 3100, 200))     # a 200 m break
    r = fairway_fit(LineString([(0, 0), (3000, 0)]), nav, r_min=420.0)
    assert any(s.reason == "too_narrow" and s.start_chainage_m < 1250 and s.end_chainage_m > 1050 for s in r.infeasible)


def test_poses_are_met_and_bad_arguments_refused():
    r = fairway_fit(LineString([(0, 0), (3000, 0)]), box(-100, -600, 3100, 600), r_min=420.0,
                    start_pose=Pose((0.0, 0.0), 0.0), end_pose=Pose((3000.0, 0.0), 0.0))
    assert r.start_heading_error_rad < math.radians(1) and r.end_heading_error_rad < math.radians(1)
    with pytest.raises(ValueError):
        fairway_fit(LineString([(0, 0), (3000, 0)]), box(0, -1, 1, 1), r_min=-1.0)
