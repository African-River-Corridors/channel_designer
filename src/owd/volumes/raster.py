"""Raster DEM volumes: dredging vs. virgin-terrain excavation, per pool and per chunk.

Since 0.2.0 the Rust core reads the GeoTIFF itself: no rasterio, scipy or GDAL is needed (the
``[dem]`` extra is kept so existing install commands keep working).

The dredged/excavated channel is a TRAPEZOIDAL prism anchored to the centerline:

    bottom width  = channel_bottom_width_m   (flat bottom at the design grade)
    grade         = pool_water_level - navigable_depth - overdepth
    side slopes   = side_slope_h_per_v : 1   (H:V), rising from the bottom edges

The design template elevation at a point whose perpendicular offset from the
centerline is ``o`` is::

    template(o) = grade + max(0, o - bottom_width/2) / side_slope_h_per_v

The cut at each cell is ``max(0, terrain - template)``. It is split by the
present-day river polygon:

    * cell INSIDE  the river polygon  -> dredging (underwater, in the channel)
    * cell OUTSIDE the river polygon  -> excavation of virgin terrain (meander cut)

The channel is integrated in short chainage chunks so memory/CPU is bounded by
reach length, not by the number of weirs.
"""
from __future__ import annotations

from .. import _rust

import warnings
from dataclasses import dataclass, field

import numpy as np
from shapely.geometry import LineString

from .pools import Weir

# cut-depth stratification: volume removed in each 1 m depth band, 0..40 m
DEPTH_NBINS = 40


@dataclass
class PoolVolume:
    weir_index: int
    pool_water_level_m: float
    design_grade_m: float
    dredge_volume_m3: float
    excavation_volume_m3: float
    dredge_area_m2: float
    excavation_area_m2: float
    max_cut_m: float
    mean_cut_m: float
    despiked_cells: int = 0
    # Subset of excavation from cells cut deeper than the threshold (channel hits
    # steep/high ground). Already INCLUDED in excavation_volume_m3.
    deep_excavation_volume_m3: float = 0.0
    deep_excavation_area_m2: float = 0.0
    # volume (m3) per 1 m cut-depth band (index i = depth i..i+1 m)
    dredge_hist: list = field(default_factory=list)
    excav_hist: list = field(default_factory=list)

    @property
    def total_volume_m3(self) -> float:
        return self.dredge_volume_m3 + self.excavation_volume_m3

    @property
    def bulk_excavation_volume_m3(self) -> float:
        """Excavation excluding the deep-cut / routing-conflict cells."""
        return self.excavation_volume_m3 - self.deep_excavation_volume_m3


def _check_file(path) -> None:
    """Refuse a missing file the way opening it always did (an ``OSError``)."""
    import os
    if not os.path.exists(path):
        raise FileNotFoundError(f"{path}: no such raster")


def _weir_arg(w: Weir) -> dict:
    """The part of a weir the integrator reads."""
    return {"index": w.index, "pool_start_m": w.pool_start_m, "pool_end_m": w.pool_end_m,
            "pool_water_level_m": w.pool_water_level_m, "bed_m": w.bed_m, "foundation_m": w.foundation_m}


def compute_pool_volumes(
    weirs: list[Weir],
    centerline: LineString,
    river_poly,
    terrain_path,
    navigable_depth_m: float,
    channel_bottom_width_m: float = 67.2,
    side_slope_h_per_v: float = 3.0,
    overdepth_m: float = 0.0,
    despike: bool = True,
    despike_window_px: int = 5,
    despike_k: float = 4.0,
    despike_floor_m: float = 3.0,
    deep_cut_threshold_m: float = 20.0,
    chunk_m: float = 2000.0,
    max_corridor_half_width_m: float = 300.0,
    section_mode: str = "user",
    left_limit=None,
    right_limit=None,
    downstream_water_level_m: float | None = None,
    read_decimate: int = 1,
) -> list[PoolVolume]:
    _check_file(terrain_path)
    rows = _rust.call("raster", "compute_pool_volumes", raster={"path": str(terrain_path)},
                      weirs=[_weir_arg(w) for w in weirs], centerline=centerline, river_poly=river_poly,
                      navigable_depth_m=navigable_depth_m, channel_bottom_width_m=channel_bottom_width_m,
                      side_slope_h_per_v=side_slope_h_per_v, overdepth_m=overdepth_m, despike=bool(despike),
                      despike_window_px=int(despike_window_px), despike_k=despike_k,
                      despike_floor_m=despike_floor_m, deep_cut_threshold_m=deep_cut_threshold_m,
                      chunk_m=chunk_m, max_corridor_half_width_m=max_corridor_half_width_m,
                      section_mode=section_mode, left_limit=left_limit, right_limit=right_limit,
                      downstream_water_level_m=downstream_water_level_m, read_decimate=int(read_decimate))
    return _rust.decode(rows, list[PoolVolume])


