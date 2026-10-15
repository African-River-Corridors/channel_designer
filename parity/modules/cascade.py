"""cascade: `optimise/cascade.py` (the weir cascade search: an exact shortest path on a DAG).

A search-class entry point. Its choices are the predecessor at every (site, level) state and the
order of the ranked endings. `search_meta_for` re-runs the DP keeping the runner-up arrival cost
at every state, and records the smallest margin on any state of any returned cascade, and the
smallest gap between neighbours in the ranked list.

Cost callables are specs (the package prices nothing, and the Rust port has no Python lambda):
  reach:     {"kind": "bumpy", "base", "amp", "centre", "width", "decay"}
             cost(a, b, level) = trapezoid of (base + amp * exp(-((x - centre) / width)^2)) over
             9 points on [a, b], times exp(-decay * level);
             {"kind": "earthworks", "table": {...LevelTable arrays...}, "wet_weight", "dry_weight"}
  structure: {"kind": "power", "fixed", "coeff", "power"}   cost(lift) = fixed + coeff * lift ** power
"""
from __future__ import annotations

import math

import numpy as np
from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, from_json, rline, rnd, search_meta, synth, terrain_from

ENTRY_POINT = "cascade"
R = "initial case set (spec 0001, ticket 23)"
SEARCH_FNS = {"search_cascade"}


def _table(cs, d):
    return cs.LevelTable(np.array(d["chainage_m"], dtype=float), np.array(d["levels_m"], dtype=float),
                         np.array(d["dredge_m2"], dtype=float), np.array(d["excav_m2"], dtype=float),
                         float(d["station_m"]), **({"level_tol_m": d["level_tol_m"]} if "level_tol_m" in d else {}))


def _reach(cs, spec):
    if spec["kind"] == "bumpy":
        def reach(a, b, level):
            x = np.linspace(a, b, 9)
            dens = spec["base"] + spec["amp"] * np.exp(-((x - spec["centre"]) / spec["width"]) ** 2)
            return float(np.trapezoid(dens, x)) * math.exp(-spec["decay"] * level)
        return reach
    if spec["kind"] == "earthworks":
        return cs.earthworks_reach_cost(_table(cs, spec["table"]), spec.get("wet_weight", 1.0),
                                        spec.get("dry_weight", 1.0))
    raise ValueError(f"unknown reach spec {spec!r}")


def _struct(spec):
    return lambda lift: spec["fixed"] + spec["coeff"] * lift ** spec["power"]


def _cfg(cs, d):
    return cs.CascadeConfig(**d)


def adapter(fn: str, args: dict):
    cs = frozen("optimise.cascade")
    if fn == "cascade_levels":
        return cs.cascade_levels(_cfg(cs, args["cfg"]))
    if fn == "search_cascade":
        cfg = _cfg(cs, args["cfg"])
        res = cs.search_cascade(cfg, _reach(cs, args["reach"]), _struct(args["structure"]))
        return {"search": res, "best": res.best, "cascades_pools": [c.pools() for c in res.cascades],
                "cascades_explicit": [c.as_explicit_weirs(args.get("structure_height_m")) for c in res.cascades],
                "reach_cost": [c.reach_cost for c in res.cascades],
                "structure_cost": [c.structure_cost for c in res.cascades]}
    if fn == "score_cascade":
        cfg = _cfg(cs, args["cfg"])
        c = cs.score_cascade(cfg, [tuple(w) for w in args["weirs"]], _reach(cs, args["reach"]), _struct(args["structure"]))
        return {"cascade": c, "reach_cost": c.reach_cost, "structure_cost": c.structure_cost, "pools": c.pools()}
    if fn == "reach_volumes":
        t = _table(cs, args["table"])
        return [list(t.reach_volumes(a, b, lv)) for a, b, lv in args["queries"]]
    if fn == "level_index":
        t = _table(cs, args["table"])
        return [t.level_index(lv) for lv in args["levels"]]
    if fn == "earthworks_reach_cost":
        f = cs.earthworks_reach_cost(_table(cs, args["table"]), args.get("wet_weight", 1.0), args.get("dry_weight", 1.0))
        return [f(a, b, lv) for a, b, lv in args["queries"]]
    if fn == "build_level_table":
        kw = {k: v for k, v in args.items() if k not in ("line", "terrain", "river_polygon", "levels_m", "depth_m", "hb")}
        t = cs.build_level_table(args["line"], terrain_from(args["terrain"]), args["river_polygon"],
                                 args["levels_m"], args["depth_m"], args["hb"], **kw)
        return t
    raise KeyError(f"cascade has no parity function {fn!r}")


