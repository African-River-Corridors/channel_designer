"""synthetic: `synthetic.py` (the synthetic meandering river). Ported in ticket 24."""
from __future__ import annotations

import numpy as np

from ..common import Rng, case as _case, frozen, rnd

ENTRY_POINT = "synthetic"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    sy = frozen("synthetic")
    if fn == "sine_generated_centreline":
        return sy.sine_generated_centreline(**args)
    if fn == "synthetic_river":
        kw = {k: v for k, v in args.items() if k not in ("probe_xs", "probe_ys", "levels_at")}
        if "hill_at" in kw:
            kw["hill_at"] = tuple(kw["hill_at"])
        r = sy.synthetic_river(**kw)
        probe = None
        if "probe_xs" in args:
            probe = r.terrain(np.array(args["probe_xs"], dtype=float), np.array(args["probe_ys"], dtype=float))
        river = {k: getattr(r, k) for k in ("centreline", "left_bank", "right_bank", "polygon", "width_m",
                                            "bed_slope", "floodplain_above_wl_m", "water_level_at_mouth_m")}
        return {"river": river, "terrain_at_probes": probe,
                "water_levels": [r.water_level(s) for s in args.get("levels_at", [])]}
    raise KeyError(f"synthetic has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    px = [0.0, 100.0, 500.0, 1000.0, 1875.0, 1900.0, 2500.0, 5000.0, 6000.0]
    py = [0.0, 20.0, -200.0, 270.0, 150.0, 140.0, 0.0, 0.0, 10.0]
    c("sine_defaults", 1, "sine_generated_centreline", {})
    c("sine_short", 1, "sine_generated_centreline", dict(length_m=500.0, wavelength_m=300.0, amplitude_m=40.0, step_m=7.5))
    c("river_defaults", 1, "synthetic_river", dict(probe_xs=px, probe_ys=py, levels_at=[0.0, 1000.0, 5000.0, 6000.0]))
    c("river_short_defaults", 1, "synthetic_river",
      dict(length_m=1800.0, probe_xs=px[:6], probe_ys=py[:6], levels_at=[0.0, 900.0]))
    c("river_wide_no_hill", 1, "synthetic_river", dict(length_m=2400.0, width_m=70.0, hill_height_m=0.0,
                                                       probe_xs=px[:6], probe_ys=py[:6], levels_at=[300.0]))
    c("river_tidal_mouth", 1, "synthetic_river", dict(length_m=2000.0, water_level_at_mouth_m=-0.5, bed_slope=0.0,
                                                      probe_xs=px[:6], probe_ys=py[:6], levels_at=[10.0]))
    rng = Rng(23023)
    for i in range(5):
        length = rng.r3(800.0, 1800.0)
        c(f"river_sweep_{i:02d}", 2, "synthetic_river", dict(
            length_m=length, wavelength_m=rng.r3(500.0, 1800.0), amplitude_m=rng.r3(40.0, 300.0),
            width_m=rng.r3(15.0, 90.0), bed_depth_m=rng.r3(1.0, 4.0), floodplain_above_wl_m=rng.r3(1.0, 6.0),
            bed_slope=rng.r3(0.0, 0.0005), water_level_at_mouth_m=rng.r3(-1.0, 3.0), hill_height_m=rng.r3(0.0, 30.0),
            hill_at=[rng.r3(300.0, length - 300.0), rng.r3(-300.0, 300.0)], hill_radius_m=rng.r3(60.0, 250.0),
            probe_xs=[rng.r3(0.0, length) for _ in range(8)], probe_ys=[rng.r3(-300.0, 300.0) for _ in range(8)],
            levels_at=[rng.r3(0.0, length) for _ in range(3)]))
    for i in range(4):
        c(f"sine_sweep_{i:02d}", 2, "sine_generated_centreline", dict(
            length_m=rng.r3(200.0, 1500.0), wavelength_m=rng.r3(100.0, 1500.0), amplitude_m=rng.r3(10.0, 300.0),
            step_m=rng.r3(2.0, 25.0)))
    return out
