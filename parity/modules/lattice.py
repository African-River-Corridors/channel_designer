"""lattice: `optimise/lattice.py` (the pool-by-pool lattice alignment search).

A search-class entry point (the slowest code in the package). Its discrete choices are the
predecessor of every lattice state and the goal state the search stops at. `search_meta_for`
re-runs the A* with a runner-up tally per state (a copy of `solve_lattice` that must reproduce
its cost and line) and records the smallest margin on any state of the winning chain, and the
margin between the winning goal state and the next goal state.

Specs for the things the package takes as callables:
  half_width: a number, or {"kind": "width_fn", "loa_m", "beam_m", "draft_m", "rule_set"}
              (`make_width_fn(...).half_width`, the width profile of ticket 25)
Terrain is a terrain spec (`parity/common.py`), the river a polygon, corridors geometries.
`seconds` is not an output: it is wall-clock time and differs on every run.
"""
from __future__ import annotations

import heapq
import math

import numpy as np
from shapely.geometry import LineString, Point, Polygon

from ..common import Rng, case as _case, frozen, from_json, rline, rnd, search_meta, terrain_from

ENTRY_POINT = "lattice"
R = "initial case set (spec 0001, ticket 23)"
SEARCH_FNS = {"solve_lattice", "optimise_pool", "optimise_pools"}
FAR = Polygon([(1e6, 1e6), (1e6 + 1, 1e6), (1e6 + 1, 1e6 + 1)])   # no river anywhere


def _section(lt, d: dict):
    d = dict(d)
    hw = d.get("half_width")
    if isinstance(hw, dict):
        wp = frozen("rules.width_profile")
        d["half_width"] = wp.make_width_fn(hw["loa_m"], hw["beam_m"], hw["draft_m"],
                                           hw.get("rule_set", "pianc")).half_width
    if isinstance(d.get("deep_threshold_m"), str):
        d["deep_threshold_m"] = float(d["deep_threshold_m"])
    return lt.SectionSpec(**d)


def _cfg(lt, d: dict):
    d = dict(d)
    if "radius_multiples" in d:
        d["radius_multiples"] = tuple(d["radius_multiples"])
    return lt.LatticeConfig(**d)


def _grid(lt, g: dict):
    return lt.build_terrain_grid(terrain_from(g["terrain"]), g["river_polygon"], g["corridor"],
                                 **{k: v for k, v in g.items() if k in ("cell_m", "margin_m")})


def _result_json(res):
    """A LatticeResult without its wall-clock fields."""
    return {"converged": res.converged, "line": res.line, "cost": res.cost, "prim_seq": res.prim_seq,
            "start_pose": res.start_pose, "goal_pose": res.goal_pose, "end_err_m": res.end_err_m,
            "expansions": res.expansions, "n_states": res.n_states, "hit_cap": res.hit_cap,
            "timed_out": res.timed_out, "diagnostics": res.diagnostics}


def _prim_json(p):
    return {k: v for k, v in p.items() if k != "scorer"}


