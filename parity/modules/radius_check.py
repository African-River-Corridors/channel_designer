"""radius_check: `alignment/radius_check.py` (the measured minimum radius, read on a chord)."""
from __future__ import annotations

import math

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rnd

ENTRY_POINT = "radius_check"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    rc = frozen("alignment.radius_check")
    if fn == "default_check_chord":
        return rc.default_check_chord(**args)
    if fn == "min_radius_on_chord":
        return rc.min_radius_on_chord(**args)
    if fn == "min_radius_on_vertices":
        return rc.min_radius_on_vertices(args["pts"], args.get("k", 1))
    if fn == "radius_ok":
        return rc.radius_ok(**args)
    if fn == "constants":
        return {"MAX_STRIDE_M": rc.MAX_STRIDE_M, "RADIUS_TOL": rc.RADIUS_TOL}
    raise KeyError(f"radius_check has no parity function {fn!r}")


def arc(radius, sweep_deg, n=400):
    t = np.linspace(0, math.radians(sweep_deg), n)
    return LineString(rnd(np.column_stack([radius * np.sin(t), radius * (1 - np.cos(t))])))


def sine_line(step=10.0, length=6000.0, wl=1500.0, amp=260.0):
    x = np.arange(0.0, length + step, step)
    return LineString(rnd(np.column_stack([x, amp * np.sin(2.0 * math.pi * x / wl)])))


def jagged(jitter=8.0, vertex=50.0):
    x = np.arange(0.0, 6000.0 + vertex, vertex)
    y = 260.0 * np.sin(2.0 * math.pi * x / 1500.0)
    y[1:-1] += jitter * np.where(np.arange(1, x.size - 1) % 2 == 0, 1.0, -1.0)
    return LineString(rnd(np.column_stack([x, y])))


def circle_pts(R_, ds, sweep=5.0):
    th = np.arange(0.0, sweep, ds / R_)
    return rnd(np.column_stack([R_ * np.sin(th), R_ * (1 - np.cos(th))]), 6)


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("tolerances", 1, "constants", {})
    for r_ in (150.0, 300.0, 450.0, 1000.0):
        c(f"chord_r{r_:g}", 1, "default_check_chord", dict(r_min=r_))
    c("arc_r500_chord100", 1, "min_radius_on_chord", dict(line=arc(500.0, 90.0), chord_m=100.0))
    c("straight_is_infinite", 1, "min_radius_on_chord",
      dict(line=LineString([(0, 0), (1000, 0)]), chord_m=100.0))
    c("sine_apex_chord60", 1, "min_radius_on_chord", dict(line=sine_line(), chord_m=60.0))
    c("sine_apex_chord200", 1, "min_radius_on_chord", dict(line=sine_line(), chord_m=200.0))
    c("last_stride_kink_is_read", 1, "min_radius_on_chord",
      dict(line=LineString([(0.0, 0.0), (1008.0, 0.0), (1009.0, 0.5)]), chord_m=100.0))
    c("line_shorter_than_chord", 1, "min_radius_on_chord",
      dict(line=LineString([(0, 0), (40, 0), (60, 30)]), chord_m=100.0))
    c("jagged_trace_chord100", 1, "min_radius_on_chord", dict(line=jagged(), chord_m=100.0))
    c("jagged_trace_chord33", 1, "min_radius_on_chord", dict(line=jagged(), chord_m=33.3))
    for R_, ds in ((150.0, 100.0), (200.0, 100.0), (100.0, 50.0)):
        for k in (1, 2):
            c(f"vertices_circle_r{R_:g}_ds{ds:g}_k{k}", 1, "min_radius_on_vertices",
              dict(pts=circle_pts(R_, ds), k=k))
    c("vertices_default_k", 1, "min_radius_on_vertices", dict(pts=circle_pts(300.0, 60.0)))
    c("vertices_too_few_points", 1, "min_radius_on_vertices", dict(pts=[[0.0, 0.0], [1.0, 1.0]], k=1))
    c("vertices_collinear", 1, "min_radius_on_vertices", dict(pts=[[float(i), 0.0] for i in range(8)], k=2))
    for m, r_ in [(300.0, 300.0), (294.0, 300.0), (293.9, 300.0), (299.9, 300.0), (400.0, 300.0),
                  ("float:inf", 300.0), ("float:nan", 300.0), (0.0, 0.0), (-1.0, 300.0)]:
        c(f"ok_{m}_{r_:g}", 1, "radius_ok", dict(measured_m=m, r_min=r_))
    rng = Rng(23006)
    for i in range(25):
        kind = i % 3
        if kind == 0:
            line = arc(rng.r3(80.0, 2000.0), rng.r3(20.0, 200.0), rng.i(60, 200))
        elif kind == 1:
            line = sine_line(rng.r3(15.0, 40.0), rng.r3(1500.0, 5000.0), rng.r3(600.0, 2000.0),
                             rng.r3(30.0, 300.0))
        else:
            line = jagged(rng.r3(0.0, 15.0), rng.r3(20.0, 80.0))
        c(f"chord_sweep_{i:02d}", 2, "min_radius_on_chord", dict(line=line, chord_m=rng.r3(10.0, 400.0)))
    for i in range(15):
        pts = circle_pts(rng.r3(60.0, 800.0), rng.r3(10.0, 100.0), rng.r3(0.5, 6.0))
        pts = pts + np.array([[rng.u(-1, 1) * 3 * (j % 2), rng.u(-1, 1) * 3 * (j % 2)]
                              for j in range(len(pts))]) if i % 2 else pts
        c(f"vertices_sweep_{i:02d}", 2, "min_radius_on_vertices", dict(pts=pts, k=rng.i(1, 4)))
    for i in range(10):
        c(f"ok_sweep_{i:02d}", 2, "radius_ok", dict(measured_m=rng.r3(50.0, 600.0), r_min=rng.r3(50.0, 600.0)))
    for bad in (0.0, -1.0, "float:nan", "float:inf"):
        c(f"chord_not_positive_{bad}", 3, "min_radius_on_chord",
          dict(line=LineString([(0, 0), (1000, 0)]), chord_m=bad))
    c("vertices_k0", 3, "min_radius_on_vertices", dict(pts=circle_pts(300.0, 60.0), k=0))
    return out
