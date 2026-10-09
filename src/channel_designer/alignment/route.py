"""Least-cost channel route: re-route a centreline laterally to reduce cut, holding R_min.

METHOD

1. COST GRID. Stations every ``station_step`` metres along a baseline centreline, times
   lateral offsets in ``±corridor_half`` at ``offset_step``. At each station ONE dense
   transect of terrain is sampled (perpendicular, ``transect_step`` spacing, wide enough that
   the template daylights for every offset) and split wet/dry. For each offset the
   trapezoidal template centred at that offset is evaluated and its cut area × station
   spacing integrated. This is the transect approximation: the cross-section is treated as
   locally representative of its chunk. It is a SEARCH heuristic, not a quantity.
2. DP. A dynamic programme over stations chooses the offset path that minimises total cost,
   with a heading-budget constraint enforcing R >= R_min on the COMBINED curvature (the
   baseline's own turn plus the slew), and a fold bound so a parallel offset cannot fold a
   bend below R_min.
3. SMOOTH + ENFORCE. A light smoothing that keeps feasibility, then Laplacian passes on the
   built geometry until its measured minimum three-point radius clears the target.
4. A corridor sweep picks the sub-corridor whose radius-enforced line saves most.

Validate a winner with a true volume integration (:mod:`channel_designer.volumes.sections`
or :mod:`channel_designer.volumes.raster`), never with the grid's own numbers.

The objective weights wet and dry cut (``wet_weight``, ``dry_weight``). They are relative
WEIGHTS, not prices: 1.0 / 1.0 minimises volume. Pricing is out of scope for this package.

Inputs are plain: a baseline ``LineString``, a terrain sampler ``terrain(xs, ys) -> z`` (NaN
where unknown), the present river polygon, and a design grade (float or ``grade(s)``).
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Callable, Optional, Sequence, Union

import numpy as np
from shapely import contains_xy
from shapely.geometry import LineString

TerrainSampler = Callable[[np.ndarray, np.ndarray], np.ndarray]
GradeLike = Union[float, Callable[[float], float]]

DEFAULT_CORRIDOR_SWEEP = (40, 60, 80, 120, 160, 220, 300)


@dataclass
class CostGrid:
    offsets: np.ndarray          # (K,) lateral offsets, m (+ = left of the baseline)
    stations_m: np.ndarray       # (N,) chainage on the baseline, m
    centres: np.ndarray          # (N, 2) baseline points
    normals: np.ndarray          # (N, 2) unit LEFT normals
    grades: np.ndarray           # (N,) design invert at each station
    vol: np.ndarray              # (N, K) transect-approx cut volume per chunk, m³
    wet: np.ndarray              # (N, K) the in-river (dredge) part of ``vol``
    station_step: float
    offset_step: float

    @property
    def dry(self) -> np.ndarray:
        return self.vol - self.wet

    def weighted(self, wet_weight: float = 1.0, dry_weight: float = 1.0) -> np.ndarray:
        return self.wet * wet_weight + self.dry * dry_weight


@dataclass
class RouteResult:
    line: LineString
    offsets: np.ndarray          # chosen lateral offset per station
    min_radius_m: float
    corridor_half_m: float
    approx_saving_pct: float     # grid-approx, vs offset 0 everywhere — a search number
    smooth_passes: int
    sweep: list = field(default_factory=list)   # (corridor, saving %, min R) per candidate


def _grade_fn(grade: GradeLike) -> Callable[[float], float]:
    if callable(grade):
        return grade
    g = float(grade)
    return lambda _s: g


def build_cost_grid(line: LineString, terrain: TerrainSampler, river_polygon, grade: GradeLike,
                    *, bottom_width: float, depth: float, side_slope: float,
                    corridor_half: float = 300.0, station_step: float = 100.0,
                    offset_step: float = 10.0, transect_step: float = 2.0,
                    daylight_m: Optional[float] = None) -> CostGrid:
    """Transect-approximation cut volume for every (station, offset).

    ``side_slope`` is H per V. The transect reaches ``corridor_half + daylight_m`` each side.
    ``daylight_m`` defaults to ``bottom_width/2 + side_slope × depth`` — where the template
    meets ground AT the waterline. Ground higher than that daylights further out, so offsets
    near the corridor edge are truncated and look cheaper than they are. Over high ground
    pass ``daylight_m = bottom_width/2 + side_slope × (max ground − grade)``.
    """
    grade_at = _grade_fn(grade)
    half_bottom = bottom_width / 2.0
    total_m = line.length
    stations_m = np.arange(0.0, total_m + 0.5 * station_step, station_step)
    stations_m = stations_m[stations_m <= total_m]
    offsets = np.arange(-corridor_half, corridor_half + offset_step, offset_step)

    daylight = (half_bottom + side_slope * depth) if daylight_m is None else float(daylight_m)
    t_half = corridor_half + daylight + transect_step
    t_off = np.arange(-t_half, t_half + transect_step, transect_step)
    n_toff = t_off.size

    centres = np.zeros((stations_m.size, 2))
    normals = np.zeros((stations_m.size, 2))
    sx = np.zeros(stations_m.size * n_toff)
    sy = np.zeros(stations_m.size * n_toff)
    for i, sm in enumerate(stations_m):
        p = line.interpolate(sm)
        p1 = line.interpolate(max(0.0, sm - 5.0))
        p2 = line.interpolate(min(total_m, sm + 5.0))
        dx, dy = (p2.x - p1.x), (p2.y - p1.y)
        n = np.hypot(dx, dy) or 1.0
        tx, ty = dx / n, dy / n
        nx, ny = -ty, tx     # unit perpendicular (left-normal)
        centres[i] = (p.x, p.y)
        normals[i] = (nx, ny)
        base = i * n_toff
        sx[base:base + n_toff] = p.x + t_off * nx
        sy[base:base + n_toff] = p.y + t_off * ny

    ground = np.asarray(terrain(sx, sy), dtype=float).reshape(stations_m.size, n_toff)
    wet = contains_xy(river_polygon, sx, sy).reshape(stations_m.size, n_toff)
    grades = np.array([grade_at(float(sm)) for sm in stations_m])

    n_off = offsets.size
    vol_grid = np.full((stations_m.size, n_off), np.nan)
    wet_grid = np.full((stations_m.size, n_off), np.nan)
    dA = transect_step * station_step
    for i in range(stations_m.size):
        g = ground[i]
        valid = np.isfinite(g)
        if valid.sum() < 5:
            vol_grid[i] = 0.0
            wet_grid[i] = 0.0
            continue
        w = wet[i]
        for k, off in enumerate(offsets):
            local = np.abs(t_off - off)
            template = grades[i] + np.maximum(0.0, local - half_bottom) / side_slope
            cut = np.where(valid, g - template, 0.0)
            np.clip(cut, 0.0, None, out=cut)
            vol_grid[i, k] = cut.sum() * dA
            wet_grid[i, k] = cut[w].sum() * dA
    return CostGrid(offsets, stations_m, centres, normals, grades, vol_grid, wet_grid,
                    float(station_step), float(offset_step))


def radii(pts) -> np.ndarray:
    """Three-point circumradius R = abc / (4·Area) at every interior vertex (inf on ends)."""
    pts = np.asarray(pts, dtype=float)
    n = len(pts)
    R = np.full(n, np.inf)
    for i in range(1, n - 1):
        A, B, C = pts[i - 1], pts[i], pts[i + 1]
        a = np.hypot(*(B - C)); b = np.hypot(*(A - C)); c = np.hypot(*(A - B))
        area = 0.5 * abs((B[0] - A[0]) * (C[1] - A[1]) - (C[0] - A[0]) * (B[1] - A[1]))
        if area < 1e-9 or a * b * c < 1e-12:
            continue
        R[i] = a * b * c / (4.0 * area)
    return R


def min_radius_of_line(pts):
    """Minimum radius of curvature (m) of the polyline and its vertex index."""
    R = radii(pts)
    imin = int(np.argmin(R))
    return float(R[imin]), imin


def baseline_turn_angles(centres) -> np.ndarray:
    """Signed per-station turn (rad, > 0 left) of the baseline between station chords."""
    n = len(centres)
    beta = np.zeros(n)
    for i in range(1, n - 1):
        a1 = np.arctan2(centres[i][1] - centres[i - 1][1], centres[i][0] - centres[i - 1][0])
        a2 = np.arctan2(centres[i + 1][1] - centres[i][1], centres[i + 1][0] - centres[i][0])
        d = a2 - a1
        beta[i] = (d + np.pi) % (2 * np.pi) - np.pi
    return beta


def offset_bounds(centres, target_R):
    """Per-station admissible offset window [o_lo, o_hi] so a PARALLEL offset cannot fold the
    alignment below ``target_R`` on the inside of a baseline bend."""
    n = len(centres)
    R = radii(centres)
    beta = baseline_turn_angles(centres)
    o_lo = np.full(n, -np.inf)
    o_hi = np.full(n, np.inf)
    for i in range(1, n - 1):
        rho = R[i]
        if not np.isfinite(rho):
            continue
        margin = rho - target_R
        if beta[i] > 1e-6:          # left turn: centre on +normal side
            o_hi[i] = margin
        elif beta[i] < -1e-6:       # right turn: centre on -normal side
            o_lo[i] = -margin
    return o_lo, o_hi


def dp_optimise(offsets, centres, cost_grid, r_min, station_step, offset_step):
    """Minimise the summed per-station cost over an offset path, subject to R >= ``r_min``.

    Heading model: the segment i->i+1 makes an extra heading phi_i = atan2(o[i+1]-o[i], ds)
    from the slew; the total turn at i is beta[i] + phi_i - phi_{i-1}, bounded by ds/R_min.
    Where the baseline itself turns more than the budget, the budget relaxes to |beta[i]| so
    the offset path may add zero curvature rather than be forced infeasible. DP over states
    (o[i-1], o[i]). Returns ``(path_indices, best_cost)``.
    """
    ds = station_step
    heading_budget = ds / r_min
    beta = baseline_turn_angles(centres)
    n = len(centres)
    K = len(offsets)
    step = offset_step

    d_idx = np.arange(-(K - 1), K)
    phi_lut = np.arctan2(d_idx * step, ds)
    D0 = K - 1

    INF = np.inf
    dp = cost_grid[0][:, None] + cost_grid[1][None, :]
    back = np.empty((n, K, K), dtype=np.int32)

    for i in range(2, n):
        ndp = np.full((K, K), INF)
        nb = np.empty((K, K), dtype=np.int32)
        bi = beta[i - 1]
        budget = max(heading_budget, abs(bi))
        for jp in range(K):
            row = dp[jp]
            finite = np.where(np.isfinite(row))[0]
            if finite.size == 0:
                continue
            for jc in finite:
                base_val = row[jc]
                phi_prev = phi_lut[D0 + (jc - jp)]
                phi_lo = -budget - bi + phi_prev
                phi_hi = budget - bi + phi_prev
                d_lo = int(np.ceil(np.tan(phi_lo) * ds / step)) if abs(phi_lo) < np.pi / 2 else -(K - 1)
                d_hi = int(np.floor(np.tan(phi_hi) * ds / step)) if abs(phi_hi) < np.pi / 2 else (K - 1)
                jn_lo = max(0, jc + d_lo)
                jn_hi = min(K - 1, jc + d_hi)
                if jn_lo > jn_hi:
                    continue
                cand = base_val + cost_grid[i][jn_lo:jn_hi + 1]
                better = cand < ndp[jc, jn_lo:jn_hi + 1]
                if better.any():
                    idx = np.where(better)[0] + jn_lo
                    ndp[jc, idx] = cand[better]
                    nb[jc, idx] = jp
        dp = ndp
        back[i] = nb

    flat = np.argmin(dp)
    jp_end, jc_end = np.unravel_index(flat, dp.shape)
    best_cost = dp[jp_end, jc_end]
    if not np.isfinite(best_cost):
        raise RuntimeError("DP found no feasible path — corridor/curvature too tight")

    path = np.empty(n, dtype=np.int32)
    path[n - 1] = jc_end
    path[n - 2] = jp_end
    jc, jp = jc_end, jp_end
    for i in range(n - 1, 1, -1):
        j_prev = back[i, jp, jc]
        path[i - 2] = j_prev
        jc, jp = jp, j_prev
    return path, float(best_cost)


def build_line_from_offsets(centres, normals, offset_vals) -> np.ndarray:
    return centres + np.asarray(offset_vals)[:, None] * normals


def _combined_turn(o, beta, ds):
    n = o.size
    phi = np.arctan2(np.diff(o), ds)
    turn = np.zeros(n)
    turn[1:-1] = beta[1:-1] + phi[1:] - phi[:-1]
    return turn


def smooth_offsets(offset_vals, heading_budget, beta, ds):
    """Gentle 3-point smoothing that keeps the DP path's bend-radius feasibility."""
    o = offset_vals.astype("float64").copy()
    for _ in range(3):
        cand = o.copy()
        cand[1:-1] = (o[:-2] + 2 * o[1:-1] + o[2:]) / 4.0
        turn = _combined_turn(cand, beta, ds)
        budget = np.maximum(heading_budget, np.abs(beta))
        bad = np.abs(turn) > budget + 1e-4
        cand[bad] = o[bad]
        o = cand
    return o


