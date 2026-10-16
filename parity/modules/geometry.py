"""geometry: the shapely (GEOS) operations the package uses, one function each. Ported in ticket 24.

The adapter calls shapely directly. Results are compared as ADR 0005 says: line vertices to 1e-9,
polygons normalised (area within 1e-9 relative, every vertex within 1e-6 m of the other shape). A
buffer also returns its exterior ring as plain coordinates (`ring`), checked vertex for vertex: the
GEOS circle and line buffers are built exactly. The points and overlaps where two lines meet are
returned sorted (`_points_and_lines`), since the package reads them as a set.

No case here makes an offset fold back on itself or asks `buffer(0)` to repair a polygon; those need
GEOS's noding and overlay and belong to the general buffer ticket (#43).
"""
from __future__ import annotations

import math

import numpy as np
import shapely
from shapely.geometry import LineString, MultiLineString, MultiPoint, Point, Polygon
from shapely.ops import linemerge, substring

from ..common import Rng, case as _case, rline, rnd

ENTRY_POINT = "geometry"
R = "initial case set (spec 0001, ticket 24)"


def adapter(fn: str, args: dict):
    if fn == "line_length":
        return args["line"].length
    if fn == "interpolate":
        line = args["line"]
        return [line.interpolate(d, normalized=args.get("normalized", False)) for d in args["distances"]]
    if fn == "project":
        line = args["line"]
        return [line.project(Point(p), normalized=args.get("normalized", False)) for p in args["points"]]
    if fn == "substring":
        return substring(args["line"], args["start"], args["end"], normalized=args.get("normalized", False))
    if fn == "distance_points_line":
        return [shapely.distance(Point(p), args["line"]) for p in args["points"]]
    if fn == "distance_points_line_vectorised":
        xs, ys = np.array(args["xs"], dtype=float), np.array(args["ys"], dtype=float)
        return shapely.distance(shapely.points(xs, ys), args["line"])
    if fn == "distance_geoms":
        return args["a"].distance(args["b"])
    if fn == "intersection_points":
        return _points_and_lines(args["a"].intersection(args["b"]))
    if fn == "line_merge":
        return linemerge(MultiLineString(args["lines"]))
    if fn == "offset_curve":
        kw = {"quad_segs": args["quad_segs"]} if "quad_segs" in args else {}
        return args["line"].offset_curve(args["distance"], **kw)
    if fn == "buffer_line":
        kw = {k: args[k] for k in ("cap_style", "quad_segs") if k in args}
        return _with_ring(args["line"].buffer(args["distance"], **kw))
    if fn == "buffer_point":
        kw = {"quad_segs": args["quad_segs"]} if "quad_segs" in args else {}
        return _with_ring(Point(args["x"], args["y"]).buffer(args["distance"], **kw))
    if fn == "polygon_op":
        a, b = args["a"], args["b"]
        return {"intersection": a.intersection, "difference": a.difference, "union": a.union}[args["op"]](b)
    if fn == "polygon_area":
        return args["polygon"].area
    if fn == "contains_xy":
        xs, ys = np.array(args["xs"], dtype=float), np.array(args["ys"], dtype=float)
        return shapely.contains_xy(args["polygon"], xs, ys)
    raise KeyError(f"geometry has no parity function {fn!r}")


def _with_ring(poly):
    """A buffer and its exterior ring as plain coordinates (compared vertex for vertex)."""
    ring = [] if poly.is_empty else [[float(x), float(y)] for x, y in poly.exterior.coords]
    return {"polygon": poly, "ring": ring}


def _key(p):
    """Sort key rounded to 1e-6 m, so a last-bit difference cannot reorder two distinct points."""
    return (round(p[0] * 1e6), round(p[1] * 1e6))


