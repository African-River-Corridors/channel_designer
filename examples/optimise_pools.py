"""Optional optimiser: weirs and pool levels, then the channel through each pool.

A straight design line across a gently meandering synthetic river, with a hill beside it.
The cascade search chooses weir sites and pool levels; the lattice search re-routes each
pool, holding R_min; the result is re-measured with the cut model. About ten seconds:

    python examples/optimise_pools.py

The same code is the optimiser example in README.md; tests/test_examples.py runs it.
"""
from shapely.geometry import LineString

from channel_designer.optimise import (CascadeConfig, LatticeConfig, SectionSpec,
                                       build_level_table, cascade_levels,
                                       earthworks_reach_cost, optimise_pools, search_cascade)
from channel_designer.synthetic import synthetic_river
from channel_designer.volumes.sections import section_volumes


def main():
    river = synthetic_river(length_m=3000.0, wavelength_m=2400.0, amplitude_m=120.0,
                            hill_at=(1500.0, -60.0), hill_height_m=10.0, hill_radius_m=80.0)
    line = LineString([(0, 0), (3000, 0)])
    depth, half_width, r_min = 3.25, 15.0, 255.0

    cfg = CascadeConfig(candidate_sites_m=[500, 1000, 1500, 2000, 2500], lifts_m=(0.5, 1.0),
                        terminal_m=3000.0, terminal_min_level_m=0.5, min_spacing_m=1000.0)
    table = build_level_table(line, river.terrain, river.polygon, cascade_levels(cfg),
                              depth, half_width, model="b", maxw_m=120.0)
    best = search_cascade(cfg, earthworks_reach_cost(table, wet_weight=1.0, dry_weight=1.5),
                          structure_cost=lambda lift: 1.0e5 + 2.0e5 * lift).best

    spec = SectionSpec(half_width=half_width, side_slope_h_per_v=3.0, dry_weight=1.5,
                       rake_half_m=80.0, rake_step_m=5.0, ds_step_m=10.0)
    lattice = LatticeConfig(r_min_m=r_min, pitch_m=20.0, n_heading=24, radius_multiples=(1, 2, 4))
    out = optimise_pools(line, river.terrain, river.polygon, best.pools(), depth_m=depth,
                         section=spec, cfg=lattice, corridor_half_m=180.0, cell_m=5.0)

    def pool_level(s):
        x = line.interpolate(s).x
        return max([lvl for a, _b, lvl in best.pools() if a <= x], default=0.0)

    results = {}
    for name, l in (("straight", line), ("optimised", out.line)):
        wl = (lambda s, l=l: pool_level(line.project(l.interpolate(s))))
        results[name] = section_volumes(l, river.terrain, river.polygon, wl, depth, half_width,
                                        model="b", maxw_m=120.0)

    print("weirs:", ", ".join(f"{w.chainage_m:.0f} m -> pool {w.pool_level_m:.1f} m"
                              for w in best.weirs))
    for name, v in results.items():
        print(f"{name:>10}: dredge {v.dredge_m3:9,.0f} m3   dry cut {v.excav_m3:9,.0f} m3")
    return best, out, results


if __name__ == "__main__":
    main()
