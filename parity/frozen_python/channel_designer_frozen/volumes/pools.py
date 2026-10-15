"""Pool levels: the longitudinal bed profile and the weir cascade that sets each pool's WL.

The design grade of a navigation channel in a canalised river is ``pool WL − depth``, so the
volume of cut depends on where the weirs sit and what level each pool holds. This module
places a cascade on a smoothed bed profile (flat-pool assumption, confirmed lift = H − d), or
takes an explicit one, and answers "what is the water level / grade at chainage s?".

Algorithm (flat-pool assumption, lift = H - d):

    Weir #1 sits at s = start (downstream, bed ~ start elevation).
        foundation = bed(s) - d
        crest = WL = foundation + H = bed(s) + (H - d)
    Each pool's water surface is horizontal at its crest elevation. The next weir is placed
    at the first chainage going upstream where the (smoothed, monotone) bed rises to the
    current pool water level. Spacing is therefore NON-uniform: it follows the bed slope.

Structures are placed as GEOMETRY only. Their design and cost are out of scope.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Optional

import numpy as np
from shapely.geometry import LineString, Point

TerrainSampler = Callable[[np.ndarray, np.ndarray], np.ndarray]


@dataclass
class BedProfile:
    """Bed elevation sampled along a centerline.

    Convention: chainage ``s`` increases from the DOWNSTREAM end (s=0, lowest
    bed) towards UPSTREAM. ``z_raw`` is the point-sampled terrain (keeps the
    real spikes). ``z_overall`` is the *overall bottom slope*: a smooth, gently
    increasing trend through the channel bottom that ignores the unrealistic
    peaks (banks, meander necks, spoil heaps). Weirs are placed against this
    generic bed, and the cascade starts where ``z_overall`` reaches the start elevation.
    """

    s: np.ndarray          # chainage (m), shape (N,)
    z_raw: np.ndarray      # raw bed elevation (m), may contain NaN
    z_overall: np.ndarray  # overall bottom slope (m), smooth & non-decreasing
    points: list[Point]    # centerline points at each station
    line: LineString       # the centerline geometry (oriented downstream->up)

    @property
    def length_m(self) -> float:
        return float(self.line.length)


def _fill_nan(z: np.ndarray) -> np.ndarray:
    """Linear-interpolate interior NaNs; edge-fill the ends."""
    z = z.copy()
    idx = np.arange(z.size)
    good = np.isfinite(z)
    if good.sum() == 0:
        return np.zeros_like(z)
    z[~good] = np.interp(idx[~good], idx[good], z[good])
    return z


def _rolling_percentile(z: np.ndarray, win: int, q: float) -> np.ndarray:
    """Centred rolling percentile (NaN-aware). Low ``q`` tracks the bottom."""
    if win < 1:
        return z.copy()
    half = win // 2
    out = np.empty_like(z)
    for i in range(z.size):
        seg = z[max(0, i - half): min(z.size, i + half + 1)]
        seg = seg[np.isfinite(seg)]
        out[i] = np.percentile(seg, q) if seg.size else np.nan
    return out


def _rolling_mean(z: np.ndarray, win: int) -> np.ndarray:
    if win < 1:
        return z.copy()
    half = win // 2
    out = np.empty_like(z)
    for i in range(z.size):
        out[i] = np.nanmean(z[max(0, i - half): min(z.size, i + half + 1)])
    return out


def overall_bottom_slope(z_raw: np.ndarray, spacing_m: float,
                         percentile: float = 20.0,
                         perc_window_m: float = 2500.0,
                         smooth_window_m: float = 5000.0) -> np.ndarray:
    """Robust, smooth, non-decreasing 'overall bottom slope' of the bed.

    1) low rolling percentile  -> the channel bottom, ignoring high spikes
    2) wide rolling mean        -> a smooth general trend
    3) cumulative maximum       -> enforce non-decreasing (a backwater can only
                                   pond water upstream, never downhill)
    """
    z = _fill_nan(z_raw)
    base = _rolling_percentile(z, max(1, int(round(perc_window_m / spacing_m))), percentile)
    base = _fill_nan(base)
    smooth = _rolling_mean(base, max(1, int(round(smooth_window_m / spacing_m))))
    return np.maximum.accumulate(smooth)


def sample_bed_profile(
    centerline: LineString,
    terrain: TerrainSampler,
    spacing_m: float = 25.0,
    slope_percentile: float = 20.0,
    slope_perc_window_m: float = 2500.0,
    slope_smooth_window_m: float = 5000.0,
) -> BedProfile:
    """Sample bed elevation along ``centerline`` with ``terrain(xs, ys)`` and build the
    overall slope. The line is oriented so s=0 is the lower-bed (downstream) end.

    For a raster DEM pass :class:`channel_designer.volumes.raster.RasterTerrain`.
    """
    line = centerline
    length = line.length
    s = np.arange(0.0, length + spacing_m, spacing_m)
    s[-1] = min(s[-1], length)
    pts = [line.interpolate(float(d)) for d in s]
    z = np.asarray(terrain(np.array([p.x for p in pts]), np.array([p.y for p in pts])),
                   dtype="float64")
    z = np.where(np.isfinite(z), z, np.nan)

    head = np.nanmean(z[: max(3, z.size // 10)])
    tail = np.nanmean(z[-max(3, z.size // 10):])
    if np.isfinite(head) and np.isfinite(tail) and head > tail:
        line = LineString(list(line.coords)[::-1])
        z = z[::-1].copy()
        pts = pts[::-1]

    z_overall = overall_bottom_slope(
        z, spacing_m, percentile=slope_percentile,
        perc_window_m=slope_perc_window_m, smooth_window_m=slope_smooth_window_m,
    )
    return BedProfile(s=s, z_raw=z, z_overall=z_overall, points=pts, line=line)


@dataclass
class Weir:
    index: int
    chainage_m: float
    point: Point
    bed_m: float          # smoothed bed elevation at the weir
    foundation_m: float   # founded d below the bed
    crest_m: float        # = upstream pool water level
    pool_water_level_m: float   # water level of the pool UPSTREAM of this weir
    pool_start_m: float   # chainage of this weir
    pool_end_m: float     # chainage of the next weir (or centerline end)

    @property
    def pool_length_m(self) -> float:
        return self.pool_end_m - self.pool_start_m


def _first_crossing(s: np.ndarray, z: np.ndarray, i0: int, level: float):
    """First chainage upstream of index ``i0`` where bed reaches ``level``.

    Returns (chainage, point_index) using linear interpolation between samples,
    or None if the bed never reaches ``level`` before the upstream end.
    """
    ahead = np.where(z[i0 + 1:] >= level)[0]
    if ahead.size == 0:
        return None
    j = i0 + 1 + int(ahead[0])
    z0, z1 = z[j - 1], z[j]
    if z1 == z0:
        return float(s[j]), j
    frac = (level - z0) / (z1 - z0)
    chainage = float(s[j - 1] + frac * (s[j] - s[j - 1]))
    return chainage, j


def place_weirs(profile: BedProfile, weir_height_m: float, implantation_depth_m: float,
                start_chainage_m: float | None = None,
                start_elevation_m: float = 0.0,
                max_weir_elevation_m: float | None = None) -> list[Weir]:
    """Cascade with exact water-level recursion: WL_n = WL_{n-1} + (H - d).

    Weir #1 sits where the overall bottom slope reaches ``start_elevation_m``
    (default 0) and founds d below it. Every
    subsequent weir is located where the overall slope rises to the previous
    pool's water level and founds d below it. This reproduces the confirmed rule
    (lift = H - d) and ignores the unrealistic terrain spikes.
    """
    s, z = profile.s, profile.z_overall
    H, d = weir_height_m, implantation_depth_m
    lift = H - d
    if lift <= 0:
        raise ValueError(f"pool lift must be > 0 (H={H}, d={d} -> lift={lift})")

    if start_chainage_m is None:
        # First chainage (upstream) where the overall bottom slope reaches the
        # start elevation -> weir #1.
        reached = np.where(z >= start_elevation_m)[0]
        i0 = int(reached[0]) if reached.size else 0
    else:
        i0 = min(max(int(np.searchsorted(s, start_chainage_m)), 0), s.size - 1)
    bed_start = float(z[i0])

    # Weir #1: founded d below the starting bed.
    weirs: list[Weir] = [Weir(
        index=1, chainage_m=float(s[i0]), point=profile.points[i0],
        bed_m=bed_start, foundation_m=bed_start - d, crest_m=bed_start + lift,
        pool_water_level_m=bed_start + lift,
        pool_start_m=float(s[i0]), pool_end_m=float(s[-1]),
    )]

    target = bed_start + lift   # WL_1: bed must rise to this for the next weir
    cur_i = i0
    idx = 2
    while True:
        hit = _first_crossing(s, z, cur_i, target)
        if hit is None:
            break                       # last pool runs to the upstream end
        chainage, j = hit
        # Upstream cutoff: don't install weirs where the bed has risen past the
        # ceiling (river is too high for navigation works above this).
        if max_weir_elevation_m is not None and target >= max_weir_elevation_m:
            break
        foundation = target - d         # founded d below the pool it dams
        crest = target + lift           # = WL of the pool upstream of this weir
        weirs.append(Weir(
            index=idx, chainage_m=chainage,
            point=profile.line.interpolate(chainage),
            bed_m=target, foundation_m=foundation, crest_m=crest,
            pool_water_level_m=crest,
            pool_start_m=chainage, pool_end_m=float(s[-1]),
        ))
        target = crest
        cur_i = j
        idx += 1

    # Stitch pool_end to the next weir's chainage.
    for a, b in zip(weirs, weirs[1:]):
        a.pool_end_m = b.pool_start_m
    return weirs


def place_weirs_explicit(profile: BedProfile, cascade: list[dict]) -> list[Weir]:
    """Build the cascade from an EXPLICIT input instead of the H-d recursion.

    ``cascade`` is a list of dicts, one per weir, providing:
        chainage_m          location along the centreline (m from the mouth),
        pool_mwl_m          minimum water level of the pool UPSTREAM of the weir
                            (= the fixed-weir crest elevation),
        structure_height_m  crest - foundation (so foundation = crest - H).
    Entries are sorted by chainage. ``bed_m`` is sampled from the smoothed
    ``z_overall`` at each weir. The reach downstream of weir #1 sits at the
    downstream water level (e.g. sea level = 0) — passed to the volume functions as
    ``downstream_water_level_m``, not stored on the weirs.
    """
    s, z = profile.s, profile.z_overall
    items = sorted(cascade, key=lambda w: float(w["chainage_m"]))
    weirs: list[Weir] = []
    for i, w in enumerate(items, start=1):
        ch = float(w["chainage_m"])
        mwl = float(w["pool_mwl_m"])
        H = float(w.get("structure_height_m") or 0.0)
        j = int(np.clip(np.searchsorted(s, ch), 0, s.size - 1))
        weirs.append(Weir(
            index=i, chainage_m=ch, point=profile.line.interpolate(ch),
            bed_m=float(z[j]), foundation_m=mwl - H, crest_m=mwl,
            pool_water_level_m=mwl,
            pool_start_m=ch, pool_end_m=float(s[-1]),
        ))
    for a, b in zip(weirs, weirs[1:]):
        a.pool_end_m = b.pool_start_m
    return weirs


def water_level_at(weirs: list, s: float, downstream_water_level_m: float = 0.0) -> float:
    """Pool water level at chainage ``s``: the pool of the last weir at or below ``s``;
    below weir #1, the downstream water level."""
    wl = downstream_water_level_m
    for w in sorted(weirs, key=lambda w: w.pool_start_m):
        if s >= w.pool_start_m:
            wl = w.pool_water_level_m
        else:
            break
    return wl


def grade_at(weirs: list, s: float, depth_m: float, downstream_water_level_m: float = 0.0,
             overdepth_m: float = 0.0) -> float:
    """Design invert at chainage ``s`` = pool WL − navigable depth − overdepth."""
    return water_level_at(weirs, s, downstream_water_level_m) - depth_m - overdepth_m


def water_level_fn(weirs: list, downstream_water_level_m: float = 0.0) -> Callable[[float], float]:
    """``wl(s)`` callable for :func:`channel_designer.volumes.sections.section_volumes`."""
    return lambda s: water_level_at(weirs, s, downstream_water_level_m)