def _points_and_lines(g):
    """Where two lines meet: the points, and the overlaps merged into lines, each run from its
    lower end and sorted."""
    points, parts = [], []

    def walk(h):
        if h.is_empty:
            return
        if isinstance(h, Point):
            points.append([float(h.x), float(h.y)])
        elif isinstance(h, LineString):
            parts.append(h)
        else:
            for sub in h.geoms:
                walk(sub)

    walk(g)
    lines = []
    if parts:
        merged = linemerge(MultiLineString(parts))
        for ln in getattr(merged, "geoms", [merged]):
            cs = [[float(x), float(y)] for x, y in ln.coords]
            lines.append(cs[::-1] if _key(cs[0]) > _key(cs[-1]) else cs)
    return {"points": sorted(points, key=_key), "lines": sorted(lines, key=lambda l: (_key(l[0]), _key(l[-1])))}


def _walk(rng, n, step=(40.0, 90.0), turn=0.5):
    """A smooth-ish random polyline: segments of 40-90 m, each turning by at most `turn` radians."""
    h = rng.u(0.0, 6.28)
    x = y = 0.0
    pts = [(0.0, 0.0)]
    for _ in range(n):
        h += rng.u(-turn, turn)
        s = rng.u(*step)
        x += s * math.cos(h)
        y += s * math.sin(h)
        pts.append((round(x, 3), round(y, 3)))
    return rline(pts)


def _clear_of_itself(ln, gap):
    """No two non-adjacent segments of `ln` come within `gap` of each other."""
    cs = list(ln.coords)
    segs = [LineString(cs[i:i + 2]) for i in range(len(cs) - 1)]
    return all(segs[i].distance(segs[j]) > gap for i in range(len(segs)) for j in range(i + 2, len(segs)))


