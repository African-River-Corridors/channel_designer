"""bends: `alignment/bends.py` (bend catalogue and the tight / gentle / straight classification)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString, Point

from ..common import Rng, case as _case, frozen, rline, rnd, synth

ENTRY_POINT = "bends"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    b = frozen("alignment.bends")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "constants":
        return {"STEP_M": b.STEP_M, "N_PSI": b.N_PSI, "PSI_MAX_DEG": b.PSI_MAX_DEG, "N_Y": b.N_Y,
                "E_MAX_M": b.E_MAX_M, "E_TOL_M": b.E_TOL_M}
    if fn == "resample":
        s, xy = b.resample(args["line"], args["step"])
        return {"s": s, "xy": xy}
    if fn == "curvature":
        return b.curvature(arr("xy"))
    if fn == "bend_units":
        return b.bend_units(arr("s"), arr("kappa"), args.get("smooth_n", 5))
    if fn == "feasible_within":
        kw = {k: v for k, v in args.items() if k not in ("kappa",)}
        return b.feasible_within(arr("kappa"), **kw)
    if fn == "e_exact":
        return b.e_exact(arr("kappa"), args["ds"], args["r_min"])
    if fn == "e_closed_form":
        return b.e_closed_form(args["r_nat"], args["defl"], args["r_min"])
    if fn == "contiguous_wetted_width":
        kw = {k: v for k, v in args.items() if k not in ("centre",)}
        return b.contiguous_wetted_width(centre=Point(*args["centre"]), **kw)
    if fn == "classify_bends":
        return b.classify_bends(**args)
    raise KeyError(f"bends has no parity function {fn!r}")


def sine_line(length, wl, amp, step=10.0):
    x = np.arange(0.0, length + step, step)
    return rline(np.column_stack([x, amp * np.sin(2 * np.pi * x / wl)]))


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("constants", 1, "constants", {})
    sine = sine_line(1800.0, 900.0, 160.0)
    c("resample_sine", 1, "resample", dict(line=sine, step=10.0))
    c("resample_coarse", 1, "resample", dict(line=sine, step=137.0))
    c("resample_short_line", 1, "resample", dict(line=LineString([(0, 0), (7, 0)]), step=10.0))
    xy = rnd(np.array(sine.coords)[::2])
    c("curvature_sine", 1, "curvature", dict(xy=xy))
    c("curvature_straight", 1, "curvature", dict(xy=[[float(i), 0.0] for i in range(7)]))
    c("curvature_duplicate_points", 1, "curvature", dict(xy=[[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [1.0, 1.0], [2.0, 3.0]]))
    k = np.concatenate([np.full(25, 1 / 400.0), np.full(25, -1 / 300.0), np.zeros(10)])
    c("bend_units", 1, "bend_units", dict(s=[10.0 * i for i in range(60)], kappa=k))
    c("bend_units_smooth_9", 1, "bend_units", dict(s=[10.0 * i for i in range(60)], kappa=k, smooth_n=9))
    c("bend_units_noisy", 1, "bend_units",
      dict(s=[10.0 * i for i in range(30)], kappa=[0.002 * ((-1) ** i) for i in range(30)]))
    # e_exact and feasible_within, from the tests
    c("e_exact_feasible_bend_is_zero", 1, "e_exact", dict(kappa=[1 / 1000.0] * 50, ds=10.0, r_min=300.0))
    c("e_exact_tight_bend", 1, "e_exact", dict(kappa=[1 / 150.0] * 50, ds=10.0, r_min=300.0))
    c("e_exact_short_tight_bend", 1, "e_exact", dict(kappa=[1 / 150.0] * 12, ds=10.0, r_min=300.0))
    c("e_exact_compound_bend", 1, "e_exact", dict(kappa=([1 / 200.0] * 15) + ([-1 / 200.0] * 15), ds=10.0, r_min=300.0))
    c("e_exact_infeasible_even_wide", 1, "e_exact", dict(kappa=[1 / 20.0] * 80, ds=10.0, r_min=500.0))
    c("e_exact_two_points", 1, "e_exact", dict(kappa=[1 / 150.0, 1 / 150.0], ds=10.0, r_min=300.0))
    for hw in (0.0, 5.0, 40.0):
        c(f"feasible_within_{hw:g}", 1, "feasible_within",
          dict(kappa=[1 / 250.0] * 30, ds=10.0, half_width=hw, r_min=300.0))
    c("feasible_within_negative_half_width", 1, "feasible_within",
      dict(kappa=[1 / 250.0] * 30, ds=10.0, half_width=-1.0, r_min=300.0))
    c("feasible_within_coarse_grid", 1, "feasible_within",
      dict(kappa=[1 / 250.0] * 30, ds=10.0, half_width=20.0, r_min=300.0, n_y=21, n_psi=11))
    for r_nat, defl in [(150.0, 1.0), (400.0, 1.0), (float("inf"), 1.0), (150.0, 0.0), (150.0, 3.5), (200.0, 0.4)]:
        c(f"closed_form_{r_nat:g}_{defl:g}", 1, "e_closed_form", dict(r_nat=r_nat, defl=defl, r_min=300.0))
    # wetted width
    river = LineString([(0, -25), (1000, -25), (1000, 25), (0, 25)])
    from shapely.geometry import Polygon
    pool = Polygon([(0, -25), (1000, -25), (1000, 25), (0, 25)])
    island = Polygon([(0, -25), (1000, -25), (1000, 25), (0, 25)],
                     [[(400, -5), (600, -5), (600, 5), (400, 5)]])
    c("wetted_width_parallel", 1, "contiguous_wetted_width", dict(river=pool, centre=[500.0, 0.0], nx=0.0, ny=1.0))
    c("wetted_width_off_centre", 1, "contiguous_wetted_width", dict(river=pool, centre=[500.0, 10.0], nx=0.0, ny=1.0))
    c("wetted_width_island_stops_at_first_exit", 1, "contiguous_wetted_width",
      dict(river=island, centre=[500.0, 0.0], nx=0.0, ny=1.0))
    c("wetted_width_slanted", 1, "contiguous_wetted_width",
      dict(river=pool, centre=[500.0, 0.0], nx=0.6, ny=0.8))
    c("wetted_width_outside_river", 1, "contiguous_wetted_width",
      dict(river=pool, centre=[500.0, 200.0], nx=0.0, ny=1.0))
    c("wetted_width_max_half_cap", 1, "contiguous_wetted_width",
      dict(river=pool, centre=[500.0, 0.0], nx=0.0, ny=1.0, max_half_m=10.0))
    c("wetted_width_coarse_probe", 1, "contiguous_wetted_width",
      dict(river=pool, centre=[500.0, 0.0], nx=0.0, ny=1.0, probe_m=4.0))
    # classify_bends
    straight = LineString([(0, 0), (3000, 0)])
    c("classify_straight_is_class_s", 1, "classify_bends", dict(
        natural=straight, river=straight.buffer(25.0, cap_style="flat"), r_min=300.0, top_width=50.0, bottom_width=30.0))
    rv = synth(length_m=2400.0)
    for r_min in (150.0, 300.0, 600.0):
        c(f"classify_synthetic_rmin{r_min:g}", 1, "classify_bends", dict(
            natural=rv["centreline"], river=rv["polygon"], r_min=r_min, top_width=50.0, bottom_width=30.0))
    c("classify_bank_cut_fraction", 1, "classify_bends", dict(
        natural=rv["centreline"], river=rv["polygon"], r_min=400.0, top_width=60.0, bottom_width=40.0,
        bank_cut_fraction=0.1))
    c("classify_coarse_step", 1, "classify_bends", dict(
        natural=rv["centreline"], river=rv["polygon"], r_min=400.0, top_width=60.0, bottom_width=40.0, step=25.0))
    # sweep
    rng = Rng(23013)
    for i in range(10):
        rv = synth(length_m=rng.r3(1500.0, 3000.0), wavelength_m=rng.r3(500.0, 1500.0),
                   amplitude_m=rng.r3(60.0, 300.0), width_m=rng.r3(20.0, 90.0))
        c(f"classify_sweep_{i:02d}", 2, "classify_bends", dict(
            natural=rv["centreline"], river=rv["polygon"], r_min=rng.r3(120.0, 800.0),
            top_width=rng.r3(30.0, 90.0), bottom_width=rng.r3(15.0, 60.0),
            bank_cut_fraction=rng.r3(0.0, 0.8), step=rng.r3(6.0, 20.0)))
    for i in range(14):
        n = rng.i(10, 80)
        kap = [rng.u(-1, 1) * rng.u(0.0, 0.01) for _ in range(n)]
        c(f"e_exact_sweep_{i:02d}", 2, "e_exact", dict(kappa=kap, ds=rng.r3(5.0, 20.0), r_min=rng.r3(100.0, 600.0)))
        c(f"units_sweep_{i:02d}", 2, "bend_units",
          dict(s=[rng.r3(5.0, 15.0) * j for j in range(n)] if False else [10.0 * j for j in range(n)],
               kappa=kap, smooth_n=rng.i(1, 9)))
        c(f"feasible_sweep_{i:02d}", 2, "feasible_within",
          dict(kappa=kap, ds=rng.r3(5.0, 20.0), half_width=rng.r3(0.0, 60.0), r_min=rng.r3(100.0, 600.0)))
        c(f"closed_form_sweep_{i:02d}", 2, "e_closed_form",
          dict(r_nat=rng.r3(80.0, 800.0), defl=rng.r3(0.0, 3.3), r_min=rng.r3(100.0, 600.0)))
    return out
