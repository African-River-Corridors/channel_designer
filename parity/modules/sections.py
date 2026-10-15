"""sections: `volumes/sections.py` (cross-section earthworks along an alignment)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rline, synth, terrain_from

ENTRY_POINT = "sections"
R = "initial case set (spec 0001, ticket 23)"


def _hb(spec):
    if isinstance(spec, dict) and spec.get("kind") == "widened_from_shape":
        cm = frozen("volumes.cut_model")
        shape = frozen("rules.rulesets").channel_shape(spec["ruleset_id"], spec["vessel_standard_id"])
        return lambda xy: cm.widened_hb_from_shape(xy, shape)
    return spec


def _wl(spec):
    if isinstance(spec, dict) and spec.get("kind") == "linear":
        return lambda s: spec["wl0"] + spec["slope"] * s
    return spec


def _tag(spec):
    if spec is None:
        return None
    return lambda s: spec["before"] if s < spec["at"] else spec["after"]


def _section_json(st):
    d = {k: getattr(st, k) for k in ("chainage_m", "x", "y", "heading_rad", "hb", "wl", "grade", "depth",
                                       "model", "dredge_area_m2", "excav_area_m2", "max_face_h_m",
                                       "bench_left", "bench_right", "tag")}
    return d


def adapter(fn: str, args: dict):
    sc = frozen("volumes.sections")
    if fn == "station_frame":
        d, xy, th = sc.station_frame(args["line"], args.get("station_m", 25.0))
        return {"chainages": d, "xy": xy, "headings": th}
    if fn == "trapezoid_area":
        return sc.trapezoid_area(**args)
    if fn == "section_volumes":
        kw = {k: v for k, v in args.items() if k in (
            "model", "wet_slope", "station_m", "maxw_m", "dx_m", "daylight_run_m", "chunk")}
        sv = sc.section_volumes(args["line"], terrain_from(args["terrain"]), args["river_polygon"],
                                _wl(args["wl"]), args["depth"], _hb(args["hb"]) if not isinstance(args["hb"], list)
                                else np.array(args["hb"], dtype=float), tag=_tag(args.get("tag")), **kw)
        detail = [{"index": i, "offsets": sv.stations[i].offsets, "terrain": sv.stations[i].terrain,
                   "wet": sv.stations[i].wet, "cutface": sv.stations[i].cutface}
                  for i in args.get("detail_stations", [])]
        return {"station_m": sv.station_m, "dredge_m3": sv.dredge_m3, "excav_m3": sv.excav_m3,
                "total_m3": sv.total_m3, "max_face_h_m": sv.max_face_h_m, "by_tag": sv.by_tag(),
                "stations": [_section_json(s) for s in sv.stations], "detail": detail}
    raise KeyError(f"sections has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    line = rline([(0, 0), (100, 0), (200, 40), (300, 40), (420, 100)])
    for st in (25.0, 40.0, 150.0):
        c(f"frame_{st:g}", 1, "station_frame", dict(line=line, station_m=st))
    c("frame_default", 1, "station_frame", dict(line=LineString([(0, 0), (60, 0)])))
    for hb, d, s in [(11.2, 3.25, 3.0), (0.0, 3.0, 3.0), (20.0, 2.0, 0.0), (5.0, 0.0, 3.0)]:
        c(f"trapezoid_{hb:g}_{d:g}_{s:g}", 1, "trapezoid_area", dict(hb=hb, depth=d, side_slope=s))
    rv = synth(length_m=1500.0)
    T = {"kind": "synthetic_river", "params": {"length_m": 1500.0}}
    base = dict(line=rv["centreline"], terrain=T, river_polygon=rv["polygon"], wl=6.0, depth=3.25, hb=11.2,
                maxw_m=60.0, station_m=50.0, detail_stations=[3, 14])
    c("synthetic_model_a", 1, "section_volumes", dict(base, model="a"))
    c("synthetic_model_b", 1, "section_volumes", dict(base, model="b"))
    c("synthetic_model_b_run_30", 1, "section_volumes", dict(base, model="b", daylight_run_m=30.0))
    c("wet_slope_2_dx_2", 1, "section_volumes", dict(base, wet_slope=2.0, dx_m=2.0))
    c("chunked", 1, "section_volumes", dict(base, chunk=4))
    c("hb_per_station", 1, "section_volumes",
      dict(base, hb=[8.0 + 0.2 * i for i in range(len(np.arange(0, rv["centreline"].length // 50 + 1)))]))
    c("hb_from_ruleset_shape", 1, "section_volumes",
      dict(base, hb={"kind": "widened_from_shape", "ruleset_id": "PIANC-B", "vessel_standard_id": "CEMT_Va"}))
    c("water_level_along_chainage", 1, "section_volumes", dict(base, wl={"kind": "linear", "wl0": 6.0, "slope": 0.0004}))
    c("tags", 1, "section_volumes", dict(base, tag={"kind": "split", "at": 500.0, "before": "straight", "after": "complex"}))
    c("default_station_and_dx", 1, "section_volumes", dict(
        line=rv["centreline"], terrain=T, river_polygon=rv["polygon"], wl=6.0, depth=3.25, hb=11.2, maxw_m=40.0))
    c("flat_ground_at_water_level", 1, "section_volumes", dict(
        line=LineString([(0, 0), (500, 0)]), terrain={"kind": "plane", "z0": 3.0, "gx": 0.0, "gy": 0.0},
        river_polygon=LineString([(0, 0), (500, 0)]).buffer(15.0, cap_style="flat"), wl=3.0, depth=3.0, hb=10.0,
        maxw_m=50.0, station_m=50.0, detail_stations=[2]))
    c("no_ground_data", 1, "section_volumes", dict(
        line=LineString([(0, 0), (500, 0)]), terrain={"kind": "plane", "z0": "float:nan", "gx": 0.0, "gy": 0.0},
        river_polygon=LineString([(0, 0), (500, 0)]).buffer(15.0, cap_style="flat"), wl=3.0, depth=3.0, hb=10.0,
        maxw_m=30.0, station_m=100.0))
    c("unknown_model_refused", 3, "section_volumes", dict(base, model="v21"))
    c("hb_wrong_length_refused", 3, "section_volumes", dict(base, hb=[10.0, 11.0]))
    rng = Rng(23018)
    for i in range(6):
        rv = synth(length_m=rng.r3(800.0, 1800.0), wavelength_m=rng.r3(500.0, 1200.0), amplitude_m=rng.r3(50.0, 220.0),
                   width_m=rng.r3(25.0, 70.0), hill_height_m=rng.r3(0.0, 25.0), hill_radius_m=rng.r3(80.0, 220.0),
                   hill_at=(rng.r3(300.0, 700.0), rng.r3(-120.0, 120.0)))
        c(f"sweep_{i:02d}", 2, "section_volumes", dict(
            line=rv["centreline"], terrain={"kind": "hills", "base": rng.r3(2.0, 8.0), "gx": rng.r3(-0.002, 0.002),
                                            "gy": rng.r3(-0.01, 0.01),
                                            "hills": [[rng.r3(100.0, 700.0), rng.r3(-100.0, 100.0), rng.r3(2.0, 20.0),
                                                       rng.r3(40.0, 200.0)] for _ in range(rng.i(1, 3))]},
            river_polygon=rv["polygon"], wl=rng.r3(3.0, 7.0), depth=rng.r3(2.0, 4.0), hb=rng.r3(5.0, 14.0),
            model="a" if i % 2 else "b", wet_slope=rng.r3(2.0, 4.0), station_m=rng.r3(25.0, 80.0),
            maxw_m=rng.r3(30.0, 70.0), dx_m=rng.r3(0.5, 2.0), detail_stations=[0, 2],
            daylight_run_m=rng.r3(5.0, 30.0) if i % 3 == 0 else None))
    for i in range(6):
        c(f"trapezoid_sweep_{i:02d}", 2, "trapezoid_area",
          dict(hb=rng.r3(0.0, 30.0), depth=rng.r3(0.0, 8.0), side_slope=rng.r3(0.0, 6.0)))
        c(f"frame_sweep_{i:02d}", 2, "station_frame",
          dict(line=rline([(0, 0), (rng.r3(100, 400), rng.r3(-50, 50)), (rng.r3(500, 900), rng.r3(-80, 80))]),
               station_m=rng.r3(10.0, 120.0)))
    return out
