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

from .. import _rust

from dataclasses import dataclass
from typing import Callable

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
    return _rust.call("pools", "overall_bottom_slope", np.ndarray, z_raw=z_raw, spacing_m=spacing_m,
                      percentile=percentile, perc_window_m=perc_window_m, smooth_window_m=smooth_window_m)


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

    For a raster DEM pass :class:`owd.volumes.raster.RasterTerrain`.
    """
    c = _rust.Call()
    r = _rust.call("pools", "sample_bed_profile", _call=c, centerline=centerline, terrain=c.terrain(terrain),
                   spacing_m=spacing_m, slope_percentile=slope_percentile,
                   slope_perc_window_m=slope_perc_window_m, slope_smooth_window_m=slope_smooth_window_m)
    return _decode_profile(r)


def _profile_arg(profile: BedProfile) -> dict:
    """A :class:`BedProfile` as the core reads it."""
    return {"s": profile.s, "z_raw": profile.z_raw, "z_overall": profile.z_overall,
            "points": [[p.x, p.y] for p in profile.points], "line": profile.line}


def _decode_profile(r: dict) -> BedProfile:
    """A :class:`BedProfile` from the core's answer."""
    return BedProfile(s=np.asarray(_rust.plain(r["s"]), dtype=float),
                      z_raw=np.asarray(_rust.plain(r["z_raw"]), dtype=float),
                      z_overall=np.asarray(_rust.plain(r["z_overall"]), dtype=float),
                      points=[Point(p) for p in _rust.plain(r["points"])], line=_rust.plain(r["line"]))


def _decode_weirs(rows: list) -> list:
    out = []
    for w in rows:
        w = _rust.plain(w)
        w.pop("pool_length_m", None)
        w["point"] = Point(w["point"])
        out.append(Weir(**w))
    return out


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
    place = {"weir_height_m": weir_height_m, "implantation_depth_m": implantation_depth_m,
             "start_chainage_m": start_chainage_m, "start_elevation_m": start_elevation_m,
             "max_weir_elevation_m": max_weir_elevation_m}
    return _decode_weirs(_rust.call("pools", "place_weirs", profile=_profile_arg(profile), place=place))


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
    return _decode_weirs(_rust.call("pools", "place_weirs", profile=_profile_arg(profile),
                                    place={"kind": "explicit", "cascade": [dict(w) for w in cascade]}))


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
    """``wl(s)`` callable for :func:`owd.volumes.sections.section_volumes`."""
    return lambda s: water_level_at(weirs, s, downstream_water_level_m)
