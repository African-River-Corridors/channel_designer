"""Raster DEM volumes: dredging vs. virgin-terrain excavation, per pool and per chunk.

Needs the ``[dem]`` extra (rasterio, scipy, affine): ``pip install channel_designer[dem]``.

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

from dataclasses import dataclass, field

import numpy as np
from shapely.geometry import LineString
from shapely.ops import substring

try:
    import rasterio
    from affine import Affine
    from rasterio.enums import Resampling
    from rasterio.features import geometry_mask, rasterize
    from rasterio.windows import Window, from_bounds
    from scipy.ndimage import distance_transform_edt, median_filter
except ImportError as exc:  # pragma: no cover - exercised only without the extra
    raise ImportError(
        "channel_designer.volumes.raster needs the [dem] extra: "
        "pip install 'channel_designer[dem]'") from exc

from .pools import Weir

# cut-depth stratification: volume removed in each 1 m depth band, 0..40 m
DEPTH_NBINS = 40
_BANDS = np.arange(DEPTH_NBINS)


def _layer_volumes(vals: np.ndarray) -> np.ndarray:
    """Volume per 1 m depth band: band i gets sum(clip(cut - i, 0, 1)) (× cell area
    applied by the caller). Sums to the total volume, so it's a true stratification."""
    if vals.size == 0:
        return np.zeros(DEPTH_NBINS)
    return np.clip(vals[:, None] - _BANDS[None, :], 0.0, 1.0).sum(axis=0)


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


def _despike(z: np.ndarray, valid: np.ndarray, fill: float,
             window: int, k: float, floor: float):
    """Spatial Hampel filter: replace cells deviating from the local median by
    more than ``max(floor, k * 1.4826 * MAD)`` with that median. Removes isolated
    DEM spikes; preserves coherent banks. Returns (cleaned_z, spike_mask)."""
    zf = np.where(valid, z, fill)
    med = median_filter(zf, size=window, mode="nearest")
    mad = median_filter(np.abs(zf - med), size=window, mode="nearest")
    tol = np.maximum(floor, k * 1.4826 * mad)
    spike = valid & (np.abs(zf - med) > tol)
    return np.where(spike, med, z), spike


class _Accum:
    __slots__ = ("dredge", "excav", "deep", "darea", "earea", "deparea",
                 "maxcut", "sumpos", "npos", "spikes", "dhist", "ehist")

    def __init__(self):
        self.dredge = self.excav = self.deep = 0.0
        self.darea = self.earea = self.deparea = 0.0
        self.maxcut = self.sumpos = 0.0
        self.npos = self.spikes = 0
        self.dhist = np.zeros(DEPTH_NBINS)
        self.ehist = np.zeros(DEPTH_NBINS)


