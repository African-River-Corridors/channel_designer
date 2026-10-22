#!/usr/bin/env python3
"""Record the fixtures of the edge fit across Sentinel-2 tiles (spec 0012, issue #264).

About 22 km of the Sassandra (5.30 to 5.45 N) that crosses the edge between tiles 29NQF and 29NQG (near 5.33 to 5.43 N), all real
data, cut small:
- `sassandra_tiles_route.json`: the route, the lines of SWORD v17c reaches 14560100091 and 14560100101 as
  Hydrocron gives them (and 14560100081). SWORD v17c, Altenau et al. 2021, CC BY 4.0.
- `jrc_occurrence_sassandra_tiles.tif`: a window of JRC occurrence over the route, LZW tiles copied byte
  for byte from the public tile. Source: EC JRC/Google. Pekel et al. 2016, Nature 540, 418-422.
- `stac_search_sassandra_tiles.json`: the Earth Search answer for 23 December 2023, the scenes of both
  tiles, trimmed to the assets the reader uses.
- `s2tiles_<scene>_<band>.tif`: the band windows of both scenes over the route, with every pixel more
  than about 450 m from the route set to 0 (no data), so the files stay small. Contains modified
  Copernicus Sentinel data 2023.

Needs the network and rasterio; run it once from the repository root:
    python3 tools/make_s2_tiles_fixture.py
"""
import json
import pathlib
import struct
import urllib.parse
import urllib.request

import numpy as np
import rasterio
from rasterio.transform import Affine
from rasterio.warp import transform as warp_transform
from rasterio.windows import Window

OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach"
UA = {"User-Agent": "owd-fixture/0.2"}
REACHES = ["14560100081", "14560100091", "14560100101"]
DATE = "2023-12-23"


def get(url, headers=None):
    return urllib.request.urlopen(urllib.request.Request(url, headers={**UA, **(headers or {})}), timeout=300).read()


# --- the route, from SWORD through Hydrocron -------------------------------------------------------
route = []
for rid in REACHES:
    url = ("https://soto.podaac.earthdatacloud.nasa.gov/hydrocron/v1/timeseries?feature=Reach"
           f"&feature_id={rid}&start_time=2023-12-01T00:00:00Z&end_time=2024-02-01T00:00:00Z&output=geojson&fields=reach_id")
    line = json.loads(get(url))["results"]["geojson"]["features"][0]["geometry"]["coordinates"]
    if route and np.hypot(*np.subtract(route[-1], line[-1])) < np.hypot(*np.subtract(route[-1], line[0])):
        line = line[::-1]
    if not route and line[0][1] > line[-1][1]:
        line = line[::-1]  # from the south
    route += line
# Keep the stretch from 5.30 to 5.45 N: a part only tile 29NQF covers, the overlap, and a part only 29NQG covers.
route = [[round(x, 6), round(y, 6)] for x, y in route if 5.30 <= y <= 5.45]
(OUT / "sassandra_tiles_route.json").write_text(json.dumps({
    "notice": "SWORD v17c, Altenau et al. 2021, CC BY 4.0 (reaches 14560100081, 14560100091 and 14560100101)",
    "route": route,
}) + "\n")
lons, lats = [p[0] for p in route], [p[1] for p in route]
BBOX = (min(lons) - 0.02, min(lats) - 0.02, max(lons) + 0.02, max(lats) + 0.02)
print("route", len(route), "points, box", BBOX)

# --- the JRC window: whole 256-pixel tiles over the box ----------------------------------------------
JRC = "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_10W_10Nv1_4_2021.tif"
TILE, PIXEL = 256, 0.00025
COLS = range(int((BBOX[0] + 10) / (TILE * PIXEL)), int((BBOX[2] + 10) / (TILE * PIXEL)) + 1)
ROWS = range(int((10 - BBOX[3]) / (TILE * PIXEL)), int((10 - BBOX[1]) / (TILE * PIXEL)) + 1)


def rng(a, n):
    return get(JRC, {"Range": f"bytes={a}-{a + n - 1}"})


ifd = struct.unpack("<I", rng(0, 8)[4:8])[0]
d = rng(ifd, 2 + 12 * 64 + 4)
tags = {}
for i in range(struct.unpack("<H", d[:2])[0]):
    tag, typ, cnt, val = struct.unpack("<HHII", d[2 + 12 * i:14 + 12 * i])
    tags[tag] = (typ, cnt, val)
