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

from dataclasses import dataclass, field
from typing import Callable, List, Optional, Union

import numpy as np
from shapely import contains_xy
from shapely.geometry import LineString

from .cut_model import (
    CUT_MODEL,
    CUT_MODEL_B,
    DX_M,
    MAXW_M,
    STATION_M,
    WET_SLOPE_H_PER_V,
    cut_model_a_bench_gate,
    cut_model_a_section_face,
    normalise_model,
    section_area,
)

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


def _level(v: LevelLike) -> Callable[[float], float]:
    if callable(v):
        return v
    f = float(v)
    return lambda _s: f


def station_frame(line: LineString, station_m: float = STATION_M):
    """(chainages, xy, headings) every ``station_m`` along ``line``."""
    n = int(line.length // station_m) + 1
    dsl = np.linspace(0, line.length, n)
    xy = np.array([(line.interpolate(d).x, line.interpolate(d).y) for d in dsl])
    dxy = np.gradient(xy, axis=0)
    th = np.arctan2(dxy[:, 1], dxy[:, 0])
    return dsl, xy, th


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
    mdl = normalise_model(model)
    if mdl not in (CUT_MODEL, CUT_MODEL_B):
        raise ValueError(f"unknown cut model {model!r}; use 'a' or 'b'")
    wl_at = _level(wl)
    dsl, xy, th = station_frame(line, station_m)
    if callable(hb):
        hb_st = np.asarray(hb(xy), dtype=float)
    else:
        hb_arr = np.asarray(hb, dtype=float)
        hb_st = np.full(len(xy), float(hb_arr)) if hb_arr.ndim == 0 else hb_arr
    if hb_st.shape != (len(xy),):
        raise ValueError(f"hb must give one half-width per station ({len(xy)}); got {hb_st.shape}")

    offs = np.arange(-maxw_m, maxw_m + 0.1 * dx_m, dx_m)
    sidewhich = [(-1 if o < 0 else 1) for o in offs]
    m = len(offs)
    out = SectionVolumes(station_m=station_m)
    for a in range(0, len(xy), chunk):
        b = min(a + chunk, len(xy))
        idx = list(range(a, b))
        nx = -np.sin(th[idx]); ny = np.cos(th[idx])
        AX = (xy[idx, 0][:, None] + offs[None, :] * nx[:, None]).ravel()
        AY = (xy[idx, 1][:, None] + offs[None, :] * ny[:, None]).ravel()
        Z = np.asarray(terrain(AX, AY), dtype=float).reshape(len(idx), m)
        W = contains_xy(river_polygon, AX, AY).reshape(len(idx), m)
        for j, i in enumerate(idx):
            s = float(dsl[i])
            wl_i = float(wl_at(s)); grade = wl_i - depth
            z = Z[j]
            zl = [None if not np.isfinite(v) else float(v) for v in z]
            wet = [bool(zl[k] is not None and W[j, k]) for k in range(m)]
            dr, ex, fh = section_area(zl, offs, sidewhich, hb_st[i], grade, wl_i, depth, mdl,
                                      wet, wet_slope=wet_slope, dx=dx_m, run_m=daylight_run_m)
            bench = cut_model_a_bench_gate(offs, zl, hb_st[i], wl_i, depth, sidewhich,
                                           wet_slope=wet_slope)
            face = cut_model_a_section_face(offs, zl, hb_st[i], grade, wl_i, depth,
                                            wet_slope=wet_slope, model=mdl,
                                            run_m=daylight_run_m)
            out.stations.append(CrossSection(
                chainage_m=s, x=float(xy[i, 0]), y=float(xy[i, 1]), heading_rad=float(th[i]),
                offsets=offs, terrain=z.copy(), wet=W[j].copy(), hb=float(hb_st[i]),
                wl=wl_i, grade=grade, depth=float(depth), model=mdl,
                cutface=np.array([np.nan if f is None else f for f in face], dtype=float),
                dredge_area_m2=float(dr), excav_area_m2=float(ex), max_face_h_m=float(fh),
                bench_left=bool(bench[-1]), bench_right=bool(bench[1]),
                tag=(tag(s) if tag is not None else ""),
            ))
    return out


def trapezoid_area(hb: float, depth: float, side_slope: float) -> float:
    """Wet cross-section area of a trapezoidal channel (m²): (bottom + top) / 2 × depth."""
    top = 2.0 * hb + 2.0 * side_slope * depth
    return 0.5 * (2.0 * hb + top) * depth