def _pool_for_chainage(weirs: list[Weir], s: float) -> Weir | None:
    chosen = None
    for w in weirs:
        if w.pool_start_m <= s < w.pool_end_m:
            return w
        if w.pool_start_m <= s:
            chosen = w
    return chosen


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
    if not weirs:
        return []

    # Reach 0 = downstream of the first weir (km 0 -> weir 1): not impounded, so the
    # navigable channel is checked against the tail/sea water level given here.
    has_reach0 = downstream_water_level_m is not None
    start = 0.0 if has_reach0 else weirs[0].pool_start_m
    end = weirs[-1].pool_end_m
    edges = sorted(set(
        [start, end]
        + [start + i * chunk_m for i in range(1, int((end - start) / chunk_m) + 1)]
        + [w.pool_start_m for w in weirs]
    ))
    edges = [e for e in edges if start <= e <= end]
    chunks = list(zip(edges, edges[1:]))

    accum: dict[int, _Accum] = {w.index: _Accum() for w in weirs}
    grade = {w.index: w.pool_water_level_m - navigable_depth_m - overdepth_m for w in weirs}
    if has_reach0:
        accum[0] = _Accum()
        grade[0] = downstream_water_level_m - navigable_depth_m - overdepth_m
    weir1_start = weirs[0].pool_start_m
    half_bottom = channel_bottom_width_m / 2.0

    read_decimate = max(1, int(read_decimate))
    with rasterio.open(terrain_path) as ds:
        nodata = ds.nodata
        px = abs(ds.res[0]) * read_decimate      # effective cell size after decimation
        cell_area = px * px
        full = Window(0, 0, ds.width, ds.height)

        for c0, c1 in chunks:
            if c1 - c0 < 1e-6:
                continue
            mid = (c0 + c1) / 2.0
            if has_reach0 and mid < weir1_start:
                idx = 0
            else:
                pool = _pool_for_chainage(weirs, mid)
                if pool is None:
                    continue
                idx = pool.index
            g = grade[idx]

            # local half bottom width: constant (user) or from the PIANC nav limits
            hb = half_bottom
            if section_mode == "pianc" and left_limit is not None and right_limit is not None:
                pmid = centerline.interpolate(mid)
                half_top = 0.5 * (left_limit.distance(pmid) + right_limit.distance(pmid))
                hb = max(half_top - navigable_depth_m * side_slope_h_per_v, 0.0)

            seg = substring(centerline, c0, c1)
            corridor = seg.buffer(max_corridor_half_width_m, cap_style=2)  # flat caps
            win = from_bounds(*corridor.bounds, transform=ds.transform)
            win = win.round_offsets().round_lengths().intersection(full)
            if win.width <= 0 or win.height <= 0:
                continue

            if read_decimate > 1:
                oh = max(1, int(win.height // read_decimate))
                ow = max(1, int(win.width // read_decimate))
                z = ds.read(1, window=win, out_shape=(oh, ow),
                            resampling=Resampling.average).astype("float64")
                wt = ds.window_transform(win) * Affine.scale(win.width / ow, win.height / oh)
            else:
                z = ds.read(1, window=win).astype("float64")
                wt = ds.window_transform(win)

            in_corridor = ~geometry_mask([corridor], z.shape, wt, invert=False)
            # perpendicular offset from the centerline via Euclidean distance
            line_mask = rasterize([(seg, 1)], out_shape=z.shape, transform=wt,
                                  all_touched=True, dtype="uint8").astype(bool)
            if not line_mask.any():
                continue
            offset = distance_transform_edt(~line_mask, sampling=px)

            template = g + np.maximum(0.0, offset - hb) / side_slope_h_per_v

            valid = in_corridor & np.isfinite(z)
            if nodata is not None:
                valid &= z != nodata

            n_spikes = 0
            z_used = z
            if despike:
                z_used, spike = _despike(z, valid, g, despike_window_px,
                                         despike_k, despike_floor_m)
                n_spikes = int((spike & in_corridor).sum())

            cut = np.where(valid, z_used - template, 0.0)
            np.clip(cut, 0.0, None, out=cut)

            in_river = ~geometry_mask([river_poly], z.shape, wt, invert=False)
            dredge_cells = valid & in_river & (cut > 0)
            excav_cells = valid & (~in_river) & (cut > 0)
            deep_cells = excav_cells & (cut > deep_cut_threshold_m)

            a = accum[idx]
            a.dredge += float(cut[dredge_cells].sum() * cell_area)
            a.excav += float(cut[excav_cells].sum() * cell_area)
            a.deep += float(cut[deep_cells].sum() * cell_area)
            a.darea += float(dredge_cells.sum() * cell_area)
            a.earea += float(excav_cells.sum() * cell_area)
            a.deparea += float(deep_cells.sum() * cell_area)
            a.maxcut = max(a.maxcut, float(cut.max()) if cut.size else 0.0)
            pos = cut[cut > 0]
            a.sumpos += float(pos.sum())
            a.npos += int(pos.size)
            a.spikes += n_spikes
            if dredge_cells.any():
                a.dhist += _layer_volumes(cut[dredge_cells]) * cell_area
            if excav_cells.any():
                a.ehist += _layer_volumes(cut[excav_cells]) * cell_area

    def _to_volume(index, wl, a):
        return PoolVolume(
            weir_index=index, pool_water_level_m=wl, design_grade_m=grade[index],
            dredge_volume_m3=a.dredge, excavation_volume_m3=a.excav,
            dredge_area_m2=a.darea, excavation_area_m2=a.earea,
            max_cut_m=a.maxcut, mean_cut_m=(a.sumpos / a.npos) if a.npos else 0.0,
            despiked_cells=a.spikes, deep_excavation_volume_m3=a.deep,
            deep_excavation_area_m2=a.deparea,
            dredge_hist=a.dhist.tolist(), excav_hist=a.ehist.tolist())

    results = []
    if has_reach0:
        results.append(_to_volume(0, float(downstream_water_level_m), accum[0]))
    for w in weirs:
        results.append(_to_volume(w.index, w.pool_water_level_m, accum[w.index]))
    return results


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
    if not weirs:
        return []
    has_reach0 = downstream_water_level_m is not None
    start = 0.0 if has_reach0 else weirs[0].pool_start_m
    end = weirs[-1].pool_end_m

    # Build uniform chunks but force a break at every pool boundary so each
    # chunk belongs to exactly one pool (single grade/WL per record).
    edges = {start, end}
    s = start + chunk_m
    while s < end:
        edges.add(s)
        s += chunk_m
    edges |= {w.pool_start_m for w in weirs}
    edges = sorted(e for e in edges if start <= e <= end)
    chunks = list(zip(edges, edges[1:]))

    weir1_start = weirs[0].pool_start_m
    half_bottom = channel_bottom_width_m / 2.0
    grade = {w.index: w.pool_water_level_m - navigable_depth_m - overdepth_m for w in weirs}
    wl_by = {w.index: w.pool_water_level_m for w in weirs}
    if has_reach0:
        grade[0] = downstream_water_level_m - navigable_depth_m - overdepth_m
        wl_by[0] = float(downstream_water_level_m)

    records: list[dict] = []
    read_decimate = max(1, int(read_decimate))
    with rasterio.open(terrain_path) as ds:
        nodata = ds.nodata
        px = abs(ds.res[0]) * read_decimate
        cell_area = px * px
        full = Window(0, 0, ds.width, ds.height)

        for c0, c1 in chunks:
            if c1 - c0 < 1e-6:
                continue
            mid = (c0 + c1) / 2.0
            if has_reach0 and mid < weir1_start:
                idx = 0
            else:
                pool = _pool_for_chainage(weirs, mid)
                if pool is None:
                    continue
                idx = pool.index
            g = grade[idx]

            hb = half_bottom
            if section_mode == "pianc" and left_limit is not None and right_limit is not None:
                pmid = centerline.interpolate(mid)
                half_top = 0.5 * (left_limit.distance(pmid) + right_limit.distance(pmid))
                hb = max(half_top - navigable_depth_m * side_slope_h_per_v, 0.0)

            # widening vs cut-through: is the design centreline on the present river here?
            is_cut = not river_poly.contains(centerline.interpolate(mid))
            design_w = 2.0 * hb + 2.0 * side_slope_h_per_v * navigable_depth_m
            base = dict(s0_m=float(c0), s1_m=float(c1), grade_m=float(g), wl_m=float(wl_by[idx]),
                        pool=int(idx), len_m=float(c1 - c0), is_cut_through=bool(is_cut),
                        design_width_m=float(design_w), dredge_m3=0.0, excav_m3=0.0,
                        widening_m3=0.0, cut_through_m3=0.0, deep_m3=0.0, river_width_m=0.0)

            seg = substring(centerline, c0, c1)
            corridor = seg.buffer(max_corridor_half_width_m, cap_style=2)
            win = from_bounds(*corridor.bounds, transform=ds.transform)
            win = win.round_offsets().round_lengths().intersection(full)
            if win.width <= 0 or win.height <= 0:
                records.append(base)
                continue

            if read_decimate > 1:
                oh = max(1, int(win.height // read_decimate))
                ow = max(1, int(win.width // read_decimate))
                z = ds.read(1, window=win, out_shape=(oh, ow),
                            resampling=Resampling.average).astype("float64")
                wt = ds.window_transform(win) * Affine.scale(win.width / ow, win.height / oh)
            else:
                z = ds.read(1, window=win).astype("float64")
                wt = ds.window_transform(win)

            in_corridor = ~geometry_mask([corridor], z.shape, wt, invert=False)
            line_mask = rasterize([(seg, 1)], out_shape=z.shape, transform=wt,
                                  all_touched=True, dtype="uint8").astype(bool)
            if not line_mask.any():
                records.append(base)
                continue
            offset = distance_transform_edt(~line_mask, sampling=px)
            template = g + np.maximum(0.0, offset - hb) / side_slope_h_per_v

            valid = in_corridor & np.isfinite(z)
            if nodata is not None:
                valid &= z != nodata

            z_used = z
            if despike:
                z_used, _ = _despike(z, valid, g, despike_window_px,
                                     despike_k, despike_floor_m)

            in_river = ~geometry_mask([river_poly], z.shape, wt, invert=False)

            if slope_mode in ("dry_bank_only", "fairway_only"):
                fairway = valid & (offset <= hb)
                fairway_cut = np.where(fairway, z_used - g, 0.0)
                # does the fairway bite into dry ground in this chunk?
                fairway_hits_dry = (slope_mode == "dry_bank_only") and bool(
                    ((fairway_cut > 0) & (~in_river)).any())
                cut = np.where(fairway, np.clip(fairway_cut, 0.0, None), 0.0)
                if fairway_hits_dry:
                    # carve the stability slope, but only through dry ground
                    slope_cells = valid & (offset > hb) & (~in_river)
                    slope_cut = np.where(slope_cells, z_used - template, 0.0)
                    cut = cut + np.clip(slope_cut, 0.0, None)
            else:
                cut = np.where(valid, z_used - template, 0.0)
                np.clip(cut, 0.0, None, out=cut)

            dredge_cells = valid & in_river & (cut > 0)
            excav_cells = valid & (~in_river) & (cut > 0)
            deep_cells = excav_cells & (cut > deep_cut_threshold_m)
            excav_vol = float(cut[excav_cells].sum() * cell_area)

            rec = dict(base)
            rec.update(
                dredge_m3=float(cut[dredge_cells].sum() * cell_area),
                excav_m3=excav_vol,
                widening_m3=0.0 if is_cut else excav_vol,
                cut_through_m3=excav_vol if is_cut else 0.0,
                deep_m3=float(cut[deep_cells].sum() * cell_area),
                river_width_m=float((in_river & in_corridor).sum() * cell_area / (c1 - c0)),
            )
            records.append(rec)
    return records


def bilinear(ds, xs, ys):
    """Bilinear sample of an open rasterio dataset at world coords, via one windowed read.

    Pixel-centre convention. NaN outside the outermost pixel centres or on nodata. Call per
    chunk of nearby points.
    """
    xs = np.asarray(xs, dtype=float); ys = np.asarray(ys, dtype=float)
    out = np.full(xs.shape, np.nan)
    if xs.size == 0:
        return out
    xmin, xmax, ymin, ymax = xs.min() - 8, xs.max() + 8, ys.min() - 8, ys.max() + 8
    win = from_bounds(xmin, ymin, xmax, ymax, ds.transform).round_offsets().round_lengths()
    try:
        win = win.intersection(Window(0, 0, ds.width, ds.height))
    except rasterio.errors.WindowError:
        return out                                   # every point is off the raster
    z = ds.read(1, window=win).astype("float64")
    if ds.nodata is not None:
        z = np.where(z == ds.nodata, np.nan, z)
    tr = ds.window_transform(win)
    # Raster values sit at pixel CENTRES, so shift by half a pixel before interpolating.
    # (The original helper omitted the 0.5 and sampled half a pixel off — see S17c.)
    col = (xs - tr.c) / tr.a - 0.5
    row = (ys - tr.f) / tr.e - 0.5
    H, W = z.shape
    c0 = np.floor(col).astype(int); r0 = np.floor(row).astype(int)
    fc = col - c0; fr = row - r0
    ok = (c0 >= 0) & (r0 >= 0) & (c0 < W - 1) & (r0 < H - 1)
    if ok.any():
        c0o, r0o, fco, fro = c0[ok], r0[ok], fc[ok], fr[ok]
        v = (z[r0o, c0o] * (1 - fco) * (1 - fro) + z[r0o, c0o + 1] * fco * (1 - fro)
             + z[r0o + 1, c0o] * (1 - fco) * fro + z[r0o + 1, c0o + 1] * fco * fro)
        out[ok] = v
    return out


class RasterTerrain:
    """A terrain sampler ``terrain(xs, ys) -> z`` over a raster DEM (bilinear, NaN outside).

    The DEM must be in the same projected CRS as the geometry. Use as a context manager,
    or call :meth:`close` when done.
    """

    def __init__(self, path):
        self.path = path
        self.ds = rasterio.open(path)

    def __call__(self, xs, ys):
        return bilinear(self.ds, xs, ys)

    def close(self):
        self.ds.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