def compute_chainage_volumes(
    weirs: list[Weir],
    centerline: LineString,
    river_poly,
    terrain_path,
    navigable_depth_m: float,
    channel_bottom_width_m: float = 67.2,
    side_slope_h_per_v: float = 3.0,
    overdepth_m: float = 0.0,
    despike: bool = True,
    despike_window_px: int = 5,
    despike_k: float = 4.0,
    despike_floor_m: float = 3.0,
    deep_cut_threshold_m: float = 20.0,
    chunk_m: float = 100.0,
    max_corridor_half_width_m: float = 300.0,
    section_mode: str = "user",
    left_limit=None,
    right_limit=None,
    downstream_water_level_m: float | None = None,
    read_decimate: int = 1,
    slope_mode: str = "always",
) -> list[dict]:
    """Per-chunk dredge/excavation breakdown along the centerline.

    slope_mode:
      "always"        — 1V:sH side slopes carved everywhere the template daylights
                        (the classical design section; default).
      "dry_bank_only" — the FAIRWAY (flat bottom at grade) is always cut, but the
                        stability slopes are carved ONLY in chunks where the fairway
                        itself intersects dry ground (widening / cut-through), and
                        then only on dry cells. Wet faces beyond the fairway are left
                        to slump and are re-dredged as maintenance (CSD retained).

    Same DEM/template logic as ``compute_pool_volumes`` but accumulates volumes
    in fine chainage chunks (default 100 m) instead of per pool. Returns a list
    of dicts (one per chunk) with keys:
        s0_m, s1_m         chunk start/end chainage (m)
        grade_m, wl_m      design grade and pool water level for this chunk
        dredge_m3, excav_m3  volumes in the chunk (m³)
        pool               pool index (0 = downstream reach if present)
    """
    _check_file(terrain_path)
    rows = _rust.call("raster", "compute_chainage_volumes", raster={"path": str(terrain_path)},
                      weirs=[_weir_arg(w) for w in weirs], centerline=centerline, river_poly=river_poly,
                      navigable_depth_m=navigable_depth_m, channel_bottom_width_m=channel_bottom_width_m,
                      side_slope_h_per_v=side_slope_h_per_v, overdepth_m=overdepth_m, despike=bool(despike),
                      despike_window_px=int(despike_window_px), despike_k=despike_k,
                      despike_floor_m=despike_floor_m, deep_cut_threshold_m=deep_cut_threshold_m,
                      chunk_m=chunk_m, max_corridor_half_width_m=max_corridor_half_width_m,
                      section_mode=section_mode, left_limit=left_limit, right_limit=right_limit,
                      downstream_water_level_m=downstream_water_level_m, read_decimate=int(read_decimate),
                      slope_mode=slope_mode)
    return _rust.plain(rows)


def bilinear(ds, xs, ys):
    """Bilinear sample of an open rasterio dataset at world coords, via one windowed read.

    Pixel-centre convention. NaN outside the outermost pixel centres or on nodata. Call per
    chunk of nearby points.
    """
    return _rust.call("raster", "bilinear", np.ndarray, raster={"path": str(ds.name)}, xs=xs, ys=ys)


class RasterTerrain:
    """A terrain sampler ``terrain(xs, ys) -> z`` over a raster DEM (bilinear, NaN outside).

    The DEM must be in the same projected CRS as the geometry. The core reads the GeoTIFF itself
    (no GDAL, no rasterio) and keeps it in memory until :meth:`close`. Use as a context manager,
    or call :meth:`close` when done.
    """

    def __init__(self, path):
        _check_file(path)
        self.path = path
        _rust.run("raster", "bilinear", {"raster": {"path": str(path)}, "xs": [], "ys": []})  # read it now

    def __call__(self, xs, ys):
        xs = np.asarray(xs, dtype=float)
        out = _rust.call("raster", "raster_terrain", np.ndarray, raster={"path": str(self.path)},
                         xs=xs.ravel(), ys=np.asarray(ys, dtype=float).ravel())
        return out.reshape(xs.shape)

    @property
    def ds(self):
        """Deprecated: the rasterio dataset this class used to hold. Needs rasterio; gone in 0.3.0."""
        warnings.warn("RasterTerrain.ds is deprecated: the Rust core reads the GeoTIFF and no rasterio "
                      "dataset is held. It is removed in 0.3.0.", DeprecationWarning, stacklevel=2)
        import rasterio
        return rasterio.open(self.path)

    def close(self):
        _rust.run("raster", "release_raster", {"path": str(self.path)})

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
