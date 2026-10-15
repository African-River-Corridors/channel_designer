"""Weir cascade search: where the weirs go, how many, and how high each pool sits.

A cascade is a monotone staircase on one chainage axis, so this is a SHORTEST PATH ON A DAG,
solved exactly — no metaheuristic, no local optimum.

    state   (site i, pool level h)   a weir at site i holding the pool UPSTREAM of it at h
    edge    (i, h) -> (j, h + l)     the next weir at site j, lifting by l
    cost    reach_cost(i -> j at h) + structure_cost(l)

    source  a virtual weir at ``start_m`` with h = ``base_level_m``
    sink    any state whose final pool, running to ``terminal_m``, is at least
            ``terminal_min_level_m``

Every cost is a caller-supplied callable returning a unitless WEIGHT, the same convention as
:mod:`channel_designer.alignment.route`. The package prices nothing. A typical reach cost is
earthworks from a :class:`LevelTable` (:func:`earthworks_reach_cost`), plus any terms the
caller cares about (flooded area, buildings, crossings), each weighted by the caller.

The pool levels the DP can reach are the base level plus every sum of allowed lifts up to
``level_cap_m``. :func:`cascade_levels` lists them; build a :class:`LevelTable` on exactly
those so no level is read by rounding to a neighbour.

Guards (each one is a silent failure in a looser version of this search):

- a level outside the table raises — it is never clamped to the table's edge;
- a terminal level above the level cap raises, rather than reporting "no feasible cascade";
- candidate sites beyond the terminal are dropped, and the result says how many.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Callable, List, Optional, Sequence

import numpy as np
from shapely.geometry import LineString

from ..volumes.cut_model import CUT_MODEL, MAXW_M, STATION_M
from ..volumes.sections import TerrainSampler, section_volumes

ReachCost = Callable[[float, float, float], float]       # (start_m, end_m, level_m) -> weight
StructureCost = Callable[[float], float]                  # lift_m -> weight

_LEVEL_DECIMALS = 3


@dataclass
class CascadeConfig:
    """The cascade problem. Chainages in metres on the caller's axis, levels in metres."""

    candidate_sites_m: Sequence[float]       # where a weir may stand
    lifts_m: Sequence[float]                 # allowed lift per structure
    terminal_m: float                        # chainage the cascade must reach
    terminal_min_level_m: float              # the top pool must be at least this
    base_level_m: float = 0.0                # water level below the first weir
    start_m: float = 0.0                     # chainage of the virtual source
    min_spacing_m: float = 0.0               # minimum distance between structures
    level_cap_m: Optional[float] = None      # highest pool; default terminal min + max lift
    k_best: int = 5                          # ranked distinct endings to return
    site_ids: Optional[Sequence[str]] = None

    def __post_init__(self) -> None:
        if not self.lifts_m or min(self.lifts_m) <= 0:
            raise ValueError("lifts_m must be non-empty and every lift > 0")
        if self.site_ids is not None and len(self.site_ids) != len(self.candidate_sites_m):
            raise ValueError("site_ids must match candidate_sites_m one to one")
        if self.level_cap_m is None:
            self.level_cap_m = self.terminal_min_level_m + max(self.lifts_m)
        if self.terminal_min_level_m > self.level_cap_m + 1e-9:
            raise ValueError(
                f"terminal_min_level_m {self.terminal_min_level_m:g} m is above level_cap_m "
                f"{self.level_cap_m:g} m, so no cascade can reach it. Raise level_cap_m to at "
                f"least {self.terminal_min_level_m + max(self.lifts_m):g}.")


@dataclass
class CascadeWeir:
    site_id: str
    chainage_m: float
    pool_level_m: float           # water level of the pool UPSTREAM of this weir
    lift_m: float
    structure_cost: float


@dataclass
class CascadeReach:
    start_m: float
    end_m: float
    level_m: float
    cost: float


@dataclass
class Cascade:
    total_cost: float
    weirs: List[CascadeWeir] = field(default_factory=list)
    reaches: List[CascadeReach] = field(default_factory=list)

    @property
    def reach_cost(self) -> float:
        return float(sum(r.cost for r in self.reaches))

    @property
    def structure_cost(self) -> float:
        return float(sum(w.structure_cost for w in self.weirs))

    def pools(self) -> list:
        """``[(start_m, end_m, level_m), ...]`` downstream to upstream, base reach included."""
        return [(r.start_m, r.end_m, r.level_m) for r in self.reaches]

    def as_explicit_weirs(self, structure_height_m: Optional[float] = None) -> list:
        """Input for :func:`channel_designer.volumes.pools.place_weirs_explicit`."""
        return [dict(chainage_m=w.chainage_m, pool_mwl_m=w.pool_level_m,
                     structure_height_m=structure_height_m) for w in self.weirs]


