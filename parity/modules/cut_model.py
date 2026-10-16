"""cut_model: `volumes/cut_model.py` (the scalar cut-model rules). Ported in ticket 22."""
from __future__ import annotations

import math
import random

import numpy as np

from ..common import case as _case, frange, frozen

ENTRY_POINT = "cut_model"
SEED = 20261015
INITIAL = "initial case set (spec 0001, ticket 22)"


def case(cid, tier, fn, args, reason=INITIAL):
    return _case(cid, tier, fn, args, reason)


def adapter(fn: str, args: dict):
    mod = frozen("volumes.cut_model")
    if fn == "cut_model_b_daylight_limit":  # JSON keys are text; the function takes int keys
        args = dict(args, bench={int(k): v for k, v in args["bench"].items()})
    if fn in ("cut_model_a_face", "cut_model_a_bench_gate", "cut_model_b_daylight_limit",
              "section_area"):
        return getattr(mod, fn)(**args)
    if fn == "widened_hb":
        return mod.widened_hb(**dict(args, xy=np.array(args["xy"], dtype=float)))
    raise KeyError(f"cut_model has no parity function {fn!r}")


DEPTH = 3.25
HB = 2.8 * 8 / 2.0
WL = 7.0
GRADE = WL - DEPTH
R_WL = 3.0 * DEPTH
G = dict(hb=HB, grade=GRADE, wl=WL, depth=DEPTH)


def sides(xs):
    return [-1 if o < 0 else 1 for o in xs]


def knoll_profile():
    xs = frange(-120.0, 120.0, 1.0)
    o_wl = HB + R_WL
    zs = []
    for o in xs:
        if o > 0 and o_wl - 4 <= o <= o_wl + 6:
            zs.append(WL + 3.0)
        elif o > 0 and 60 <= o <= 80:
            zs.append(WL + 60.0)
        else:
            zs.append(WL - 4.0)
    return xs, zs


def prism_profile(intrude_m):
    xs = frange(-40.0, 40.0, 1.0)
    wall_start = HB + R_WL - intrude_m
    return xs, [WL + 8.0 if (o > 0 and abs(o) >= wall_start) else WL - 5.0 for o in xs]


