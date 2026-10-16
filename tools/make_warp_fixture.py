"""What rasterio says a warped copy of a geographic DEM holds, at test points in UTM.

    python tools/make_warp_fixture.py

Needs rasterio. Warps `f32_geographic.tif` (EPSG:4326, 1 arc second) to WGS84 / UTM 30N at 10 m with
bilinear resampling, then reads the warped copy bilinearly (pixel centres, as the Rust sampler does)
at 150 deterministic UTM points. Writes `crates/cd-format/tests/fixtures/dem/warp_expected.json`.
`crates/cd-format/tests/geographic_dem.rs` holds the Rust sampler, which converts each point to
longitude and latitude and samples the geographic file where it is, to this file.

`tolerance_m` is the warp's own interpolation error: a bilinear surface differs from the
surface it interpolates by at most h^2/8 times the largest second difference (h = one source pixel),
and the two routes interpolate once each, so twice that bound.
"""
import json
from pathlib import Path

import numpy as np
import rasterio
from rasterio.transform import Affine
from rasterio.warp import Resampling, calculate_default_transform, reproject

DIR = Path(__file__).resolve().parent.parent / "crates/cd-format/tests/fixtures/dem"
src = rasterio.open(DIR / "f32_geographic.tif")
z = src.read(1).astype("float64")

transform, w, h = calculate_default_transform(src.crs, "EPSG:32630", src.width, src.height, *src.bounds, resolution=10.0)
dst = np.full((h, w), np.nan, dtype="float64")
reproject(z, dst, src_transform=src.transform, src_crs=src.crs, dst_transform=transform,
          dst_crs="EPSG:32630", resampling=Resampling.bilinear, dst_nodata=np.nan)


def bilinear(arr, tr, x, y):
    col = (x - tr.c) / tr.a - 0.5
    row = (y - tr.f) / tr.e - 0.5
    c0, r0 = int(np.floor(col)), int(np.floor(row))
    fc, fr = col - c0, row - r0
    v = (arr[r0, c0] * (1 - fc) * (1 - fr) + arr[r0, c0 + 1] * fc * (1 - fr)
         + arr[r0 + 1, c0] * (1 - fc) * fr + arr[r0 + 1, c0 + 1] * fc * fr)
    return float(v)


# Interior points only, away from the edge where the warp has no data.
rng = np.random.default_rng(20261016)
x0, y1 = transform.c, transform.f
pts = []
while len(pts) < 150:
    x = x0 + rng.uniform(60, w * 10 - 60)
    y = y1 - rng.uniform(60, h * 10 - 60)
    col, row = int((x - x0) / 10 - 0.5), int((y1 - y) / 10 - 0.5)
    block = dst[row:row + 2, col:col + 2]
    if block.shape == (2, 2) and np.isfinite(block).all():
        pts.append((x, y, bilinear(dst, transform, x, y)))

d2 = max(np.abs(np.diff(z, 2, axis=0)).max(), np.abs(np.diff(z, 2, axis=1)).max())
out = {
    "rasterio": rasterio.__version__,
    "epsg": 32630,
    "tolerance_m": 2 * d2 / 8,
    "points": [{"x": x, "y": y, "z": zz} for x, y, zz in pts],
}
(DIR / "warp_expected.json").write_text(json.dumps(out, indent=1) + "\n")
print(len(pts), "points; tolerance_m", out["tolerance_m"])
