"""owd — navigation channel design for inland waterways.

Three calculation layers, an optional optimiser layer and an optional drawing layer:

- :mod:`owd.rules` — channel width, bend widening, bend radius and depth from a
  named standard (PIANC WG 141, Chinese GB 50139) and a vessel class. Width methods
  are pluggable, so a private package can add its own.
- :mod:`owd.alignment` — centreline from banks, meander analysis, bend
  classification, rolling-circle straightening, channel limits and least-cost route search.
- :mod:`owd.volumes` — cut models, cross-section and raster volume integration,
  pool levels, and the excavation footprint. Quantities only; no prices.
- :mod:`owd.optimise` — optional searches: weir sites and pool levels, and the
  least-cost R_min-feasible channel through each pool. Weights, not prices.
- :mod:`owd.drawings` — cross-section sheets. Needs ``open-waterway-designer[drawings]``.

Screening tool: not for construction, no design warranty.
"""
__version__ = "0.2.0"
