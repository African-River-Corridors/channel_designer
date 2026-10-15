"""rolling_circle: `alignment/rolling_circle.py` (rolling-circle meander straightening).

The first search-class entry point. Its one discrete choice is the circle contact: of every point
where the circle touches the line again, the FURTHEST forward is taken. `search_meta` records the
closest call: the smallest gap between the chosen contact's chainage and the runner-up's.
"""
from __future__ import annotations

import math

import numpy as np
from shapely.geometry import LineString, MultiLineString, Point

from ..common import Rng, case as _case, frozen, rline, rnd, search_meta, synth

ENTRY_POINT = "rolling_circle"
R = "initial case set (spec 0001, ticket 23)"
SEARCH_FNS = {"rolling_circle", "find_forward_circle_contact"}


def adapter(fn: str, args: dict):
    rc = frozen("alignment.rolling_circle")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "primary_line":
        return rc.primary_line(args["geom"])
    if fn == "resample_coordinate_sequence":
        return rc.resample_coordinate_sequence(arr("points"), args["step"])
    if fn == "resample_linestring":
        return rc.resample_linestring(args["line"], args["step"])
    if fn == "cumulative_chainages":
        return rc.cumulative_chainages(arr("points"))
    if fn == "compute_unit_tangent":
        return rc.compute_unit_tangent(arr("points"), args["index"])
    if fn == "place_circle_at_point":
        centre, ang = rc.place_circle_at_point(arr("points"), args["index"], args["radius"], args["side"])
        return {"centre": centre, "tangent_angle": ang}
    if fn == "create_arc_between_points":
        return rc.create_arc_between_points(arr("centre"), args["radius"], arr("start_point"),
                                            arr("end_point"), args["side"], args.get("num_segments", 32))
    if fn == "interpolate_point_on_line":
        return rc.interpolate_point_on_line(arr("coords"), arr("chainages"), args["distance"])
    if fn == "replace_segment_with_arc":
        return rc.replace_segment_with_arc(args["line"], args["start_distance"], args["end_distance"],
                                           arr("arc_points"))
    if fn == "find_forward_circle_contact":
        r = rc.find_forward_circle_contact(args["centreline"], arr("tangent_point"), args["tangent_chainage"],
                                           arr("centre"), args["radius"], args["min_chainage_gap"])
        if r is None:
            return None
        return {"contact_point": r[0], "centre": r[1], "tangent_point": r[2], "contact_chainage": r[3]}
    if fn == "rolling_circle":
        res = rc.rolling_circle(**args)
        return {"result": res, "n_arcs": res.n_arcs}
    raise KeyError(f"rolling_circle has no parity function {fn!r}")


def _candidate_chainages(rc, centreline, centre, radius, tangent_chainage, gap):
    """The chainages `find_forward_circle_contact` chooses between (a copy of its filter)."""
    circle = Point(float(centre[0]), float(centre[1])).buffer(radius, quad_segs=128)
    inter = circle.boundary.intersection(centreline)
    if inter.is_empty:
        return []
    return sorted((float(centreline.project(p)) for p in rc._gather_points(inter)
                   if float(centreline.project(p)) - tangent_chainage > gap), reverse=True)


def search_meta_for(fn: str, args: dict) -> dict:
    """Run the search with the contact choice instrumented; return the closest call."""
    from ..common import from_json
    rc = frozen("alignment.rolling_circle")
    args = from_json(args)
    seen = []
    orig = rc.find_forward_circle_contact

    def wrapped(centreline, tangent_point, tangent_chainage, centre, radius, min_chainage_gap):
        c = _candidate_chainages(rc, centreline, centre, radius, tangent_chainage, min_chainage_gap)
        if len(c) >= 1:
            seen.append((c[0], c[1] if len(c) > 1 else None))
        return orig(centreline, tangent_point, tangent_chainage, centre, radius, min_chainage_gap)

    rc.find_forward_circle_contact = wrapped
    try:
        adapter(fn, args)
    finally:
        rc.find_forward_circle_contact = orig
    pairs = [(b, r) for b, r in seen if r is not None]
    if not pairs:
        return search_meta(seen[0][0] if seen else None, None, len(seen), "contact chainage, m")
    best, runner = min(pairs, key=lambda p: abs(p[0] - p[1]) / max(abs(p[0]), 1.0))
    return search_meta(best, runner, len(seen), "contact chainage, m")


search_meta_fn = search_meta_for


def sine_line(length=3000.0, wl=1000.0, amp=200.0, step=10.0):
    x = np.arange(0.0, length + step, step)
    return rline(np.column_stack([x, amp * np.sin(2 * np.pi * x / wl)]))


