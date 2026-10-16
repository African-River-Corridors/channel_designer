"""Pool-by-pool lattice alignment search: the least-cost channel through each pool that never
bends tighter than R_min.

Per pool (the reach between two weirs, one water level), search an (x, y, heading) state
lattice with A*:

- **State** — position snapped to ``pitch_m``, heading snapped to one of ``n_heading`` bins,
  plus the turn direction that arrived (so an arc may not reverse straight into the opposite
  arc; a straight must sit between them).
- **Edges** — left and right arcs at ``r_min × radius_multiples``, each turning EXACTLY one
  heading bin (so forward integration lands on bin headings and never drifts), plus one
  straight. R_min holds BY CONSTRUCTION; no radius check after the fact.
- **Edge cost** — the earthwork field of a trapezoidal channel swept along the primitive
  (:class:`FieldScorer`): fairway cut to grade everywhere; side-slope cut only in sections
  where the fairway bites dry ground, and only on dry cells; wet and dry weighted separately;
  an optional deep-cut multiplier on dry cut. Weights, not prices: 1.0 / 1.0 minimises volume.
- **Heuristic** — ``"c_min"``: straight-line distance × the cheapest per-metre cost anywhere in
  the corridor; ``"reverse_dijkstra"``: a relaxed 2-D cost-to-go from the goal (tighter, but
  an 8-connected grid can slightly over-estimate); ``"none"``: plain Dijkstra.
- **Endpoints** — pinned to the baseline's position and heading at both pool ends, so each
  result splices back into the baseline. The goal is reached within ``goal_pos_tol_m``
  (default half a pitch) and one heading bin; ``end_err_m`` reports the miss.

The search cost is a SEARCH objective. Re-measure the winner with
:func:`channel_designer.volumes.sections.section_volumes` (or ``volumes.raster``) before you
quote a quantity.

Terrain comes in through the package's sampler interface ``terrain(xs, ys) -> z`` (NaN where
unknown). It is sampled once onto a regular grid over the corridor (:func:`build_terrain_grid`);
despiking a DEM is the caller's job before that.
"""
from __future__ import annotations

from .. import _rust

import heapq
import dataclasses
import math
import time
from dataclasses import dataclass, field
from typing import Callable, List, Optional, Sequence, Tuple, Union

import numpy as np
from shapely.geometry import LineString

from ..volumes.sections import TerrainSampler

HalfWidth = Union[float, Callable[[float], float]]
Pose = Tuple[float, float, float]


# =====================================================================================
# Configuration
# =====================================================================================

@dataclass
class SectionSpec:
    """The channel the search sweeps, and how its cut is weighted.

    ``half_width`` is the bottom half-width: a number, or ``half_width(radius_m)`` so arcs are
    widened in bends (e.g. ``make_width_fn(...).half_width``). Each arc is costed at the
    half-width for its own radius; the straight at ``half_width(inf)``.
    """

    half_width: HalfWidth
    side_slope_h_per_v: float = 2.0       # template side slope above the bottom edge
    footprint_corr_m: float = 0.0         # added to the half-width (e.g. to match a
                                          # rasterised volume integrator's footprint)
    wet_weight: float = 1.0               # per m³ cut inside the river polygon
    dry_weight: float = 1.0               # per m³ cut outside it
    deep_threshold_m: float = math.inf    # dry cut deeper than this ...
    deep_multiplier: float = 1.0          # ... is weighted by this as well
    rake_half_m: float = 250.0            # transverse reach of each cross-section
    rake_step_m: float = 4.0              # transverse sampling step
    ds_step_m: float = 8.0                # cross-section spacing along a primitive
    min_valid_fraction: float = 0.6       # fairway samples that must be on known terrain

    def half_width_at(self, radius_m: float) -> float:
        hw = self.half_width
        return float(hw(radius_m) if callable(hw) else hw)


