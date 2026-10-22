#!/usr/bin/env python3
"""Record the fixtures of the Sassandra acceptance window (spec 0013, ticket T6).

The Sassandra from km 148 to km 152 of the spec 0012 worked example (about 5.655 to 5.670 N, where the
river opens into the Soubre reservoir), all real data, cut small:
- `s2win_trace.json` (written by hand, not by this script): the route, drawn on the true-colour image of
  the 5 February 2025 scene down the middle of the water, along the worked example's route from its
  km 148 to its km 152. The windows below are cut around it.
- `jrc_occurrence_s2win.tif`: a window of JRC occurrence over the route, LZW tiles copied byte for byte
  from the public tile. Source: EC JRC/Google. Pekel et al. 2016, Nature 540, 418-422.
- `stac_search_s2win.json`: the Earth Search answer for the 2024-25 dry season over the window, cut to
  the two scenes of tile 29NQG used here (26 January and 5 February 2025) and to the assets read.
- `s2win_<date>_<band>.tif`: the windows of blue B02, green B03, red B04, near-infrared B08 (10 m),
  shortwave-infrared B11 and the scene classification SCL (20 m) of both scenes, with every pixel more
  than about 700 m from the route set to 0 (no data), so the files stay small. Contains modified
  Copernicus Sentinel data 2025.

The Sentinel-2 windows are read from the public COGs by range requests and decoded here (DEFLATE with
the horizontal predictor), so no GDAL is needed; they are written with tifffile. Needs the network,
numpy, pyproj and tifffile; run it once from the repository root:
    python3 tools/make_s2_window_fixture.py
"""
import json
import pathlib
import struct
import urllib.parse
import urllib.request
import zlib

import numpy as np
import pyproj
import tifffile

OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach"
UA = {"User-Agent": "owd-fixture/0.3"}
SCENES = ("S2B_29NQG_20250126_0_L2A", "S2B_29NQG_20250205_0_L2A")
BANDS = (("blue", "B02"), ("green", "B03"), ("red", "B04"), ("nir", "B08"), ("swir16", "B11"), ("scl", "SCL"))
KEEP_M = 700.0


def get(url, headers=None):
    return urllib.request.urlopen(urllib.request.Request(url, headers={**UA, **(headers or {})}), timeout=300).read()


def rng(url, a, n):
    return get(url, {"Range": f"bytes={a}-{a + n - 1}"})


# --- the route: the hand-drawn trace (examples/reach/s2win_trace.json) -------------------------------
route = json.loads((OUT / "s2win_trace.json").read_text())["trace"]["coordinates"]
to_utm = pyproj.Transformer.from_crs(4326, 32629, always_xy=True)
lons, lats = [p[0] for p in route], [p[1] for p in route]
BBOX = (min(lons) - 0.02, min(lats) - 0.02, max(lons) + 0.02, max(lats) + 0.02)
print("route", len(route), "points, box", BBOX)

# --- the JRC window: whole 256-pixel tiles over the box ----------------------------------------------
JRC = "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_10W_10Nv1_4_2021.tif"
TILE, PIXEL = 256, 0.00025
COLS = range(int((BBOX[0] + 10) / (TILE * PIXEL)), int((BBOX[2] + 10) / (TILE * PIXEL)) + 1)
ROWS = range(int((10 - BBOX[3]) / (TILE * PIXEL)), int((10 - BBOX[1]) / (TILE * PIXEL)) + 1)
ifd = struct.unpack("<I", rng(JRC, 0, 8)[4:8])[0]
d = rng(JRC, ifd, 2 + 12 * 64 + 4)
tags = {}
for i in range(struct.unpack("<H", d[:2])[0]):
    tag, typ, cnt, val = struct.unpack("<HHII", d[2 + 12 * i:14 + 12 * i])
    tags[tag] = (typ, cnt, val)