def _margin_dp(cs, cfg, reach_cost, structure_cost):
    """`search_cascade`'s DP with a runner-up tally. Returns (dp, sec, par, S, lvls, lifts, finals)."""
    sites, _ = cs._sites(cfg)
    S = [(float(cfg.start_m), "SOURCE")] + sites
    lvls = cs.cascade_levels(cfg)
    LI = {v: i for i, v in enumerate(lvls)}
    base = round(float(cfg.base_level_m), cs._LEVEL_DECIMALS)
    INF = math.inf
    dp = np.full((len(S), len(lvls)), INF)
    sec = np.full((len(S), len(lvls)), INF)
    par = np.full((len(S), len(lvls), 3), -1, dtype=int)
    dp[0, LI[base]] = 0.0
    lifts = [float(L) for L in cfg.lifts_m]
    s_cost = {L: float(structure_cost(L)) for L in lifts}
    for j in range(1, len(S)):
        for i in range(0, j):
            if S[j][0] - S[i][0] < cfg.min_spacing_m:
                continue
            for hi, h in enumerate(lvls):
                if dp[i, hi] == INF:
                    continue
                b = dp[i, hi] + float(reach_cost(S[i][0], S[j][0], h))
                for li, L in enumerate(lifts):
                    k = LI.get(round(h + L, cs._LEVEL_DECIMALS))
                    if k is None:
                        continue
                    c = b + s_cost[L]
                    if c < dp[j, k]:
                        sec[j, k] = min(sec[j, k], dp[j, k])
                        dp[j, k] = c
                        par[j, k] = (i, hi, li)
                    else:
                        sec[j, k] = min(sec[j, k], c)
    return dp, sec, par, S, lvls


def search_meta_for(fn: str, args: dict) -> dict:
    cs = frozen("optimise.cascade")
    args = from_json(args)
    cfg = _cfg(cs, args["cfg"])
    reach, struct = _reach(cs, args["reach"]), _struct(args["structure"])
    res = cs.search_cascade(cfg, reach, struct)
    dp, sec, par, S, lvls = _margin_dp(cs, cfg, reach, struct)
    pairs = []
    # neighbours in the ranked list (and the first one left out, if the search kept only k)
    wider = cs.search_cascade(_cfg(cs, dict(args["cfg"], k_best=cfg.k_best + 1)), reach, struct)
    costs = [c.total_cost for c in wider.cascades]
    pairs += [(costs[i], costs[i + 1]) for i in range(len(costs) - 1)]
    # every state on every returned cascade: the winner's arrival against the next-best arrival
    LI = {v: i for i, v in enumerate(lvls)}
    for casc in res.cascades:
        chain = []
        site_idx = {c: n for n, (c, _) in enumerate(S)}
        for w in casc.weirs:
            chain.append((site_idx[w.chainage_m], LI[round(w.pool_level_m, cs._LEVEL_DECIMALS)]))
        for j, k in chain:
            if math.isfinite(sec[j, k]):
                pairs.append((casc.total_cost, casc.total_cost + float(sec[j, k] - dp[j, k])))
    decisions = int(np.isfinite(dp).sum())
    if not pairs:
        return search_meta(res.best.total_cost if res.best else None, None, decisions, "cascade total cost")
    best, runner = min(pairs, key=lambda p: abs(p[0] - p[1]) / max(abs(p[0]), 1.0))
    return search_meta(best, runner, decisions, "cascade total cost")


search_meta_fn = search_meta_for

BUMPY = {"kind": "bumpy", "base": 100.0, "amp": 400.0, "centre": 2500.0, "width": 500.0, "decay": 0.4}
POWER = {"kind": "power", "fixed": 3.0e5, "coeff": 1.5e5, "power": 1.5}
CFG = dict(candidate_sites_m=[700, 1200, 1900, 2600, 3300, 4100], lifts_m=[1.0, 1.5, 2.5], terminal_m=5000.0,
           terminal_min_level_m=3.0, min_spacing_m=800.0, level_cap_m=6.0)