@dataclass
class LatticeConfig:
    """How the lattice is built and searched. Every field can change the answer."""

    r_min_m: float
    radius_multiples: Sequence[float] = (1.0, 1.5, 3.0)
    pitch_m: float = 12.0
    n_heading: int = 16
    straight_len_m: Optional[float] = None    # default 2 × pitch
    arc_step_m: float = 10.0                  # sampling step along each primitive
    goal_pos_tol_m: Optional[float] = None    # default half a pitch
    goal_head_tol_bins: float = 1.0
    heuristic: str = "c_min"                  # "c_min" | "reverse_dijkstra" | "none"
    heuristic_weight: float = 1.0             # > 1 is weighted A*: faster, not optimal
    prune_bound: float = math.inf             # drop states with g + h above this
    max_expand: int = 4_000_000
    wall_cap_s: Optional[float] = None
    bounds_margin_m: float = 20.0

    def __post_init__(self) -> None:
        if self.r_min_m <= 0:
            raise ValueError("r_min_m must be > 0")
        if min(self.radius_multiples) < 1.0:
            raise ValueError("radius_multiples must all be >= 1 so no arc is tighter than R_min")
        if self.heuristic not in ("c_min", "reverse_dijkstra", "none"):
            raise ValueError("heuristic must be 'c_min', 'reverse_dijkstra' or 'none'")
        if self.straight_len_m is None:
            self.straight_len_m = 2.0 * self.pitch_m
        if self.goal_pos_tol_m is None:
            self.goal_pos_tol_m = 0.5 * self.pitch_m


# =====================================================================================
# Terrain on a grid
# =====================================================================================

@dataclass
class TerrainGrid:
    """Terrain sampled on a regular north-up grid. ``(x0, y0)`` is the CENTRE of the top-left
    cell; row index grows southward. ``valid`` marks known ground inside the search corridor."""

    ground: np.ndarray
    wet: np.ndarray
    valid: np.ndarray
    x0: float
    y0: float
    cell_m: float

    def _rc(self, xs, ys):
        col = (np.asarray(xs, dtype=float) - self.x0) / self.cell_m
        row = (self.y0 - np.asarray(ys, dtype=float)) / self.cell_m
        return row, col

    def sample_ground(self, xs, ys) -> np.ndarray:
        """Bilinear ground level between cell centres; NaN off the grid or on unknown ground."""
        row, col = self._rc(xs, ys)
        H, W = self.ground.shape
        c0 = np.floor(col).astype(int); r0 = np.floor(row).astype(int)
        ok = (c0 >= 0) & (r0 >= 0) & (c0 < W - 1) & (r0 < H - 1)
        out = np.full(np.shape(col), np.nan)
        if ok.any():
            fc = (col - c0)[ok]; fr = (row - r0)[ok]
            cc = c0[ok]; rr = r0[ok]
            g = self.ground
            out[ok] = (g[rr, cc] * (1 - fc) * (1 - fr) + g[rr, cc + 1] * fc * (1 - fr)
                       + g[rr + 1, cc] * (1 - fc) * fr + g[rr + 1, cc + 1] * fc * fr)
        return out

    def sample_wet(self, xs, ys) -> np.ndarray:
        """Nearest-cell wet mask; False off the grid."""
        row, col = self._rc(xs, ys)
        H, W = self.wet.shape
        r = np.rint(row).astype(int); c = np.rint(col).astype(int)
        inb = (c >= 0) & (r >= 0) & (c < W) & (r < H)
        out = np.zeros(np.shape(col), bool)
        out[inb] = self.wet[r[inb], c[inb]]
        return out

    def cell_centres(self, rows, cols):
        return self.x0 + np.asarray(cols) * self.cell_m, self.y0 - np.asarray(rows) * self.cell_m

    @property
    def bounds(self):
        H, W = self.ground.shape
        return (self.x0, self.y0 - (H - 1) * self.cell_m, self.x0 + (W - 1) * self.cell_m, self.y0)


