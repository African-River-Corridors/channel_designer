#!/usr/bin/env python3
"""Record the fixtures of the default water level (spec 0012, ticket T6), lower Sassandra.

Real open data, cut small:
- `sword_v17c_shp_subset.zip`: a zip laid out as the SWORD v17c shapefile release on Zenodo (members under
  `shp/`), holding the reaches of basin hb14 near the Sassandra mouth (six reaches) and one reach of basin
  hb15, as shapefiles (.shp, .shx, .dbf). The records are copied byte for byte from the release; the dbf
  keeps every field. SWORD v17c, Altenau et al. 2021, CC BY 4.0 (Zenodo record 22259077).
- `hydrocron_<reach_id>.json`: the Hydrocron time series of five of those reaches, July 2023 to October 2026,
  as Hydrocron answers it (CSV output inside JSON). SWOT data: NASA and CNES, served by PO.DAAC Hydrocron.
- `glo30_sassandra_lower.tif`: a window of the Copernicus GLO-30 DEM over the same reaches, joined from the
  tiles N04 W007 and N05 W007, with every pixel more than about 400 m from a reach set to nodata (so the
  file stays small). Produced using Copernicus WorldDEM-30 (c) DLR e.V. 2010-2014 and (c) Airbus
  Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved.

Needs the network and rasterio; run it once from the repository root:
    python3 tools/make_level_fixture.py
"""
import io
import json
import pathlib
import struct
import urllib.request
import zipfile
import zlib

import numpy as np
import rasterio
from rasterio.merge import merge

OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach"
ZIP = "https://zenodo.org/records/22259077/files/SWORD_v17c_shp.zip?download=1"
UA = {"User-Agent": "owd-fixture/0.2"}
KEEP = {"hb14": ["14560100016", "14560100021", "14560100033", "14560100041", "14560100051", "14560100061"],
        "hb15": None}  # None: the first reach only
LEVELS = ["14560100021", "14560100033", "14560100041", "14560100051", "14560100061"]
BBOX = (-6.30, 4.96, -6.06, 5.14)  # west, south, east, north


def rng(a, b):
    req = urllib.request.Request(ZIP, headers={**UA, "Range": f"bytes={a}-{b}"})
    return urllib.request.urlopen(req, timeout=600).read()


def members():
    size = int(urllib.request.urlopen(urllib.request.Request(ZIP, method="HEAD", headers=UA)).headers["Content-Length"])
    tail = rng(size - 65_000, size - 1)
    i = tail.rfind(b"PK\x05\x06")
    n, cd_size, cd_off = struct.unpack("<HII", tail[i + 10:i + 20])
    cd = rng(cd_off, cd_off + cd_size - 1)
    out, p = {}, 0
    while p < len(cd):
        csz, = struct.unpack("<I", cd[p + 20:p + 24])
        fl, el, cl = struct.unpack("<HHH", cd[p + 28:p + 34])
        off, = struct.unpack("<I", cd[p + 42:p + 46])
        out[cd[p + 46:p + 46 + fl].decode()] = (off, csz)
        p += 46 + fl + el + cl
    return out


def member(m, name):
    off, csz = m[name]
    h = rng(off, off + 29)
    fl, el = struct.unpack("<HH", h[26:30])
    return zlib.decompress(rng(off + 30 + fl + el, off + 30 + fl + el + csz - 1), -15)


