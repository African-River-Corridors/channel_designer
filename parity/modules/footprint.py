"""footprint: `volumes/footprint.py` (the excavation footprint in plan).

Polygon results are compared normalised (ADR 0005): the ring may start at another vertex."""
from __future__ import annotations

import numpy as np
from shapely.geometry import (GeometryCollection, LineString, MultiLineString, MultiPolygon, Point,
                              Polygon)

from ..common import Rng, case as _case, frozen, rline, synth

ENTRY_POINT = "footprint"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    fp = frozen("volumes.footprint")
    if fn == "orient_like":
        return fp.orient_like(args["reference"], args["candidate"])
    if fn == "polygon_from_bank_pair":
        return fp.polygon_from_bank_pair(args["left"], args["right"])
    if fn == "iter_polygons":
        return fp.iter_polygons(args["geom"])
    if fn == "subtract_polygons":
        kw = {"min_area": args["min_area"]} if "min_area" in args else {}
        return fp.subtract_polygons(args["channel_poly"], args["river_poly"], **kw)
    if fn == "classify_sides":
        return fp.classify_sides(args["polygons"], args["left_line"], args["right_line"])
    if fn == "compute_excavation":
        return fp.compute_excavation(**args)
    if fn == "channel_footprint":
        res = fp.channel_footprint(args["centreline"], args["bank_left"], args["bank_right"],
                                   **{k: v for k, v in args.items()
                                      if k not in ("centreline", "bank_left", "bank_right")})
        return {"footprint": res, "total_excavation_area_m2": res.total_excavation_area_m2}
    raise KeyError(f"footprint has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    a = LineString([(0, 0), (100, 0)])
    c("orient_same", 1, "orient_like", dict(reference=a, candidate=LineString([(1, 5), (99, 5)])))
    c("orient_flips", 1, "orient_like", dict(reference=a, candidate=LineString([(99, 5), (1, 5)])))
    c("orient_tie_keeps", 1, "orient_like", dict(reference=a, candidate=LineString([(50, 5), (50, -5)])))
    left = LineString([(0, 20), (500, 20)])
    right = LineString([(0, -20), (500, -20)])
    c("pair_polygon", 1, "polygon_from_bank_pair", dict(left=left, right=LineString(list(right.coords)[::-1])))
    c("pair_polygon_closed_already", 1, "polygon_from_bank_pair",
      dict(left=LineString([(0, 0), (10, 0), (10, 10)]), right=LineString([(0, 0), (0, 10), (10, 10)])))
    c("pair_polygon_self_touching_is_repaired", 1, "polygon_from_bank_pair",
      dict(left=LineString([(0, 0), (100, 100)]), right=LineString([(0, 100), (100, 0)])))
    c("pair_polygon_empty_refused", 3, "polygon_from_bank_pair",
      dict(left=LineString([(0, 0), (10, 0)]), right=LineString([(0, 0), (10, 0)])))
    sq = Polygon([(0, 0), (10, 0), (10, 10), (0, 10)])
    c("iter_polygon", 1, "iter_polygons", dict(geom=sq))
    c("iter_multipolygon", 1, "iter_polygons",
      dict(geom=MultiPolygon([sq, Polygon([(20, 20), (30, 20), (30, 30)])])))
    c("iter_line_is_not_a_polygon", 1, "iter_polygons", dict(geom=LineString([(0, 0), (1, 1)])))
    c("iter_empty", 1, "iter_polygons", dict(geom=Polygon()))
    big = Polygon([(0, -40), (500, -40), (500, 40), (0, 40)])
    river = Polygon([(0, -20), (500, -20), (500, 20), (0, 20)])
    c("subtract_two_strips", 1, "subtract_polygons", dict(channel_poly=big, river_poly=river))
    c("subtract_min_area_drops_slivers", 1, "subtract_polygons",
      dict(channel_poly=Polygon([(0, -20.01), (500, -20.01), (500, 20.01), (0, 20.01)]), river_poly=river, min_area=20.0))
    c("subtract_nothing_left", 1, "subtract_polygons", dict(channel_poly=river, river_poly=big))
    c("subtract_negative_min_area", 1, "subtract_polygons", dict(channel_poly=big, river_poly=river, min_area=-1.0))
    p_left = Polygon([(0, 20), (500, 20), (500, 40), (0, 40)])
    p_right = Polygon([(0, -40), (500, -40), (500, -20), (0, -20)])
    c("sides", 1, "classify_sides", dict(polygons=[p_left, p_right], left_line=LineString([(0, 40), (500, 40)]),
                                         right_line=LineString([(0, -40), (500, -40)])))
    c("sides_equidistant_is_left", 1, "classify_sides", dict(
        polygons=[Polygon([(0, -5), (10, -5), (10, 5), (0, 5)])], left_line=LineString([(0, 10), (10, 10)]),
        right_line=LineString([(0, -10), (10, -10)])))
    c("excavation_wider_channel", 1, "compute_excavation", dict(
        channel_left=LineString([(0, 40), (500, 40)]), channel_right=LineString([(0, -40), (500, -40)]),
        bank_left=left, bank_right=right))
    c("excavation_reversed_lines", 1, "compute_excavation", dict(
        channel_left=LineString([(0, 40), (500, 40)]), channel_right=LineString([(500, -40), (0, -40)]),
        bank_left=left, bank_right=LineString(list(right.coords)[::-1])))
    c("excavation_channel_inside_river", 1, "compute_excavation", dict(
        channel_left=LineString([(0, 10), (500, 10)]), channel_right=LineString([(0, -10), (500, -10)]),
        bank_left=left, bank_right=right))
    c("excavation_one_sided", 1, "compute_excavation", dict(
        channel_left=LineString([(0, 40), (500, 40)]), channel_right=LineString([(0, -10), (500, -10)]),
        bank_left=left, bank_right=right))
    rv = synth(length_m=2000.0)
    kw = dict(loa=85.0, beam=9.5, depth=3.0)
    c("footprint_straight", 1, "channel_footprint", dict(
        centreline=LineString([(0, 0), (1500, 0)]), bank_left=LineString([(0, 20), (1500, 20)]),
        bank_right=LineString([(0, -20), (1500, -20)]), **kw))
    c("footprint_synthetic", 1, "channel_footprint", dict(
        centreline=rv["centreline"], bank_left=rv["left_bank"], bank_right=rv["right_bank"], **kw))
    c("footprint_synthetic_bigger_vessel", 1, "channel_footprint", dict(
        centreline=rv["centreline"], bank_left=rv["left_bank"], bank_right=rv["right_bank"],
        loa=110.0, beam=11.4, depth=3.5, side_slope=2.5, width_beam_multiple=3.2,
        min_radius_length_multiple=4.0, extra_width_coefficient=0.5, radius_sample_length=15.0,
        max_extra_width_radius=1500.0, min_area=5.0))
    c("footprint_multilinestring_centreline", 1, "channel_footprint", dict(
        centreline=MultiLineString([LineString([(0, 0), (500, 0)]), LineString([(500, 0), (900, 80)])]),
        bank_left=LineString([(0, 20), (500, 20), (900, 100)]), bank_right=LineString([(0, -20), (500, -20), (900, 60)]),
        **kw))
    c("footprint_no_line_refused", 3, "channel_footprint", dict(
        centreline=Point(0, 0), bank_left=left, bank_right=right, **kw))
    rng = Rng(23019)
    for i in range(6):
        rv = synth(length_m=rng.r3(1000.0, 1800.0), wavelength_m=rng.r3(600.0, 1500.0), amplitude_m=rng.r3(60.0, 260.0),
                   width_m=rng.r3(25.0, 70.0))
        c(f"footprint_sweep_{i:02d}", 2, "channel_footprint", dict(
            centreline=rv["centreline"], bank_left=rv["left_bank"], bank_right=rv["right_bank"],
            loa=rng.r3(50.0, 140.0), beam=rng.r3(7.0, 16.0), depth=rng.r3(2.0, 4.5), side_slope=rng.r3(2.0, 4.0),
            width_beam_multiple=rng.r3(2.4, 3.6), min_radius_length_multiple=rng.r3(2.0, 5.0),
            extra_width_coefficient=rng.r3(0.2, 0.9), radius_sample_length=rng.r3(5.0, 25.0),
            max_extra_width_radius=rng.r3(800.0, 3000.0), min_area=rng.r3(0.0, 20.0)))
    return out
