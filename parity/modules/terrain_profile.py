"""terrain_profile: `terrain/profile.py` (long profile of a line over a terrain sampler)."""
from __future__ import annotations

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rline, synth, terrain_from

ENTRY_POINT = "terrain_profile"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    tp = frozen("terrain.profile")
    if fn == "line_profile":
        kw = {"step_m": args["step_m"]} if "step_m" in args else {}
        p = tp.line_profile(args["line"], terrain_from(args["terrain"]), **kw)
        return {"chainage_m": p.chainage_m, "x": p.x, "y": p.y, "z": p.z, "length_m": p.length_m,
                "n_nodata": p.n_nodata, "min_z": p.min_z, "max_z": p.max_z, "ascent_m": p.ascent_m,
                "descent_m": p.descent_m, "at": [p.at(s) for s in args.get("at", [])]}
    raise KeyError(f"terrain_profile has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, args):
        out.append(_case(f"line_profile/{cid}", tier, "line_profile", args, R))

    plane = {"kind": "plane", "z0": 2.0, "gx": 0.01, "gy": 0.0}
    ln = LineString([(0, 0), (1000, 0)])
    at = [-1.0, 0.0, 33.3, 500.0, 999.9, 1000.0, 1000.1]
    c("plane_exact", 1, dict(line=ln, terrain=plane, step_m=100.0, at=at))
    c("plane_default_step", 1, dict(line=ln, terrain=plane, at=at))
    c("valley_ascent_and_descent", 1, dict(line=ln, terrain={"kind": "valley", "x0": 500.0, "run": 50.0, "depth": 1.0},
                                           step_m=50.0, at=[0.0, 500.0, 750.0]))
    c("both_ends_included_on_a_bend", 1, dict(line=LineString([(0, 0), (300, 400), (300, 1037)]), terrain=plane,
                                              step_m=100.0, at=[0.0, 250.0, 1136.9]))
    c("follows_a_bend", 1, dict(line=LineString([(0, 0), (0, 100), (100, 100)]),
                                terrain={"kind": "plane", "z0": 0.0, "gx": 0.0, "gy": 1.0}, step_m=10.0, at=[50.0, 150.0]))
    c("nodata_counted_and_skipped", 1, dict(
        line=ln, terrain={"kind": "valley", "x0": 0.0, "run": 100.0, "depth": 1.0, "gap": [300.0, 600.0]},
        step_m=100.0, at=[250.0, 450.0, 700.0]))
    c("all_nodata", 1, dict(line=ln, terrain={"kind": "plane", "z0": "float:nan", "gx": 0.0, "gy": 0.0}, step_m=250.0,
                            at=[100.0]))
    c("step_longer_than_line", 1, dict(line=LineString([(0, 0), (40, 0)]), terrain=plane, step_m=100.0))
    c("step_exact_divisor", 1, dict(line=ln, terrain=plane, step_m=250.0))
    rv = synth(length_m=2000.0)
    c("synthetic_river_centreline", 1, dict(line=rv["centreline"],
                                            terrain={"kind": "synthetic_river", "params": {"length_m": 2000.0}},
                                            step_m=30.0, at=[0.0, 700.0, 1500.0]))
    for bad in (0.0, -5.0):
        c(f"bad_step_{bad:g}", 3, dict(line=ln, terrain=plane, step_m=bad))
    c("zero_length_line", 3, dict(line=LineString([(5, 5), (5, 5)]), terrain=plane, step_m=10.0))
    c("wrong_sampler_shape", 3, dict(line=LineString([(0, 0), (100, 0)]), terrain={"kind": "wrong_shape", "n": 3},
                                     step_m=10.0))
    rng = Rng(23022)
    for i in range(14):
        n = rng.i(2, 6)
        pts = np.cumsum(np.array([[rng.u(50, 400), rng.u(-200, 200)] for _ in range(n)]), axis=0)
        line = rline(pts)
        hills = [[rng.r3(0.0, float(pts[-1][0])), rng.r3(-200.0, 200.0), rng.r3(2.0, 30.0), rng.r3(50.0, 250.0)]
                 for _ in range(rng.i(0, 3))]
        c(f"sweep_{i:02d}", 2, dict(
            line=line, terrain={"kind": "hills", "base": rng.r3(0.0, 20.0), "gx": rng.r3(-0.01, 0.01),
                                "gy": rng.r3(-0.01, 0.01), "hills": hills},
            step_m=rng.r3(5.0, 120.0), at=[rng.r3(-10.0, float(line.length) + 10.0) for _ in range(4)]))
    return out