@dataclass
class CascadeSearch:
    """Ranked cascades plus what the search did."""

    cascades: List[Cascade]
    levels_m: List[float]
    states_explored: int
    feasible_endings: int
    sites_dropped_beyond_terminal: int

    @property
    def best(self) -> Optional[Cascade]:
        return self.cascades[0] if self.cascades else None


def cascade_levels(cfg: CascadeConfig) -> List[float]:
    """Every pool level the DP can reach: base + any sum of lifts, up to the cap."""
    cap = float(cfg.level_cap_m)
    base = round(float(cfg.base_level_m), _LEVEL_DECIMALS)
    reach = {base}
    frontier = {base}
    while frontier:
        nxt = set()
        for r in frontier:
            for L in cfg.lifts_m:
                v = round(r + float(L), _LEVEL_DECIMALS)
                if v <= cap + 1e-9 and v not in reach:
                    nxt.add(v)
        reach |= nxt
        frontier = nxt
    return sorted(reach)


def _sites(cfg: CascadeConfig):
    ids = list(cfg.site_ids) if cfg.site_ids is not None else [
        f"S{i + 1}" for i in range(len(cfg.candidate_sites_m))]
    rows = sorted(zip(cfg.candidate_sites_m, ids))
    kept = [(float(c), i) for c, i in rows if float(cfg.start_m) < float(c) <= float(cfg.terminal_m)]
    return kept, len(rows) - len(kept)


def search_cascade(cfg: CascadeConfig, reach_cost: ReachCost,
                   structure_cost: StructureCost) -> CascadeSearch:
    """Exact least-cost cascade(s). Returns up to ``k_best`` distinct endings, cheapest first.

    "Distinct endings" means distinct (last weir, top pool level) states, each with its own
    cheapest history — not the k cheapest cascades overall.
    """
    sites, dropped = _sites(cfg)
    S = [(float(cfg.start_m), "SOURCE")] + sites
    lvls = cascade_levels(cfg)
    LI = {v: i for i, v in enumerate(lvls)}
    base = round(float(cfg.base_level_m), _LEVEL_DECIMALS)
    INF = math.inf
    dp = np.full((len(S), len(lvls)), INF)
    par = np.full((len(S), len(lvls), 3), -1, dtype=int)     # (site, level, lift index)
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
                    h2 = round(h + L, _LEVEL_DECIMALS)
                    k = LI.get(h2)
                    if k is None:
                        continue
                    c = b + s_cost[L]
                    if c < dp[j, k]:
                        dp[j, k] = c
                        par[j, k] = (i, hi, li)
    finals = []
    for j in range(1, len(S)):
        for hi, h in enumerate(lvls):
            if dp[j, hi] == INF or h < cfg.terminal_min_level_m - 1e-9:
                continue
            finals.append((dp[j, hi] + float(reach_cost(S[j][0], cfg.terminal_m, h)), j, hi))
    finals.sort()
    out = []
    for tot, j, hi in finals[:cfg.k_best]:
        chain = []
        cj, ch = j, hi
        while cj > 0:
            pi, ph, pl = par[cj, ch]
            chain.append((S[cj][1], S[cj][0], lvls[ch], lifts[int(pl)]))
            cj, ch = int(pi), int(ph)
        chain.reverse()
        out.append(_assemble(cfg, chain, reach_cost, s_cost, total=float(tot)))
    return CascadeSearch(out, lvls, int(np.isfinite(dp).sum()), len(finals), dropped)


def _assemble(cfg, chain, reach_cost, s_cost, total=None) -> Cascade:
    weirs = [CascadeWeir(sid, km, lvl, lift, s_cost[lift]) for sid, km, lvl, lift in chain]
    bounds = [(float(cfg.start_m), float(cfg.base_level_m))] + [(w.chainage_m, w.pool_level_m)
                                                                 for w in weirs]
    ends = [w.chainage_m for w in weirs] + [float(cfg.terminal_m)]
    reaches = [CascadeReach(a, b, lvl, float(reach_cost(a, b, lvl)))
               for (a, lvl), b in zip(bounds, ends)]
    c = Cascade(0.0, weirs, reaches)
    c.total_cost = c.reach_cost + c.structure_cost if total is None else total
    return c


