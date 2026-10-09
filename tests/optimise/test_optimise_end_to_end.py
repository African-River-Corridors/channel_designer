"""End to end: synthetic river + terrain -> weir cascade -> pool-by-pool alignment -> volumes.

The baseline is a straight design line across a gently meandering river, with a hill beside
it. The cascade search picks weir sites and pool levels on the baseline's earthworks; the
lattice search then re-routes each pool, and the result is re-measured with the cut-model
integrator, not with the search's own numbers.
"""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString

from channel_designer.alignment.limits import resample_line
from channel_designer.alignment.route import min_radius_of_line
from channel_designer.optimise import (
    CascadeConfig,
    LatticeConfig,
    SectionSpec,
    build_level_table,
    cascade_levels,
    earthworks_reach_cost,
    optimise_pools,
    search_cascade,
)
from channel_designer.synthetic import synthetic_river
from channel_designer.volumes.sections import section_volumes

DEPTH, HB, R_MIN = 3.25, 15.0, 255.0
WET_W, DRY_W = 1.0, 1.5


def test_synthetic_river_cascade_then_pools_then_volumes():
    river = synthetic_river(length_m=3000.0, wavelength_m=2400.0, amplitude_m=120.0,
                            hill_at=(1500.0, -60.0), hill_height_m=10.0, hill_radius_m=80.0)
    base = LineString([(0, 0), (3000, 0)])

    # 1. weirs and pool levels
    cfg = CascadeConfig(candidate_sites_m=[500, 1000, 1500, 2000, 2500], lifts_m=(0.5, 1.0),
                        terminal_m=3000.0, terminal_min_level_m=0.5, min_spacing_m=1000.0)
    table = build_level_table(base, river.terrain, river.polygon, cascade_levels(cfg), DEPTH, HB,
                              model="b", maxw_m=120.0)
    search = search_cascade(cfg, earthworks_reach_cost(table, WET_W, DRY_W),
                            lambda lift: 1.0e5 + 2.0e5 * lift)
    best = search.best
    assert best is not None and best.weirs
    assert best.weirs[-1].pool_level_m >= cfg.terminal_min_level_m
    pools = best.pools()
    assert pools[0][0] == 0.0 and pools[-1][1] == 3000.0

    # 2. alignment, pool by pool
    spec = SectionSpec(half_width=HB, side_slope_h_per_v=3.0, wet_weight=WET_W, dry_weight=DRY_W,
                       rake_half_m=80.0, rake_step_m=5.0, ds_step_m=10.0)
    lc = LatticeConfig(r_min_m=R_MIN, pitch_m=20.0, n_heading=24, radius_multiples=(1, 2, 4))
    out = optimise_pools(base, river.terrain, river.polygon, pools, depth_m=DEPTH, section=spec,
                         cfg=lc, corridor_half_m=180.0, cell_m=5.0)
    assert out.n_replaced == len(pools)
    for p in out.pools:
        assert p.result.cost < p.baseline_cost
        rmin, _ = min_radius_of_line(resample_line(p.result.line, 50.0))
        assert rmin >= 0.99 * R_MIN, rmin

    # 3. volumes, re-measured on the cut model with each pool's level
    def wl(line):
        def at(s):
            x = line.interpolate(s).x
            level = cfg.base_level_m
            for a, _b, lvl in pools:
                if a <= x:
                    level = lvl
            return level
        return at

    v0 = section_volumes(base, river.terrain, river.polygon, wl(base), DEPTH, HB, model="b",
                         maxw_m=120.0)
    v1 = section_volumes(out.line, river.terrain, river.polygon, wl(out.line), DEPTH, HB,
                         model="b", maxw_m=120.0)
    w0 = WET_W * v0.dredge_m3 + DRY_W * v0.excav_m3
    w1 = WET_W * v1.dredge_m3 + DRY_W * v1.excav_m3
    assert v1.dredge_m3 > v0.dredge_m3          # it moved into the wet river ...
    assert v1.excav_m3 < 0.6 * v0.excav_m3      # ... and out of the dry ground
    assert w1 < 0.75 * w0
