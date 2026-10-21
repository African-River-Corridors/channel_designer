#!/usr/bin/env python3
"""Record a window of the JRC Global Surface Water occurrence layer for the owd-reach bank tests
(spec 0012, ticket T3).

The window is the lower Sassandra, from the sea at Sassandra town upstream. It is real data: the LZW
tiles are copied byte for byte from the public tile `occurrence_10W_10Nv1_4_2021.tif` (a tiled GeoTIFF
with its directory at the end, so only the directory and the wanted tiles are fetched). Only the
directory is rewritten, for a smaller image that keeps the georeferencing. Needs the network; run it
once, from the repository root:
    python3 tools/make_jrc_fixture.py

Source: EC JRC/Google. Pekel et al. 2016, Nature 540, 418-422. Copernicus data, free and open.
"""
import pathlib
import struct
import urllib.request

URL = "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_10W_10Nv1_4_2021.tif"
OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach" / "jrc_occurrence_sassandra_estuary.tif"
# Tile columns and rows of the window (256 pixels of 0.00025 degrees each; tile 0,0 has its corner at 10 W, 10 N).
COLS = range(59, 64)
ROWS = range(76, 81)
TILE = 256
PIXEL = 0.00025


def rng(a, n):
    req = urllib.request.Request(URL, headers={"Range": f"bytes={a}-{a + n - 1}", "User-Agent": "owd-fixture/0.2"})
    return urllib.request.urlopen(req, timeout=120).read()


ifd = struct.unpack("<I", rng(0, 8)[4:8])[0]
d = rng(ifd, 2 + 12 * 64 + 4)
n = struct.unpack("<H", d[:2])[0]
tags = {}
for i in range(n):
    tag, typ, cnt, val = struct.unpack("<HHII", d[2 + 12 * i:14 + 12 * i])
    tags[tag] = (typ, cnt, val)
across = -(-tags[256][2] // TILE)
offs = struct.unpack(f"<{tags[324][1]}I", rng(tags[324][2], 4 * tags[324][1]))
cnts = struct.unpack(f"<{tags[325][1]}I", rng(tags[325][2], 4 * tags[325][1]))
blobs = []
for r in ROWS:
    for c in COLS:
        i = r * across + c
        blobs.append(rng(offs[i], cnts[i]))
w, h = len(COLS) * TILE, len(ROWS) * TILE
west = -10.0 + COLS[0] * TILE * PIXEL
north = 10.0 - ROWS[0] * TILE * PIXEL

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
OUT.write_bytes(bytes(out))
print(f"wrote {OUT} ({len(out)} bytes), window {w} x {h} px, west {west}, north {north}")