def score_cascade(cfg: CascadeConfig, weirs: Sequence[tuple], reach_cost: ReachCost,
                  structure_cost: StructureCost) -> Cascade:
    """Cost of a FIXED cascade, ``weirs = [(chainage_m, lift_m), ...]``, under the same
    objective. The only fair way to compare two cascades is one objective, both scored."""
    spec = sorted((float(c), float(L)) for c, L in weirs)
    lvl = round(float(cfg.base_level_m), _LEVEL_DECIMALS)
    chain = []
    for n, (c, L) in enumerate(spec, 1):
        lvl = round(lvl + L, _LEVEL_DECIMALS)
        chain.append((f"W{n}", c, lvl, L))
    s_cost = {L: float(structure_cost(L)) for _, L in spec}
    return _assemble(cfg, chain, reach_cost, s_cost)


# =====================================================================================
# Earthworks by level — the usual reach cost
# =====================================================================================

@dataclass
class LevelTable:
    """Cut area per station at each candidate pool level.

    ``dredge_m2`` / ``excav_m2`` are (stations, levels). ``chainage_m`` is each station's
    chainage on the cascade's axis (pass your own axis to :func:`build_level_table` when the
    cascade is set out on a different line from the one the volumes are measured on).
    """

    chainage_m: np.ndarray
    levels_m: np.ndarray
    dredge_m2: np.ndarray
    excav_m2: np.ndarray
    station_m: float
    level_tol_m: float = 1e-6

    def level_index(self, level_m: float) -> int:
        i = int(np.argmin(np.abs(self.levels_m - level_m)))
        if abs(self.levels_m[i] - level_m) > self.level_tol_m:
            raise ValueError(
                f"pool level {level_m:g} m is not in the table (levels "
                f"{self.levels_m.min():g}-{self.levels_m.max():g} m). Build the table on "
                f"cascade_levels(cfg); a level is never read from its neighbour.")
        return i

    def _cum(self):
        if not hasattr(self, "_cache"):
            order = np.argsort(self.chainage_m)
            ch = self.chainage_m[order]
            z = np.zeros((1, self.levels_m.size))
            cd = np.vstack([z, np.cumsum(self.dredge_m2[order], axis=0)]) * self.station_m
            ce = np.vstack([z, np.cumsum(self.excav_m2[order], axis=0)]) * self.station_m
            self._cache = (ch, cd, ce)
        return self._cache

    def reach_volumes(self, start_m: float, end_m: float, level_m: float):
        """(dredge m³, excavation m³) over stations with start_m <= chainage < end_m."""
        ch, cd, ce = self._cum()
        j = self.level_index(level_m)
        i0 = int(np.searchsorted(ch, start_m)); i1 = int(np.searchsorted(ch, end_m))
        return float(cd[i1, j] - cd[i0, j]), float(ce[i1, j] - ce[i0, j])


def build_level_table(line: LineString, terrain: TerrainSampler, river_polygon,
                      levels_m: Sequence[float], depth_m: float, hb, *,
                      model: str = CUT_MODEL, station_m: float = STATION_M,
                      maxw_m: float = MAXW_M, chainage_m: Optional[Sequence[float]] = None,
                      **section_kwargs) -> LevelTable:
    """Run :func:`channel_designer.volumes.sections.section_volumes` once per level, with the
    pool water level = that level and grade = level − depth, and keep the per-station areas.

    ``chainage_m`` (one per station) re-labels the stations onto the cascade's axis.
    """
    levels = np.asarray(sorted(float(v) for v in levels_m))
    dredge = excav = None
    ch = None
    for k, lvl in enumerate(levels):
        v = section_volumes(line, terrain, river_polygon, float(lvl), depth_m, hb, model=model,
                            station_m=station_m, maxw_m=maxw_m, **section_kwargs)
        if dredge is None:
            n = len(v.stations)
            dredge = np.zeros((n, levels.size)); excav = np.zeros((n, levels.size))
            ch = np.array([s.chainage_m for s in v.stations])
        dredge[:, k] = [s.dredge_area_m2 for s in v.stations]
        excav[:, k] = [s.excav_area_m2 for s in v.stations]
    if chainage_m is not None:
        ch = np.asarray(chainage_m, dtype=float)
        if ch.shape != (dredge.shape[0],):
            raise ValueError(f"chainage_m needs one value per station ({dredge.shape[0]})")
    return LevelTable(ch, levels, dredge, excav, float(station_m))


def earthworks_reach_cost(table: LevelTable, wet_weight: float = 1.0,
                          dry_weight: float = 1.0) -> ReachCost:
    """``reach_cost(start_m, end_m, level_m) = wet_weight × dredge + dry_weight × excavation``.

    Weights, not prices: 1.0 / 1.0 minimises volume.
    """
    def cost(a: float, b: float, level: float) -> float:
        d, e = table.reach_volumes(a, b, level)
        return wet_weight * d + dry_weight * e
    return cost