def _sine(length=1500.0, wl=1000.0, amp=150.0, step=10.0):
    x = np.arange(0.0, length + step, step)
    return rline(np.column_stack([x, amp * np.sin(2 * math.pi * x / wl)]))


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    bent = LineString([(0, 0), (100, 0), (100, 60), (40, 100)])
    straight = LineString([(0, 0), (300, 0)])
    sine = _sine()
    ring = LineString([(0, 0), (80, 0), (80, 80), (0, 80), (0, 0)])
    dup = LineString([(0, 0), (0, 0), (50, 0), (50, 0), (50, 40)])
    lines = {"straight": straight, "bent": bent, "sine": sine, "ring": ring, "duplicate_vertices": dup,
             "two_point": LineString([(0, 0), (30, 40)])}
    # --- length, interpolate, project ---
    for name, ln in lines.items():
        c(f"length_{name}", 1, "line_length", dict(line=ln))
    c("length_empty", 1, "line_length", dict(line=LineString()))
    ds = [-1000.0, -250.0, -1.0, 0.0, 1e-9, 37.5, 100.0, 160.0, 160.0000001, 259.9, 1e9]
    for name in ("bent", "straight", "two_point", "duplicate_vertices"):
        c(f"interpolate_{name}", 1, "interpolate", dict(line=lines[name], distances=ds))
    c("interpolate_sine", 1, "interpolate", dict(line=sine, distances=[0.0, 10.0, 123.456, 800.0, 1500.0, 1700.0, -300.0]))
    c("interpolate_normalized", 1, "interpolate", dict(line=bent, distances=[-0.5, 0.0, 0.25, 0.5, 0.999, 1.0, 2.0], normalized=True))
    pts = [[-20.0, 5.0], [0.0, 0.0], [50.0, 30.0], [100.0, 10.0], [130.0, 55.0], [90.0, 90.0], [20.0, 120.0], [300.0, -40.0]]
    for name in ("bent", "straight", "sine", "ring", "duplicate_vertices"):
        c(f"project_{name}", 1, "project", dict(line=lines[name], points=pts))
    c("project_normalized", 1, "project", dict(line=bent, points=pts, normalized=True))
    c("project_onto_the_line_itself", 1, "project", dict(line=bent, points=[list(p) for p in bent.coords]))
    c("project_ring_start_and_end", 1, "project", dict(line=ring, points=[[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [-3.0, 40.0]]))
    # --- substring (shapely.ops) ---
    for tag, s, e in [("forward", 30.0, 200.0), ("reversed", 200.0, 30.0), ("equal", 50.0, 50.0), ("from_end", -80.0, -10.0),
                      ("mixed_signs", 20.0, -50.0), ("start_negative_end_positive", -60.0, 120.0), ("clamped", -1e9, 1e9),
                      ("past_end", 400.0, 500.0), ("both_before_start", -500.0, -400.0), ("meet_in_the_middle", -100.0, 150.0),
                      ("whole", 0.0, 1e9), ("one_vertex_to_next", 100.0, 160.0), ("inside_one_segment", 10.0, 90.0)]:
        c(f"substring_{tag}", 1, "substring", dict(line=bent, start=s, end=e))
    for tag, s, e in [("quarter_to_three_quarters", 0.25, 0.75), ("reversed", 0.9, 0.1), ("negative", -0.5, -0.1), ("past_end", 1.5, 2.0)]:
        c(f"substring_normalized_{tag}", 1, "substring", dict(line=sine, start=s, end=e, normalized=True))
    c("substring_sine_middle", 1, "substring", dict(line=sine, start=200.0, end=1000.0))
    # --- distance ---
    for name in ("bent", "sine", "ring"):
        c(f"distance_points_{name}", 1, "distance_points_line", dict(line=lines[name], points=pts))
    xs = [float(x) for x in range(-50, 1600, 70)]
    c("distance_vectorised_sine", 1, "distance_points_line_vectorised",
      dict(line=sine, xs=xs, ys=[40.0 * math.cos(x / 130.0) for x in xs]))
    c("distance_vectorised_empty", 1, "distance_points_line_vectorised", dict(line=bent, xs=[], ys=[]))
    c("distance_line_line", 1, "distance_geoms", dict(a=LineString([(0, 0), (10, 0)]), b=LineString([(5, 7), (20, 9)])))
    c("distance_line_line_crossing", 1, "distance_geoms", dict(a=LineString([(0, 0), (10, 10)]), b=LineString([(0, 10), (10, 0)])))
    c("distance_line_line_endpoint_to_interior", 1, "distance_geoms", dict(a=bent, b=LineString([(150, 70), (120, 30)])))
    c("distance_point_polygon_outside", 1, "distance_geoms", dict(a=Point(20, 0), b=Polygon([(0, 0), (10, 0), (10, 10), (0, 10)])))
    c("distance_point_polygon_inside_is_zero", 1, "distance_geoms", dict(a=Point(5, 5), b=Polygon([(0, 0), (10, 0), (10, 10), (0, 10)])))
    # --- intersection of lines (the points the package picks from) ---
    X = LineString([(0, 10), (20, -10)])
    c("x_crossing", 1, "intersection_points", dict(a=LineString([(0, 0), (20, 0)]), b=X))
    c("t_junction", 1, "intersection_points", dict(a=LineString([(0, 0), (20, 0)]), b=LineString([(10, 0), (10, 15)])))
    c("end_to_end", 1, "intersection_points", dict(a=LineString([(0, 0), (10, 0)]), b=LineString([(10, 0), (10, 15)])))
    c("disjoint", 1, "intersection_points", dict(a=LineString([(0, 0), (10, 0)]), b=LineString([(0, 5), (10, 5)])))
    c("collinear_overlap", 1, "intersection_points", dict(a=LineString([(0, 0), (20, 0)]), b=LineString([(10, 0), (30, 0)])))
    c("sine_with_horizontal", 1, "intersection_points", dict(a=sine, b=LineString([(-10, 20), (1600, 20)])))
    c("sine_with_sine", 1, "intersection_points", dict(a=sine, b=_sine(1500.0, 700.0, 120.0)))
    c("cross_line_with_bank", 1, "intersection_points", dict(a=LineString([(500, 100), (500, -100)]), b=sine))
    circle = Point(100, 50).buffer(60.0, quad_segs=128)
    c("circle_boundary_with_line", 1, "intersection_points", dict(a=circle.boundary, b=bent))
    c("circle_boundary_with_straight", 1, "intersection_points", dict(a=circle.boundary, b=LineString([(0, 0), (300, 0)])))
    c("circle_boundary_misses", 1, "intersection_points", dict(a=circle.boundary, b=LineString([(0, 200), (300, 200)])))
    # --- line merge ---
    a, b2, c3 = LineString([(0, 0), (10, 0)]), LineString([(10, 0), (20, 5)]), LineString([(20, 5), (30, 5)])
    c("merge_two_in_order", 1, "line_merge", dict(lines=[a, b2]))
    c("merge_chain_shuffled", 1, "line_merge", dict(lines=[c3, a, b2]))
    c("merge_one_reversed", 1, "line_merge", dict(lines=[a, LineString([(20, 5), (10, 0)]), c3]))
    c("merge_two_disjoint", 1, "line_merge", dict(lines=[a, c3]))
    c("merge_t_junction_stays_apart", 1, "line_merge", dict(lines=[a, b2, LineString([(10, 0), (10, -9)])]))
    c("merge_closed_loop", 1, "line_merge", dict(lines=[LineString([(0, 0), (10, 0), (10, 10)]), LineString([(10, 10), (0, 10), (0, 0)])]))
    c("merge_single", 1, "line_merge", dict(lines=[bent]))
    c("merge_repeated_points_dropped", 1, "line_merge", dict(lines=[LineString([(0, 0), (0, 0), (10, 0)]), LineString([(10, 0), (10, 0), (20, 5)])]))
    c("merge_two_chains_and_a_loop", 1, "line_merge", dict(lines=[
        LineString([(0, 0), (5, 0)]), LineString([(5, 0), (9, 3)]), LineString([(50, 50), (60, 50)]),
        LineString([(60, 50), (60, 60)]), LineString([(100, 0), (110, 0), (110, 10), (100, 0)])]))
    c("merge_chain_split_at_a_crossing_node", 1, "line_merge", dict(lines=[
        LineString([(0, 0), (10, 0)]), LineString([(10, 0), (20, 0)]), LineString([(10, -10), (10, 0)]),
        LineString([(10, 0), (10, 10)])]))
    # --- offset curve (a line whose offsets do not fold) ---
    for side, d in (("left", 20.0), ("right", -20.0)):
        c(f"offset_sine_{side}", 1, "offset_curve", dict(line=sine, distance=d))
        c(f"offset_bent_{side}", 1, "offset_curve", dict(line=bent, distance=d / 4))
        c(f"offset_straight_{side}", 1, "offset_curve", dict(line=straight, distance=d))
    c("offset_two_point_line", 1, "offset_curve", dict(line=LineString([(0, 0), (30, 40)]), distance=7.0))
    c("offset_two_point_line_right", 1, "offset_curve", dict(line=LineString([(0, 0), (30, 40)]), distance=-7.0))
    c("offset_zero_is_the_line", 1, "offset_curve", dict(line=bent, distance=0.0))
    c("offset_coarse_quadrant_segments", 1, "offset_curve", dict(line=bent, distance=6.0, quad_segs=1))
    c("offset_fine_quadrant_segments", 1, "offset_curve", dict(line=bent, distance=6.0, quad_segs=32))
    c("offset_duplicate_vertices", 1, "offset_curve", dict(line=dup, distance=5.0))
    c("offset_empty_line", 1, "offset_curve", dict(line=LineString(), distance=5.0))
    c("offset_with_shallow_vertices_simplified", 1, "offset_curve",
      dict(line=rline([(0, 0), (50, 0.05), (100, 0), (150, -0.04), (200, 0)]), distance=30.0))
    # --- buffer of a line ---
    for cap in ("round", "flat", "square"):
        c(f"buffer_bent_{cap}", 1, "buffer_line", dict(line=bent, distance=8.0, cap_style=cap))
        c(f"buffer_sine_{cap}", 1, "buffer_line", dict(line=sine, distance=20.0, cap_style=cap))
        c(f"buffer_straight_{cap}", 1, "buffer_line", dict(line=straight, distance=12.0, cap_style=cap))
    c("buffer_default_style_is_round", 1, "buffer_line", dict(line=bent, distance=8.0))
    c("buffer_synthetic_river_polygon", 1, "buffer_line", dict(line=_sine(2000.0, 1500.0, 260.0), distance=20.0, cap_style="flat"))
    c("buffer_quad_segs_1", 1, "buffer_line", dict(line=bent, distance=8.0, quad_segs=1))
    c("buffer_quad_segs_16", 1, "buffer_line", dict(line=bent, distance=8.0, quad_segs=16))
    c("buffer_two_point_line", 1, "buffer_line", dict(line=LineString([(0, 0), (30, 40)]), distance=9.0))
    c("buffer_collinear_vertices", 1, "buffer_line", dict(line=LineString([(0, 0), (50, 0), (120, 0), (200, 0)]), distance=9.0))
    c("buffer_zigzag_corners", 1, "buffer_line", dict(line=LineString([(0, 0), (60, 20), (120, -10), (180, 25)]), distance=8.0))
    c("buffer_sharp_turn_outside_fillet", 1, "buffer_line", dict(line=LineString([(0, 0), (100, 0), (60, 35)]), distance=5.0))
    c("buffer_zero_distance_is_empty", 1, "buffer_line", dict(line=bent, distance=0.0))
    c("buffer_negative_distance_is_empty", 1, "buffer_line", dict(line=bent, distance=-5.0))
    # --- buffer of a point (the rolling circle's 512-gon) ---
    for qs in (128, 8, 1, 2, 5, 64):
        c(f"circle_quad_segs_{qs}", 1, "buffer_point", dict(x=100.0, y=50.0, distance=60.0, quad_segs=qs))
    c("circle_default_quad_segs", 1, "buffer_point", dict(x=0.0, y=0.0, distance=1.0))
    c("circle_offset_centre", 1, "buffer_point", dict(x=7431.25, y=-1294.5, distance=300.0, quad_segs=128))
    c("circle_tiny_radius", 1, "buffer_point", dict(x=5.0, y=5.0, distance=1e-6, quad_segs=128))
    c("circle_zero_radius_is_empty", 1, "buffer_point", dict(x=5.0, y=5.0, distance=0.0, quad_segs=128))
    # --- polygon overlay and containment ---
    sq = Polygon([(0, 0), (100, 0), (100, 100), (0, 100)])
    sq2 = Polygon([(50, 50), (150, 50), (150, 150), (50, 150)])
    holey = Polygon([(0, 0), (100, 0), (100, 100), (0, 100)], [[(20, 20), (40, 20), (40, 40), (20, 40)]])
    strip = Polygon([(-10, 40), (110, 40), (110, 60), (-10, 60)])
    for op in ("intersection", "difference", "union"):
        c(f"overlap_{op}", 1, "polygon_op", dict(op=op, a=sq, b=sq2))
        c(f"disjoint_{op}", 1, "polygon_op", dict(op=op, a=sq, b=Polygon([(200, 0), (260, 0), (260, 60)])))
        c(f"hole_with_strip_{op}", 1, "polygon_op", dict(op=op, a=holey, b=strip))
        c(f"contained_{op}", 1, "polygon_op", dict(op=op, a=sq, b=Polygon([(10, 10), (30, 10), (30, 30), (10, 30)])))
        c(f"touching_edge_{op}", 1, "polygon_op", dict(op=op, a=sq, b=Polygon([(100, 0), (160, 0), (160, 100), (100, 100)])))
    c("difference_splits_into_two", 1, "polygon_op", dict(op="difference", a=sq, b=Polygon([(-10, 40), (110, 40), (110, 60), (-10, 60)])))
    c("circle_minus_circle", 1, "polygon_op", dict(op="difference", a=Point(0, 0).buffer(50.0, quad_segs=32),
                                                    b=Point(30, 0).buffer(40.0, quad_segs=32)))
    c("buffered_river_minus_channel", 1, "polygon_op", dict(
        op="difference", a=sine.buffer(60.0, cap_style="flat"), b=sine.buffer(30.0, cap_style="flat")))
    c("area_square", 1, "polygon_area", dict(polygon=sq))
    c("area_with_hole", 1, "polygon_area", dict(polygon=holey))
    c("area_circle", 1, "polygon_area", dict(polygon=Point(0, 0).buffer(100.0, quad_segs=128)))
    gx = [float(v) for v in range(-10, 111, 10)]
    gy = [float(v) for v in range(-10, 111, 10)]
    X2, Y2 = np.meshgrid(gx, gy)
    c("contains_grid_square_boundary_excluded", 1, "contains_xy", dict(polygon=sq, xs=X2.ravel(), ys=Y2.ravel()))
    c("contains_grid_with_hole", 1, "contains_xy", dict(polygon=holey, xs=X2.ravel(), ys=Y2.ravel()))
    c("contains_circle", 1, "contains_xy", dict(polygon=Point(50, 50).buffer(40.0, quad_segs=128),
                                                  xs=[50.0, 89.9, 90.1, 50.0, 20.0, 80.0], ys=[50.0, 50.0, 50.0, 10.1, 79.0, 21.0]))
    c("contains_none", 1, "contains_xy", dict(polygon=sq, xs=[], ys=[]))
    c("contains_river", 1, "contains_xy", dict(polygon=sine.buffer(20.0, cap_style="flat"),
                                                 xs=[100.0, 250.0, 500.0, 750.0, 1000.0], ys=[0.0, 150.0, 5.0, -150.0, 18.0]))
    # --- tier 2: seeded sweep ---
    rng = Rng(24001)
    for i in range(14):
        ln = _walk(rng, rng.i(3, 9))
        c(f"sweep_{i:02d}_length", 2, "line_length", dict(line=ln))
        L = ln.length
        c(f"sweep_{i:02d}_interpolate", 2, "interpolate", dict(line=ln, distances=[rng.r3(-L * 1.2, L * 1.2) for _ in range(6)]))
        c(f"sweep_{i:02d}_project", 2, "project", dict(line=ln, points=[[rng.r3(-100, 400), rng.r3(-200, 200)] for _ in range(6)]))
        c(f"sweep_{i:02d}_substring", 2, "substring", dict(line=ln, start=rng.r3(-L, L), end=rng.r3(-L, L)))
        d = rng.r3(3.0, 8.0)
        c(f"sweep_{i:02d}_offset_left", 2, "offset_curve", dict(line=ln, distance=d, quad_segs=[8, 4, 16][i % 3]))
        c(f"sweep_{i:02d}_offset_right", 2, "offset_curve", dict(line=ln, distance=-d, quad_segs=[8, 4, 16][i % 3]))
        c(f"sweep_{i:02d}_buffer", 2, "buffer_line", dict(line=ln, distance=d, cap_style=["round", "flat", "square"][i % 3],
                                                           quad_segs=[8, 4, 16][(i + 1) % 3]))
        c(f"sweep_{i:02d}_distance", 2, "distance_geoms", dict(a=ln, b=_walk(rng, 3)))
        c(f"sweep_{i:02d}_intersection", 2, "intersection_points", dict(a=ln, b=_walk(rng, 4)))
    for i in range(10):
        c(f"circle_sweep_{i:02d}", 2, "buffer_point", dict(x=rng.r3(-1000, 1000), y=rng.r3(-1000, 1000),
                                                          distance=rng.r3(0.5, 500.0), quad_segs=[128, 8, 3, 17, 40][i % 5]))
    for i in range(8):
        k = rng.i(3, 6)
        def poly():
            cx, cy = rng.u(0, 50), rng.u(0, 50)
            ang = sorted(rng.u(0, 6.28) for _ in range(k))
            rs = [rng.u(30, 80) for _ in range(k)]
            return Polygon([(round(cx + r * math.cos(a), 3), round(cy + r * math.sin(a), 3)) for a, r in zip(ang, rs)])
        pa, pb = poly(), poly()
        if not (pa.is_valid and pb.is_valid):
            pa, pb = sq, sq2
        for op in ("intersection", "difference", "union"):
            c(f"polygon_sweep_{i:02d}_{op}", 2, "polygon_op", dict(op=op, a=pa, b=pb))
        c(f"polygon_sweep_{i:02d}_contains", 2, "contains_xy", dict(
            polygon=pa, xs=[rng.r3(-80, 130) for _ in range(12)], ys=[rng.r3(-80, 130) for _ in range(12)]))
    for i in range(6):
        ls = [_walk(rng, rng.i(1, 3)) for _ in range(rng.i(2, 5))]
        # chain some of them end to start so that merging has something to do
        chained, last = [], None
        for ln in ls:
            if last is not None:
                shift = np.array(last) - np.array(ln.coords[0])
                ln = rline(np.array(ln.coords) + shift)
            chained.append(ln)
            last = list(ln.coords)[-1] if rng.p(0.7) else (rng.u(-300, 300), rng.u(-300, 300))
        c(f"merge_sweep_{i:02d}", 2, "line_merge", dict(lines=chained))
    # --- tier 2: the GEOS probe (offset curves, buffers and circles, vertex for vertex) ---
    # Walks are kept only if nothing folds: a simple line whose non-adjacent segments stay more than
    # 2.5 offsets apart. A folding offset is ticket 43's.
    rng = Rng(24002)
    kept = 0
    while kept < 20:
        ln = _walk(rng, rng.i(3, 9))
        d = rng.r3(2.0, 9.0)
        if not _clear_of_itself(ln, 2.5 * d):
            continue
        q = [4, 8, 16, 128][kept % 4]
        c(f"probe_walk_{kept:02d}_offset_left", 2, "offset_curve", dict(line=ln, distance=d, quad_segs=q))
        c(f"probe_walk_{kept:02d}_offset_right", 2, "offset_curve", dict(line=ln, distance=-d, quad_segs=q))
        c(f"probe_walk_{kept:02d}_buffer", 2, "buffer_line", dict(line=ln, distance=d, cap_style=["round", "flat", "square"][kept % 3],
                                                                  quad_segs=q))
        kept += 1
    for name, ln in (("sine_default", _sine(6000.0, 1500.0, 260.0)), ("sine_short", _sine(500.0, 300.0, 40.0, 7.5)),
                     ("sine_tight", _sine(1800.0, 700.0, 120.0))):
        for d in (20.0, -20.0, 5.0, -5.0):
            for q in (8, 128):
                c(f"probe_{name}_offset_{d:+g}_q{q}", 2, "offset_curve", dict(line=ln, distance=d, quad_segs=q))
        for cap in ("round", "flat", "square"):
            c(f"probe_{name}_buffer_{cap}", 2, "buffer_line", dict(line=ln, distance=20.0, cap_style=cap))
    for i in range(6):
        c(f"probe_circle_{i:02d}", 2, "buffer_point", dict(x=rng.r3(-8000, 8000), y=rng.r3(-8000, 8000),
                                                          distance=rng.r3(1e-3, 900.0), quad_segs=[128, 3, 17, 128, 8, 64][i]))
    # Tier 3: this module has none. Every refusal GEOS or shapely makes carries their own wording, which is
    # not ours to reproduce.
    return out