def build_terrain_grid(terrain: TerrainSampler, river_polygon, corridor, cell_m: float = 2.0,
                       margin_m: float = 20.0) -> TerrainGrid:
    """Sample ``terrain`` at cell centres over the corridor's bounds (plus ``margin_m``).

    Ground outside the corridor is kept (a side slope may daylight there); ``valid`` is
    known ground INSIDE the corridor, which is where the heuristic samples.
    """
    c = _rust.Call()
    r = _rust.call("lattice", "build_terrain_grid", _call=c,
                   grid={"terrain": c.terrain(terrain), "river_polygon": river_polygon, "corridor": corridor,
                         "cell_m": cell_m, "margin_m": margin_m})
    return TerrainGrid(ground=np.asarray(_rust.plain(r["ground"]), dtype=float),
                       wet=np.asarray(r["wet"], dtype=bool), valid=np.asarray(r["valid"], dtype=bool),
                       x0=float(_rust.plain(r["x0"])), y0=float(_rust.plain(r["y0"])),
                       cell_m=float(_rust.plain(r["cell_m"])))


# =====================================================================================
# Edge cost
# =====================================================================================

class FieldScorer:
    """Weighted cut of a constant-width trapezoidal channel swept along one primitive.

    ``score(prim, pose) -> (cost, ok)``; ``ok`` is False when less than
    ``min_valid_fraction`` of the fairway samples are on known terrain.
    """

    def __init__(self, grid: TerrainGrid, grade_m: float, hb_m: float, spec: SectionSpec):
        self.grid = grid
        self.grade = float(grade_m)
        self.hb = float(hb_m)
        self.spec = spec
        self.offs = np.arange(-spec.rake_half_m, spec.rake_half_m + 1e-6, spec.rake_step_m)
        self.tmpl = self.grade + np.maximum(0.0, np.abs(self.offs) - self.hb) / spec.side_slope_h_per_v
        self.in_fairway = np.abs(self.offs) <= self.hb

    def score(self, prim: dict, pose: Pose):
        x0, y0, th0 = pose
        sp = self.spec
        offs = self.offs
        pts = prim["pts"]
        arc_len = prim["arc_len"]
        ds_native = arc_len / len(pts)
        stride = max(1, int(round(sp.ds_step_m / ds_native))) if ds_native > 0 else 1
        sel = list(pts[::stride])
        if sel[-1] is not pts[-1]:
            sel.append(pts[-1])
        n_sec = len(sel)
        ds = arc_len / n_sec

        ct, st = math.cos(th0), math.sin(th0)
        loc = np.asarray(sel, dtype=float)                    # (n_sec, 4): s, lx, ly, dth
        wx = x0 + ct * loc[:, 1] - st * loc[:, 2]
        wy = y0 + st * loc[:, 1] + ct * loc[:, 2]
        hth = th0 + loc[:, 3]
        nx, ny = -np.sin(hth), np.cos(hth)                    # left normal
        gx = (wx[:, None] + offs[None, :] * nx[:, None])
        gy = (wy[:, None] + offs[None, :] * ny[:, None])

        gz = self.grid.sample_ground(gx.ravel(), gy.ravel()).reshape(n_sec, offs.size)
        finite = np.isfinite(gz)
        fair = np.broadcast_to(self.in_fairway, gz.shape)
        if not self.in_fairway.any() or finite[fair].mean() < sp.min_valid_fraction:
            return math.inf, False
        wet = self.grid.sample_wet(gx.ravel(), gy.ravel()).reshape(n_sec, offs.size)

        fair_cut = np.where(finite & fair, np.clip(gz - self.grade, 0.0, None), 0.0)
        hits_dry = ((fair_cut > 0) & ~wet).any(axis=1)
        slope_cells = finite & ~fair & ~wet & hits_dry[:, None]
        slope_cut = np.where(slope_cells, np.clip(gz - self.tmpl[None, :], 0.0, None), 0.0)
        cut = fair_cut + slope_cut

        cell = sp.rake_step_m * ds
        dredge = cut * wet
        excav = cut * ~wet
        deep = excav > sp.deep_threshold_m
        total = (dredge.sum() * sp.wet_weight
                 + np.where(deep, 0.0, excav).sum() * sp.dry_weight
                 + np.where(deep, excav, 0.0).sum() * sp.dry_weight * sp.deep_multiplier) * cell
        return float(total), True


