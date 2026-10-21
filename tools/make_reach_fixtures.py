#!/usr/bin/env python3
"""Make the synthetic Overpass answers for the owd-reach route tests (spec 0012, ticket 0012 T2).

Synthetic, not recorded: the network is drawn here so each test has a known answer. The files use the
shape of a real Overpass `out geom` and `out tags bb` answer. Run from the repository root:
    python3 tools/make_reach_fixtures.py
"""
import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parents[1] / "examples" / "reach" / "synthetic"

# node id -> (lon, lat). One degree of longitude here is about 1.1 km.
N = {
    1: (-6.00, 5.00), 2: (-5.99, 5.00), 3: (-5.98, 5.00),
    6: (-5.975, 5.012), 7: (-5.965, 5.012),          # north branch (longer)
    10: (-5.975, 4.99), 11: (-5.965, 4.99),          # south branch (shorter)
    5: (-5.96, 5.00), 12: (-5.95, 5.00), 13: (-5.94, 5.00),
    20: (-5.95, 4.98), 21: (-5.95, 5.02),            # river B crosses river A at node 12
    30: (-5.90, 5.00), 31: (-5.88, 5.00),            # beyond a 4.4 km gap in the network
}
# way id -> (name, kind, nodes)
W = {
    101: ("Alpha", "river", [1, 2, 3]),
    102: ("Alpha", "river", [3, 6, 7, 5]),           # north branch
    103: ("Alpha", "river", [3, 10, 11, 5]),         # south branch
    104: ("Alpha", "river", [5, 12, 13]),
    201: ("Beta", "stream", [20, 12, 21]),
    301: ("Gamma", "river", [30, 31]),
    105: ("Alpha", "canal", [30, 31]),
}


def geom_way(wid):
    name, kind, nodes = W[wid]
    return {
        "type": "way", "id": wid, "nodes": nodes,
        "geometry": [{"lat": N[n][1], "lon": N[n][0]} for n in nodes],
        "tags": {"name": name, "waterway": kind},
    }


def bb_way(wid):
    name, kind, nodes = W[wid]
    lons = [N[n][0] for n in nodes]
    lats = [N[n][1] for n in nodes]
    return {
        "type": "way", "id": wid,
        "bounds": {"minlat": min(lats), "minlon": min(lons), "maxlat": max(lats), "maxlon": max(lons)},
        "tags": {"name": name, "waterway": kind},
    }


head = {"version": 0.6, "generator": "make_reach_fixtures.py (synthetic)", "osm3s": {"copyright": "synthetic test data"}}
network = dict(head, elements=[geom_way(w) for w in (101, 102, 103, 104, 201, 301)])
search = dict(head, elements=[bb_way(w) for w in (101, 102, 103, 104, 105)])
(OUT / "network.json").write_text(json.dumps(network, indent=1) + "\n")
(OUT / "search_alpha.json").write_text(json.dumps(search, indent=1) + "\n")
print("wrote", OUT)
