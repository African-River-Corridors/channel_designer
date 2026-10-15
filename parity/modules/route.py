"""route: `alignment/route.py` (least-cost lateral re-route of a centreline, holding R_min).

A search-class entry point. Its discrete choices are the DP's predecessor at every state and the
corridor the sweep picks. `search_meta_for` re-runs the DP with a second tally that keeps the
runner-up value at every state, and records the smallest margin along the chosen path; it also
compares the best and second-best corridor savings."""
from __future__ import annotations

import math

import numpy as np
from shapely.geometry import LineString, Point

from ..common import Rng, case as _case, frozen, from_json, rline, rnd, search_meta, synth, terrain_from

ENTRY_POINT = "route"
R = "initial case set (spec 0001, ticket 23)"
SEARCH_FNS = {"dp_optimise", "optimise_route", "solve_at_corridor"}

HILL = {"kind": "hills", "base": 2.0, "gx": 0.0, "gy": 0.0, "hills": [[2000.0, 40.0, 20.0, 120.0]]}
FLAT = {"kind": "plane", "z0": 3.0, "gx": 0.0, "gy": 0.0}
HIGH = {"kind": "plane", "z0": 8.0, "gx": 0.0, "gy": 0.0}


def _grid_obj(rt, d: dict):
    arr = lambda k: np.array(d[k], dtype=float)  # noqa: E731
    return rt.CostGrid(arr("offsets"), arr("stations_m"), arr("centres"), arr("normals"), arr("grades"),
                       arr("vol"), arr("wet"), float(d["station_step"]), float(d["offset_step"]))


def _grade(g):
    return g


def adapter(fn: str, args: dict):
    rt = frozen("alignment.route")
    arr = lambda k: np.array(args[k], dtype=float)  # noqa: E731
    if fn == "build_cost_grid":
        kw = {k: v for k, v in args.items() if k not in ("line", "terrain", "river_polygon", "grade")}
        g = rt.build_cost_grid(args["line"], terrain_from(args["terrain"]), args["river_polygon"],
                               args["grade"], **kw)
        return g
    if fn == "radii":
        return rt.radii(arr("pts"))
    if fn == "min_radius_of_line":
        r, i = rt.min_radius_of_line(arr("pts"))
        return {"radius": r, "index": i}
    if fn == "baseline_turn_angles":
        return rt.baseline_turn_angles(arr("centres"))
    if fn == "offset_bounds":
        lo, hi = rt.offset_bounds(arr("centres"), args["target_R"])
        return {"lo": lo, "hi": hi}
    if fn == "dp_optimise":
        path, cost = rt.dp_optimise(arr("offsets"), arr("centres"), arr("cost_grid"), args["r_min"],
                                    args["station_step"], args["offset_step"])
        return {"path": path, "cost": cost}
    if fn == "build_line_from_offsets":
        return rt.build_line_from_offsets(arr("centres"), arr("normals"), arr("offset_vals"))
    if fn == "smooth_offsets":
        return rt.smooth_offsets(arr("offset_vals"), args["heading_budget"], arr("beta"), args["ds"])
    if fn == "enforce_min_radius":
        kw = {k: v for k, v in args.items() if k in ("max_iter", "taper_stations")}
        o, passes, rmin = rt.enforce_min_radius(arr("offset_vals"), arr("centres"), arr("normals"),
                                                args["target_R"], **kw)
        return {"offsets": o, "passes": passes, "min_radius": rmin}
    if fn == "approx_saving_pct":
        return rt.approx_saving_pct(_grid_obj(rt, args["grid"]), arr("cost"), arr("ovals"))
    if fn == "solve_at_corridor":
        grid = _grid_obj(rt, args["grid"])
        sm, passes, rmin, sav = rt.solve_at_corridor(grid, arr("cost"), args["corridor_half"], args["r_min"])
        return {"offsets": sm, "passes": passes, "min_radius": rmin, "saving_pct": sav}
    if fn == "optimise_route":
        kw = {k: v for k, v in args.items()
              if k not in ("line", "terrain", "river_polygon", "grade", "grid")}
        if "corridor_sweep" in kw:
            kw["corridor_sweep"] = tuple(kw["corridor_sweep"])
        grid = _grid_obj(rt, args["grid"]) if args.get("grid") else None
        return rt.optimise_route(args["line"], terrain_from(args["terrain"]), args["river_polygon"],
                                 args["grade"], grid=grid, **kw)
    raise KeyError(f"route has no parity function {fn!r}")


