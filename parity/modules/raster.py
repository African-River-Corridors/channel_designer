"""raster: `volumes/raster.py` (dredge and excavation volumes read from a DEM).

The DEM is a GeoTIFF the driver writes from a `raster` spec (below), so a case carries no file:

    {"x0": left edge, "y_top": top edge, "res": cell size, "width": cols, "height": rows,
     "terrain": <terrain spec>, "nodata": null or a value, "holes": [[x0, y0, x1, y1], ...]}

Cell (r, c) holds `terrain(x0 + (c + 0.5) * res, y_top - (r + 0.5) * res)` rounded to float32 (a
real DEM's type), or `nodata` inside a hole box. The Rust port builds the same grid in memory from
the spec, so the volume integrator is tested here and the GeoTIFF reader apart from parity (ticket 33).
"""
from __future__ import annotations

import os
import tempfile
from contextlib import contextmanager

import numpy as np
from shapely.geometry import LineString, Point

from ..common import Rng, case as _case, frozen, rline, synth, terrain_from

ENTRY_POINT = "raster"
R = "initial case set (spec 0001, ticket 23)"


@contextmanager
def tif_from(spec: dict):
    import rasterio
    from rasterio.transform import from_origin
    w, h, res = spec["width"], spec["height"], spec["res"]
    xs = spec["x0"] + (np.arange(w) + 0.5) * res
    ys = spec["y_top"] - (np.arange(h) + 0.5) * res
    gx, gy = np.meshgrid(xs, ys)
    z = np.asarray(terrain_from(spec["terrain"])(gx.ravel(), gy.ravel()), dtype=float).reshape(h, w)
    nodata = spec.get("nodata")
    for x0, y0, x1, y1 in spec.get("holes", []):
        z[(gx >= x0) & (gx <= x1) & (gy >= y0) & (gy <= y1)] = nodata if nodata is not None else np.nan
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "dem.tif")
        with rasterio.open(path, "w", driver="GTiff", height=h, width=w, count=1, dtype="float32",
                           crs="EPSG:3857", transform=from_origin(spec["x0"], spec["y_top"], res, res),
                           nodata=nodata) as ds:
            ds.write(z.astype("float32"), 1)
        yield path


def _weirs(args):
    pl = frozen("volumes.pools")
    line = args["centerline"]
    out = []
    for w in args["weirs"]:
        out.append(pl.Weir(index=w["index"], chainage_m=w["pool_start_m"], point=line.interpolate(w["pool_start_m"]),
                           bed_m=w.get("bed_m", 0.0), foundation_m=w.get("foundation_m", 0.0),
                           crest_m=w["pool_water_level_m"], pool_water_level_m=w["pool_water_level_m"],
                           pool_start_m=w["pool_start_m"], pool_end_m=w["pool_end_m"]))
    return out


