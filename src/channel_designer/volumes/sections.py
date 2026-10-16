"""Cross-section earthworks along an alignment: the station walk around the cut models.

Walks the alignment every ``station_m``, samples a transverse terrain profile out to
``±maxw_m`` at ``dx_m`` spacing, splits it wet/dry against the present river polygon,
applies Cut Model A or B (:mod:`.cut_model`), and integrates cut area × station spacing into
dredge (wet) and excavation (dry) volumes. The pool water level per chainage comes from the
caller (a float, or ``wl(s)`` — see :func:`channel_designer.volumes.pools.water_level_fn`).

Terrain comes in as a sampler ``terrain(xs, ys) -> z`` (NaN where unknown), so the same code
runs on an analytic surface, a numpy grid, or a raster DEM
(:class:`channel_designer.volumes.raster.RasterTerrain`, ``[dem]`` extra).

Quantities only — m², m³, metres of face height.
"""
from __future__ import annotations

from .. import _rust

from dataclasses import dataclass, field
from typing import Callable, List, Optional, Union

import numpy as np
from shapely.geometry import LineString

from .cut_model import CUT_MODEL, DX_M, MAXW_M, STATION_M, WET_SLOPE_H_PER_V

TerrainSampler = Callable[[np.ndarray, np.ndarray], np.ndarray]
LevelLike = Union[float, Callable[[float], float]]


@dataclass
class CrossSection:
    """One transverse profile and its cut. Offsets are signed: − left bank, + right bank."""

    chainage_m: float
    x: float
    y: float
    heading_rad: float
    offsets: np.ndarray
    terrain: np.ndarray          # NaN where unknown
    wet: np.ndarray              # True inside the present river
    hb: float                    # bottom half-width
    wl: float
    grade: float
    depth: float
    model: str
    cutface: np.ndarray          # design face elevation, NaN where no cut face
    dredge_area_m2: float
    excav_area_m2: float
    max_face_h_m: float
    bench_left: bool
    bench_right: bool
    tag: str = ""


@dataclass
class SectionVolumes:
    stations: List[CrossSection] = field(default_factory=list)
    station_m: float = STATION_M

    @property
    def dredge_m3(self) -> float:
        return float(sum(s.dredge_area_m2 for s in self.stations) * self.station_m)

    @property
    def excav_m3(self) -> float:
        return float(sum(s.excav_area_m2 for s in self.stations) * self.station_m)

    @property
    def total_m3(self) -> float:
        return self.dredge_m3 + self.excav_m3

    @property
    def max_face_h_m(self) -> float:
        return max((s.max_face_h_m for s in self.stations), default=0.0)

    def by_tag(self) -> dict:
        out: dict = {}
        for s in self.stations:
            t = out.setdefault(s.tag, {"dredge_m3": 0.0, "excav_m3": 0.0})
            t["dredge_m3"] += s.dredge_area_m2 * self.station_m
            t["excav_m3"] += s.excav_area_m2 * self.station_m
        return out


def station_frame(line: LineString, station_m: float = STATION_M):
    """(chainages, xy, headings) every ``station_m`` along ``line``."""
    r = _rust.call("sections", "station_frame", line=line, station_m=station_m)
    return (np.asarray(r["chainages"], dtype=float), np.asarray(r["xy"], dtype=float).reshape(-1, 2),
            np.asarray(r["headings"], dtype=float))


def section_volumes(line: LineString, terrain: TerrainSampler, river_polygon,
                    wl: LevelLike, depth: float, hb, *, model: str = CUT_MODEL,
                    wet_slope: float = WET_SLOPE_H_PER_V, station_m: float = STATION_M,
                    maxw_m: float = MAXW_M, dx_m: float = DX_M,
                    daylight_run_m: Optional[float] = None,
                    tag: Optional[Callable[[float], str]] = None,
                    chunk: int = 40) -> SectionVolumes:
    """Dredge and excavation volumes along ``line`` under Cut Model A or B.

    ``hb`` is the bottom half-width: a float, a per-station array (length = number of
    stations from :func:`station_frame`), or ``hb(xy) -> array`` (e.g.
    ``lambda xy: cut_model.widened_hb_from_shape(xy, shape)``).
    ``wl`` is the pool water level, a float or ``wl(s)``; grade = wl − depth.
    ``tag(s)`` optionally labels each station (e.g. "complex"/"straight") for ``by_tag``.
    """
    c = _rust.Call()
    r = _rust.call("sections", "section_volumes", _call=c, line=line, terrain=c.terrain(terrain),
                   river_polygon=river_polygon, wl=c.scalar(wl) if callable(wl) else wl, depth=depth,
                   hb=c.stations(hb) if callable(hb) else hb, model=model, wet_slope=wet_slope,
                   station_m=station_m, maxw_m=maxw_m, dx_m=dx_m, daylight_run_m=daylight_run_m,
                   tag=c.text(tag) if tag is not None else None, chunk=int(chunk), detail_stations="all")
    stations = []
    for st, detail in zip(r["stations"], r["detail"]):
        st = _rust.plain(st)
        stations.append(CrossSection(
            **st, offsets=np.asarray(_rust.plain(detail["offsets"]), dtype=float),
            terrain=np.asarray(_rust.plain(detail["terrain"]), dtype=float),
            wet=np.asarray(detail["wet"], dtype=bool),
            cutface=np.asarray(_rust.plain(detail["cutface"]), dtype=float)))
    return SectionVolumes(stations=stations, station_m=float(_rust.plain(r["station_m"])))


def trapezoid_area(hb: float, depth: float, side_slope: float) -> float:
    """Wet cross-section area of a trapezoidal channel (m²): (bottom + top) / 2 × depth."""
    return _rust.call("sections", "trapezoid_area", float, hb=hb, depth=depth, side_slope=side_slope)