# =====================================================================================
# Primitives
# =====================================================================================

def lattice_primitives(cfg: LatticeConfig) -> list:
    """Left/right arcs at ``r_min × radius_multiples`` turning exactly one heading bin, plus
    one straight of ``straight_len_m``. Dicts: R, sign, pts (s, x, y, heading), end, arc_len."""
    out = []
    for p in _rust.call("lattice", "lattice_primitives", cfg=cfg):
        out.append(dict(R=p["R"], sign=p["sign"], pts=[tuple(q) for q in p["pts"]], end=tuple(p["end"]),
                        arc_len=p["arc_len"]))
    return out


def _unit_straight(length: float) -> dict:
    n = max(2, int(round(length / 10.0)))
    return dict(R=math.inf, sign=0, pts=[(length * k / n, length * k / n, 0.0, 0.0)
                                         for k in range(1, n + 1)],
                end=(length, 0.0, 0.0), arc_len=length)


# =====================================================================================
# Heuristics
# =====================================================================================

def _valid_points(grid: TerrainGrid, stride_m: float, max_samples: Optional[int] = None):
    s = max(1, int(round(stride_m / grid.cell_m)))
    rr, cc = np.nonzero(grid.valid)
    keep = (rr % s == 0) & (cc % s == 0)
    rr, cc = rr[keep], cc[keep]
    if max_samples is not None and rr.size > max_samples:
        idx = np.linspace(0, rr.size - 1, max_samples).astype(int)
        rr, cc = rr[idx], cc[idx]
    xs, ys = grid.cell_centres(rr, cc)
    return rr // s, cc // s, xs, ys, s


def min_cost_per_metre(grid: TerrainGrid, scorer: FieldScorer, n_heading: int, length_m: float,
                       stride_m: float = 24.0, max_samples: int = 4000) -> float:
    """The cheapest per-metre straight cost anywhere in the corridor (0 if none is valid).
    Straight cost is symmetric under a half turn, so half the heading bins are swept."""
    c = _rust.Call()
    return _rust.call("lattice", "min_cost_per_metre", float, _call=c, grid=grid, grade_m=scorer.grade,
                      hb_m=scorer.hb, section=_section_arg(c, scorer.spec), n_heading=int(n_heading),
                      length_m=length_m, stride_m=stride_m, max_samples=int(max_samples))


