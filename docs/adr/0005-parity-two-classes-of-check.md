---
status: accepted
---
# Parity has two classes of check: exact values, and search outcomes

Deterministic geometry and volumes must match the frozen Python reference to 1e-9. The searches
(rolling circle, least-cost re-route, cascade, pool lattice) make discrete choices, and where two
candidates cost almost the same, a change in floating-point order can pick the other one. For
search outputs, parity means the same objective (within 1e-9 relative) and every constraint met
(radius, corridor). The exact path must also match unless the case generator recorded a near-tie
for that case, from the runner-up cost it stores with each case. A tie is a known property of a
case, never a judgement made after a failure. Real-DEM reading is tested apart from parity, on a
few GeoTIFF fixtures checked against rasterio to 1e-6 m.

## Geometry from shapely (added 2026-10-15)

The Python takes its geometry from shapely, which wraps GEOS, and the Rust geometry library builds
some shapes differently. Where a GEOS approximation changes a number, the port reproduces it: the
rolling circle's 512-sided circle polygon (`quad_segs=128`) is built vertex for vertex as GEOS
builds it, never as an exact circle. Polygon results (footprints, corridors, wetted areas) are
compared normalised, because two correct implementations may start a ring at a different vertex:
area within 1e-9 relative and every vertex within 1e-6 m of the other shape.
