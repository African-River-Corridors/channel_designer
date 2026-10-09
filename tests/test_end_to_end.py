"""End to end on a synthetic meandering river: rules -> alignment -> volumes.

No data files: the river, banks and terrain are generated in code.
"""
from __future__ import annotations

import numpy as np

from channel_designer.alignment.bends import classify_bends
from channel_designer.alignment.centreline import build_centreline
from channel_designer.alignment.limits import resample_line
from channel_designer.alignment.meanders import find_meanders
from channel_designer.alignment.rolling_circle import rolling_circle
from channel_designer.alignment.route import min_radius_of_line, optimise_route
from channel_designer.rules import channel_shape
from channel_designer.synthetic import synthetic_river
from channel_designer.volumes.cut_model import widened_hb_from_shape
from channel_designer.volumes.footprint import channel_footprint
from channel_designer.volumes.sections import section_volumes


def test_synthetic_river_rules_alignment_volumes():
    river = synthetic_river()

    # rules
    shape = channel_shape("PIANC-B", "CEMT_IV", draught_m=2.5)
    r_min, depth = shape.min_bend_radius_m, shape.design_depth_m
    assert r_min == 3.0 * shape.loa_m and abs(depth - 1.3 * 2.5) < 1e-12

    # alignment: rebuild the centreline from the banks, find and classify the tight bends
    centre = build_centreline(river.left_bank, river.right_bank, resample_step=20.0)
    tight = find_meanders(centre, max_radius=r_min)
    assert len(tight) == 8 and all(m["radius"] < r_min for m in tight)
    top = shape.bottom_width_straight_m + 2 * 3.0 * depth
    cat = classify_bends(centre, river.polygon, r_min, top, shape.bottom_width_straight_m)
    assert cat["summary"]["n_bends"] >= 8

    rolled = rolling_circle(centre, r_min, step=10.0)
    assert rolled.n_arcs >= 8
    assert min_radius_of_line(resample_line(rolled.centreline, 50.0))[0] >= 0.95 * r_min

    route = optimise_route(rolled.centreline, river.terrain, river.polygon,
                           grade=lambda s: river.water_level(s) - depth,
                           bottom_width=shape.bottom_width_straight_m, depth=depth,
                           side_slope=3.0, r_min=r_min, corridor_sweep=(40, 80, 120))
    assert route.min_radius_m >= 0.98 * r_min
    assert route.approx_saving_pct > 0.0

    # volumes: the least-cost line must cut less than the line it started from
    def hb(xy):
        return widened_hb_from_shape(xy, shape)

    before = section_volumes(rolled.centreline, river.terrain, river.polygon, river.water_level,
                             depth, hb, model="b", maxw_m=150.0)
    after = section_volumes(route.line, river.terrain, river.polygon, river.water_level,
                            depth, hb, model="b", maxw_m=150.0)
    assert before.dredge_m3 > 0 and before.excav_m3 > 0
    assert after.total_m3 < before.total_m3
    # bend widening is applied: some station is wider than the straight bottom
    assert max(s.hb for s in before.stations) > shape.bottom_width_straight_m / 2 + 1e-6

    # plan footprint outside the present banks is positive and finite
    fp = channel_footprint(route.line, river.left_bank, river.right_bank, loa=shape.loa_m,
                           beam=shape.beam_m, depth=depth)
    assert np.isfinite(fp.total_excavation_area_m2) and fp.total_excavation_area_m2 > 0