def reverse_dijkstra_heuristic(grid: TerrainGrid, scorer: FieldScorer, goal_xy, n_heading: int,
                               length_m: float, stride_m: float):
    """A 2-D cost-to-go from the goal on a coarse grid, curvature dropped and the cheapest
    heading taken per cell. Returns ``h(x, y)`` (a large number off the reachable grid)."""
    s = max(1, int(round(stride_m / grid.cell_m)))
    H, W = grid.ground.shape
    nr, nc = (H + s - 1) // s, (W + s - 1) // s
    cg = np.full((nr, nc), np.inf)
    unit = _unit_straight(length_m)
    bs = 2.0 * math.pi / n_heading
    ir, ic, xs, ys, _ = _valid_points(grid, stride_m)
    for i, j, x, y in zip(ir, ic, xs, ys):
        best = math.inf
        for b in range(max(1, n_heading // 2)):
            c, ok = scorer.score(unit, (x, y, b * bs))
            if ok:
                best = min(best, c / length_m)
        cg[i, j] = best
    step = s * grid.cell_m
    gr = int(round((grid.y0 - goal_xy[1]) / grid.cell_m / s))
    gc = int(round((goal_xy[0] - grid.x0) / grid.cell_m / s))
    gr = min(max(gr, 0), nr - 1); gc = min(max(gc, 0), nc - 1)
    if not np.isfinite(cg[gr, gc]):
        fin = np.argwhere(np.isfinite(cg))
        if len(fin):
            k = int(np.argmin(np.abs(fin[:, 0] - gr) + np.abs(fin[:, 1] - gc)))
            gr, gc = int(fin[k, 0]), int(fin[k, 1])
    D = np.full((nr, nc), np.inf)
    D[gr, gc] = 0.0
    pq = [(0.0, gr, gc)]
    diag = math.hypot(step, step)
    neigh = [(-1, 0, step), (1, 0, step), (0, -1, step), (0, 1, step),
             (-1, -1, diag), (-1, 1, diag), (1, -1, diag), (1, 1, diag)]
    while pq:
        d, i, j = heapq.heappop(pq)
        if d > D[i, j] + 1e-9:
            continue
        for di, dj, sl in neigh:
            a, b = i + di, j + dj
            if 0 <= a < nr and 0 <= b < nc and np.isfinite(cg[a, b]):
                nd = d + sl * 0.5 * (cg[i, j] + cg[a, b])
                if nd < D[a, b] - 1e-9:
                    D[a, b] = nd
                    heapq.heappush(pq, (nd, a, b))

    def h(x, y):
        i = int(round((grid.y0 - y) / grid.cell_m / s)); j = int(round((x - grid.x0) / grid.cell_m / s))
        if 0 <= i < nr and 0 <= j < nc and np.isfinite(D[i, j]):
            return float(D[i, j])
        return 1e18
    return h


# =====================================================================================
# The search
# =====================================================================================

@dataclass
class LatticeResult:
    converged: bool = False
    line: Optional[LineString] = None          # forward-integrated, tangent-continuous
    cost: float = math.inf                     # search objective (weights, not prices)
    prim_seq: list = field(default_factory=list)   # (R or None, sign, arc_len) per primitive
    start_pose: Optional[Pose] = None
    goal_pose: Optional[Pose] = None
    end_err_m: float = math.inf
    expansions: int = 0
    n_states: int = 0
    seconds: float = 0.0
    hit_cap: bool = False
    timed_out: bool = False
    diagnostics: dict = field(default_factory=dict)
    measured_min_radius_m: float = math.nan     # min radius of ``line`` on the chord (R_min/3)
    radius_check_chord_m: float = math.nan
    radius_ok: bool = False                     # the chord reading holds R_min (2 % tol)


def _section_arg(call, spec: SectionSpec) -> dict:
    """A :class:`SectionSpec` as the core reads it: a half-width function becomes a callback."""
    d = _rust.enc({**{f.name: getattr(spec, f.name) for f in dataclasses.fields(spec)}, "half_width": 0.0})
    hw = spec.half_width
    d["half_width"] = call.scalar(hw) if callable(hw) else hw
    return d


def _decode_result(r: dict, seconds: float) -> LatticeResult:
    """A :class:`LatticeResult` from the core's answer (the core keeps no clock)."""
    r = _rust.plain(r)
    return LatticeResult(
        converged=bool(r["converged"]), line=r["line"], cost=r["cost"],
        prim_seq=[tuple(p) for p in r["prim_seq"]],
        start_pose=None if r["start_pose"] is None else tuple(r["start_pose"]),
        goal_pose=None if r["goal_pose"] is None else tuple(r["goal_pose"]),
        end_err_m=r["end_err_m"], expansions=int(r["expansions"]), n_states=int(r["n_states"]),
        seconds=seconds, hit_cap=bool(r["hit_cap"]), timed_out=bool(r["timed_out"]),
        diagnostics=r["diagnostics"])


def _decode_pool(r: dict, seconds: float) -> PoolAlignment:
    res = _decode_result(r["result"], seconds)
    p = _rust.plain({k: v for k, v in r.items() if k != "result"})
    return PoolAlignment(start_m=p["start_m"], end_m=p["end_m"], water_level_m=p["water_level_m"],
                         grade_m=p["grade_m"], result=res, baseline_cost=p["baseline_cost"])


def solve_lattice(grid: TerrainGrid, start_pose: Pose, goal_pose: Pose, grade_m: float,
                  section: SectionSpec, cfg: LatticeConfig) -> LatticeResult:
    """A* from ``start_pose`` to ``goal_pose`` over the lattice. See the module docstring."""
    c = _rust.Call()
    t0 = time.perf_counter()
    r = _rust.call("lattice", "solve_lattice", _call=c, grid=grid, start_pose=list(start_pose),
                   goal_pose=list(goal_pose), grade_m=grade_m, section=_section_arg(c, section), cfg=cfg)
    return _decode_result(r, time.perf_counter() - t0)


# =====================================================================================
# Front door: per pool, then the whole line
# =====================================================================================

def heading_at(line: LineString, s: float, arm_m: float = 20.0) -> float:
    """Tangent bearing (rad) at chainage ``s``."""
    return _rust.call("lattice", "heading_at", chainages=[s], arm_m=arm_m)[0]


@dataclass
class PoolAlignment:
    start_m: float
    end_m: float
    water_level_m: float
    grade_m: float
    result: LatticeResult
    baseline_cost: float          # the baseline segment under the same scorer (straight width)


def optimise_pool(baseline: LineString, terrain: TerrainSampler, river_polygon,
                  start_m: float, end_m: float, *, water_level_m: float, depth_m: float,
                  section: SectionSpec, cfg: LatticeConfig,
                  corridor_line: Optional[LineString] = None, corridor_half_m: float = 350.0,
                  cell_m: float = 2.0) -> PoolAlignment:
    """Least-cost R_min-feasible alignment for one pool, pinned to ``baseline`` at
    ``start_m`` and ``end_m`` (position and heading). Grade = ``water_level_m - depth_m``.

    The corridor is ``corridor_half_m`` around the baseline segment, unioned with the same
    buffer around ``corridor_line`` (e.g. the natural river) between the projections of the
    two end points, when given.
    """
    c = _rust.Call()
    t0 = time.perf_counter()
    r = _rust.call("lattice", "optimise_pool", _call=c, baseline=baseline, terrain=c.terrain(terrain),
                   river_polygon=river_polygon, start_m=start_m, end_m=end_m, water_level_m=water_level_m,
                   depth_m=depth_m, section=_section_arg(c, section), cfg=cfg, corridor_line=corridor_line,
                   corridor_half_m=corridor_half_m, cell_m=cell_m)
    return _decode_pool(r, time.perf_counter() - t0)


@dataclass
class PoolsResult:
    line: LineString
    pools: List[PoolAlignment]
    measured_min_radius_m: float = math.nan     # min radius of ``line`` on the chord (R_min/3)
    radius_check_chord_m: float = math.nan
    radius_ok: bool = False

    @property
    def n_replaced(self) -> int:
        return sum(1 for p in self.pools if p.result.converged)


def optimise_pools(baseline: LineString, terrain: TerrainSampler, river_polygon,
                   pools: Sequence[Tuple[float, float, float]], *, depth_m: float,
                   section: SectionSpec, cfg: LatticeConfig,
                   corridor_line: Optional[LineString] = None, corridor_half_m: float = 350.0,
                   cell_m: float = 2.0, keep_if_dearer: bool = True) -> PoolsResult:
    """Run :func:`optimise_pool` on each ``(start_m, end_m, water_level_m)`` and splice the
    results into ``baseline``. A pool keeps its baseline segment when the search does not
    converge, or (``keep_if_dearer``) when the result costs more than the baseline under the
    same scorer. Pools come from e.g. :meth:`channel_designer.optimise.cascade.Cascade.pools`.

    Each spliced segment joins the baseline within ``cfg.goal_pos_tol_m`` at its far end; the
    join is a straight step of at most that length.
    """
    c = _rust.Call()
    t0 = time.perf_counter()
    r = _rust.call("lattice", "optimise_pools", _call=c, baseline=baseline, terrain=c.terrain(terrain),
                   river_polygon=river_polygon, pools=[list(p) for p in pools], depth_m=depth_m,
                   section=_section_arg(c, section), cfg=cfg, corridor_line=corridor_line,
                   corridor_half_m=corridor_half_m, cell_m=cell_m, keep_if_dearer=bool(keep_if_dearer))
    seconds = time.perf_counter() - t0
    return PoolsResult(line=_rust.plain(r["line"]),
                       pools=[_decode_pool(p, seconds / max(1, len(r["pools"]))) for p in r["pools"]])