across = -(-tags[256][2] // TILE)
offs = struct.unpack(f"<{tags[324][1]}I", rng(JRC, tags[324][2], 4 * tags[324][1]))
cnts = struct.unpack(f"<{tags[325][1]}I", rng(JRC, tags[325][2], 4 * tags[325][1]))
blobs = [rng(JRC, offs[r * across + c], cnts[r * across + c]) for r in ROWS for c in COLS]
w, h = len(COLS) * TILE, len(ROWS) * TILE
west, north = -10.0 + COLS[0] * TILE * PIXEL, 10.0 - ROWS[0] * TILE * PIXEL
out = bytearray(b"II*\0\0\0\0\0")
o, c = [], []
for blob in blobs:
    o.append(len(out))
    c.append(len(blob))
    out += blob


def put(blob):
    at = len(out)
    out.extend(blob)
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
(OUT / "jrc_occurrence_s2win.tif").write_bytes(bytes(out))
print("jrc", len(out), "bytes")

# --- the scenes ------------------------------------------------------------------------------------------
search = ("https://earth-search.aws.element84.com/v1/search?collections=sentinel-2-l2a"
          f"&bbox={BBOX[0]},{BBOX[1]},{BBOX[2]},{BBOX[3]}"
          "&datetime=" + urllib.parse.quote("2024-12-01T00:00:00Z/2025-03-01T23:59:59Z") + "&limit=100")
stac = json.loads(get(search))
stac["features"] = [f for f in stac["features"] if f["id"] in SCENES]
assert sorted(f["id"] for f in stac["features"]) == sorted(SCENES), [f["id"] for f in stac["features"]]
for f in stac["features"]:
    f["assets"] = {k: v for k, v in f["assets"].items() if k in [b for b, _ in BANDS]}
    f["links"] = []
stac.pop("links", None)
(OUT / "stac_search_s2win.json").write_text(json.dumps(stac, indent=1) + "\n")


def cog_window(url, left, top, px, cw, ch):
    """A window of the first image of a tiled COG (DEFLATE, horizontal predictor), by range reads."""
    head = rng(url, 0, 65536)
    first = struct.unpack("<I", head[4:8])[0]
    n = struct.unpack("<H", head[first:first + 2])[0]
    t = {}
    for i in range(n):
        tag, typ, cnt, val = struct.unpack("<HHII", head[first + 2 + 12 * i:first + 14 + 12 * i])
        t[tag] = (typ, cnt, val)
    width, tw, th, bits = t[256][2], t[322][2], t[323][2], t[258][2]
    pred, comp = t.get(317, (3, 1, 1))[2], t[259][2]
    assert comp == 8, comp
    arr = lambda tag: struct.unpack(f"<{t[tag][1]}{'I' if t[tag][0] == 4 else 'Q'}",
                                    head[t[tag][2]:t[tag][2] + t[tag][1] * (4 if t[tag][0] == 4 else 8)])
    toffs, tcnts = arr(324), arr(325)
    dbl = lambda tag, k: struct.unpack(f"<{k}d", head[t[tag][2]:t[tag][2] + 8 * k])
    sx = dbl(33550, 3)[0]
    tie = dbl(33922, 6)
    assert abs(sx - px) < 1e-9
    col0, row0 = int(round((left - tie[3]) / px)), int(round((tie[4] - top) / px))
    dtype = np.uint16 if bits == 16 else np.uint8
    out = np.zeros((ch, cw), dtype)
    across = -(-width // tw)
    for tr in range(row0 // th, (row0 + ch - 1) // th + 1):
        for tc in range(col0 // tw, (col0 + cw - 1) // tw + 1):
            k = tr * across + tc
            raw = zlib.decompress(rng(url, toffs[k], tcnts[k]))
            tile = np.frombuffer(raw, dtype).reshape(th, tw).copy()
            if pred == 2:
                tile = np.cumsum(tile, axis=1, dtype=dtype)
            r0, c0 = tr * th - row0, tc * tw - col0
            rs, cs = max(r0, 0), max(c0, 0)
            re, ce = min(r0 + th, ch), min(c0 + tw, cw)
            out[rs:re, cs:ce] = tile[rs - r0:re - r0, cs - c0:ce - c0]
    return out


xs, ys = to_utm.transform([BBOX[0], BBOX[2]], [BBOX[1], BBOX[3]])
rx, ry = zip(*[to_utm.transform(x, y) for x, y in route])
keys = (1, 1, 0, 4, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 32629)
for item in stac["features"]:
    date = item["properties"]["datetime"][:10].replace("-", "")
    for band, name in BANDS:
        href = item["assets"][band]["href"]
        px = 20.0 if name in ("B11", "SCL") else 10.0
        left, top = np.floor(xs[0] / 20) * 20, np.ceil(ys[1] / 20) * 20
        cw, ch = int(np.ceil((xs[1] - left) / px)), int(np.ceil((top - ys[0]) / px))
        data = cog_window(href, left, top, px, cw, ch)
        keep = np.zeros(data.shape, bool)
        r = int(KEEP_M / px)
        for x0, y0, x1, y1 in zip(rx, ry, rx[1:], ry[1:]):
            for tt in np.linspace(0, 1, 10):
                col, row = int((x0 + tt * (x1 - x0) - left) / px), int((top - (y0 + tt * (y1 - y0))) / px)
                keep[max(row - r, 0):row + r + 1, max(col - r, 0):col + r + 1] = True
        data[~keep] = 0
        path = OUT / f"s2win_{date}_{name}.tif"
        tifffile.imwrite(path, data, tile=(256, 256), compression="zlib", predictor=True,
                         extratags=[(33550, "d", 3, (px, px, 0.0), False),
                                    (33922, "d", 6, (0.0, 0.0, 0.0, left, top, 0.0), False),
                                    (34735, "H", len(keys), keys, False)])
        print(path.name, cw, ch, path.stat().st_size, "bytes")
