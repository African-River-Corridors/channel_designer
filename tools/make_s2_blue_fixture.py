#!/usr/bin/env python3
"""Add the blue band to the recorded lower-Sassandra scene window (spec 0013, ticket T7).

`tools/make_s2_fixture.py` (spec 0012) recorded green, red, near-infrared, shortwave-infrared and the
scene classification of scene S2B_29NRF_20231223_0_L2A over a window of the lower Sassandra. The QA
overlay draws true colour, so it needs blue (B02) too. This reads the same window of B02 from the
public COG by range requests (DEFLATE with the horizontal predictor, decoded here, no GDAL), writes it
on the grid of the recorded green band (`s2_sassandra_B03.tif`), and adds the `blue` asset to the
recorded Earth Search answer (`stac_search_sassandra.json`). Contains modified Copernicus Sentinel
data 2023.

Needs the network, numpy and tifffile; run it once from the repository root:
    python3 tools/make_s2_blue_fixture.py
"""
import json
import pathlib
import struct
import urllib.request
import zlib

import numpy as np
import tifffile

OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach"
SCENE = "S2B_29NRF_20231223_0_L2A"
UA = {"User-Agent": "owd-fixture/0.3"}


def get(url, headers=None):
    return urllib.request.urlopen(urllib.request.Request(url, headers={**UA, **(headers or {})}), timeout=300).read()


def rng(url, a, n):
    return get(url, {"Range": f"bytes={a}-{a + n - 1}"})


def cog_window(url, left, top, px, cw, ch):
    """A window of the first image of a tiled COG (DEFLATE, horizontal predictor), by range reads."""
    head = rng(url, 0, 65536)
    first = struct.unpack("<I", head[4:8])[0]
    n = struct.unpack("<H", head[first:first + 2])[0]
    t = {}
    for i in range(n):
        tag, typ, cnt, val = struct.unpack("<HHII", head[first + 2 + 12 * i:first + 14 + 12 * i])
        t[tag] = (typ, cnt, val)
    width, tw, th = t[256][2], t[322][2], t[323][2]
    assert t[259][2] == 8 and t[258][2] == 16
    pred = t.get(317, (3, 1, 1))[2]
    arr = lambda tag: struct.unpack(f"<{t[tag][1]}{'I' if t[tag][0] == 4 else 'Q'}",
                                    head[t[tag][2]:t[tag][2] + t[tag][1] * (4 if t[tag][0] == 4 else 8)])
    toffs, tcnts = arr(324), arr(325)
    tie = struct.unpack("<6d", head[t[33922][2]:t[33922][2] + 48])
    col0, row0 = int(round((left - tie[3]) / px)), int(round((tie[4] - top) / px))
    out = np.zeros((ch, cw), np.uint16)
    across = -(-width // tw)
    for tr in range(row0 // th, (row0 + ch - 1) // th + 1):
        for tc in range(col0 // tw, (col0 + cw - 1) // tw + 1):
            k = tr * across + tc
            tile = np.frombuffer(zlib.decompress(rng(url, toffs[k], tcnts[k])), np.uint16).reshape(th, tw).copy()
            if pred == 2:
                tile = np.cumsum(tile, axis=1, dtype=np.uint16)
            r0, c0 = tr * th - row0, tc * tw - col0
            rs, cs, re, ce = max(r0, 0), max(c0, 0), min(r0 + th, ch), min(c0 + tw, cw)
            out[rs:re, cs:ce] = tile[rs - r0:re - r0, cs - c0:ce - c0]
    return out


stac_path = OUT / "stac_search_sassandra.json"
stac = json.loads(stac_path.read_text())
item = next(f for f in stac["features"] if f["id"] == SCENE)
green = item["assets"]["green"]["href"]
blue = green.replace("/B03.tif", "/B02.tif")
item["assets"]["blue"] = {**item["assets"]["green"], "href": blue, "title": "Blue (band 2) - 10m"}
for k in ("eo:bands", "raster:bands"):
    item["assets"]["blue"].pop(k, None)
stac_path.write_text(json.dumps(stac, indent=1) + "\n")

page = tifffile.TiffFile(OUT / "s2_sassandra_B03.tif").pages[0]
(ch, cw), tie = page.shape, page.tags[33922].value
left, top = tie[3], tie[4]
data = cog_window(blue, left, top, 10.0, cw, ch)
keys = (1, 1, 0, 4, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 32629)
path = OUT / "s2_sassandra_B02.tif"
tifffile.imwrite(path, data, tile=(256, 256), compression="zlib", predictor=True,
                 extratags=[(33550, "d", 3, (10.0, 10.0, 0.0), False),
                            (33922, "d", 6, (0.0, 0.0, 0.0, left, top, 0.0), False),
                            (34735, "H", len(keys), keys, False)])
print(path.name, cw, ch, path.stat().st_size, "bytes")
