"""owd — navigation channel design for inland waterways.

Three calculation layers, an optional optimiser layer and drawings:

- :mod:`owd.rules` — channel width, bend widening, bend radius and depth from a
  named standard (PIANC WG 141, Chinese GB 50139) and a vessel class. Width methods
  are pluggable, so a private package can add its own.
- :mod:`owd.alignment` — centreline from banks, meander analysis, bend
  classification, rolling-circle straightening, channel limits and least-cost route search.
- :mod:`owd.volumes` — cut models, cross-section and raster volume integration,
  pool levels, and the excavation footprint. Quantities only; no prices.
- :mod:`owd.optimise` — optional searches: weir sites and pool levels, and the
  least-cost R_min-feasible channel through each pool. Weights, not prices.
- :func:`owd.draw` — engineering drawings (cross-section gallery now): drawing documents, SVG, PDF and
  DXF, the same sheets as ``owd draw``. Drawn by the Rust core through ``otd``.

Screening tool: not for construction, no design warranty.
"""
__version__ = "0.3.1"

from ._draw import draw  # noqa: E402,F401  (imports the Rust core only when called)