def cases():
    out = []
    # Tier 1: named cases from tests/volumes/test_cut_model.py.
    face_named = [
        ("flat_bottom_centre", 0.0, False), ("flat_bottom_edge", HB, True),
        ("wet_batter_1m", HB + 3.0, False), ("wet_batter_1m_benched", HB + 3.0, True),
        ("wet_batter_top", HB + R_WL, True), ("no_bench_above_wl", HB + R_WL + 5.0, False),
        ("berm_1m", HB + R_WL + 1.0, True), ("berm_end", HB + R_WL + 2.0, True),
        ("face_1m_up", HB + R_WL + 2.0 + 1.5, True), ("face_far", HB + R_WL + 80.0, True),
    ]
    for name, o, bench in face_named:
        out.append(case(f"cut_model_a_face/named/{name}", 1, "cut_model_a_face",
                        dict(o=o, side_bench=bench, **G)))
    out.append(case("cut_model_a_face/named/steep_wet_slope_2", 1, "cut_model_a_face",
                    dict(o=HB + 4.0, side_bench=True, wet_slope=2.0, **G)))

    for intrude in (1.0, 5.0):
        xs, zs = prism_profile(intrude)
        out.append(case(f"cut_model_a_bench_gate/named/prism_{intrude:g}m", 1,
                        "cut_model_a_bench_gate",
                        dict(offsets=xs, terrain=zs, hb=HB, wl=WL, depth=DEPTH)))
        out.append(case(f"section_area/named/prism_{intrude:g}m_model_a", 1, "section_area",
                        dict(zs=zs, xs=xs, sidewhich=sides(xs), model="a",
                             wet=[False] * len(xs), **G)))
    xs, zs = knoll_profile()
    zs_gap = list(zs)
    for k in range(150, 156):
        zs_gap[k] = None
    out.append(case("cut_model_a_bench_gate/named/knoll", 1, "cut_model_a_bench_gate",
                    dict(offsets=xs, terrain=zs, hb=HB, wl=WL, depth=DEPTH,
                         sidewhich=sides(xs))))
    out.append(case("cut_model_a_bench_gate/named/knoll_with_dem_gap", 1,
                    "cut_model_a_bench_gate",
                    dict(offsets=xs, terrain=zs_gap, hb=HB, wl=WL, depth=DEPTH)))
    bench = {"1": True, "-1": False}
    out.append(case("cut_model_b_daylight_limit/named/knoll", 1, "cut_model_b_daylight_limit",
                    dict(offsets=xs, terrain=zs, bench=bench, **G)))
    out.append(case("cut_model_b_daylight_limit/named/knoll_run_30", 1,
                    "cut_model_b_daylight_limit",
                    dict(offsets=xs, terrain=zs, bench=bench, run_m=30.0, **G)))
    out.append(case("cut_model_b_daylight_limit/named/knoll_with_dem_gap", 1,
                    "cut_model_b_daylight_limit",
                    dict(offsets=xs, terrain=zs_gap, bench=bench, sidewhich=sides(xs), **G)))
    for m in ("a", "b"):
        wet = [False] * len(xs)
        out.append(case(f"section_area/named/knoll_model_{m}", 1, "section_area",
                        dict(zs=zs, xs=xs, sidewhich=sides(xs), model=m, wet=wet, **G)))
    xs2 = frange(-40.0, 40.0, 1.0)
    out.append(case("section_area/named/all_below_wl_model_b", 1, "section_area",
                    dict(zs=[WL - 1.0] * len(xs2), xs=xs2, sidewhich=sides(xs2), model="b",
                         wet=[True] * len(xs2), **G)))
    xs3 = frange(-60.0, 60.0, 0.5)
    out.append(case("section_area/named/flat_bed_trapezoid", 1, "section_area",
                    dict(zs=[WL - 0.5] * len(xs3), xs=xs3, sidewhich=sides(xs3), model="a",
                         wet=[True] * len(xs3), dx=0.5, **G)))
    # Model ids normalise: long ids, upper case, outer spaces.
    for mid in ("cut_model_a", "A", " cut_model_b ", "CUT_MODEL_B"):
        out.append(case(f"section_area/named/model_id_{mid.strip().lower()}_{len(mid)}", 1,
                        "section_area",
                        dict(zs=zs, xs=xs, sidewhich=sides(xs), model=mid,
                             wet=[False] * len(xs), **G)))

    R, loa, beam = 400.0, 80.0, 10.0
    t = frange(0.0, 2 * math.pi - 25.0 / R, 25.0 / R)
    circle = [[R * math.cos(a), R * math.sin(a)] for a in t]
    out.append(case("widened_hb/named/circle_r400", 1, "widened_hb",
                    dict(xy=circle, loa=loa, beam=beam, radius_multiple=3.0, width_mult=3.2,
                         c_c=0.5)))
    straight = [[25.0 * i, 0.0] for i in range(12)]
    out.append(case("widened_hb/named/straight", 1, "widened_hb",
                    dict(xy=straight, loa=loa, beam=beam, radius_multiple=3.0, width_mult=3.2,
                         c_c=0.5)))
    tight = [[60.0 * math.cos(a), 60.0 * math.sin(a)] for a in frange(0.0, 3.0, 0.4)]
    out.append(case("widened_hb/named/tighter_than_rmin_is_capped", 1, "widened_hb",
                    dict(xy=tight, loa=loa, beam=beam, radius_multiple=3.0, width_mult=3.2,
                         c_c=0.5)))
    out.append(case("widened_hb/named/rmax_makes_it_straight", 1, "widened_hb",
                    dict(xy=circle, loa=loa, beam=beam, radius_multiple=3.0, width_mult=3.2,
                         c_c=0.5, rmax=300.0)))
    out.append(case("widened_hb/named/two_points", 1, "widened_hb",
                    dict(xy=[[0.0, 0.0], [25.0, 0.0]], loa=loa, beam=beam, radius_multiple=3.0,
                         width_mult=3.2, c_c=0.5)))

    # Ground EXACTLY at the water level (ticket 44). Mutation testing of ticket 22 showed that no case had
    # ground level equal to the water level, so the strict `z > wl` could become `z >= wl` unseen.
    # Every value is a whole number or a short binary fraction, so the comparison is exact in Python
    # and in Rust (no knife edge from rounding).
    wl_reason = "kills mutant `z > wl` -> `z >= wl` in {fn} (ground exactly at the water level), ticket 44"
    xs_wl = frange(-40.0, 40.0, 1.0)

    def at_wl_profile(on_wl, others=WL - 5.0):
        """Ground well below the water level, except the given offsets, which sit exactly on it."""
        return [WL if abs(o) in on_wl else others for o in xs_wl]

    for tag, on_wl in [("single_point", {15.0}), ("flat_run", {14.0, 15.0, 16.0, 17.0, 18.0})]:
        zs_wl = at_wl_profile(on_wl)
        out.append(case(f"cut_model_a_bench_gate/named/ground_at_wl_{tag}", 1,
                        "cut_model_a_bench_gate",
                        dict(offsets=xs_wl, terrain=zs_wl, hb=HB, wl=WL, depth=DEPTH,
                             sidewhich=sides(xs_wl)),
                        wl_reason.format(fn="cut_model_a_bench_gate")))
        out.append(case(f"section_area/named/ground_at_wl_{tag}_model_a", 1, "section_area",
                        dict(zs=zs_wl, xs=xs_wl, sidewhich=sides(xs_wl), model="a",
                             wet=[False] * len(xs_wl), **G),
                        wl_reason.format(fn="section_area")))
        out.append(case(f"section_area/named/ground_at_wl_{tag}_model_b", 1, "section_area",
                        dict(zs=zs_wl, xs=xs_wl, sidewhich=sides(xs_wl), model="b",
                             wet=[False] * len(xs_wl), **G),
                        wl_reason.format(fn="section_area")))
    # Ground just above the water level on a flat run (the same profile, one centimetre up): the
    # neighbouring case on the other side of the comparison.
    zs_up = [WL + 0.5 if abs(o) in {15.0} else WL - 5.0 for o in xs_wl]
    out.append(case("cut_model_a_bench_gate/named/ground_just_above_wl_single_point", 1,
                    "cut_model_a_bench_gate",
                    dict(offsets=xs_wl, terrain=zs_up, hb=HB, wl=WL, depth=DEPTH,
                         sidewhich=sides(xs_wl)),
                    wl_reason.format(fn="cut_model_a_bench_gate")))
    # Intrusion of exactly PRISM_INTRUDE_M (2 m): hb 8, depth 4, slope 3 -> o_wl 20; ground above the
    # water level from offset 18. `intrusion > 2.0` is false, `>= 2.0` would be true.
    xs_e = frange(-40.0, 40.0, 1.0)
    zs_e = [WL + 3.0 if abs(o) >= 18.0 and abs(o) <= 20.0 else WL - 5.0 for o in xs_e]
    out.append(case("cut_model_a_bench_gate/named/intrusion_exactly_two_metres", 1,
                    "cut_model_a_bench_gate",
                    dict(offsets=xs_e, terrain=zs_e, hb=8.0, wl=WL, depth=4.0,
                         sidewhich=sides(xs_e)),
                    "kills mutant `intrusion > PRISM_INTRUDE_M` -> `>=` in cut_model_a_bench_gate "
                    "(intrusion exactly 2 m), ticket 44"))

    # Tier 2: seeded sweep.
    rng = random.Random(SEED)
    r = rng.random

    def uni(a, b):
        return a + (b - a) * r()

    def rounded(x):
        return round(x, 3)

    for i in range(24):
        hb = rounded(uni(2.0, 14.0)); depth = rounded(uni(1.5, 6.0)); wl = rounded(uni(2.0, 30.0))
        ws = rounded(uni(2.0, 4.0))
        out.append(case(f"cut_model_a_face/sweep/{i:02d}", 2, "cut_model_a_face",
                        dict(o=rounded(uni(0.0, 90.0)), side_bench=r() < 0.5, hb=hb,
                             grade=rounded(wl - depth), wl=wl, depth=depth, wet_slope=ws)))

    def random_profile(step):
        half = rounded(uni(40.0, 100.0))
        xs = frange(-half, half, step)
        base = uni(-3.0, 3.0); amp = uni(0.5, 9.0); ph = uni(0.0, 6.28); fr = uni(0.02, 0.12)
        zs = []
        for o in xs:
            z = rounded(WL + base + amp * math.sin(fr * o + ph) + uni(-0.4, 0.4))
            u = r()
            zs.append(None if u < 0.03 else (math.nan if u < 0.05 else z))
        return xs, zs

    for i in range(24):
        step = 1.0 if i % 3 else 0.5
        xs, zs = random_profile(step)
        hb = rounded(uni(5.0, 13.0)); depth = rounded(uni(2.0, 4.5)); wl = WL
        grade = wl - depth
        wet = [abs(o) <= hb + 3.0 * depth * r() * 1.2 or r() < 0.2 for o in xs]
        sw = sides(xs)
        ws = 3.0 if i % 4 else 2.0
        model = "a" if i % 2 else "b"
        g = dict(hb=hb, grade=grade, wl=wl, depth=depth, wet_slope=ws)
        out.append(case(f"section_area/sweep/{i:02d}", 2, "section_area",
                        dict(zs=zs, xs=xs, sidewhich=sw, model=model, wet=wet, dx=step, **g)))
        out.append(case(f"cut_model_a_bench_gate/sweep/{i:02d}", 2, "cut_model_a_bench_gate",
                        dict(offsets=xs, terrain=zs, hb=hb, wl=wl, depth=depth, wet_slope=ws,
                             sidewhich=sw if i % 2 else None)))
        gate_args = dict(offsets=xs, terrain=zs, **g)
        out.append(case(f"cut_model_b_daylight_limit/sweep/{i:02d}", 2,
                        "cut_model_b_daylight_limit",
                        dict(bench={"1": r() < 0.6, "-1": r() < 0.6},
                             run_m=None if i % 3 else rounded(uni(3.0, 30.0)), **gate_args)))

    for i in range(10):
        n = 8 + int(r() * 25)
        step = uni(15.0, 40.0)
        x = y = 0.0; h = uni(0.0, 6.28)
        xy = []
        for _ in range(n):
            xy.append([rounded(x), rounded(y)])
            h += uni(-0.35, 0.35)
            x += step * math.cos(h); y += step * math.sin(h)
        out.append(case(f"widened_hb/sweep/{i:02d}", 2, "widened_hb",
                        dict(xy=xy, loa=rounded(uni(40.0, 120.0)), beam=rounded(uni(6.0, 15.0)),
                             radius_multiple=rounded(uni(2.0, 5.0)),
                             width_mult=rounded(uni(2.0, 4.0)), c_c=rounded(uni(0.2, 1.0)),
                             rmax=rounded(uni(500.0, 2500.0)))))

    # Tier 3: error cases. The one refusal in this module is an unknown model name.
    base = dict(zs=[WL] * 3, xs=[-1.0, 0.0, 1.0], sidewhich=[-1, 1, 1], wet=[False] * 3, **G)
    for name, model in [("v21", "v21"), ("empty", ""), ("single_quote", "a'b"),
                        ("both_quotes", "a'b\"c"), ("backslash", "a\\b"), ("newline", "a\nb"),
                        ("tab", "a\tb"), ("unicode", "modèle"), ("number_text", "1")]:
        out.append(case(f"section_area/error/unknown_model_{name}", 3, "section_area",
                        dict(model=model, **base)))
    return out