def adapter(fn: str, args: dict):
    lt = frozen("optimise.lattice")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "section_half_width_at":
        sec = _section(lt, args["section"])
        return [sec.half_width_at(r) for r in args["radii"]]
    if fn == "config":
        cfg = _cfg(lt, args["cfg"])
        return cfg
    if fn == "build_terrain_grid":
        g = _grid(lt, args["grid"])
        rows, cols = args.get("rc", [[0], [0]])
        cx, cy = g.cell_centres(rows, cols)
        sx, sy = args.get("probe_xs", []), args.get("probe_ys", [])
        return {"ground": g.ground, "wet": g.wet, "valid": g.valid, "x0": g.x0, "y0": g.y0, "cell_m": g.cell_m,
                "bounds": g.bounds, "cell_centres": [cx, cy],
                "sample_ground": g.sample_ground(np.array(sx, dtype=float), np.array(sy, dtype=float)),
                "sample_wet": g.sample_wet(np.array(sx, dtype=float), np.array(sy, dtype=float))}
    if fn == "lattice_primitives":
        return [_prim_json(p) for p in lt.lattice_primitives(_cfg(lt, args["cfg"]))]
    if fn == "score":
        g = _grid(lt, args["grid"])
        sc = lt.FieldScorer(g, args["grade_m"], args["hb_m"], _section(lt, args["section"]))
        prims = {"unit": lt._unit_straight, "cfg": None}
        out = []
        for q in args["queries"]:
            if q["prim"]["kind"] == "straight":
                n = int(q["prim"]["length_m"] / q["prim"].get("step_m", 1.0))
                L = q["prim"]["length_m"]
                prim = dict(pts=[(L * k / n, L * k / n, 0.0, 0.0) for k in range(1, n + 1)], arc_len=L)
            else:
                cfg = _cfg(lt, q["prim"]["cfg"])
                prim = lt.lattice_primitives(cfg)[q["prim"]["index"]]
            c, ok = sc.score(prim, tuple(q["pose"]))
            out.append({"cost": c, "ok": ok})
        return out
    if fn == "min_cost_per_metre":
        g = _grid(lt, args["grid"])
        sc = lt.FieldScorer(g, args["grade_m"], args["hb_m"], _section(lt, args["section"]))
        kw = {k: v for k, v in args.items() if k in ("stride_m", "max_samples")}
        return lt.min_cost_per_metre(g, sc, args["n_heading"], args["length_m"], **kw)
    if fn == "reverse_dijkstra_heuristic":
        g = _grid(lt, args["grid"])
        sc = lt.FieldScorer(g, args["grade_m"], args["hb_m"], _section(lt, args["section"]))
        h = lt.reverse_dijkstra_heuristic(g, sc, tuple(args["goal_xy"]), args["n_heading"], args["length_m"],
                                          args["stride_m"])
        return [h(x, y) for x, y in args["probes"]]
    if fn == "solve_lattice":
        g = _grid(lt, args["grid"])
        res = lt.solve_lattice(g, tuple(args["start_pose"]), tuple(args["goal_pose"]), args["grade_m"],
                               _section(lt, args["section"]), _cfg(lt, args["cfg"]))
        return _result_json(res)
    if fn == "heading_at":
        return [lt.heading_at(args["line"], s, args.get("arm_m", 20.0)) for s in args["chainages"]]
    if fn == "optimise_pool":
        kw = {k: v for k, v in args.items() if k in ("corridor_half_m", "cell_m")}
        pa = lt.optimise_pool(args["baseline"], terrain_from(args["terrain"]), args["river_polygon"],
                              args["start_m"], args["end_m"], water_level_m=args["water_level_m"],
                              depth_m=args["depth_m"], section=_section(lt, args["section"]),
                              cfg=_cfg(lt, args["cfg"]), corridor_line=args.get("corridor_line"), **kw)
        return {"start_m": pa.start_m, "end_m": pa.end_m, "water_level_m": pa.water_level_m,
                "grade_m": pa.grade_m, "result": _result_json(pa.result), "baseline_cost": pa.baseline_cost}
    if fn == "optimise_pools":
        kw = {k: v for k, v in args.items() if k in ("corridor_half_m", "cell_m", "keep_if_dearer")}
        pr = lt.optimise_pools(args["baseline"], terrain_from(args["terrain"]), args["river_polygon"],
                               [tuple(p) for p in args["pools"]], depth_m=args["depth_m"],
                               section=_section(lt, args["section"]), cfg=_cfg(lt, args["cfg"]),
                               corridor_line=args.get("corridor_line"), **kw)
        return {"line": pr.line, "n_replaced": pr.n_replaced,
                "pools": [{"start_m": p.start_m, "end_m": p.end_m, "water_level_m": p.water_level_m,
                           "grade_m": p.grade_m, "result": _result_json(p.result),
                           "baseline_cost": p.baseline_cost} for p in pr.pools]}
    raise KeyError(f"lattice has no parity function {fn!r}")