def _dp_with_margin(rt, offsets, centres, cost_grid, r_min, station_step, offset_step):
    """`dp_optimise` again, keeping the runner-up arrival value at every state.

    Returns the path, its cost, and the (best, runner_up) TOTAL path costs at every decision on
    that path: the winner's against the cheapest path that leaves it at that state by another
    predecessor, which costs `best + (second - best)` since the rest of the path is shared."""
    ds, heading_budget = station_step, station_step / r_min
    beta = rt.baseline_turn_angles(centres)
    n, K, step = len(centres), len(offsets), offset_step
    d_idx = np.arange(-(K - 1), K)
    phi_lut = np.arctan2(d_idx * step, ds)
    D0 = K - 1
    INF = np.inf
    dp = cost_grid[0][:, None] + cost_grid[1][None, :]
    back = np.empty((n, K, K), dtype=np.int32)
    arrive = np.full((n, K, K), INF)
    second = np.full((n, K, K), INF)
    for i in range(2, n):
        ndp = np.full((K, K), INF)
        nsec = np.full((K, K), INF)
        nb = np.empty((K, K), dtype=np.int32)
        bi = beta[i - 1]
        budget = max(heading_budget, abs(bi))
        for jp in range(K):
            row = dp[jp]
            for jc in np.where(np.isfinite(row))[0]:
                base_val = row[jc]
                phi_prev = phi_lut[D0 + (jc - jp)]
                phi_lo, phi_hi = -budget - bi + phi_prev, budget - bi + phi_prev
                d_lo = int(np.ceil(np.tan(phi_lo) * ds / step)) if abs(phi_lo) < np.pi / 2 else -(K - 1)
                d_hi = int(np.floor(np.tan(phi_hi) * ds / step)) if abs(phi_hi) < np.pi / 2 else (K - 1)
                jn_lo, jn_hi = max(0, jc + d_lo), min(K - 1, jc + d_hi)
                if jn_lo > jn_hi:
                    continue
                cand = base_val + cost_grid[i][jn_lo:jn_hi + 1]
                cur = ndp[jc, jn_lo:jn_hi + 1]
                better = cand < cur
                nsec[jc, jn_lo:jn_hi + 1] = np.where(better, cur, np.minimum(nsec[jc, jn_lo:jn_hi + 1], cand))
                if better.any():
                    idx = np.where(better)[0] + jn_lo
                    ndp[jc, idx] = cand[better]
                    nb[jc, idx] = jp
        dp, back[i], arrive[i], second[i] = ndp, nb, ndp, nsec
    flat = np.argmin(dp)
    jp_end, jc_end = np.unravel_index(flat, dp.shape)
    best_cost = float(dp[jp_end, jc_end])
    if not np.isfinite(best_cost):
        raise RuntimeError("DP found no feasible path — corridor/curvature too tight")
    path = np.empty(n, dtype=np.int32)
    path[n - 1], path[n - 2] = jc_end, jp_end
    jc, jp = jc_end, jp_end
    calls = []
    finite_end = np.sort(dp[np.isfinite(dp)].ravel())
    if len(finite_end) > 1:
        calls.append((float(finite_end[0]), float(finite_end[1])))
    for i in range(n - 1, 1, -1):
        if np.isfinite(second[i, jp, jc]):
            calls.append((best_cost, best_cost + float(second[i, jp, jc] - arrive[i, jp, jc])))
        j_prev = back[i, jp, jc]
        path[i - 2] = j_prev
        jc, jp = jp, j_prev
    return path, best_cost, calls


