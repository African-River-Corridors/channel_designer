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
   built geometry until its measured minimum three-point radius clears ``r_min`` less the
   fixed reading tolerance ``RADIUS_TOL`` (2 %), the same one ``radius_ok`` allows. The target
   is never relaxed to the baseline's own radius. The passes stop after ``max_iter`` even if
   it is not cleared, so the result is measured, not assumed (step 5).
4. CHECK + SWEEP. Each candidate's minimum radius is read on its own station vertices
   (:func:`.radius_check.min_radius_on_vertices`, the reading the enforcement uses). The sweep
   picks the sub-corridor that saves most AMONG THOSE THAT HOLD ``r_min``; only if none does,
   the one that saves most, flagged. The winner reports ``radius_ok``,
   ``measured_min_radius_m`` and ``radius_check_chord_m``. The search only shifts the
   baseline sideways: it cannot lift a baseline that is itself tighter than ``r_min``. Roll
   the baseline first (:mod:`.rolling_circle`) and check ``radius_ok``.

Validate a winner with a true volume integration (:mod:`channel_designer.volumes.sections`
or :mod:`channel_designer.volumes.raster`), never with the grid's own numbers.

The objective weights wet and dry cut (``wet_weight``, ``dry_weight``). They are relative
WEIGHTS, not prices: 1.0 / 1.0 minimises volume. Pricing is out of scope for this package.

Inputs are plain: a baseline ``LineString``, a terrain sampler ``terrain(xs, ys) -> z`` (NaN
where unknown), the present river polygon, and a design grade (float or ``grade(s)``).
"""
from __future__ import annotations

from .. import _rust

from dataclasses import dataclass, field
from typing import Callable, Optional, Sequence, Union

import numpy as np
from shapely.geometry import LineString

from channel_designer.alignment.radius_check import RADIUS_TOL, radius_ok

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
    min_radius_m: float          # enforcement's reading: three-point radius on the stations
    corridor_half_m: float
    approx_saving_pct: float     # grid-approx, vs offset 0 everywhere — a search number
    smooth_passes: int
    sweep: list = field(default_factory=list)   # (corridor, saving %, min R) per candidate
    measured_min_radius_m: float = np.nan       # minimum radius of ``line`` on the chord
    radius_check_chord_m: float = np.nan        # NOMINAL chord, 2 × k × station_step, m
    radius_ok: bool = False                     # measured holds ``r_min`` (2 % reading tol)


def _grade_arg(call, grade: GradeLike):
    """The grade as the core reads it: a number, or a callback for a function of chainage."""
    return call.scalar(grade) if callable(grade) else float(grade)


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
    c = _rust.Call()
    r = _rust.call("route", "build_cost_grid", CostGrid, _call=c, line=line, terrain=c.terrain(terrain),
                   river_polygon=river_polygon, grade=_grade_arg(c, grade), bottom_width=bottom_width, depth=depth,
                   side_slope=side_slope, corridor_half=corridor_half, station_step=station_step,
                   offset_step=offset_step, transect_step=transect_step, daylight_m=daylight_m)
    return r


def radii(pts) -> np.ndarray:
    """Three-point circumradius R = abc / (4·Area) at every interior vertex (inf on ends)."""
    return _rust.call("route", "radii", np.ndarray, pts=pts)


def min_radius_of_line(pts):
    """Minimum radius of curvature (m) of the polyline and its vertex index."""
    r = _rust.call("route", "min_radius_of_line", pts=pts)
    return float(r["radius"]), int(r["index"])


def baseline_turn_angles(centres) -> np.ndarray:
    """Signed per-station turn (rad, > 0 left) of the baseline between station chords."""
    return _rust.call("route", "baseline_turn_angles", np.ndarray, centres=centres)


def offset_bounds(centres, target_R):
    """Per-station admissible offset window [o_lo, o_hi] so a PARALLEL offset cannot fold the
    alignment below ``target_R`` on the inside of a baseline bend.

    Where the baseline is already tighter than ``target_R`` the window only forbids moving
    further inside (margin 0); it does not force the line outside, which would leave the DP
    no feasible path. ``radius_ok`` on the result reports what that leaves."""
    r = _rust.call("route", "offset_bounds", centres=centres, target_R=target_R)
    return np.asarray(r["lo"], dtype=float), np.asarray(r["hi"], dtype=float)


def dp_optimise(offsets, centres, cost_grid, r_min, station_step, offset_step):
    """Minimise the summed per-station cost over an offset path, subject to R >= ``r_min``.

    Heading model: the segment i->i+1 makes an extra heading phi_i = atan2(o[i+1]-o[i], ds)
    from the slew; the total turn at i is beta[i] + phi_i - phi_{i-1}, bounded by ds/R_min.
    Where the baseline itself turns more than the budget, the budget relaxes to |beta[i]| so
    the offset path may add zero curvature rather than be forced infeasible. DP over states
    (o[i-1], o[i]). Returns ``(path_indices, best_cost)``.
    """
    r = _rust.call("route", "dp_optimise", offsets=offsets, centres=centres, cost_grid=cost_grid, r_min=r_min,
                   station_step=station_step, offset_step=offset_step)
    return np.asarray(r["path"], dtype=float), float(r["cost"])


def build_line_from_offsets(centres, normals, offset_vals) -> np.ndarray:
    return _rust.call("route", "build_line_from_offsets", np.ndarray, centres=centres, normals=normals,
                      offset_vals=offset_vals)


def smooth_offsets(offset_vals, heading_budget, beta, ds):
    """Gentle 3-point smoothing that keeps the DP path's bend-radius feasibility."""
    return _rust.call("route", "smooth_offsets", np.ndarray, offset_vals=offset_vals, heading_budget=heading_budget,
                      beta=beta, ds=ds)