def _solve_with_margin(lt, grid, start_pose, goal_pose, grade_m, section, cfg):
    """`solve_lattice` again (same moves, same order) with a runner-up arrival cost per state.
    Returns (result, calls): calls are (best_total, runner_up_total) pairs along the winning chain
    and for the goal state against the next goal-satisfying state."""
    prims = lt.lattice_primitives(cfg)
    scorers = {}
    for p in prims:
        hb = section.half_width_at(p["R"]) + section.footprint_corr_m
        key = round(hb, 4)
        if key not in scorers:
            scorers[key] = lt.FieldScorer(grid, grade_m, hb, section)
        p["scorer"] = scorers[key]
    straight_scorer = prims[-1]["scorer"]
    gx0, gy0, gth = goal_pose
    if cfg.heuristic == "c_min":
        c_min = lt.min_cost_per_metre(grid, straight_scorer, cfg.n_heading, cfg.pitch_m)
        h = lambda x, y: c_min * math.hypot(x - gx0, y - gy0)  # noqa: E731
    elif cfg.heuristic == "reverse_dijkstra":
        h = lt.reverse_dijkstra_heuristic(grid, straight_scorer, (gx0, gy0), cfg.n_heading, cfg.pitch_m, cfg.pitch_m)
    else:
        h = lambda x, y: 0.0  # noqa: E731
    w = cfg.heuristic_weight
    bs = 2.0 * math.pi / cfg.n_heading
    head_tol = cfg.goal_head_tol_bins * bs

    def key(x, y, th, insign):
        return (int(round(x / cfg.pitch_m)), int(round(y / cfg.pitch_m)), lt._bin(th, cfg.n_heading), insign)

    def is_goal(x, y, th):
        if math.hypot(x - gx0, y - gy0) > cfg.goal_pos_tol_m:
            return False
        return abs(((th - gth + math.pi) % (2 * math.pi)) - math.pi) <= head_tol

    xmin, ymin, xmax, ymax = grid.bounds
    m = cfg.bounds_margin_m
    sk = key(*start_pose, 0)
    g, sec, pose, prev = {sk: 0.0}, {}, {sk: tuple(start_pose)}, {sk: None}
    pq = [(w * h(start_pose[0], start_pose[1]), 0.0, sk)]
    expanded, best = 0, None
    while pq:
        _, gc, k = heapq.heappop(pq)
        if gc > g.get(k, math.inf) + 1e-9:
            continue
        x, y, th = pose[k]
        if is_goal(x, y, th):
            best = k
            break
        expanded += 1
        if expanded >= cfg.max_expand:
            break
        insign = k[3]
        ct, st = math.cos(th), math.sin(th)
        for p in prims:
            sgn = p["sign"]
            if insign != 0 and sgn != 0 and sgn == -insign:
                continue
            ex, ey, eth = p["end"]
            nx, ny = x + ct * ex - st * ey, y + st * ex + ct * ey
            if not (xmin - m <= nx <= xmax + m and ymin - m <= ny <= ymax + m):
                continue
            c, ok = p["scorer"].score(p, (x, y, th))
            if not ok:
                continue
            ng = gc + c
            hn = h(nx, ny)
            if ng + hn > cfg.prune_bound:
                continue
            nk = key(nx, ny, th + eth, sgn)
            if ng < g.get(nk, math.inf) - 1e-6:
                if nk in g:
                    sec[nk] = min(sec.get(nk, math.inf), g[nk])
                g[nk] = ng
                pose[nk] = (nx, ny, th + eth)
                prev[nk] = k
                heapq.heappush(pq, (ng + w * hn, ng, nk))
            elif nk in g and nk != k:
                sec[nk] = min(sec.get(nk, math.inf), ng)
    calls = []
    if best is not None:
        total = g[best]
        k = best
        while k is not None:
            if k in sec and math.isfinite(sec[k]):
                calls.append((g[k], sec[k]))
            k = prev[k]
        others = sorted(g[o] for o, p_ in pose.items() if o != best and is_goal(*p_))
        if others:
            calls.append((total, others[0]))
    return best, g, calls


