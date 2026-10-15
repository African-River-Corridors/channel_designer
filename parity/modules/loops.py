"""loops: `alignment/loops.py` (is a candidate a loop artefact, or a legitimately winding channel)."""
from __future__ import annotations

import math

import numpy as np

from ..common import Rng, case as _case, frozen, rnd

ENTRY_POINT = "loops"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    lp = frozen("alignment.loops")
    if fn == "constants":
        return {"REJECT_SELF_INTERSECTING": lp.REJECT_SELF_INTERSECTING,
                "TURN_LIMIT_DEG": lp.TURN_LIMIT_DEG, "LEN_RATIO_BACKSTOP": lp.LEN_RATIO_BACKSTOP}
    if fn in ("cumulative_turn_deg", "is_self_intersecting"):
        return getattr(lp, fn)(args["coords"])
    if fn in ("loop_verdict", "is_loop_artefact"):
        return getattr(lp, fn)(args["coords"], args["window_len_m"])
    raise KeyError(f"loops has no parity function {fn!r}")


def arc(radius, sweep_deg, n=400, cx=0.0, cy=0.0, t0=0.0):
    t = np.linspace(t0, t0 + math.radians(sweep_deg), n)
    return rnd(np.column_stack([cx + radius * np.cos(t), cy + radius * np.sin(t)]))


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("constants", 1, "constants", {})
    straight = np.column_stack([np.linspace(0, 1000, 50), np.zeros(50)])
    shapes = {
        "straight": (straight, 1000.0),
        "half_circle": (arc(500.0, 180.0, 120), 1500.0),
        "full_circle_crosses": (arc(300.0, 361.0, 120), 1000.0),
        "closed_ring_exact": (arc(300.0, 360.0, 120), 1000.0),
        "spiral_no_cross": (rnd(np.column_stack([
            (200.0 + 4.0 * np.arange(400)) * np.cos(np.arange(400) * 0.04),
            (200.0 + 4.0 * np.arange(400)) * np.sin(np.arange(400) * 0.04)])), 5000.0),
        "winding_window_long": (rnd(np.column_stack([np.linspace(0, 3000, 120),
                                                 200.0 * np.sin(np.linspace(0, 12 * math.pi, 120))])), 3000.0),
        "long_ratio_backstop": (rnd(np.column_stack([np.linspace(0, 1000, 120),
                                                 900.0 * np.sin(np.linspace(0, 8 * math.pi, 120))])), 400.0),
        "two_points": (np.array([[0.0, 0.0], [1.0, 1.0]]), 1.0),
        "one_point": (np.array([[0.0, 0.0]]), 1.0),
        "duplicate_points": (np.array([[0.0, 0.0], [0.0, 0.0], [10.0, 0.0], [10.0, 0.0], [10.0, 10.0]]), 20.0),
        "zero_window": (straight, 0.0),
        "three_columns": (np.column_stack([np.linspace(0, 100, 20), np.linspace(0, 50, 20), np.arange(20.0)]), 120.0),
    }
    for name, (coords, win) in shapes.items():
        for fn in ("cumulative_turn_deg", "is_self_intersecting"):
            c(f"{name}", 1, fn, dict(coords=coords))
        for fn in ("loop_verdict", "is_loop_artefact"):
            c(f"{name}", 1, fn, dict(coords=coords, window_len_m=win))
    c("direction_reversed", 1, "cumulative_turn_deg", dict(coords=arc(500.0, 180.0)[::-1]))
    c("monotone_ratio_1", 1, "loop_verdict", dict(coords=straight, window_len_m=900.0))
    c("monotone_ratio_4", 1, "loop_verdict", dict(coords=straight, window_len_m=250.0))
    rng = Rng(23007)
    for i in range(30):
        kind = i % 3
        if kind == 0:
            coords = arc(rng.r3(100.0, 800.0), rng.r3(20.0, 700.0), rng.i(40, 160))
        elif kind == 1:
            n = rng.i(20, 120)
            xs = np.linspace(0.0, rng.r3(500.0, 4000.0), n)
            coords = rnd(np.column_stack([xs, rng.r3(20.0, 400.0) * np.sin(xs * rng.r3(0.001, 0.02))]))
        else:
            n = rng.i(10, 120)
            coords = rnd(np.cumsum(np.array([[rng.u(-10, 30), rng.u(-20, 20)] for _ in range(n)]), axis=0))
        win = rng.r3(100.0, 3000.0)
        c(f"verdict_sweep_{i:02d}", 2, "loop_verdict", dict(coords=coords, window_len_m=win))
        c(f"turn_sweep_{i:02d}", 2, "cumulative_turn_deg", dict(coords=coords))
        c(f"selfx_sweep_{i:02d}", 2, "is_self_intersecting", dict(coords=coords))
    return out
