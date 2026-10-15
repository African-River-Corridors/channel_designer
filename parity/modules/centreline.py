"""centreline: `alignment/centreline.py` (a smoothed centreline from a pair of bank lines)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rline, rnd, synth

ENTRY_POINT = "centreline"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    cl = frozen("alignment.centreline")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "ensure_orientation":
        return list(cl.ensure_orientation(arr("left"), arr("right")))
    if fn == "resample_polyline":
        return cl.resample_polyline(args["points"], args["step"])
    if fn == "reparameterize_fraction":
        return list(cl.reparameterize_fraction(arr("points")))
    if fn == "moving_average":
        return cl.moving_average(arr("points"), args["window"])
    if fn == "chaikin_smoothing":
        return cl.chaikin_smoothing(arr("points"), args["iterations"])
    if fn == "map_points_monotonic":
        return cl.map_points_monotonic(arr("source"), args["target_line"], args["max_step"])
    if fn == "compute_paired_centreline":
        return cl.compute_paired_centreline(arr("left"), arr("right"), args["resample_step"],
                                            args["smooth_window"], args["chaikin_iterations"])
    if fn == "build_centreline":
        return cl.build_centreline(**args)
    if fn == "trim_polyline":
        return cl.trim_polyline(arr("points"), args["cut_length"])
    raise KeyError(f"centreline has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    par_l = LineString([(0, 25), (1000, 25)])
    par_r = LineString([(1000, -25), (0, -25)])
    par_r_fwd = LineString([(0, -25), (1000, -25)])
    river = synth(length_m=1800.0)
    wide = synth(length_m=2400.0, width_m=70.0, amplitude_m=180.0, wavelength_m=1200.0)
    # --- named: tests/alignment/test_alignment.py ---
    c("parallel_banks_reversed_right", 1, "build_centreline", dict(left_bank=par_l, right_bank=par_r, resample_step=20.0))
    c("parallel_banks_same_direction", 1, "build_centreline", dict(left_bank=par_l, right_bank=par_r_fwd))
    c("synthetic_river", 1, "build_centreline",
      dict(left_bank=river["left_bank"], right_bank=river["right_bank"], resample_step=20.0))
    c("synthetic_river_no_smoothing", 1, "build_centreline",
      dict(left_bank=river["left_bank"], right_bank=river["right_bank"], resample_step=20.0,
           smooth_window=1, chaikin_iterations=0))
    c("synthetic_river_even_window", 1, "build_centreline",
      dict(left_bank=river["left_bank"], right_bank=river["right_bank"], resample_step=15.0,
           smooth_window=6, chaikin_iterations=1))
    c("wide_river_fine_step", 1, "build_centreline",
      dict(left_bank=wide["left_bank"], right_bank=wide["right_bank"], resample_step=10.0))
    c("trim_refuses_short", 3, "trim_polyline", dict(points=[[0, 0], [10, 0]], cut_length=6.0))
    c("trim_both_ends", 1, "trim_polyline",
      dict(points=[[float(i * 10), float(i % 3)] for i in range(12)], cut_length=25.0))
    c("trim_zero_cut", 1, "trim_polyline", dict(points=[[0.0, 0.0], [10.0, 0.0], [20.0, 5.0]], cut_length=0.0))
    c("trim_on_vertex", 1, "trim_polyline",
      dict(points=[[float(i * 10), 0.0] for i in range(12)], cut_length=30.0))
    c("trim_single_vertex", 3, "trim_polyline", dict(points=[[0.0, 0.0]], cut_length=5.0))
    c("orientation_flips", 1, "ensure_orientation",
      dict(left=[[0.0, 0.0], [10.0, 0.0]], right=[[10.0, 5.0], [0.0, 5.0]]))
    c("orientation_keeps", 1, "ensure_orientation",
      dict(left=[[0.0, 0.0], [10.0, 0.0]], right=[[0.0, 5.0], [10.0, 5.0]]))
    pts = rnd(np.array([[0.0, 0.0], [30.0, 10.0], [55.0, 40.0], [90.0, 45.0], [130.0, 20.0]]))
    for step in (7.0, 20.0, 41.0, 1000.0):
        c(f"resample_step{step:g}", 1, "resample_polyline", dict(points=pts, step=step))
    c("resample_one_vertex", 3, "resample_polyline", dict(points=[[0.0, 0.0]], step=10.0))
    c("resample_zero_step", 3, "resample_polyline", dict(points=pts, step=0.0))
    c("resample_zero_length", 3, "resample_polyline", dict(points=[[1.0, 1.0], [1.0, 1.0]], step=5.0))
    c("fractions", 1, "reparameterize_fraction", dict(points=pts))
    c("fractions_zero_length", 3, "reparameterize_fraction", dict(points=[[1.0, 1.0], [1.0, 1.0]]))
    for w in (0, 1, 3, 4, 5, 9):
        c(f"moving_average_w{w}", 1, "moving_average", dict(points=pts, window=w))
    for n in (0, 1, 2, 3):
        c(f"chaikin_{n}", 1, "chaikin_smoothing", dict(points=pts, iterations=n))
    tl = LineString([(0, 0), (50, 20), (100, 0), (150, 30), (200, 10)])
    c("map_monotonic", 1, "map_points_monotonic",
      dict(source=[[0, 5], [40, 30], [60, -5], [110, 20], [130, 0], [210, 15]], target_line=tl, max_step=60.0))
    c("map_monotonic_small_steps", 1, "map_points_monotonic",
      dict(source=[[0, 5], [40, 30], [60, -5], [110, 20], [130, 0], [210, 15]], target_line=tl, max_step=12.0))
    c("paired_parallel", 1, "compute_paired_centreline",
      dict(left=[[float(x), 25.0] for x in range(0, 101, 20)], right=[[float(x), -25.0] for x in range(0, 101, 20)],
           resample_step=20.0, smooth_window=5, chaikin_iterations=2))
    # --- sweep ---
    rng = Rng(23009)
    for i in range(14):
        rv = synth(length_m=rng.r3(1000.0, 2600.0), wavelength_m=rng.r3(600.0, 1500.0),
                   amplitude_m=rng.r3(60.0, 260.0), width_m=rng.r3(20.0, 80.0))
        c(f"build_sweep_{i:02d}", 2, "build_centreline",
          dict(left_bank=rv["left_bank"], right_bank=rv["right_bank"] if i % 2 else
               LineString(list(rv["right_bank"].coords)[::-1]),
               resample_step=rng.r3(8.0, 40.0), smooth_window=rng.i(1, 9),
               chaikin_iterations=rng.i(0, 3)))
    for i in range(12):
        n = rng.i(4, 30)
        p = rnd(np.cumsum(np.array([[rng.u(5, 40), rng.u(-15, 15)] for _ in range(n)]), axis=0))
        c(f"resample_sweep_{i:02d}", 2, "resample_polyline", dict(points=p, step=rng.r3(3.0, 50.0)))
        c(f"moving_average_sweep_{i:02d}", 2, "moving_average", dict(points=p, window=rng.i(0, 11)))
        c(f"chaikin_sweep_{i:02d}", 2, "chaikin_smoothing", dict(points=p, iterations=rng.i(0, 3)))
        c(f"trim_sweep_{i:02d}", 2, "trim_polyline", dict(points=p, cut_length=rng.r3(0.0, 40.0)))
    return out