def _gap(pair):
    return abs(pair[0] - pair[1]) / max(abs(pair[0]), 1.0)


def search_meta_for(fn: str, args: dict) -> dict:
    lt = frozen("optimise.lattice")
    args = from_json(args)
    pairs, decisions = [], [0]
    orig = lt.solve_lattice

    def wrapped(grid, start_pose, goal_pose, grade_m, section, cfg):
        res = orig(grid, start_pose, goal_pose, grade_m, section, cfg)
        best, g, calls = _solve_with_margin(lt, grid, start_pose, goal_pose, grade_m, section, cfg)
        if res.converged:
            assert best is not None and abs(g[best] - res.cost) <= 1e-9 * max(1.0, res.cost), \
                "margin A* disagrees with solve_lattice"
        pairs.extend(calls)
        decisions[0] += res.n_states
        return res

    lt.solve_lattice = wrapped
    try:
        adapter(fn, args)
    finally:
        lt.solve_lattice = orig
    what = "lattice total path cost"
    if not pairs:
        return search_meta(None, None, decisions[0], what)
    best, runner = min(pairs, key=_gap)
    return search_meta(best, runner, decisions[0], what)


search_meta_fn = search_meta_for


def _hill(cx=300.0, cy=0.0, h=12.0, r=50.0, base=1.0):
    return {"kind": "hills", "base": base, "gx": 0.0, "gy": 0.0, "hills": [[cx, cy, h, r]]}


FLAT0 = {"kind": "plane", "z0": 0.0, "gx": 0.0, "gy": 0.0}
FLAT1 = {"kind": "plane", "z0": 1.0, "gx": 0.0, "gy": 0.0}
SPEC = dict(half_width=10.0, side_slope_h_per_v=2.0, rake_half_m=40.0, rake_step_m=0.5, ds_step_m=2.0)
SPEC_FAST = dict(half_width=8.0, side_slope_h_per_v=2.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0)