def _gap(pair):
    return abs(pair[0] - pair[1]) / max(abs(pair[0]), 1.0)


def search_meta_for(fn: str, args: dict) -> dict:
    """Run the search with the DP instrumented; return the closest call."""
    rt = frozen("alignment.route")
    args = from_json(args)
    pairs, decisions = [], [0]
    orig = rt.dp_optimise

    def wrapped(offsets, centres, cost_grid, r_min, station_step, offset_step):
        path, cost = orig(offsets, centres, cost_grid, r_min, station_step, offset_step)
        p2, c2, calls = _dp_with_margin(rt, offsets, centres, cost_grid, r_min, station_step, offset_step)
        assert np.array_equal(path, p2) and c2 == cost, "margin DP disagrees with dp_optimise"
        pairs.extend(calls)
        decisions[0] += len(path)
        return path, cost

    rt.dp_optimise = wrapped
    try:
        out = adapter(fn, args)
    finally:
        rt.dp_optimise = orig
    what = "DP total path cost"
    if fn == "optimise_route":  # the sweep also chooses: best corridor saving against the next
        sav = sorted((s[1] for s in out.sweep), reverse=True)
        if len(sav) > 1:
            pairs.append((sav[0], sav[1]))
            what += ", and corridor saving %"
        decisions[0] += len(out.sweep)
    if not pairs:
        return search_meta(None, None, decisions[0], what)
    best, runner = min(pairs, key=_gap)
    return search_meta(best, runner, decisions[0], what)


search_meta_fn = search_meta_for


def _straight(length):
    return LineString([(0.0, 0.0), (float(length), 0.0)])


def _grid_json(line, terrain, river, grade, **kw):
    """A cost grid built by the frozen code, as plain arrays (rounded to 6 places)."""
    rt = frozen("alignment.route")
    g = rt.build_cost_grid(line, terrain_from(terrain), river, grade, **kw)
    return {"offsets": g.offsets, "stations_m": g.stations_m, "centres": rnd(g.centres, 6),
            "normals": rnd(g.normals, 9), "grades": g.grades, "vol": rnd(g.vol, 3), "wet": rnd(g.wet, 3),
            "station_step": g.station_step, "offset_step": g.offset_step}