def enforce_min_radius(offset_vals, centres, normals, target_R, max_iter=600,
                       taper_stations=8):
    """Laplacian passes on the built geometry until its measured min radius clears target.

    Offsets are tapered to zero over the first/last ``taper_stations`` (the channel ties
    into fixed points at both ends), which makes zero offset the smoothing fixed point, so
    the passes converge to the baseline. Returns ``(offsets, passes, min_R)``.

    Returns after ``max_iter`` passes even when ``min_R < target_R`` (e.g. the baseline itself
    is tighter than the target). Compare ``min_R`` with ``target_R``; never assume it cleared.
    """
    r = _rust.call("route", "enforce_min_radius", offset_vals=offset_vals, centres=centres, normals=normals,
                   target_R=target_R, max_iter=int(max_iter), taper_stations=int(taper_stations))
    return np.asarray(r["offsets"], dtype=float), int(r["passes"]), float(r["min_radius"])


def approx_saving_pct(grid: CostGrid, cost: np.ndarray, ovals) -> float:
    """Grid-approx cost of ``ovals`` vs the baseline (offset 0 everywhere), as a %."""
    return _rust.call("route", "approx_saving_pct", float, grid=grid, cost=cost, ovals=ovals)


def solve_at_corridor(grid: CostGrid, cost: np.ndarray, corridor_half: float, r_min: float):
    """DP within ``±corridor_half`` and the fold band, then smooth to ``r_min``.

    The target is ``r_min`` less the fixed reading tolerance, never relaxed to the
    baseline's own radius (issue #9).
    """
    r = _rust.call("route", "solve_at_corridor", grid=grid, cost=cost, corridor_half=corridor_half, r_min=r_min)
    return np.asarray(r["offsets"], dtype=float), int(r["passes"]), float(r["min_radius"]), float(r["saving_pct"])


def optimise_route(line: LineString, terrain: TerrainSampler, river_polygon, grade: GradeLike,
                   *, bottom_width: float, depth: float, side_slope: float, r_min: float,
                   wet_weight: float = 1.0, dry_weight: float = 1.0,
                   corridor_sweep: Sequence[float] = DEFAULT_CORRIDOR_SWEEP,
                   station_step: float = 100.0, offset_step: float = 10.0,
                   transect_step: float = 2.0, daylight_m: Optional[float] = None,
                   grid: Optional[CostGrid] = None,
                   check_chord_m: Optional[float] = None) -> RouteResult:
    """Least-cost lateral re-route of ``line``. See the module docstring for the method.

    Wide excursions force sharp reconnection curves that the radius enforcement then smooths
    away, so a moderate corridor usually wins; the sweep finds it.

    ``radius_ok`` on the result is False when the returned line is tighter than ``r_min``.
    It is read on the line's own station vertices, ``k`` stations each side, so the chord is
    ``2 × k × station_step``. The default ``k = 1`` is the reading the enforcement uses. Pass
    ``check_chord_m`` to read a longer chord (``k = floor(check_chord_m / (2 × step) + 0.5)``,
    at least 1). The chord is NOMINAL, measured along the baseline: offsets stretch segments,
    so the real chord at a corner can be longer. A resampled check on one LOA can read well
    below a ``radius_ok`` route line at a corner between stations (see docs/known-issues.md).
    """
    c = _rust.Call()
    return _rust.call("route", "optimise_route", RouteResult, _call=c, line=line, terrain=c.terrain(terrain),
                      river_polygon=river_polygon, grade=_grade_arg(c, grade), bottom_width=bottom_width,
                      depth=depth, side_slope=side_slope, r_min=r_min, wet_weight=wet_weight,
                      dry_weight=dry_weight, corridor_sweep=list(corridor_sweep), station_step=station_step,
                      offset_step=offset_step, transect_step=transect_step, daylight_m=daylight_m,
                      grid=grid, check_chord_m=check_chord_m)