across = -(-tags[256][2] // TILE)
offs = struct.unpack(f"<{tags[324][1]}I", rng(tags[324][2], 4 * tags[324][1]))
cnts = struct.unpack(f"<{tags[325][1]}I", rng(tags[325][2], 4 * tags[325][1]))
blobs = [rng(offs[r * across + c], cnts[r * across + c]) for r in ROWS for c in COLS]
w, h = len(COLS) * TILE, len(ROWS) * TILE
west, north = -10.0 + COLS[0] * TILE * PIXEL, 10.0 - ROWS[0] * TILE * PIXEL
out = bytearray(b"II*\0\0\0\0\0")
o, c = [], []
for b in blobs:
    o.append(len(out))
    c.append(len(b))
    out += b


def put(b):
    at = len(out)
    out.extend(b)
    return at


off_at = put(struct.pack(f"<{len(o)}I", *o))
cnt_at = put(struct.pack(f"<{len(c)}I", *c))
scale_at = put(struct.pack("<3d", PIXEL, PIXEL, 0.0))
tie_at = put(struct.pack("<6d", 0.0, 0.0, 0.0, west, north, 0.0))
keys = (1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326)
keys_at = put(struct.pack(f"<{len(keys)}H", *keys))
ifd_at = len(out)
S = lambda t, v: (t, 3, 1, v)
L = lambda t, v: (t, 4, 1, v)
e = sorted([L(256, w), L(257, h), S(258, 8), S(259, 5), S(262, 1), S(277, 1), S(284, 1), S(317, 1), S(322, TILE), S(323, TILE),
            (324, 4, len(o), off_at), (325, 4, len(c), cnt_at), S(339, 1), (33550, 12, 3, scale_at), (33922, 12, 6, tie_at),
            (34735, 3, len(keys), keys_at)])
out += struct.pack("<H", len(e))
for t, ty, cn, v in e:
    out += struct.pack("<HHII", t, ty, cn, v)
out += struct.pack("<I", 0)
out[4:8] = struct.pack("<I", ifd_at)
(OUT / "jrc_occurrence_sassandra_tiles.tif").write_bytes(bytes(out))
print("jrc", len(out), "bytes")

# --- the scenes of 23 December 2023 over the box -------------------------------------------------------
search = ("https://earth-search.aws.element84.com/v1/search?collections=sentinel-2-l2a"
          f"&bbox={BBOX[0]},{BBOX[1]},{BBOX[2]},{BBOX[3]}&datetime={DATE}T00:00:00Z/{DATE}T23:59:59Z&limit=20")
stac = json.loads(get(search))
stac["features"] = [f for f in stac["features"] if f["id"].split("_")[1] in ("29NQF", "29NQG")]
for f in stac["features"]:
    f["assets"] = {k: v for k, v in f["assets"].items() if k in ("green", "red", "nir", "swir16", "scl")}
    f["links"] = []
(OUT / "stac_search_sassandra_tiles.json").write_text(json.dumps(stac, indent=1) + "\n")
print("scenes", [f["id"] for f in stac["features"]])

xs, ys = warp_transform("EPSG:4326", "EPSG:32629", [BBOX[0], BBOX[2]], [BBOX[1], BBOX[3]])
rx, ry = warp_transform("EPSG:4326", "EPSG:32629", lons, lats)
for item in stac["features"]:
    tile = item["id"].split("_")[1]
    for band, name in (("green", "B03"), ("red", "B04"), ("nir", "B08"), ("swir16", "B11"), ("scl", "SCL")):
        with rasterio.open("/vsicurl/" + item["assets"][band]["href"]) as src:
            px = src.transform.a
            left, top = np.floor(xs[0] / 20) * 20, np.ceil(ys[1] / 20) * 20
            cw, ch = int(np.ceil((xs[1] - left) / px)), int(np.ceil((top - ys[0]) / px))
            win = Window((left - src.transform.c) / px, (src.transform.f - top) / px, cw, ch)
            data = src.read(1, window=win, boundless=True, fill_value=0)
            # Keep a corridor of about 450 m around the route.
            keep = np.zeros(data.shape, bool)
            r = int(450 / px)
            for x0, y0, x1, y1 in zip(rx, ry, rx[1:], ry[1:]):
                for t in np.linspace(0, 1, 20):
                    col, row = int((x0 + t * (x1 - x0) - left) / px), int((top - (y0 + t * (y1 - y0))) / px)
                    keep[max(row - r, 0):row + r + 1, max(col - r, 0):col + r + 1] = True
            data[~keep] = 0
            profile = dict(driver="GTiff", dtype=str(src.dtypes[0]), count=1, width=cw, height=ch, crs=src.crs,
                           transform=Affine(px, 0, left, 0, -px, top), tiled=True, blockxsize=256, blockysize=256,
                           compress="deflate", predictor=2, zlevel=9)
        path = OUT / f"s2tiles_{tile}_{name}.tif"
        with rasterio.open(path, "w", **profile) as dst:
            dst.write(data, 1)
        print(path.name, cw, ch, path.stat().st_size, "bytes")
