"""limits: `alignment/limits.py` (navigation channel limits with curvature-adaptive widening)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString, MultiLineString, Point, Polygon

from ..common import Rng, case as _case, frozen, rline, rnd, synth

ENTRY_POINT = "limits"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    lm = frozen("alignment.limits")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "extract_lines":
        return lm.extract_lines(args["geom"])
    if fn == "merge_lines":
        return lm.merge_lines(args["lines"])
    if fn == "ensure_multilinestring":
        return lm.ensure_multilinestring(args["geom"])
    if fn == "resample_line":
        return lm.resample_line(args["line"], args["step"])
    if fn == "circumcircle":
        r = lm.circumcircle(arr("p0"), arr("p1"), arr("p2"))
        return None if r is None else {"center": r[0], "radius": r[1]}
    if fn == "compute_curvature_radius":
        return lm.compute_curvature_radius(arr("points"))
    if fn == "compute_normals":
        return lm.compute_normals(arr("points"))
    if fn == "compute_chainages":
        return lm.compute_chainages(arr("points"))
    if fn == "rolling_average_radius":
        return lm.rolling_average_radius(arr("radii"), arr("chainages"), args["window"])
    if fn == "compute_extra_width":
        return lm.compute_extra_width(arr("radii"), arr("chainages"), args["total_length"], args["loa"],
                                      args["extra_width_coefficient"], args["min_radius_length_multiple"],
                                      args["max_extra_width_radius"], args["radius_threshold"])
    if fn == "build_variable_offset_line":
        left, right, radii, extra = lm.build_variable_offset_line(**args)
        return {"left": left, "right": right, "smoothed_radii": radii, "extra": extra}
    if fn == "build_channel_limits":
        left, right, diag = lm.build_channel_limits(**args)
        return {"left": left, "right": right, "diagnostics": diag}
    raise KeyError(f"limits has no parity function {fn!r}")


def arc_line(radius, sweep_deg, n=120):
    t = np.linspace(0, np.radians(sweep_deg), n)
    return rline(np.column_stack([radius * np.sin(t), radius * (1 - np.cos(t))]))


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    a = LineString([(0, 0), (10, 0)])
    b = LineString([(10, 0), (20, 5)])
    far = LineString([(100, 100), (120, 100)])
    # --- geometry plumbing ---
    c("extract_line", 1, "extract_lines", dict(geom=a))
    c("extract_multi", 1, "extract_lines", dict(geom=MultiLineString([a, far])))
    # A polygon or a point makes GEOS refuse with its own message (it quotes the shape as WKT).
    # That text is not ours to reproduce, so no case feeds one in.
    c("merge_one", 1, "merge_lines", dict(lines=[a]))
    c("merge_contiguous", 1, "merge_lines", dict(lines=[a, b]))
    c("merge_contiguous_reversed_order", 1, "merge_lines", dict(lines=[b, a]))
    c("merge_disjoint", 1, "merge_lines", dict(lines=[a, far]))
    c("merge_three_with_gap", 1, "merge_lines", dict(lines=[a, b, far]))
    c("merge_empty_refused", 3, "merge_lines", dict(lines=[]))
    c("ensure_multi_from_line", 1, "ensure_multilinestring", dict(geom=a))
    c("ensure_multi_keeps_multi", 1, "ensure_multilinestring", dict(geom=MultiLineString([a, far])))
    # --- numeric helpers ---
    c("resample_line", 1, "resample_line", dict(line=LineString([(0, 0), (30, 40), (80, 40)]), step=7.0))
    c("resample_line_coarse", 1, "resample_line", dict(line=a, step=1000.0))
    c("resample_zero_step", 3, "resample_line", dict(line=a, step=0.0))
    c("resample_zero_length", 3, "resample_line", dict(line=LineString([(1, 1), (1, 1)]), step=1.0))
    c("circumcircle_unit", 1, "circumcircle", dict(p0=[1.0, 0.0], p1=[0.0, 1.0], p2=[-1.0, 0.0]))
    c("circumcircle_collinear", 1, "circumcircle", dict(p0=[0.0, 0.0], p1=[1.0, 1.0], p2=[2.0, 2.0]))
    c("circumcircle_almost_collinear", 1, "circumcircle", dict(p0=[0.0, 0.0], p1=[1.0, 1e-10], p2=[2.0, 0.0]))
    arc = np.array(arc_line(300.0, 90.0).coords)
    c("curvature_radius_arc", 1, "compute_curvature_radius", dict(points=arc))
    c("curvature_radius_two_points", 1, "compute_curvature_radius", dict(points=[[0.0, 0.0], [1.0, 0.0]]))
    c("curvature_radius_straight", 1, "compute_curvature_radius", dict(points=[[float(i), 0.0] for i in range(6)]))
    c("normals_arc", 1, "compute_normals", dict(points=arc))
    c("normals_duplicate_points", 1, "compute_normals", dict(points=[[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [2.0, 1.0]]))
    c("normals_one_point", 1, "compute_normals", dict(points=[[0.0, 0.0]]))
    c("chainages", 1, "compute_chainages", dict(points=arc))
    ch = np.arange(0.0, 500.0, 10.0)
    rr = np.where((ch > 150) & (ch < 300), 250.0, np.inf)
    rr[10] = 0.0
    rr[11] = np.nan
    c("rolling_average", 1, "rolling_average_radius", dict(radii=rr, chainages=ch, window=50.0))
    c("rolling_average_wide_window", 1, "rolling_average_radius", dict(radii=rr, chainages=ch, window=1000.0))
    c("rolling_average_empty", 1, "rolling_average_radius", dict(radii=[], chainages=[], window=50.0))
    base = dict(radii=rr, chainages=ch, total_length=490.0, loa=85.0, extra_width_coefficient=0.6,
                min_radius_length_multiple=3.0, max_extra_width_radius=2000.0, radius_threshold=2000.0)
    c("extra_width_banded", 1, "compute_extra_width", base)
    c("extra_width_unbanded", 1, "compute_extra_width", dict(base, max_extra_width_radius=None))
    c("extra_width_zero_loa", 1, "compute_extra_width", dict(base, loa=0.0))
    c("extra_width_zero_coefficient", 1, "compute_extra_width", dict(base, extra_width_coefficient=0.0))
    c("extra_width_no_multiple", 1, "compute_extra_width", dict(base, min_radius_length_multiple=0.0))
    c("extra_width_threshold_low", 1, "compute_extra_width", dict(base, radius_threshold=200.0))
    # --- the variable offset line and the channel limits ---
    cl = arc_line(250.0, 160.0, 200)
    c("variable_offset_arc", 1, "build_variable_offset_line", dict(line=cl, base_offset=30.0, loa=85.0))
    c("variable_offset_straight", 1, "build_variable_offset_line",
      dict(line=LineString([(0, 0), (800, 0)]), base_offset=30.0, loa=85.0))
    c("variable_offset_fine_sample", 1, "build_variable_offset_line",
      dict(line=cl, base_offset=25.0, loa=110.0, radius_sample_length=0.2))
    c("variable_offset_unbanded", 1, "build_variable_offset_line",
      dict(line=cl, base_offset=25.0, loa=110.0, max_extra_width_radius=None))
    rv = synth(length_m=2200.0)
    c("limits_single_centreline", 1, "build_channel_limits",
      dict(centreline_geoms=[rv["centreline"]], base_offset=30.0, loa=85.0))
    c("limits_two_pieces", 1, "build_channel_limits",
      dict(centreline_geoms=[LineString([(0, 0), (500, 0)]), LineString([(500, 0), (900, 100)])],
           base_offset=20.0, loa=60.0))
    c("limits_disjoint_pieces", 1, "build_channel_limits",
      dict(centreline_geoms=[LineString([(0, 0), (500, 0)]), LineString([(2000, 0), (2500, 100)])],
           base_offset=20.0, loa=60.0))
    c("limits_multilinestring_input", 1, "build_channel_limits",
      dict(centreline_geoms=[MultiLineString([LineString([(0, 0), (500, 0)]), LineString([(500, 0), (900, 100)])])],
           base_offset=20.0, loa=60.0))
    c("limits_skips_empty", 1, "build_channel_limits",
      dict(centreline_geoms=[LineString(), LineString([(0, 0), (500, 0)])], base_offset=20.0, loa=60.0))
    c("limits_nothing_to_build", 3, "build_channel_limits",
      dict(centreline_geoms=[LineString()], base_offset=20.0, loa=60.0))
    c("limits_no_geoms", 3, "build_channel_limits", dict(centreline_geoms=[], base_offset=20.0, loa=60.0))
    # --- sweep ---
    rng = Rng(23011)
    for i in range(7):
        rv = synth(length_m=rng.r3(1000.0, 2000.0), wavelength_m=rng.r3(600.0, 1500.0),
                   amplitude_m=rng.r3(60.0, 280.0))
        c(f"limits_sweep_{i:02d}", 2, "build_channel_limits",
          dict(centreline_geoms=[rv["centreline"]], base_offset=rng.r3(10.0, 60.0), loa=rng.r3(40.0, 140.0),
               extra_width_coefficient=rng.r3(0.1, 1.0), min_radius_length_multiple=rng.r3(2.0, 5.0),
               max_extra_width_radius=rng.r3(800.0, 3000.0) if i % 4 else None,
               radius_threshold=rng.r3(800.0, 3000.0), radius_sample_length=rng.r3(2.0, 30.0)))
    for i in range(10):
        n = rng.i(5, 60)
        p = rnd(np.cumsum(np.array([[rng.u(5, 40), rng.u(-20, 20)] for _ in range(n)]), axis=0))
        chn = np.concatenate([[0.0], np.cumsum(np.hypot(*np.diff(p, axis=0).T))])
        rad = np.array([rng.r3(60.0, 3000.0) if rng.p(0.8) else np.inf for _ in range(n)])
        c(f"helpers_sweep_{i:02d}_radius", 2, "compute_curvature_radius", dict(points=p))
        c(f"helpers_sweep_{i:02d}_normals", 2, "compute_normals", dict(points=p))
        c(f"helpers_sweep_{i:02d}_rolling", 2, "rolling_average_radius",
          dict(radii=rad, chainages=chn, window=rng.r3(10.0, 200.0)))
        c(f"helpers_sweep_{i:02d}_extra", 2, "compute_extra_width",
          dict(radii=rad, chainages=chn, total_length=float(chn[-1]), loa=rng.r3(30.0, 150.0),
               extra_width_coefficient=rng.r3(0.0, 1.0), min_radius_length_multiple=rng.r3(0.0, 5.0),
               max_extra_width_radius=rng.r3(500.0, 3000.0) if i % 3 else None,
               radius_threshold=rng.r3(500.0, 3000.0)))
    return out