def _table_for(levels, n_st=20, seed=1):
    r = np.random.RandomState(seed)  # numpy's legacy generator is frozen across versions
    ch = np.arange(n_st) * 250.0
    return {"chainage_m": ch, "levels_m": levels, "station_m": 250.0,
            "dredge_m2": rnd(r.rand(n_st, len(levels)) * 80.0, 2), "excav_m2": rnd(r.rand(n_st, len(levels)) * 120.0, 2)}


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    # --- named: tests/optimise/test_optimise_cascade.py ---
    c("levels_default", 1, "cascade_levels", dict(cfg=CFG))
    c("levels_no_cap_default", 1, "cascade_levels", dict(cfg={k: v for k, v in CFG.items() if k != "level_cap_m"}))
    c("levels_base_offset", 1, "cascade_levels", dict(cfg=dict(CFG, base_level_m=0.25, lifts_m=[0.75, 1.25])))
    c("search_golden", 1, "search_cascade", dict(cfg=CFG, reach=BUMPY, structure=POWER))
    c("search_k_best_1", 1, "search_cascade", dict(cfg=dict(CFG, k_best=1), reach=BUMPY, structure=POWER))
    c("search_k_best_20", 1, "search_cascade", dict(cfg=dict(CFG, k_best=20), reach=BUMPY, structure=POWER,
                                                    structure_height_m=6.0))
    c("search_site_ids", 1, "search_cascade",
      dict(cfg=dict(CFG, site_ids=["a", "b", "c", "d", "e", "f"]), reach=BUMPY, structure=POWER))
    c("search_sites_beyond_terminal_dropped", 1, "search_cascade", dict(
        cfg=dict(CFG, candidate_sites_m=[700, 1200, 1900, 2600, 3300, 4100, 5200, 6000, 0, -50]), reach=BUMPY, structure=POWER))
    c("search_start_offset", 1, "search_cascade", dict(cfg=dict(CFG, start_m=500.0), reach=BUMPY, structure=POWER))
    c("search_no_spacing", 1, "search_cascade", dict(cfg=dict(CFG, min_spacing_m=0.0), reach=BUMPY, structure=POWER))
    c("search_no_feasible_cascade", 1, "search_cascade",
      dict(cfg=dict(CFG, candidate_sites_m=[700], lifts_m=[1.0], terminal_min_level_m=4.0, level_cap_m=5.0),
           reach=BUMPY, structure=POWER))
    c("search_base_level_offset", 1, "search_cascade", dict(cfg=dict(CFG, base_level_m=1.0, terminal_min_level_m=4.0,
                                                                     level_cap_m=7.0), reach=BUMPY, structure=POWER))
    lv = [0.0, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0]
    tb = _table_for(lv)
    c("search_earthworks_cost", 1, "search_cascade", dict(
        cfg=dict(candidate_sites_m=[600, 1500, 2400, 3300, 4200], lifts_m=[1.0, 2.0], terminal_m=5000.0,
                 terminal_min_level_m=3.0, min_spacing_m=700.0, level_cap_m=5.0),
        reach={"kind": "earthworks", "table": _table_for([0.0, 1.0, 2.0, 3.0, 4.0, 5.0]), "wet_weight": 2.0, "dry_weight": 1.0},
        structure={"kind": "power", "fixed": 50.0, "coeff": 10.0, "power": 1.0}))
    c("score_reproduces_search", 1, "score_cascade", dict(cfg=CFG, weirs=[[700.0, 1.5], [1900.0, 1.5]], reach=BUMPY, structure=POWER))
    c("score_unsorted_weirs", 1, "score_cascade", dict(cfg=CFG, weirs=[[2600.0, 2.5], [700.0, 1.0]], reach=BUMPY, structure=POWER))
    c("score_no_weirs", 1, "score_cascade", dict(cfg=CFG, weirs=[], reach=BUMPY, structure=POWER))
    c("table_reach_volumes", 1, "reach_volumes",
      dict(table=tb, queries=[[0.0, 5000.0, 2.0], [250.0, 1000.0, 3.0], [900.0, 900.0, 1.0], [-100.0, 300.0, 0.0],
                              [4000.0, 9000.0, 6.0]]))
    c("table_reach_volumes_level_tolerance", 1, "reach_volumes",
      dict(table=tb, queries=[[0.0, 1000.0, 2.0000004]]))
    c("table_level_index", 1, "level_index", dict(table=tb, levels=[0.0, 1.5, 3.0, 6.0, 1.9999995]))
    c("table_level_not_in_table_refused", 3, "level_index", dict(table=tb, levels=[2.2]))
    c("table_reach_cost_weights", 1, "earthworks_reach_cost",
      dict(table=tb, queries=[[0.0, 2500.0, 2.0], [500.0, 3000.0, 4.0]], wet_weight=3.0, dry_weight=0.5))
    rv = synth(length_m=1500.0)
    T = {"kind": "synthetic_river", "params": {"length_m": 1500.0}}
    c("level_table_synthetic_river", 1, "build_level_table", dict(
        line=rv["centreline"], terrain=T, river_polygon=rv["polygon"], levels_m=[2.0, 0.5, 1.0], depth_m=3.0, hb=11.0,
        station_m=75.0, maxw_m=50.0))
    c("level_table_relabelled_chainage", 1, "build_level_table", dict(
        line=rv["centreline"], terrain=T, river_polygon=rv["polygon"], levels_m=[0.5, 1.0], depth_m=3.0, hb=11.0,
        station_m=300.0, maxw_m=40.0, chainage_m=[10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0]))
    c("level_table_model_b", 1, "build_level_table", dict(
        line=rv["centreline"], terrain=T, river_polygon=rv["polygon"], levels_m=[0.5, 1.0], depth_m=3.0, hb=11.0,
        station_m=300.0, maxw_m=40.0, model="b"))
    c("level_table_wrong_chainage_count_refused", 3, "build_level_table", dict(
        line=rv["centreline"], terrain=T, river_polygon=rv["polygon"], levels_m=[0.5], depth_m=3.0, hb=11.0,
        station_m=300.0, maxw_m=40.0, chainage_m=[1.0, 2.0]))
    # --- the guards ---
    c("no_lifts_refused", 3, "cascade_levels", dict(cfg=dict(CFG, lifts_m=[])))
    c("zero_lift_refused", 3, "cascade_levels", dict(cfg=dict(CFG, lifts_m=[1.0, 0.0])))
    c("negative_lift_refused", 3, "cascade_levels", dict(cfg=dict(CFG, lifts_m=[-1.0])))
    c("site_ids_mismatch_refused", 3, "cascade_levels", dict(cfg=dict(CFG, site_ids=["a"])))
    c("terminal_above_cap_refused", 3, "cascade_levels", dict(cfg=dict(CFG, terminal_min_level_m=7.0)))
    # --- sweep ---
    rng = Rng(23024)
    for i in range(10):
        n_sites = rng.i(3, 8)
        sites = sorted({rng.r3(300.0, 4800.0) for _ in range(n_sites)})
        lifts = sorted({rng.r3(0.5, 3.0) for _ in range(rng.i(1, 3))})
        term_lvl = rng.r3(2.0, 5.0)
        cfg = dict(candidate_sites_m=sites, lifts_m=lifts, terminal_m=5000.0, terminal_min_level_m=term_lvl,
                   min_spacing_m=rng.r3(0.0, 900.0), base_level_m=rng.r3(0.0, 1.0), k_best=rng.i(1, 6))
        reach = dict(BUMPY, base=rng.r3(50.0, 200.0), amp=rng.r3(100.0, 600.0), centre=rng.r3(1000.0, 4000.0),
                     width=rng.r3(200.0, 900.0), decay=rng.r3(0.1, 0.8))
        struct = dict(POWER, fixed=rng.r3(1e5, 5e5), coeff=rng.r3(5e4, 2e5), power=rng.r3(1.0, 2.0))
        c(f"search_sweep_{i:02d}", 2, "search_cascade", dict(cfg=cfg, reach=reach, structure=struct))
        c(f"score_sweep_{i:02d}", 2, "score_cascade", dict(
            cfg=cfg, weirs=[[rng.r3(300.0, 4800.0), lifts[0]] for _ in range(rng.i(1, 3))], reach=reach, structure=struct))
    return out