def enforce_min_radius(offset_vals, centres, normals, target_R, max_iter=600,
                       taper_stations=8):
    """Laplacian passes on the built geometry until its measured min radius clears target.

    Offsets are tapered to zero over the first/last ``taper_stations`` (the channel ties
    into fixed points at both ends), which makes zero offset the smoothing fixed point, so
    the passes converge to the baseline. Returns ``(offsets, passes, min_R)``.
    """
    o = offset_vals.astype("float64").copy()
    n = o.size
    corridor = max(abs(o.min()), abs(o.max()), 1.0)

    def apply_taper(a):
        for k in range(min(taper_stations, n // 2)):
            f = k / taper_stations
            a[k] *= f
            a[-1 - k] *= f
        return a

    o = apply_taper(o)
    passes = 0
    rmin = float(np.min(radii(centres + o[:, None] * normals)))
    for _ in range(max_iter):
        if rmin >= target_R - 1e-6:
            break
        sm = o.copy()
        sm[1:-1] = 0.25 * o[:-2] + 0.5 * o[1:-1] + 0.25 * o[2:]
        o = apply_taper(np.clip(sm, -corridor, corridor))
        rmin = float(np.min(radii(centres + o[:, None] * normals)))
        passes += 1
    return o, passes, rmin


def approx_saving_pct(grid: CostGrid, cost: np.ndarray, ovals) -> float:
    """Grid-approx cost of ``ovals`` vs the baseline (offset 0 everywhere), as a %."""
    offsets = grid.offsets
    idx = np.clip(np.round((ovals - offsets[0]) / grid.offset_step).astype(int),
                  0, offsets.size - 1)
    opt = cost[np.arange(ovals.size), idx].sum()
    base = cost[np.arange(ovals.size), int(np.argmin(np.abs(offsets)))].sum()
    return 100.0 * (base - opt) / base if base else 0.0


def solve_at_corridor(grid: CostGrid, cost: np.ndarray, corridor_half: float, r_min: float):
    """DP within ``±corridor_half`` and the fold band, then smooth to the radius floor."""
    offsets = grid.offsets
    baseline_rmin, _ = min_radius_of_line(grid.centres)
    target_R = min(r_min, baseline_rmin)
    o_lo, o_hi = offset_bounds(grid.centres, target_R)
    g = cost.copy()
    for i in range(g.shape[0]):
        bad = ((np.abs(offsets) > corridor_half + 1e-6)
               | (offsets < o_lo[i] - 1e-6) | (offsets > o_hi[i] + 1e-6))
        g[i, bad] = np.inf
    path, _ = dp_optimise(offsets, grid.centres, g, r_min, grid.station_step, grid.offset_step)
    beta = baseline_turn_angles(grid.centres)
    sm = smooth_offsets(offsets[path], grid.station_step / r_min, beta, grid.station_step)
    sm, passes, rmin = enforce_min_radius(sm, grid.centres, grid.normals, target_R)
    return sm, passes, rmin, approx_saving_pct(grid, cost, sm)


def optimise_route(line: LineString, terrain: TerrainSampler, river_polygon, grade: GradeLike,
                   *, bottom_width: float, depth: float, side_slope: float, r_min: float,
                   wet_weight: float = 1.0, dry_weight: float = 1.0,
                   corridor_sweep: Sequence[float] = DEFAULT_CORRIDOR_SWEEP,
                   station_step: float = 100.0, offset_step: float = 10.0,
                   transect_step: float = 2.0, daylight_m: Optional[float] = None,
                   grid: Optional[CostGrid] = None) -> RouteResult:
    """Least-cost lateral re-route of ``line``. See the module docstring for the method.

    Wide excursions force sharp reconnection curves that the radius enforcement then smooths
    away, so a moderate corridor usually wins; the sweep finds it.
    """
    if grid is None:
        grid = build_cost_grid(line, terrain, river_polygon, grade, bottom_width=bottom_width,
                               depth=depth, side_slope=side_slope,
                               corridor_half=float(max(corridor_sweep)),
                               station_step=station_step, offset_step=offset_step,
                               transect_step=transect_step, daylight_m=daylight_m)
    cost = grid.weighted(wet_weight, dry_weight)
    best, sweep = None, []
    for ch in corridor_sweep:
        sm, passes, rmin, sav = solve_at_corridor(grid, cost, float(ch), r_min)
        sweep.append((float(ch), float(sav), float(rmin)))
        if best is None or sav > best[1]:
            best = (float(ch), sav, sm, rmin, passes)
    ch, sav, sm, rmin, passes = best
    pts = build_line_from_offsets(grid.centres, grid.normals, sm)
    return RouteResult(line=LineString(pts), offsets=sm, min_radius_m=float(rmin),
                       corridor_half_m=ch, approx_saving_pct=float(sav),
                       smooth_passes=int(passes), sweep=sweep)
