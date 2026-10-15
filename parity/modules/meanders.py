"""meanders: `alignment/meanders.py` (bends tighter than a radius threshold along a centreline)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rline, rnd, synth

ENTRY_POINT = "meanders"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    m = frozen("alignment.meanders")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "resample_polyline":
        return m.resample_polyline(args["points"], args["step"])
    if fn == "circumcircle":
        r = m.circumcircle(arr("p0"), arr("p1"), arr("p2"))
        return None if r is None else {"center": r[0], "radius": r[1]}
    if fn == "curvature_radius":
        curv, rad, signed = m.curvature_radius(arr("points"))
        return {"curvature": curv, "radius": rad, "signed_curvature": signed}
    if fn == "compute_heading":
        return m.compute_heading(arr("points"))
    if fn == "detect_curvature_runs":
        return m.detect_curvature_runs(arr("signed_curvature"), args["min_points"])
    if fn == "find_meander_runs":
        return m.find_meander_runs(arr("radius"), arr("signed_curvature"), args["threshold"],
                                   args["min_points"])
    if fn == "tangent_angle":
        return [m.tangent_angle(arr("heading"), i) for i in args["indices"]]
    if fn == "find_meanders":
        return m.find_meanders(**args)
    raise KeyError(f"meanders has no parity function {fn!r}")


def sine_line(length, wl, amp, step=10.0):
    x = np.arange(0.0, length + step, step)
    return rline(np.column_stack([x, amp * np.sin(2 * np.pi * x / wl)]))


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    lam, amp = 1500.0, 260.0
    apex = lam ** 2 / (4 * np.pi ** 2 * amp)
    sine = sine_line(6000.0, lam, amp)
    c("sine_apex_radius", 1, "find_meanders", dict(centreline=sine, max_radius=2 * apex, sample_step=10.0))
    c("sine_default_threshold_below_apex", 1, "find_meanders", dict(centreline=sine, max_radius=150.0))
    c("sine_wide_threshold", 1, "find_meanders", dict(centreline=sine, max_radius=1e6))
    c("straight_line", 1, "find_meanders",
      dict(centreline=LineString([(0, 0), (2000, 0)]), max_radius=1e6))
    c("sine_min_points_20", 1, "find_meanders", dict(centreline=sine, max_radius=2 * apex, min_points=20))
    c("sine_min_deflection_120", 1, "find_meanders",
      dict(centreline=sine, max_radius=2 * apex, min_deflection_deg=120.0))
    c("sine_coarse_sampling", 1, "find_meanders",
      dict(centreline=sine, max_radius=2 * apex, sample_step=40.0, min_points=4))
    for tag, kw in [("default", {}), ("tight", dict(amplitude_m=300.0, wavelength_m=800.0))]:
        c(f"synthetic_{tag}", 1, "find_meanders",
          dict(centreline=synth(length_m=2400.0, **kw)["centreline"], max_radius=600.0))
    c("one_vertex", 3, "find_meanders", dict(centreline=LineString([(0, 0), (0, 0)]), max_radius=100.0))
    c("zero_step", 3, "find_meanders", dict(centreline=sine, max_radius=100.0, sample_step=0.0))
    # --- the pieces ---
    pts = np.array(sine.coords)[::5]
    c("resample", 1, "resample_polyline", dict(points=pts, step=17.0))
    c("resample_one_vertex", 3, "resample_polyline", dict(points=[[0.0, 0.0]], step=10.0))
    c("resample_zero_step", 3, "resample_polyline", dict(points=pts, step=0.0))
    c("resample_zero_length", 3, "resample_polyline", dict(points=[[1.0, 1.0], [1.0, 1.0]], step=5.0))
    c("circumcircle_unit", 1, "circumcircle", dict(p0=[1.0, 0.0], p1=[0.0, 1.0], p2=[-1.0, 0.0]))
    c("circumcircle_collinear", 1, "circumcircle", dict(p0=[0.0, 0.0], p1=[1.0, 1.0], p2=[2.0, 2.0]))
    c("curvature_arc", 1, "curvature_radius", dict(points=pts))
    c("curvature_straight", 1, "curvature_radius", dict(points=[[float(i), 0.0] for i in range(6)]))
    c("curvature_two_points", 3, "curvature_radius", dict(points=[[0.0, 0.0], [1.0, 0.0]]))
    c("curvature_duplicate_points", 1, "curvature_radius",
      dict(points=[[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [2.0, 1.0], [3.0, 3.0]]))
    c("heading_sine", 1, "compute_heading", dict(points=pts))
    c("heading_loop_unwraps", 1, "compute_heading",
      dict(points=rnd(np.column_stack([np.cos(np.linspace(0, 5 * np.pi, 40)), np.sin(np.linspace(0, 5 * np.pi, 40))]))))
    sc = np.array([0, 0, 1, 1, 1, 0, 0, -1, -1, 1, 1, 1, 1e-7, 1, 1, 1], dtype=float) * 0.01
    c("runs_mixed", 1, "detect_curvature_runs", dict(signed_curvature=sc, min_points=2))
    c("runs_long_min", 1, "detect_curvature_runs", dict(signed_curvature=sc, min_points=4))
    c("runs_all_zero", 1, "detect_curvature_runs", dict(signed_curvature=[0.0] * 8, min_points=2))
    c("runs_empty", 1, "detect_curvature_runs", dict(signed_curvature=[], min_points=2))
    rad = np.where(sc != 0, 1.0 / np.maximum(np.abs(sc), 1e-9), np.inf)
    c("meander_runs", 1, "find_meander_runs", dict(radius=rad, signed_curvature=sc, threshold=150.0, min_points=2))
    c("meander_runs_none_below_threshold", 1, "find_meander_runs",
      dict(radius=rad, signed_curvature=sc, threshold=10.0, min_points=2))
    c("meander_runs_infinite_radius", 1, "find_meander_runs",
      dict(radius=[float("inf")] * 6, signed_curvature=[0.01] * 6, threshold=100.0, min_points=2))
    head = [0.0, 0.1, 0.2, 0.5, 0.9]
    c("tangent_angle", 1, "tangent_angle", dict(heading=head, indices=[-3, 0, 1, 2, 4, 5, 9]))
    c("tangent_angle_empty", 1, "tangent_angle", dict(heading=[], indices=[0, 1]))
    rng = Rng(23012)
    for i in range(16):
        L = rng.r3(2000.0, 6000.0); wl = rng.r3(500.0, 2000.0); a = rng.r3(40.0, 320.0)
        apex_r = wl ** 2 / (4 * np.pi ** 2 * a)
        c(f"find_sweep_{i:02d}", 2, "find_meanders",
          dict(centreline=sine_line(L, wl, a, rng.r3(8.0, 25.0)), max_radius=rng.r3(0.6, 3.0) * apex_r,
               sample_step=rng.r3(6.0, 30.0), min_points=rng.i(3, 14), min_deflection_deg=rng.r3(10.0, 100.0)))
    for i in range(8):
        n = rng.i(8, 60)
        p = rnd(np.cumsum(np.array([[rng.u(5, 30), rng.u(-12, 12)] for _ in range(n)]), axis=0))
        c(f"find_random_walk_{i:02d}", 2, "find_meanders",
          dict(centreline=rline(p), max_radius=rng.r3(50.0, 800.0), sample_step=rng.r3(5.0, 20.0),
               min_points=rng.i(2, 8), min_deflection_deg=rng.r3(5.0, 60.0)))
        c(f"curvature_sweep_{i:02d}", 2, "curvature_radius", dict(points=p))
        c(f"heading_sweep_{i:02d}", 2, "compute_heading", dict(points=p))
    return out
