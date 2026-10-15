"""cut_model_extras: the rest of `volumes/cut_model.py` that the `cut_model` entry point leaves out:
`normalise_model`, `local_radii`, `widened_hb_from_shape` and `cut_model_a_section_face`."""
from __future__ import annotations

import math

import numpy as np

from ..common import Rng, case as _case, frozen, rnd

ENTRY_POINT = "cut_model_extras"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    cm = frozen("volumes.cut_model")
    if fn == "normalise_model":
        return [cm.normalise_model(n) for n in args["names"]]
    if fn == "local_radii":
        return cm.local_radii(args["xs"], args["ys"], args.get("stride", 2))
    if fn == "widened_hb_from_shape":
        shape = frozen("rules.rulesets").channel_shape(args["ruleset_id"], args["vessel_standard_id"])
        kw = {"rmax": args["rmax"]} if "rmax" in args else {}
        return cm.widened_hb_from_shape(np.array(args["xy"], dtype=float), shape, **kw)
    if fn == "cut_model_a_section_face":
        kw = {k: v for k, v in args.items() if k in ("wet_slope", "model", "run_m")}
        return cm.cut_model_a_section_face(args["offsets"], args["terrain"], args["hb"], args["grade"],
                                           args["wl"], args["depth"], **kw)
    raise KeyError(f"cut_model_extras has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("model_ids", 1, "normalise_model",
      dict(names=["a", "A", "b", "B", "cut_model_a", "CUT_MODEL_B", " a ", "\tb\n", "v21", "", "Äb", "cut_model_c"]))
    xs = [float(x) for x in range(0, 300, 25)]
    ys = [60.0 * math.sin(x / 70.0) for x in xs]
    c("local_radii_sine", 1, "local_radii", dict(xs=rnd(xs, 3), ys=rnd(ys, 3)))
    c("local_radii_stride_1", 1, "local_radii", dict(xs=rnd(xs, 3), ys=rnd(ys, 3), stride=1))
    c("local_radii_straight", 1, "local_radii", dict(xs=xs, ys=[0.0] * len(xs)))
    c("local_radii_too_short", 1, "local_radii", dict(xs=[0.0, 1.0], ys=[0.0, 1.0]))
    c("local_radii_empty", 1, "local_radii", dict(xs=[], ys=[]))
    c("local_radii_duplicate_points", 1, "local_radii",
      dict(xs=[0.0, 0.0, 0.0, 10.0, 20.0, 30.0, 30.0], ys=[0.0, 0.0, 0.0, 5.0, 12.0, 20.0, 20.0]))
    R_, step = 400.0, 25.0
    t = np.arange(0, 2 * math.pi, step / R_)
    circle = rnd(np.column_stack([R_ * np.cos(t), R_ * np.sin(t)]), 4)
    for ruleset, vessel in [("PIANC-B", "CEMT_VIa"), ("PIANC-A", "CEMT_IV"), ("CN", "CN_II_motor"),
                            ("CN-reduced", "CN_I_motor")]:
        c(f"from_shape_{ruleset}_{vessel}", 1, "widened_hb_from_shape",
          dict(ruleset_id=ruleset, vessel_standard_id=vessel, xy=circle))
    c("from_shape_rmax", 1, "widened_hb_from_shape",
      dict(ruleset_id="PIANC-B", vessel_standard_id="CEMT_Va", xy=circle, rmax=300.0))
    c("from_shape_unknown_ruleset", 3, "widened_hb_from_shape",
      dict(ruleset_id="nope", vessel_standard_id="CEMT_Va", xy=circle))
    # section face, on the cut-model profiles
    DEPTH, HB, WL = 3.25, 11.2, 7.0
    GRADE = WL - DEPTH
    offs = [float(o) for o in range(-60, 61)]
    wall = [WL + 10.0 if abs(o) >= HB + 3 * DEPTH - 5.0 else WL - 5.0 for o in offs]
    knoll = [(WL + 3.0 if 0 < o and (HB + 3 * DEPTH - 4) <= o <= (HB + 3 * DEPTH + 6) else
              (WL + 60.0 if 0 < o and 40 <= o <= 55 else WL - 4.0)) for o in offs]
    gap = list(knoll)
    for k in range(95, 100):
        gap[k] = None
    for name, ter in [("wall", wall), ("knoll", knoll), ("knoll_with_gap", gap)]:
        for model in ("a", "b"):
            c(f"face_{name}_{model}", 1, "cut_model_a_section_face",
              dict(offsets=offs, terrain=ter, hb=HB, grade=GRADE, wl=WL, depth=DEPTH, model=model))
    c("face_model_b_run_30", 1, "cut_model_a_section_face",
      dict(offsets=offs, terrain=knoll, hb=HB, grade=GRADE, wl=WL, depth=DEPTH, model="b", run_m=30.0))
    c("face_wet_slope_2", 1, "cut_model_a_section_face",
      dict(offsets=offs, terrain=wall, hb=HB, grade=GRADE, wl=WL, depth=DEPTH, wet_slope=2.0))
    c("face_default_model_id", 1, "cut_model_a_section_face",
      dict(offsets=offs, terrain=wall, hb=HB, grade=GRADE, wl=WL, depth=DEPTH, model="cut_model_b"))
    rng = Rng(23016)
    for i in range(14):
        n = rng.i(30, 120)
        half = rng.r3(30.0, 80.0)
        o = [round(-half + 2 * half * j / (n - 1), 3) for j in range(n)]
        z = [None if rng.p(0.04) else rng.r3(-2.0, 14.0) for _ in range(n)]
        hb = rng.r3(5.0, 14.0)
        depth = rng.r3(2.0, 4.5)
        wl = rng.r3(5.0, 9.0)
        c(f"face_sweep_{i:02d}", 2, "cut_model_a_section_face",
          dict(offsets=o, terrain=z, hb=hb, grade=wl - depth, wl=wl, depth=depth,
               model="b" if i % 2 else "a", wet_slope=rng.r3(2.0, 4.0), run_m=rng.r3(5.0, 30.0) if i % 3 == 0 else None))
    for i in range(10):
        n = rng.i(5, 40)
        pts = rnd(np.cumsum(np.array([[rng.u(5, 40), rng.u(-20, 20)] for _ in range(n)]), axis=0))
        c(f"local_radii_sweep_{i:02d}", 2, "local_radii", dict(xs=pts[:, 0], ys=pts[:, 1], stride=rng.i(1, 4)))
        c(f"from_shape_sweep_{i:02d}", 2, "widened_hb_from_shape",
          dict(ruleset_id=["PIANC-A", "PIANC-B", "PIANC-C", "CN", "CN-conservative", "CN-reduced"][i % 6],
               vessel_standard_id=["CEMT_IV", "CEMT_Va", "CEMT_VIa", "CN_II_motor", "CN_III_convoy_2ab"][i % 5],
               xy=pts, rmax=rng.r3(300.0, 2500.0)))
    return out