def jagged(jitter=8.0, vertex=50.0, length=3000.0):
    x = np.arange(0.0, length + vertex, vertex)
    y = 260.0 * np.sin(2.0 * np.pi * x / 1500.0)
    y[1:-1] += jitter * np.where(np.arange(1, x.size - 1) % 2 == 0, 1.0, -1.0)
    return rline(np.column_stack([x, y]))


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    pts = np.array([[0.0, 0.0], [10.0, 0.0], [20.0, 5.0], [30.0, 15.0], [35.0, 30.0]])
    # --- exact-class pieces ---
    c("line", 1, "primary_line", dict(geom=LineString([(0, 0), (1, 1)])))
    c("multi_contiguous_merges", 1, "primary_line",
      dict(geom=MultiLineString([LineString([(0, 0), (5, 0)]), LineString([(5, 0), (9, 3)])])))
    c("multi_disjoint_takes_longest", 1, "primary_line",
      dict(geom=MultiLineString([LineString([(0, 0), (5, 0)]), LineString([(50, 0), (90, 30)])])))
    c("not_a_line", 3, "primary_line", dict(geom=Point(1, 1)))
    c("resample_sequence", 1, "resample_coordinate_sequence", dict(points=pts, step=8.0))
    c("resample_sequence_one_point", 3, "resample_coordinate_sequence", dict(points=[[0.0, 0.0]], step=8.0))
    c("resample_sequence_bad_step", 3, "resample_coordinate_sequence", dict(points=pts, step=0.0))
    c("resample_sequence_zero_length", 3, "resample_coordinate_sequence",
      dict(points=[[1.0, 1.0], [1.0, 1.0]], step=1.0))
    c("resample_linestring", 1, "resample_linestring", dict(line=LineString(pts.tolist()), step=11.0))
    c("chainages", 1, "cumulative_chainages", dict(points=pts))
    c("chainages_empty", 1, "cumulative_chainages", dict(points=np.zeros((0, 2))))
    c("chainages_one", 1, "cumulative_chainages", dict(points=[[3.0, 4.0]]))
    for i in (-1, 0, 2, 4, 9):
        c(f"tangent_{i}", 1, "compute_unit_tangent", dict(points=pts, index=i))
    c("tangent_one_point", 1, "compute_unit_tangent", dict(points=[[0.0, 0.0]], index=0))
    c("tangent_duplicate_points", 1, "compute_unit_tangent", dict(points=[[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]], index=1))
    for side in ("left", "right"):
        c(f"circle_{side}", 1, "place_circle_at_point", dict(points=pts, index=2, radius=100.0, side=side))
    c("circle_index_out_of_range", 3, "place_circle_at_point", dict(points=pts, index=9, radius=100.0, side="left"))
    c("circle_bad_side", 3, "place_circle_at_point", dict(points=pts, index=2, radius=100.0, side="up"))
    c("circle_bad_radius", 3, "place_circle_at_point", dict(points=pts, index=2, radius=0.0, side="left"))
    c("circle_no_tangent", 3, "place_circle_at_point",
      dict(points=[[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]], index=1, radius=10.0, side="left"))
    for side in ("left", "right"):
        c(f"arc_{side}", 1, "create_arc_between_points", dict(
            centre=[0.0, 100.0], radius=100.0, start_point=[0.0, 0.0], end_point=[100.0, 100.0], side=side))
        c(f"arc_{side}_wraps", 1, "create_arc_between_points", dict(
            centre=[0.0, 0.0], radius=50.0, start_point=[50.0, 0.0], end_point=[0.0, -50.0], side=side,
            num_segments=8))
    ch = np.array([0.0, 10.0, 22.0, 40.0, 41.0])
    cc = np.array([[0.0, 0.0], [10.0, 0.0], [20.0, 7.0], [30.0, 20.0], [31.0, 21.0]])
    for d in (-5.0, 0.0, 5.0, 10.0, 31.0, 40.5, 41.0, 100.0):
        c(f"interpolate_{d:g}", 1, "interpolate_point_on_line", dict(coords=cc, chainages=ch, distance=d))
    c("interpolate_empty", 3, "interpolate_point_on_line", dict(coords=np.zeros((0, 2)), chainages=[0.0], distance=1.0))
    line = LineString([(0, 0), (100, 0), (200, 30), (300, 30)])
    arc = [[100.0, 0.0], [130.0, 8.0], [160.0, 12.0], [190.0, 20.0], [200.0, 30.0]]
    c("splice_arc", 1, "replace_segment_with_arc", dict(line=line, start_distance=100.0, end_distance=200.0, arc_points=arc))
    c("splice_between_vertices", 1, "replace_segment_with_arc",
      dict(line=line, start_distance=40.0, end_distance=250.0, arc_points=[[40.0, 0.0], [120.0, 40.0], [250.0, 30.0]]))
    c("splice_empty_interval_is_noop", 1, "replace_segment_with_arc",
      dict(line=line, start_distance=50.0, end_distance=50.0, arc_points=arc))
    c("splice_one_arc_point_is_noop", 1, "replace_segment_with_arc",
      dict(line=line, start_distance=50.0, end_distance=150.0, arc_points=[[1.0, 1.0]]))
    c("splice_clipped_to_line", 1, "replace_segment_with_arc",
      dict(line=line, start_distance=-20.0, end_distance=500.0, arc_points=[[0.0, 0.0], [150.0, 60.0], [300.0, 30.0]]))
    # --- the contact (search-class) ---
    sine = sine_line()
    rc_mod = frozen("alignment.rolling_circle")
    s = np.array(sine.coords)
    found = 0
    for idx in range(30, 280, 7):  # the first few (sample, side) pairs whose circle touches the line again
        for side in ("left", "right"):
            t_ = s[idx + 1] - s[idx - 1]
            t_ = t_ / np.hypot(*t_)
            centre = rnd(s[idx] + (np.array([-t_[1], t_[0]]) if side == "left" else np.array([t_[1], -t_[0]])) * 150.0, 4)
            ch_ = float(sine.project(Point(*s[idx])))
            if rc_mod.find_forward_circle_contact(sine, s[idx], ch_, centre, 150.0, 40.0) is not None and found < 6:
                c(f"contact_{side}_{idx}", 1, "find_forward_circle_contact", dict(
                    centreline=sine, tangent_point=s[idx], tangent_chainage=ch_, centre=centre,
                    radius=150.0, min_chainage_gap=40.0))
                found += 1
    c("contact_none", 1, "find_forward_circle_contact", dict(
        centreline=LineString([(0, 0), (1000, 0)]), tangent_point=[0.0, 0.0], tangent_chainage=0.0,
        centre=[0.0, 200.0], radius=100.0, min_chainage_gap=10.0))
    c("contact_bad_radius", 3, "find_forward_circle_contact", dict(
        centreline=sine, tangent_point=[0.0, 0.0], tangent_chainage=0.0, centre=[0.0, 0.0], radius=0.0,
        min_chainage_gap=10.0))
    c("contact_bad_gap", 3, "find_forward_circle_contact", dict(
        centreline=sine, tangent_point=[0.0, 0.0], tangent_chainage=0.0, centre=[0.0, 0.0], radius=10.0,
        min_chainage_gap=0.0))
    # --- the search ---
    c("sine_r300", 1, "rolling_circle", dict(centreline=sine_line(6000.0, 1500.0, 260.0), min_radius=300.0, step=10.0))
    c("sine_r300_step25", 1, "rolling_circle", dict(centreline=sine_line(6000.0, 1500.0, 260.0), min_radius=300.0, step=25.0))
    c("sine_gentle_is_left_alone", 1, "rolling_circle",
      dict(centreline=sine_line(6000.0, 1500.0, 260.0), min_radius=100.0, step=10.0))
    c("sine_r255_chord", 1, "rolling_circle",
      dict(centreline=sine_line(6000.0, 1500.0, 260.0), min_radius=255.0, step=10.0, check_chord_m=85.0))
    c("sine_one_pass_left_only", 1, "rolling_circle",
      dict(centreline=sine, min_radius=250.0, step=10.0, passes=1, sides=["left"]))
    c("sine_explicit_gap", 1, "rolling_circle",
      dict(centreline=sine, min_radius=250.0, step=10.0, min_chainage_gap=30.0))
    c("jagged_not_resolved", 1, "rolling_circle",
      dict(centreline=jagged(), min_radius=360.0, step=10.0, passes=6, check_chord_m=90.0))
    c("synthetic_river_r300", 1, "rolling_circle",
      dict(centreline=synth(length_m=2400.0)["centreline"], min_radius=300.0, step=10.0))
    c("multilinestring_input", 1, "rolling_circle",
      dict(centreline=MultiLineString([LineString(list(sine.coords)[:150]), LineString(list(sine.coords)[149:])]),
           min_radius=250.0, step=10.0))
    c("bad_min_radius", 3, "rolling_circle", dict(centreline=sine, min_radius=0.0, step=10.0))
    c("bad_step", 3, "rolling_circle", dict(centreline=sine, min_radius=250.0, step=0.0))
    c("bad_chord", 3, "rolling_circle", dict(centreline=sine, min_radius=250.0, step=10.0, check_chord_m=-1.0))
    c("not_a_line_input", 3, "rolling_circle", dict(centreline=Point(0, 0), min_radius=250.0, step=10.0))
    rng = Rng(23014)
    for i in range(8):
        L = rng.r3(1500.0, 3000.0)
        wl = rng.r3(700.0, 1600.0)
        amp = rng.r3(80.0, 320.0)
        apex = wl ** 2 / (4 * math.pi ** 2 * amp)
        c(f"search_sweep_{i:02d}", 2, "rolling_circle", dict(
            centreline=sine_line(L, wl, amp, rng.r3(8.0, 20.0)), min_radius=rng.r3(0.8, 2.2) * apex,
            step=rng.r3(8.0, 30.0), passes=rng.i(1, 3),
            sides=[["left", "right"], ["right", "left"], ["left"], ["right"]][i % 4]))
    for i in range(8):
        n = rng.i(6, 25)
        p = rnd(np.cumsum(np.array([[rng.u(10, 60), rng.u(-30, 30)] for _ in range(n)]), axis=0))
        c(f"resample_sweep_{i:02d}", 2, "resample_coordinate_sequence", dict(points=p, step=rng.r3(5.0, 40.0)))
        c(f"chainages_sweep_{i:02d}", 2, "cumulative_chainages", dict(points=p))
    return out