def subset(shp, dbf, keep_ids):
    """The records of `keep_ids` (or the first record), as new .shp, .shx and .dbf bytes."""
    n, hl, rl = struct.unpack("<IHH", dbf[4:12])
    rid_len = dbf[32 + 16]  # reach_id is the first field
    ids = [dbf[hl + i * rl + 1:hl + i * rl + 1 + rid_len].decode().strip() for i in range(n)]
    pick = [ids.index(r) for r in keep_ids] if keep_ids else [0]
    recs, p = [], 100
    while p < len(shp):
        _, clen = struct.unpack(">ii", shp[p:p + 8])
        recs.append(shp[p + 8:p + 8 + 2 * clen])
        p += 8 + 2 * clen
    body, index, off = b"", b"", 50
    xs, ys = [], []
    for k, i in enumerate(pick):
        c = recs[i]
        xmin, ymin, xmax, ymax = struct.unpack("<4d", c[4:36])
        xs += [xmin, xmax]
        ys += [ymin, ymax]
        body += struct.pack(">ii", k + 1, len(c) // 2) + c
        index += struct.pack(">ii", off, len(c) // 2)
        off += 4 + len(c) // 2
    def header(length_words):
        return (struct.pack(">i5ii", 9994, 0, 0, 0, 0, 0, length_words) + struct.pack("<ii", 1000, 3)
                + struct.pack("<4d", min(xs), min(ys), max(xs), max(ys)) + struct.pack("<4d", 0, 0, 0, 0))
    new_shp = header(50 + len(body) // 2) + body
    new_shx = header(50 + len(index) // 2) + index
    new_dbf = bytearray(dbf[:hl]) + b"".join(dbf[hl + i * rl:hl + (i + 1) * rl] for i in pick) + b"\x1a"
    struct.pack_into("<I", new_dbf, 4, len(pick))
    return new_shp, new_shx, bytes(new_dbf)


m = members()
buf = io.BytesIO()
with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
    for basin, keep in KEEP.items():
        stem = f"shp/af_sword_reaches_{basin}_v17c"
        shp, shx, dbf = subset(member(m, stem + ".shp"), member(m, stem + ".dbf"), keep)
        z.writestr(stem + ".shp", shp)
        z.writestr(stem + ".shx", shx)
        z.writestr(stem + ".dbf", dbf)
        z.writestr(stem + ".prj", member(m, stem + ".prj"))
(OUT / "sword_v17c_shp_subset.zip").write_bytes(buf.getvalue())
print("sword_v17c_shp_subset.zip", len(buf.getvalue()), "bytes")

for rid in LEVELS:
    url = ("https://soto.podaac.earthdatacloud.nasa.gov/hydrocron/v1/timeseries?feature=Reach"
           f"&feature_id={rid}&start_time=2023-07-01T00:00:00Z&end_time=2026-10-01T00:00:00Z&output=csv"
           "&fields=reach_id,time_str,wse,wse_u,reach_q,width,geoid_hght")
    body = json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=300))
    (OUT / f"hydrocron_{rid}.json").write_text(json.dumps(body, indent=1) + "\n")
    print(f"hydrocron_{rid}.json", body.get("hits"))

tiles = ["Copernicus_DSM_COG_10_N04_00_W007_00_DEM", "Copernicus_DSM_COG_10_N05_00_W007_00_DEM"]
srcs = [rasterio.open(f"/vsicurl/https://copernicus-dem-30m.s3.eu-central-1.amazonaws.com/{t}/{t}.tif") for t in tiles]
data, transform = merge(srcs, bounds=BBOX)
# Keep a corridor of about 400 m (13 pixels of 1 arc second) around the vertices of the recorded reaches.
keep = np.zeros(data.shape[1:], bool)
z = zipfile.ZipFile(OUT / "sword_v17c_shp_subset.zip")
shp = z.read("shp/af_sword_reaches_hb14_v17c.shp")
p = 100
while p < len(shp):
    _, clen = struct.unpack(">ii", shp[p:p + 8])
    c = shp[p + 8:p + 8 + 2 * clen]
    nparts, npts = struct.unpack("<ii", c[36:44])
    for k in range(npts):
        x, y = struct.unpack("<2d", c[44 + 4 * nparts + 16 * k:60 + 4 * nparts + 16 * k])
        col, row = ~transform * (x, y)
        r0, c0 = int(row) - 13, int(col) - 13
        keep[max(r0, 0):max(r0 + 27, 0), max(c0, 0):max(c0 + 27, 0)] = True
    p += 8 + 2 * clen
NODATA = -32767.0
data[0][~keep] = NODATA
profile = dict(driver="GTiff", dtype="float32", nodata=NODATA, count=1, width=data.shape[2], height=data.shape[1], crs="EPSG:4326",
               transform=transform, tiled=True, blockxsize=256, blockysize=256, compress="deflate", predictor=3)
with rasterio.open(OUT / "glo30_sassandra_lower.tif", "w", **profile) as dst:
    dst.write(data[0].astype(np.float32), 1)
print("glo30_sassandra_lower.tif", (OUT / "glo30_sassandra_lower.tif").stat().st_size, "bytes", data.shape)
