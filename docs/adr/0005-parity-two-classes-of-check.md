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
