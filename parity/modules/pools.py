"""pools: `volumes/pools.py` (the bed profile and the weir cascade that sets each pool's level)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rline, rnd, terrain_from

ENTRY_POINT = "pools"
R = "initial case set (spec 0001, ticket 23)"


def _profile(pl, spec: dict):
    kw = {k: v for k, v in spec.items() if k not in ("centerline", "terrain")}
    return pl.sample_bed_profile(spec["centerline"], terrain_from(spec["terrain"]), **kw)


def _weirs(pl, spec: dict):
    prof = _profile(pl, spec["profile"])
    place = spec["place"]
    if place["kind"] == "explicit":
        return pl.place_weirs_explicit(prof, place["cascade"])
    kw = {k: v for k, v in place.items() if k != "kind"}
    return pl.place_weirs(prof, **kw)


def _weir_json(w):
    return {"index": w.index, "chainage_m": w.chainage_m, "point": [w.point.x, w.point.y], "bed_m": w.bed_m,
            "foundation_m": w.foundation_m, "crest_m": w.crest_m, "pool_water_level_m": w.pool_water_level_m,
            "pool_start_m": w.pool_start_m, "pool_end_m": w.pool_end_m, "pool_length_m": w.pool_length_m}


def adapter(fn: str, args: dict):
    pl = frozen("volumes.pools")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "overall_bottom_slope":
        kw = {k: v for k, v in args.items() if k not in ("z_raw",)}
        return pl.overall_bottom_slope(arr("z_raw"), **kw)
    if fn == "sample_bed_profile":
        p = _profile(pl, args)
        return {"s": p.s, "z_raw": p.z_raw, "z_overall": p.z_overall,
                "points": [[pt.x, pt.y] for pt in p.points], "line": p.line,
                "length_m": p.length_m}
    if fn == "place_weirs":
        return [_weir_json(w) for w in _weirs(pl, args)]
    if fn == "water_levels":
        weirs = _weirs(pl, args["weirs"])
        d = args.get("downstream_water_level_m", 0.0)
        fnl = pl.water_level_fn(weirs, d)
        return [{"s": s, "water_level": pl.water_level_at(weirs, s, d), "water_level_fn": fnl(s),
                 "grade": pl.grade_at(weirs, s, args["depth_m"], d, args.get("overdepth_m", 0.0))}
                for s in args["chainages"]]
    raise KeyError(f"pools has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    # a straight river, bed rising upstream at 1:5000 with two bumps (spoil heaps)
    def river(length, slope=0.0002, z0=-1.0, bumps=((3000.0, 6.0, 80.0), (7000.0, 9.0, 120.0)), **kw):
        return dict(centerline=LineString([(0.0, 0.0), (float(length), 0.0)]),
                    terrain={"kind": "hills", "base": z0, "gx": slope, "gy": 0.0,
                             "hills": [[x, 0.0, h, r] for x, h, r in bumps]}, **kw)

    prof = river(9000.0)
    c("profile_straight_river", 1, "sample_bed_profile", prof)
    c("profile_window_options", 1, "sample_bed_profile",
      river(6000.0, spacing_m=20.0, slope_percentile=30.0, slope_perc_window_m=1000.0, slope_smooth_window_m=2000.0))
    c("profile_flows_the_wrong_way_is_reversed", 1, "sample_bed_profile", river(8000.0, slope=-0.0002))
    c("profile_with_no_ground_data", 1, "sample_bed_profile", dict(
        centerline=LineString([(0.0, 0.0), (3000.0, 0.0)]),
        terrain={"kind": "plane", "z0": "float:nan", "gx": 0.0, "gy": 0.0}))
    c("profile_bent_line", 1, "sample_bed_profile", dict(
        centerline=rline([(0, 0), (2000, 600), (4000, -300), (7000, 100)]),
        terrain={"kind": "hills", "base": 0.0, "gx": 0.0003, "gy": 0.0, "hills": []}))
    c("profile_window_zero", 1, "sample_bed_profile", river(2000.0, slope_perc_window_m=0.0, slope_smooth_window_m=0.0))
    rng = Rng(23020)
    z = rnd(np.array([0.2 + 0.0003 * 25 * i + (6.0 if 40 < i < 46 else 0.0) for i in range(120)]), 4)
    z_nan = z.copy()
    z_nan[10:14] = np.nan
    z_nan[0] = np.nan
    z_nan[-1] = np.nan
    c("slope_trend_with_spike", 1, "overall_bottom_slope", dict(z_raw=z, spacing_m=25.0))
    c("slope_with_nans", 1, "overall_bottom_slope", dict(z_raw=z_nan, spacing_m=25.0))
    c("slope_all_nan", 1, "overall_bottom_slope", dict(z_raw=[float("nan")] * 10, spacing_m=25.0))
    c("slope_short_windows", 1, "overall_bottom_slope",
      dict(z_raw=z, spacing_m=25.0, percentile=50.0, perc_window_m=100.0, smooth_window_m=200.0))
    c("slope_falling_bed_is_made_non_decreasing", 1, "overall_bottom_slope",
      dict(z_raw=list(np.linspace(5, 0, 60)), spacing_m=25.0, perc_window_m=100.0, smooth_window_m=100.0))
    # weirs
    H, d = 5.0, 2.0
    p = dict(profile=prof)
    c("weirs_recursion", 1, "place_weirs", dict(profile=prof, place=dict(kind="recursion", weir_height_m=H, implantation_depth_m=d)))
    c("weirs_start_elevation", 1, "place_weirs", dict(profile=prof, place=dict(
        kind="recursion", weir_height_m=H, implantation_depth_m=d, start_elevation_m=1.0)))
    c("weirs_start_chainage", 1, "place_weirs", dict(profile=prof, place=dict(
        kind="recursion", weir_height_m=H, implantation_depth_m=d, start_chainage_m=2500.0)))
    c("weirs_ceiling", 1, "place_weirs", dict(profile=prof, place=dict(
        kind="recursion", weir_height_m=H, implantation_depth_m=d, max_weir_elevation_m=4.0)))
    c("weirs_no_lift_refused", 3, "place_weirs", dict(profile=prof, place=dict(
        kind="recursion", weir_height_m=2.0, implantation_depth_m=2.0)))
    c("weirs_start_chainage_past_end", 1, "place_weirs", dict(profile=prof, place=dict(
        kind="recursion", weir_height_m=H, implantation_depth_m=d, start_chainage_m=99999.0)))
    casc = [{"chainage_m": 6000.0, "pool_mwl_m": 6.5, "structure_height_m": 5.0},
            {"chainage_m": 1500.0, "pool_mwl_m": 2.0, "structure_height_m": 4.0},
            {"chainage_m": 9000.0, "pool_mwl_m": 9.0}]
    c("weirs_explicit", 1, "place_weirs", dict(profile=prof, place=dict(kind="explicit", cascade=casc)))
    c("weirs_explicit_empty", 1, "place_weirs", dict(profile=prof, place=dict(kind="explicit", cascade=[])))
    c("weirs_explicit_chainage_past_end", 1, "place_weirs",
      dict(profile=prof, place=dict(kind="explicit", cascade=[{"chainage_m": 99999.0, "pool_mwl_m": 3.0}])))
    ss = [-10.0, 0.0, 100.0, 1500.0, 1500.0001, 4000.0, 6000.0, 8999.0, 9000.0, 20000.0]
    c("levels_recursion", 1, "water_levels", dict(
        weirs=dict(profile=prof, place=dict(kind="recursion", weir_height_m=H, implantation_depth_m=d)),
        chainages=ss, depth_m=3.0))
    c("levels_explicit_downstream_sea_level", 1, "water_levels", dict(
        weirs=dict(profile=prof, place=dict(kind="explicit", cascade=casc)), chainages=ss, depth_m=3.0,
        downstream_water_level_m=0.5, overdepth_m=0.4))
    c("levels_no_weirs", 1, "water_levels", dict(
        weirs=dict(profile=prof, place=dict(kind="explicit", cascade=[])), chainages=ss, depth_m=3.0,
        downstream_water_level_m=1.2))
    # sweep
    for i in range(5):
        length = rng.r3(3000.0, 9000.0)
        slope = rng.r3(0.0001, 0.0004)
        bumps = tuple((rng.r3(500.0, length - 500.0), rng.r3(2.0, 12.0), rng.r3(60.0, 200.0)) for _ in range(rng.i(0, 3)))
        pr = river(length, slope=slope, z0=rng.r3(-3.0, 2.0), bumps=bumps, spacing_m=rng.r3(15.0, 40.0),
                   slope_percentile=rng.r3(10.0, 50.0), slope_perc_window_m=rng.r3(500.0, 3000.0),
                   slope_smooth_window_m=rng.r3(1000.0, 5000.0))
        c(f"profile_sweep_{i:02d}", 2, "sample_bed_profile", pr)
        hh = rng.r3(3.0, 8.0)
        dd = rng.r3(0.5, hh - 0.5)
        c(f"weirs_sweep_{i:02d}", 2, "place_weirs", dict(profile=pr, place=dict(
            kind="recursion", weir_height_m=hh, implantation_depth_m=dd, start_elevation_m=rng.r3(-1.0, 1.5),
            max_weir_elevation_m=rng.r3(3.0, 12.0) if i % 2 else None)))
        c(f"levels_sweep_{i:02d}", 2, "water_levels", dict(
            weirs=dict(profile=pr, place=dict(kind="recursion", weir_height_m=hh, implantation_depth_m=dd)),
            chainages=[rng.r3(0.0, length) for _ in range(6)], depth_m=rng.r3(2.0, 4.0),
            downstream_water_level_m=rng.r3(0.0, 1.0), overdepth_m=rng.r3(0.0, 0.5)))
    for i in range(8):
        zz = rnd(np.array([rng.u(-1, 12) if rng.p(0.9) else float("nan") for _ in range(rng.i(20, 150))]), 3)
        c(f"slope_sweep_{i:02d}", 2, "overall_bottom_slope", dict(
            z_raw=zz, spacing_m=rng.r3(10.0, 50.0), percentile=rng.r3(5.0, 60.0),
            perc_window_m=rng.r3(100.0, 2500.0), smooth_window_m=rng.r3(100.0, 4000.0)))
    return out