def adapter(fn: str, args: dict):
    rs = frozen("volumes.raster")
    if fn == "bilinear":
        import rasterio
        with tif_from(args["raster"]) as path, rasterio.open(path) as ds:
            return rs.bilinear(ds, np.array(args["xs"], dtype=float), np.array(args["ys"], dtype=float))
    if fn in ("compute_pool_volumes", "compute_chainage_volumes"):
        kw = {k: v for k, v in args.items() if k not in ("raster", "weirs", "centerline", "river_poly")}
        for lim in ("left_limit", "right_limit"):
            if lim in kw and kw[lim] is None:
                del kw[lim]
        with tif_from(args["raster"]) as path:
            res = getattr(rs, fn)(_weirs(args), args["centerline"], args["river_poly"], path,
                                  kw.pop("navigable_depth_m"), **kw)
        return res
    if fn == "layer_volumes":
        return rs._layer_volumes(np.array(args["vals"], dtype=float))
    if fn == "despike":
        z, spike = rs._despike(np.array(args["z"], dtype=float), np.array(args["valid"], dtype=bool),
                               args["fill"], args["window"], args["k"], args["floor"])
        return {"z": z, "spike": spike}
    if fn == "raster_terrain":
        with tif_from(args["raster"]) as path:
            t = rs.RasterTerrain(path)
            try:
                return t(np.array(args["xs"], dtype=float), np.array(args["ys"], dtype=float))
            finally:
                t.close()
    raise KeyError(f"raster has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    hills = {"kind": "hills", "base": 4.0, "gx": 0.0, "gy": 0.0, "hills": [[500.0, 60.0, 12.0, 90.0]]}
    ras = dict(x0=0.0, y_top=200.0, res=10.0, width=100, height=40, terrain=hills)
    pts_x = [3.0, 14.9, 15.0, 15.1, 500.0, 995.0, 4.0, -5.0, 1002.0, 505.5]
    pts_y = [100.0, 100.0, 99.0, 98.5, 60.0, 5.0, 195.0, 100.0, 100.0, 61.3]
    c("bilinear_hills", 1, "bilinear", dict(raster=ras, xs=pts_x, ys=pts_y))
    c("bilinear_nodata", 1, "bilinear", dict(raster=dict(ras, nodata=-9999.0, holes=[[400, 50, 600, 120]]),
                                              xs=pts_x, ys=pts_y))
    c("bilinear_off_raster", 1, "bilinear", dict(raster=ras, xs=[-500.0, 5000.0], ys=[100.0, 100.0]))
    c("bilinear_empty", 1, "bilinear", dict(raster=ras, xs=[], ys=[]))
    c("raster_terrain_sampler", 1, "raster_terrain", dict(raster=ras, xs=pts_x, ys=pts_y))
    c("layer_volumes", 1, "layer_volumes", dict(vals=[0.2, 0.9, 1.0, 1.5, 3.25, 12.0, 41.0]))
    c("layer_volumes_empty", 1, "layer_volumes", dict(vals=[]))
    zz = np.full((12, 12), 5.0)
    zz[5, 5] = 40.0
    zz[2, 8] = -20.0
    c("despike_isolated_spikes", 1, "despike", dict(z=zz, valid=np.ones((12, 12), dtype=bool), fill=5.0,
                                                    window=5, k=4.0, floor=3.0))
    vm = np.ones((12, 12), dtype=bool)
    vm[0:3, :] = False
    c("despike_with_invalid_cells", 1, "despike", dict(z=zz, valid=vm, fill=5.0, window=3, k=2.0, floor=1.0))
    c("despike_coherent_step_is_kept", 1, "despike", dict(
        z=np.hstack([np.full((12, 6), 2.0), np.full((12, 6), 12.0)]), valid=np.ones((12, 12), dtype=bool),
        fill=2.0, window=5, k=4.0, floor=3.0))
    # a river with a hill beside it
    rv = synth(length_m=2000.0, hill_height_m=18.0)
    T = {"kind": "synthetic_river", "params": {"length_m": 2000.0, "hill_height_m": 18.0}}
    # The origin is off the 5 m pixel grid on purpose: a corridor edge that lands exactly on a pixel edge
    # rounds one pixel either way depending on the last bit of the platform's arithmetic (a knife edge).
    dem = dict(x0=-101.7, y_top=452.3, res=5.0, width=440, height=180, terrain=T)
    cl, poly = rv["centreline"], rv["polygon"]
    wA = [{"index": 1, "pool_water_level_m": 0.6, "pool_start_m": 300.0, "pool_end_m": 1100.0},
          {"index": 2, "pool_water_level_m": 1.4, "pool_start_m": 1100.0, "pool_end_m": 2000.0}]
    base = dict(raster=dem, weirs=wA, centerline=cl, river_poly=poly, navigable_depth_m=3.0,
                channel_bottom_width_m=40.0, max_corridor_half_width_m=200.0)
    c("pools_default", 1, "compute_pool_volumes", dict(base))
    c("pools_no_despike", 1, "compute_pool_volumes", dict(base, despike=False))
    c("pools_side_slope_2_overdepth", 1, "compute_pool_volumes", dict(base, side_slope_h_per_v=2.0, overdepth_m=0.5))
    c("pools_with_downstream_reach", 1, "compute_pool_volumes", dict(base, downstream_water_level_m=0.0))
    c("pools_small_chunks", 1, "compute_pool_volumes", dict(base, chunk_m=250.0))
    c("pools_deep_cut_threshold", 1, "compute_pool_volumes", dict(base, deep_cut_threshold_m=1.0))
    c("pools_read_decimate_2", 1, "compute_pool_volumes", dict(base, read_decimate=2))
    c("pools_no_weirs", 1, "compute_pool_volumes", dict(base, weirs=[]))
    c("pools_with_nodata", 1, "compute_pool_volumes", dict(
        base, raster=dict(dem, nodata=-9999.0, holes=[[600, -60, 900, 60]])))
    c("pools_pianc_section_mode", 1, "compute_pool_volumes", dict(
        base, section_mode="pianc", left_limit=LineString(list(cl.coords)).offset_curve(30.0),
        right_limit=LineString(list(cl.coords)).offset_curve(-30.0)))
    c("pools_pianc_without_limits_falls_back", 1, "compute_pool_volumes", dict(base, section_mode="pianc"))
    c("chainage_default", 1, "compute_chainage_volumes", dict(base, chunk_m=200.0))
    c("chainage_dry_bank_only", 1, "compute_chainage_volumes", dict(base, chunk_m=200.0, slope_mode="dry_bank_only"))
    c("chainage_fairway_only", 1, "compute_chainage_volumes", dict(base, chunk_m=200.0, slope_mode="fairway_only"))
    c("chainage_downstream_reach", 1, "compute_chainage_volumes", dict(base, chunk_m=300.0, downstream_water_level_m=0.0))
    c("chainage_no_weirs", 1, "compute_chainage_volumes", dict(base, weirs=[]))
    # A corridor wholly off the raster makes rasterio raise its own WindowError; that is not ours to port.
    rng = Rng(23021)
    for i in range(8):
        rv2 = synth(length_m=rng.r3(1200.0, 2200.0), hill_height_m=rng.r3(0.0, 25.0),
                    hill_at=(rng.r3(400.0, 900.0), rng.r3(-100.0, 100.0)), hill_radius_m=rng.r3(80.0, 220.0))
        L2 = rv2["centreline"].bounds[2]
        T2 = {"kind": "hills", "base": rng.r3(0.0, 4.0), "gx": rng.r3(-0.0005, 0.0005), "gy": rng.r3(-0.005, 0.005),
              "hills": [[rng.r3(300.0, L2 - 300.0), rng.r3(-150.0, 150.0), rng.r3(3.0, 25.0), rng.r3(50.0, 200.0)]
                        for _ in range(rng.i(1, 3))]}
        dem2 = dict(x0=-100.0 - rng.r3(0.1, 0.9), y_top=rng.r3(300.0, 450.0), res=rng.r3(5.0, 12.0), width=int(L2 // 8) + 40, height=100,
                    terrain=T2)
        dem2["height"] = int(2 * dem2["y_top"] / dem2["res"])
        s1 = rng.r3(L2 * 0.2, L2 * 0.5)
        ws = [{"index": 1, "pool_water_level_m": rng.r3(0.0, 2.0), "pool_start_m": s1, "pool_end_m": s1 + L2 * 0.3},
              {"index": 2, "pool_water_level_m": rng.r3(2.0, 4.0), "pool_start_m": s1 + L2 * 0.3, "pool_end_m": L2}]
        kwargs = dict(raster=dem2, weirs=ws, centerline=rv2["centreline"], river_poly=rv2["polygon"],
                      navigable_depth_m=rng.r3(2.0, 4.0), channel_bottom_width_m=rng.r3(25.0, 60.0),
                      side_slope_h_per_v=rng.r3(2.0, 4.0), overdepth_m=rng.r3(0.0, 0.5),
                      despike=bool(i % 2), max_corridor_half_width_m=rng.r3(100.0, 250.0),
                      chunk_m=rng.r3(150.0, 500.0))
        c(f"pools_sweep_{i:02d}", 2, "compute_pool_volumes", kwargs)
        c(f"chainage_sweep_{i:02d}", 2, "compute_chainage_volumes",
          dict(kwargs, slope_mode=["always", "dry_bank_only", "fairway_only"][i % 3]))
    for i in range(6):
        c(f"bilinear_sweep_{i:02d}", 2, "bilinear", dict(
            raster=ras, xs=[rng.r3(-20.0, 1020.0) for _ in range(8)], ys=[rng.r3(-10.0, 210.0) for _ in range(8)]))
    return out
