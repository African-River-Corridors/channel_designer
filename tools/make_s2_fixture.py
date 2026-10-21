#!/usr/bin/env python3
"""Record a window of a Sentinel-2 L2A scene for the owd-reach edge-fit tests (spec 0012, ticket T4).

The window is the lower Sassandra near Sassandra town, a dry-season, low-cloud scene found in the Earth
Search STAC. It is real data: five band windows (green B03, red B04, near-infrared B08 at 10 m, and
shortwave-infrared B11 and the scene classification SCL at 20 m) cut from the public COGs and re-written as small
tiled DEFLATE files with the same georeferencing, plus the STAC answer trimmed to a few items. Needs the
network and rasterio; run it once from the repository root:
    python3 tools/make_s2_fixture.py

Contains modified Copernicus Sentinel data 2023.
"""
import json
import pathlib
import urllib.request

import numpy as np
import rasterio
from rasterio.transform import Affine
from rasterio.warp import transform as warp_transform
from rasterio.windows import Window

OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach"
BBOX = (-6.0905, 4.958, -6.0545, 4.989)  # west, south, east, north (degrees)
SEARCH = ("https://earth-search.aws.element84.com/v1/search?collections=sentinel-2-l2a&bbox=-6.15,4.93,-6.05,5.0"
          "&datetime=2023-12-01T00:00:00Z/2024-03-01T00:00:00Z&limit=30")
SCENE = "S2B_29NRF_20231223_0_L2A"  # no cloud over the window (scene classification), offset already applied
KEEP = {"S2B_29NRF_20231223_0_L2A", "S2A_29NRF_20231218_0_L2A", "S2A_29NRF_20240127_0_L2A", "S2B_29NRF_20240122_0_L2A"}

req = urllib.request.Request(SEARCH, headers={"User-Agent": "owd-fixture/0.2"})
stac = json.load(urllib.request.urlopen(req, timeout=120))
stac["features"] = [f for f in stac["features"] if f["id"] in KEEP]
# Trim the items to what the reader uses, so the fixture is small.
for f in stac["features"]:
    f["assets"] = {k: v for k, v in f["assets"].items() if k in ("green", "red", "nir", "swir16", "scl")}
    f["links"] = []
(OUT / "stac_search_sassandra.json").write_text(json.dumps(stac, indent=1) + "\n")

item = next(f for f in stac["features"] if f["id"] == SCENE)
xs, ys = warp_transform("EPSG:4326", "EPSG:32629", [BBOX[0], BBOX[2]], [BBOX[1], BBOX[3]])
x0, x1, y0, y1 = xs[0], xs[1], ys[0], ys[1]
for band, name in (("green", "B03"), ("red", "B04"), ("nir", "B08"), ("swir16", "B11"), ("scl", "SCL")):
    href = item["assets"][band]["href"]
    with rasterio.open("/vsicurl/" + href) as src:
        px = src.transform.a
        # Snap to whole 20 m cells so every band shares the same corners.
        left = np.floor(x0 / 20) * 20
        top = np.ceil(y1 / 20) * 20
        w = int(np.ceil((x1 - left) / px))
        h = int(np.ceil((top - y0) / px))
        win = Window((left - src.transform.c) / px, (src.transform.f - top) / px, w, h)
        data = src.read(1, window=win)
        profile = dict(driver="GTiff", dtype=str(src.dtypes[0]), count=1, width=w, height=h, crs=src.crs,
                       transform=Affine(px, 0, left, 0, -px, top), tiled=True, blockxsize=256, blockysize=256,
                       compress="deflate", predictor=2, zlevel=9)
    path = OUT / f"s2_sassandra_{name}.tif"
    with rasterio.open(path, "w", **profile) as dst:
        dst.write(data, 1)
    print(path.name, w, h, path.stat().st_size, "bytes; DN range", int(data.min()), int(data.max()))
