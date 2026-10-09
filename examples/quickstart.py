"""Quickstart: a synthetic meandering river -> channel rules -> alignment -> volumes.

Runs in a few seconds with the core install (no DEM, no data files):

    python examples/quickstart.py

The same code is the worked example in README.md; tests/test_examples.py runs it.
"""
from channel_designer.alignment.meanders import find_meanders
from channel_designer.alignment.rolling_circle import rolling_circle
from channel_designer.alignment.route import optimise_route
from channel_designer.rules import channel_shape
from channel_designer.synthetic import synthetic_river
from channel_designer.volumes.cut_model import widened_hb_from_shape
from channel_designer.volumes.sections import section_volumes


def main():
    # 1. A river: centreline, banks, wetted polygon and a terrain sampler, built in code.
    river = synthetic_river()

    # 2. Rules: a named standard plus a vessel class give the channel shape.
    shape = channel_shape("PIANC-B", "CEMT_IV", draught_m=2.5)
    print(f"{shape.ruleset_id} x {shape.vessel_standard_id}: bottom "
          f"{shape.bottom_width_straight_m:.1f} m, R_min {shape.min_bend_radius_m:.0f} m, "
          f"depth {shape.design_depth_m:.2f} m")

    # 3. Alignment: find the bends tighter than R_min, then straighten them.
    tight = find_meanders(river.centreline, max_radius=shape.min_bend_radius_m)
    rolled = rolling_circle(river.centreline, shape.min_bend_radius_m, step=10.0)
    print(f"{len(tight)} bends tighter than R_min; {rolled.n_arcs} arcs spliced in")

    # 4. Least-cost route: shift the line sideways where that cuts less ground.
    route = optimise_route(
        rolled.centreline, river.terrain, river.polygon,
        grade=lambda s: river.water_level(s) - shape.design_depth_m,
        bottom_width=shape.bottom_width_straight_m, depth=shape.design_depth_m,
        side_slope=3.0, r_min=shape.min_bend_radius_m, corridor_sweep=(40, 80, 120))

    # 5. Volumes: Cut Model B along both lines, with bend widening from the ruleset.
    def hb(xy):
        return widened_hb_from_shape(xy, shape)

    results = {}
    for name, line in (("rolling circle", rolled.centreline), ("least-cost", route.line)):
        v = section_volumes(line, river.terrain, river.polygon, river.water_level,
                            shape.design_depth_m, hb, model="b", maxw_m=150.0)
        results[name] = v
        print(f"{name:>14}: dredge {v.dredge_m3:>9,.0f} m3   dry cut {v.excav_m3:>9,.0f} m3"
              f"   tallest face {v.max_face_h_m:4.1f} m")
    return shape, rolled, route, results


if __name__ == "__main__":
    main()