def _arc_pts(radius, sweep_deg, n):
    t = np.linspace(0, math.radians(sweep_deg), n)
    return rnd(np.column_stack([radius * np.sin(t), radius * (1 - np.cos(t))]), 4)


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    # --- the pieces on small point sets ---
    arc = _arc_pts(300.0, 90.0, 12)
    sine = rnd(np.column_stack([np.arange(0.0, 1500.0, 100.0), 150.0 * np.sin(np.arange(0.0, 1500.0, 100.0) / 160.0)]), 4)
    straight = np.column_stack([np.arange(0.0, 1000.0, 100.0), np.zeros(10)])
    for name, pts in [("arc", arc), ("sine", sine), ("straight", straight)]:
        c(f"radii_{name}", 1, "radii", dict(pts=pts))
        c(f"min_radius_{name}", 1, "min_radius_of_line", dict(pts=pts))
        c(f"turn_angles_{name}", 1, "baseline_turn_angles", dict(centres=pts))
        for tr in (150.0, 300.0, 900.0):
            c(f"offset_bounds_{name}_{tr:g}", 1, "offset_bounds", dict(centres=pts, target_R=tr))
    c("radii_duplicate_points", 1, "radii", dict(pts=[[0.0, 0.0], [0.0, 0.0], [10.0, 0.0], [20.0, 5.0]]))
    c("radii_two_points", 1, "radii", dict(pts=[[0.0, 0.0], [10.0, 0.0]]))
    nrm = np.column_stack([-np.gradient(sine[:, 1]), np.gradient(sine[:, 0])])
    nrm = rnd(nrm / np.hypot(*nrm.T)[:, None], 9)
    c("line_from_offsets", 1, "build_line_from_offsets",
      dict(centres=sine, normals=nrm, offset_vals=[(-1) ** i * 5.0 * i for i in range(len(sine))]))
    c("smooth_offsets", 1, "smooth_offsets", dict(
        offset_vals=[0.0, 10.0, -10.0, 20.0, -20.0, 20.0, -10.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        heading_budget=0.4, beta=np.zeros(15), ds=100.0))
    c("smooth_offsets_baseline_turning", 1, "smooth_offsets", dict(
        offset_vals=[0.0, 5.0, 15.0, 30.0, 40.0, 30.0, 15.0, 5.0, 0.0], heading_budget=0.2,
        beta=[0.0, 0.1, 0.1, 0.1, 0.0, -0.1, -0.1, -0.1, 0.0], ds=100.0))
    c("enforce_baseline_clear", 1, "enforce_min_radius", dict(
        offset_vals=[0.0, 8.0, 16.0, 24.0, 16.0, 8.0, 0.0, 0.0, 0.0, 0.0],
        centres=straight, normals=[[0.0, 1.0]] * 10, target_R=500.0))
    c("enforce_needs_passes", 1, "enforce_min_radius", dict(
        offset_vals=[0.0, 0.0, 60.0, -60.0, 60.0, -60.0, 60.0, 0.0, 0.0, 0.0],
        centres=straight, normals=[[0.0, 1.0]] * 10, target_R=900.0))
    c("enforce_unreachable_stops_at_max_iter", 1, "enforce_min_radius", dict(
        offset_vals=[0.0] * 12, centres=_arc_pts(100.0, 160.0, 12), normals=[[0.0, 1.0]] * 12,
        target_R=500.0, max_iter=50))
    c("enforce_no_taper", 1, "enforce_min_radius", dict(
        offset_vals=[0.0, 30.0, -30.0, 30.0, 0.0, 0.0], centres=straight[:6], normals=[[0.0, 1.0]] * 6,
        target_R=800.0, taper_stations=0))
    # --- the cost grid ---
    river = _straight(4000.0).buffer(20.0, cap_style="flat")
    kw = dict(bottom_width=30.0, depth=3.0, side_slope=3.0)
    line4 = _straight(4000.0)
    c("grid_hill", 1, "build_cost_grid", dict(line=line4, terrain=HILL, river_polygon=river, grade=0.0,
                                              corridor_half=60.0, station_step=200.0, offset_step=20.0, **kw))
    c("grid_flat", 1, "build_cost_grid", dict(line=_straight(2000.0), terrain=FLAT,
                                              river_polygon=_straight(2000.0).buffer(20.0, cap_style="flat"),
                                              grade=0.0, corridor_half=40.0, station_step=200.0, offset_step=20.0, **kw))
    c("grid_high_ground_default_daylight", 1, "build_cost_grid", dict(
        line=_straight(2000.0), terrain=HIGH, river_polygon=_straight(2000.0).buffer(20.0, cap_style="flat"),
        grade=0.0, corridor_half=40.0, station_step=200.0, offset_step=20.0, **kw))
    c("grid_high_ground_wide_daylight", 1, "build_cost_grid", dict(
        line=_straight(2000.0), terrain=HIGH, river_polygon=_straight(2000.0).buffer(20.0, cap_style="flat"),
        grade=0.0, corridor_half=40.0, station_step=200.0, offset_step=20.0, daylight_m=39.0, **kw))
    c("grid_synthetic_river_hill", 1, "build_cost_grid", dict(
        line=synth(length_m=2000.0)["centreline"], terrain={"kind": "synthetic_river", "params": {"length_m": 2000.0}},
        river_polygon=synth(length_m=2000.0)["polygon"], grade=-2.0, corridor_half=60.0, station_step=100.0,
        offset_step=20.0, **kw))
    c("grid_no_ground_data", 1, "build_cost_grid", dict(
        line=_straight(1000.0), terrain={"kind": "plane", "z0": "float:nan", "gx": 0.0, "gy": 0.0},
        river_polygon=_straight(1000.0).buffer(20.0, cap_style="flat"), grade=0.0, corridor_half=40.0,
        station_step=200.0, offset_step=20.0, **kw))
    # --- the DP, on grids the frozen code built ---
    g_hill = _grid_json(line4, HILL, river, 0.0, corridor_half=60.0, station_step=200.0, offset_step=20.0, **kw)
    g_flat = _grid_json(_straight(2000.0), FLAT, _straight(2000.0).buffer(20.0, cap_style="flat"), 0.0,
                        corridor_half=40.0, station_step=200.0, offset_step=20.0, **kw)
    cost_hill = np.array(g_hill["vol"])
    c("dp_hill", 1, "dp_optimise", dict(offsets=g_hill["offsets"], centres=g_hill["centres"], cost_grid=cost_hill,
                                        r_min=600.0, station_step=200.0, offset_step=20.0))
    c("dp_flat_every_path_ties", 1, "dp_optimise", dict(
        offsets=g_flat["offsets"], centres=g_flat["centres"], cost_grid=np.array(g_flat["vol"]),
        r_min=600.0, station_step=200.0, offset_step=20.0))
    c("dp_tight_budget", 1, "dp_optimise", dict(offsets=g_hill["offsets"], centres=g_hill["centres"], cost_grid=cost_hill,
                                                r_min=6000.0, station_step=200.0, offset_step=20.0))
    c("dp_infeasible_corridor", 3, "dp_optimise", dict(
        offsets=[-10.0, 0.0, 10.0], centres=[[float(i) * 100.0, 0.0] for i in range(10)],
        cost_grid=np.full((10, 3), np.inf), r_min=300.0, station_step=100.0, offset_step=10.0))
    # --- the solve and the saving ---
    c("saving_hill", 1, "approx_saving_pct", dict(
        grid=g_hill, cost=cost_hill, ovals=[0.0] * 10 + [-20.0] * 5 + [0.0] * (len(cost_hill) - 15)))
    c("saving_zero_when_offset_zero", 1, "approx_saving_pct", dict(
        grid=g_hill, cost=cost_hill, ovals=[0.0] * len(cost_hill)))
    c("solve_hill_corridor40", 1, "solve_at_corridor", dict(grid=g_hill, cost=cost_hill, corridor_half=40.0, r_min=600.0))
    c("solve_hill_corridor60", 1, "solve_at_corridor", dict(grid=g_hill, cost=cost_hill, corridor_half=60.0, r_min=600.0))
    # --- the route (tests/alignment) ---
    sl = _straight(3000.0)
    sriver = sl.buffer(20.0, cap_style="flat")
    c("flat_ground_stays_put", 1, "optimise_route", dict(
        line=sl, terrain=FLAT, river_polygon=sriver, grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
        r_min=300.0, corridor_sweep=[40.0], station_step=200.0, offset_step=20.0))
    c("hill_moves_away", 1, "optimise_route", dict(
        line=line4, terrain=HILL, river_polygon=river, grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
        r_min=600.0, corridor_sweep=[40.0, 80.0], station_step=200.0, offset_step=20.0))
    c("hill_check_chord_400", 1, "optimise_route", dict(
        line=line4, terrain=HILL, river_polygon=river, grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
        r_min=600.0, corridor_sweep=[40.0, 80.0], station_step=200.0, offset_step=20.0, check_chord_m=500.0))
    c("hill_weighted_wet_dry", 1, "optimise_route", dict(
        line=line4, terrain=HILL, river_polygon=river, grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
        r_min=600.0, corridor_sweep=[60.0], station_step=200.0, offset_step=20.0, wet_weight=2.0, dry_weight=0.5))
    c("tight_baseline_not_relaxed", 1, "optimise_route", dict(
        line=rline(np.column_stack([np.arange(0.0, 3001.0, 10.0),
                                    260.0 * np.sin(2 * math.pi * np.arange(0.0, 3001.0, 10.0) / 1500.0)])),
        terrain=HILL, river_polygon=rline(np.column_stack([
            np.arange(0.0, 3001.0, 10.0), 260.0 * np.sin(2 * math.pi * np.arange(0.0, 3001.0, 10.0) / 1500.0)
        ])).buffer(20.0, cap_style="flat"), grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
        r_min=300.0, corridor_sweep=[40.0], station_step=100.0, offset_step=20.0))
    c("precomputed_grid", 1, "optimise_route", dict(
        line=line4, terrain=HILL, river_polygon=river, grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
        r_min=900.0, corridor_sweep=[40.0, 60.0], station_step=200.0, offset_step=20.0, grid=g_hill))
    c("synthetic_river", 1, "optimise_route", dict(
        line=synth(length_m=2000.0)["centreline"], terrain={"kind": "synthetic_river", "params": {"length_m": 2000.0}},
        river_polygon=synth(length_m=2000.0)["polygon"], grade=-2.0, bottom_width=30.0, depth=3.0,
        side_slope=3.0, r_min=250.0, corridor_sweep=[40.0, 80.0], station_step=100.0, offset_step=20.0))
    for bad in (0.0, -100.0, "float:nan", "float:inf"):
        c(f"check_chord_{bad}_refused", 3, "optimise_route", dict(
            line=sl, terrain=HILL, river_polygon=sriver, grade=0.0, bottom_width=30.0, depth=3.0, side_slope=3.0,
            r_min=600.0, corridor_sweep=[40.0], station_step=200.0, offset_step=20.0, check_chord_m=bad))
    # --- sweep ---
    rng = Rng(23015)
    for i in range(10):
        length = rng.r3(2000.0, 4000.0)
        ln = _straight(length)
        rv = ln.buffer(rng.r3(15.0, 40.0), cap_style="flat")
        hills = [[rng.r3(500.0, length - 500.0), rng.r3(-80.0, 80.0), rng.r3(5.0, 30.0), rng.r3(60.0, 200.0)]
                 for _ in range(rng.i(1, 3))]
        terrain = {"kind": "hills", "base": rng.r3(1.0, 4.0), "gx": 0.0, "gy": rng.r3(-0.01, 0.01), "hills": hills}
        step = rng.r3(100.0, 250.0)
        c(f"route_sweep_{i:02d}", 2, "optimise_route", dict(
            line=ln, terrain=terrain, river_polygon=rv, grade=rng.r3(-1.0, 1.0),
            bottom_width=rng.r3(20.0, 50.0), depth=rng.r3(2.0, 4.0), side_slope=rng.r3(2.0, 4.0),
            r_min=rng.r3(300.0, 1200.0), corridor_sweep=sorted({rng.r3(30.0, 100.0) // 20 * 20 + 20
                                                               for _ in range(2)}),
            station_step=step, offset_step=20.0))
    for i in range(8):
        n = rng.i(8, 20)
        pts = rnd(np.cumsum(np.array([[rng.u(80, 120), rng.u(-40, 40)] for _ in range(n)]), axis=0))
        c(f"pieces_sweep_{i:02d}_radii", 2, "radii", dict(pts=pts))
        c(f"pieces_sweep_{i:02d}_turns", 2, "baseline_turn_angles", dict(centres=pts))
        c(f"pieces_sweep_{i:02d}_bounds", 2, "offset_bounds", dict(centres=pts, target_R=rng.r3(100.0, 800.0)))
        c(f"pieces_sweep_{i:02d}_smooth", 2, "smooth_offsets", dict(
            offset_vals=[rng.r3(-40.0, 40.0) for _ in range(n)], heading_budget=rng.r3(0.05, 0.5),
            beta=[rng.r3(-0.2, 0.2) for _ in range(n)], ds=100.0))
    return out
