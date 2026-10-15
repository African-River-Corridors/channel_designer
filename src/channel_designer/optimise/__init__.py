"""Optimisers — an optional layer on top of rules, alignment and volumes.

- :mod:`.cascade` — weir positions and pool levels: an exact shortest path on a DAG of
  (candidate site, pool level), with the reach and structure costs supplied by the caller as
  unitless weights. :class:`~.cascade.LevelTable` turns cut volumes at each level into the
  usual reach cost.
- :mod:`.lattice` — the channel alignment through each pool: an A* search over an
  (x, y, heading) lattice of R_min-feasible arcs, costed by the swept earthwork field.

Everything is configured by typed dataclasses the caller passes in (``CascadeConfig``,
``LatticeConfig``, ``SectionSpec``). Terrain comes through the package's sampler interface
``terrain(xs, ys) -> z``. No paths, no environment variables, no prices. Core dependencies
only (numpy, shapely).
"""
from .cascade import (  # noqa: F401
    Cascade,
    CascadeConfig,
    CascadeSearch,
    LevelTable,
    build_level_table,
    cascade_levels,
    earthworks_reach_cost,
    score_cascade,
    search_cascade,
)
from .lattice import (  # noqa: F401
    LatticeConfig,
    LatticeResult,
    PoolAlignment,
    PoolsResult,
    SectionSpec,
    TerrainGrid,
    build_terrain_grid,
    optimise_pool,
    optimise_pools,
    solve_lattice,
)
