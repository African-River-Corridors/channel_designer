"""No-cut zones on the least-cost route (alignment.route, ``no_cut``).

Synthetic: a straight baseline along +x (left = +y) in a river 40 m wide (|y| < 20). Dry ground
is 1 m high on the left bank and 4 m on the right, so an unconstrained route moves left. The
template (bottom 20 m, 1:2 sides, invert -1 m) meets 1 m ground 10 + 2 x 2 = 14 m from its
centre, so with the left bank protected the widest clear offset is 20 - 14 = +6 m.
"""
import numpy as np
import pytest
from shapely.geometry import LineString, box

from owd.alignment.route import NoCutZone, build_cost_grid, optimise_route

LINE = LineString([(0, 0), (1000, 0)])
RIVER = box(-50, -20, 1050, 20)
KW = dict(bottom_width=20.0, depth=2.0, side_slope=2.0, station_step=100.0, offset_step=2.0,
          transect_step=1.0, daylight_m=30.0)


def terrain(xs, ys):
    return np.where(np.asarray(ys) < 0, 4.0, 1.0)


def route(no_cut=None):
    return optimise_route(LINE, terrain, RIVER, -1.0, r_min=300.0, corridor_sweep=(10, 40),
                          no_cut=no_cut, **KW)


def test_golden_a_protected_left_bank_stops_the_line_at_plus_6_m():
    free = route()
    assert free.no_cut_ok and free.protected_cut_m3 == 0.0
    assert free.offsets.max() > 6.0                       # property: unconstrained, it goes left
    held = route([NoCutZone(0.0, 1000.0, "left")])
    assert held.no_cut_ok and held.protected_cut_m3 == 0.0
    assert held.offsets.max() == pytest.approx(6.0)       # golden: 20 m bank - 14 m half top width


def test_property_no_zones_leave_the_grid_unchanged():
    g = build_cost_grid(LINE, terrain, RIVER, -1.0, corridor_half=40.0, **KW)
    assert g.protected is None
    z = build_cost_grid(LINE, terrain, RIVER, -1.0, corridor_half=40.0,
                        no_cut=[NoCutZone(300.0, 500.0, "right")], **KW)
    np.testing.assert_array_equal(g.vol, z.vol)
    blocked = z.protected > 0
    assert blocked[3:6, 0].all() and not blocked[6, 0] and not blocked[:, -1].any()


def test_feasibility_a_zone_with_no_clear_offset_fails_loudly():
    narrow = box(-50, -5, 1050, 5)
    with pytest.raises(RuntimeError, match="no-cut zone"):
        optimise_route(LINE, lambda x, y: np.ones_like(np.asarray(x, float)), narrow, -1.0,
                       r_min=300.0, corridor_sweep=(10, 40), **KW,
                       no_cut=[NoCutZone(0, 1000, "left"), NoCutZone(0, 1000, "right")])
    with pytest.raises(ValueError):
        NoCutZone(0, 1, "north")


def test_property_the_bank_is_read_on_the_river_not_the_baseline():
    # A re-route on the right-bank land (y = -100), with ground on both of its sides: all of it is
    # the river's RIGHT bank. Read on the river centreline, a left-bank zone protects none of it;
    # read on the baseline (the default), the ground left of the baseline would look like it.
    on_land = LineString([(0, -100), (1000, -100)])
    centre = LineString([(-50, 0), (1050, 0)])
    kw = dict(corridor_half=20.0, **KW)
    left = [NoCutZone(0.0, 1100.0, "left")]
    g = build_cost_grid(on_land, terrain, RIVER, -1.0, no_cut=left, bank_line=centre, **kw)
    assert (g.protected == 0).all()
    g = build_cost_grid(on_land, terrain, RIVER, -1.0, no_cut=[NoCutZone(0.0, 1100.0, "right")],
                        bank_line=centre, **kw)
    assert (g.protected > 0).all()
    g = build_cost_grid(on_land, terrain, RIVER, -1.0, no_cut=[NoCutZone(0.0, 1000.0, "left")], **kw)
    assert (g.protected[:, -1] > 0).all() and (g.protected[:, 0] == 0).all()