def _grid_spec(terrain, river=FAR, half=200.0, cell=2.0, line=((0, 0), (400, 0))):
    return {"terrain": terrain, "river_polygon": river, "corridor": LineString(line).buffer(half), "cell_m": cell}


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    river = LineString([(-100, 0), (600, 0)]).buffer(150.0)
    # --- configuration ---
    c("half_width_number", 1, "section_half_width_at", dict(section=SPEC, radii=[100.0, "float:inf"]))
    c("half_width_from_width_profile", 1, "section_half_width_at", dict(
        section=dict(SPEC, half_width={"kind": "width_fn", "loa_m": 80.0, "beam_m": 12.0, "draft_m": 2.5}),
        radii=[100.0, 255.0, 1000.0, "float:inf"]))
    c("config_defaults", 1, "config", dict(cfg=dict(r_min_m=150.0)))
    c("config_all_set", 1, "config", dict(cfg=dict(r_min_m=150.0, radius_multiples=[1.0, 2.0], pitch_m=15.0,
                                                  n_heading=24, straight_len_m=40.0, arc_step_m=5.0,
                                                  goal_pos_tol_m=9.0, goal_head_tol_bins=2.0, heuristic="none",
                                                  heuristic_weight=1.5, prune_bound=1e9, max_expand=1000,
                                                  bounds_margin_m=10.0)))
    c("config_r_min_not_positive", 3, "config", dict(cfg=dict(r_min_m=0.0)))
    c("config_radius_multiple_below_one", 3, "config", dict(cfg=dict(r_min_m=150.0, radius_multiples=[0.8, 1.0])))
    c("config_unknown_heuristic", 3, "config", dict(cfg=dict(r_min_m=150.0, heuristic="magic")))
    # --- terrain grid ---
    grid_args = dict(grid=_grid_spec(_hill(), river=river, half=60.0, cell=5.0, line=((0, 0), (200, 0))),
                     rc=[[0, 3, 10], [0, 4, 20]], probe_xs=[-70.0, 0.0, 12.3, 100.0, 140.0, 400.0],
                     probe_ys=[0.0, 0.0, 7.1, -3.3, 59.0, 0.0])
    c("grid_hill_with_river", 1, "build_terrain_grid", grid_args)
    c("grid_margin", 1, "build_terrain_grid", dict(grid_args, grid=dict(grid_args["grid"], margin_m=0.0)))
    c("grid_unknown_ground", 1, "build_terrain_grid", dict(
        grid_args, grid=_grid_spec({"kind": "plane", "z0": "float:nan", "gx": 0.0, "gy": 0.0}, half=30.0, cell=5.0,
                                   line=((0, 0), (100, 0)))))
    # --- primitives ---
    c("primitives_default", 1, "lattice_primitives", dict(cfg=dict(r_min_m=150.0)))
    c("primitives_24_bins", 1, "lattice_primitives", dict(cfg=dict(r_min_m=150.0, radius_multiples=[1.0, 2.0], n_heading=24)))
    c("primitives_custom_straight_and_step", 1, "lattice_primitives",
      dict(cfg=dict(r_min_m=120.0, radius_multiples=[1.0], n_heading=16, straight_len_m=33.0, arc_step_m=4.0)))
    # --- the edge cost (tests/optimise/test_optimise_lattice.py) ---
    sc = dict(grade_m=0.0, hb_m=10.0, section=SPEC)
    straight = {"kind": "straight", "length_m": 100.0}
    c("score_ground_at_grade", 1, "score", dict(grid=_grid_spec(FLAT0), queries=[{"prim": straight, "pose": [100, 0, 0]}], **sc))
    c("score_dry_cut", 1, "score", dict(grid=_grid_spec(FLAT1), queries=[{"prim": straight, "pose": [100, 0, 0]}], **sc))
    c("score_wet_cut_weights", 1, "score", dict(
        grid=_grid_spec(FLAT1, river=river), queries=[{"prim": straight, "pose": [100, 0, 0]}],
        grade_m=0.0, hb_m=10.0, section=dict(SPEC, wet_weight=2.0, dry_weight=7.0)))
    c("score_deep_cut_multiplier", 1, "score", dict(
        grid=_grid_spec({"kind": "plane", "z0": 30.0, "gx": 0.0, "gy": 0.0}), queries=[
            {"prim": {"kind": "straight", "length_m": 50.0}, "pose": [100, 0, 0]}],
        grade_m=0.0, hb_m=10.0, section=dict(SPEC, deep_threshold_m=20.0, deep_multiplier=2.0)))
    c("score_off_the_terrain", 1, "score", dict(
        grid=_grid_spec(FLAT1), queries=[{"prim": {"kind": "straight", "length_m": 50.0}, "pose": [5000, 0, 0]}], **sc))
    cfg_p = dict(r_min_m=150.0, radius_multiples=[1.0, 2.0], n_heading=24)
    c("score_arcs_and_straight_on_a_hill", 1, "score", dict(
        grid=_grid_spec(_hill(), river=river), grade_m=0.0, hb_m=8.0, section=SPEC_FAST,
        queries=[{"prim": {"kind": "cfg", "cfg": cfg_p, "index": i}, "pose": [200.0, 0.0, 0.2]} for i in range(5)]))
    # --- the heuristics ---
    hg = dict(grid=_grid_spec(_hill(), half=70.0, cell=5.0), grade_m=0.0, hb_m=8.0, section=SPEC_FAST, n_heading=16, length_m=15.0)
    c("c_min", 1, "min_cost_per_metre", hg)
    c("c_min_coarse_stride", 1, "min_cost_per_metre", dict(hg, stride_m=60.0, max_samples=50))
    c("c_min_no_valid_ground", 1, "min_cost_per_metre", dict(
        hg, grid=_grid_spec({"kind": "plane", "z0": "float:nan", "gx": 0.0, "gy": 0.0}, half=30.0, cell=5.0)))
    c("reverse_dijkstra", 1, "reverse_dijkstra_heuristic", dict(
        hg, goal_xy=[400.0, 0.0], stride_m=15.0, probes=[[0.0, 0.0], [200.0, 30.0], [400.0, 0.0], [300.0, 60.0],
                                                         [-500.0, 0.0], [199.0, 5.5]]))
    c("reverse_dijkstra_goal_off_valid_ground", 1, "reverse_dijkstra_heuristic", dict(
        hg, goal_xy=[900.0, 300.0], stride_m=15.0, probes=[[0.0, 0.0], [400.0, 0.0]]))
    # --- the search ---
    gh = _grid_spec(_hill(), river=FAR, half=70.0, cell=5.0)
    base_cfg = dict(r_min_m=120.0, radius_multiples=[1.0, 2.0], pitch_m=15.0, n_heading=16)
    for h in ("none", "c_min", "reverse_dijkstra"):
        c(f"solve_hill_{h}", 1, "solve_lattice", dict(
            grid=gh, start_pose=[0.0, 0.0, 0.0], goal_pose=[400.0, 0.0, 0.0], grade_m=0.0,
            section=dict(half_width=8.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0),
            cfg=dict(base_cfg, heuristic=h)))
    c("solve_expansion_cap_reports", 1, "solve_lattice", dict(
        grid=_grid_spec(_hill(), half=120.0, cell=4.0), start_pose=[0.0, 0.0, 0.0], goal_pose=[400.0, 0.0, 0.0],
        grade_m=0.0, section=SPEC, cfg=dict(r_min_m=120.0, pitch_m=10.0, n_heading=16, max_expand=5)))
    c("solve_weighted_heuristic", 1, "solve_lattice", dict(
        grid=gh, start_pose=[0.0, 0.0, 0.0], goal_pose=[400.0, 0.0, 0.0], grade_m=0.0,
        section=dict(half_width=8.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0),
        cfg=dict(base_cfg, heuristic_weight=2.0)))
    c("solve_prune_bound_makes_it_fail", 1, "solve_lattice", dict(
        grid=gh, start_pose=[0.0, 0.0, 0.0], goal_pose=[400.0, 0.0, 0.0], grade_m=0.0,
        section=dict(half_width=8.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0),
        cfg=dict(base_cfg, prune_bound=10.0)))
    c("solve_flat_ground_every_path_ties", 1, "solve_lattice", dict(
        grid=_grid_spec(FLAT1, half=70.0, cell=5.0), start_pose=[0.0, 0.0, 0.0], goal_pose=[300.0, 0.0, 0.0],
        grade_m=0.0, section=dict(half_width=8.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0),
        cfg=dict(base_cfg, heuristic="c_min")))
    c("solve_goal_off_the_grid", 1, "solve_lattice", dict(
        grid=gh, start_pose=[0.0, 0.0, 0.0], goal_pose=[4000.0, 0.0, 0.0], grade_m=0.0,
        section=dict(half_width=8.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0),
        cfg=dict(base_cfg, max_expand=300)))
    c("heading_at", 1, "heading_at", dict(line=LineString([(0, 0), (100, 0), (100, 100), (200, 150)]),
                                          chainages=[-5.0, 0.0, 50.0, 100.0, 150.0, 250.0, 1e6], arm_m=20.0))
    # --- the front door ---
    base600 = LineString([(0, 0), (600, 0)])
    fd_cfg = dict(r_min_m=150.0, radius_multiples=[1.0, 2.0, 4.0], pitch_m=20.0, n_heading=16)
    fd = dict(baseline=base600, terrain=_hill(), river_polygon=FAR, depth_m=3.0, section=SPEC_FAST, cfg=fd_cfg,
              corridor_half_m=120.0, cell_m=6.0)
    c("pool_round_a_hill", 1, "optimise_pool", dict(fd, start_m=0.0, end_m=600.0, water_level_m=3.0))
    c("pool_with_corridor_line", 1, "optimise_pool", dict(
        fd, start_m=100.0, end_m=500.0, water_level_m=3.0,
        corridor_line=LineString([(0, 0), (300, 90), (600, 0)])))
    c("pool_flat_ground_keeps_straight", 1, "optimise_pool", dict(
        fd, terrain=FLAT0, start_m=0.0, end_m=400.0, water_level_m=0.0))
    c("pool_bad_interval_refused", 3, "optimise_pool", dict(fd, start_m=500.0, end_m=100.0, water_level_m=3.0))
    c("pool_end_past_baseline_refused", 3, "optimise_pool", dict(fd, start_m=0.0, end_m=900.0, water_level_m=3.0))
    c("pools_two_pools", 1, "optimise_pools", dict(fd, pools=[[0.0, 300.0, 3.0], [300.0, 600.0, 3.5]]))
    c("pools_unsorted_and_keep_if_dearer_off", 1, "optimise_pools", dict(
        fd, pools=[[300.0, 600.0, 3.5], [0.0, 300.0, 3.0]], keep_if_dearer=False))
    c("pools_gap_between_pools_keeps_baseline", 1, "optimise_pools", dict(
        fd, pools=[[0.0, 200.0, 3.0], [400.0, 600.0, 3.0]]))
    c("pools_none", 1, "optimise_pools", dict(fd, pools=[]))
    # --- sweep ---
    rng = Rng(23025)
    for i in range(6):
        L = rng.r3(300.0, 500.0)
        hx = rng.r3(L * 0.3, L * 0.7)
        sec = dict(half_width=rng.r3(5.0, 12.0), side_slope_h_per_v=rng.r3(1.5, 3.0), rake_half_m=50.0, rake_step_m=4.0,
                   ds_step_m=8.0, wet_weight=rng.r3(0.5, 2.0), dry_weight=rng.r3(0.5, 2.0))
        cfgd = dict(r_min_m=rng.r3(100.0, 160.0), radius_multiples=[1.0, 2.0], pitch_m=rng.r3(12.0, 20.0),
                    n_heading=[12, 16, 20][i % 3], heuristic=["c_min", "none", "reverse_dijkstra"][i % 3])
        g = _grid_spec(_hill(hx, rng.r3(-30.0, 30.0), rng.r3(4.0, 15.0), rng.r3(30.0, 60.0), rng.r3(0.5, 2.0)),
                       half=70.0, cell=5.0, line=((0, 0), (L, 0)))
        c(f"solve_sweep_{i:02d}", 2, "solve_lattice", dict(
            grid=g, start_pose=[0.0, 0.0, 0.0], goal_pose=[L, 0.0, 0.0], grade_m=rng.r3(-0.5, 0.5), section=sec, cfg=cfgd))
    for i in range(6):
        c(f"score_sweep_{i:02d}", 2, "score", dict(
            grid=_grid_spec(_hill(250.0, rng.r3(-40.0, 40.0), rng.r3(2.0, 12.0), rng.r3(30.0, 70.0)), river=river,
                            half=100.0, cell=5.0),
            grade_m=rng.r3(-1.0, 1.0), hb_m=rng.r3(5.0, 12.0),
            section=dict(SPEC_FAST, wet_weight=rng.r3(0.5, 2.0), dry_weight=rng.r3(0.5, 3.0)),
            queries=[{"prim": {"kind": "cfg", "cfg": cfg_p, "index": rng.i(0, 4)},
                      "pose": [rng.r3(50.0, 300.0), rng.r3(-30.0, 30.0), rng.r3(-0.6, 0.6)]} for _ in range(4)]))
    return out
