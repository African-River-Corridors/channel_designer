# Changelog

All notable changes to `channel_designer`. The format follows [Keep a Changelog](https://keepachangelog.com/);
versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

### Design impact

Every pull request is checked for design impact (`tools/design_impact.py`, CI job `design-impact`): fixed
scenarios run on the base branch and on the pull request, and a result that moves by more than 1e-9
(ADR 0005) must have a bullet here, under this heading, saying what moved and why. Add the bullet in the
pull request that moves the result. Nothing has moved yet in this section.

- Cut Model volumes in the Rust core rise: the berm at the waterline is 2.5 m, not 2.0 m (demonstration of the check, not a real change).

## 0.2.0

The calculations now run in a Rust core. The Python API is the same: the same names, arguments and
results, and the existing test suite passes unchanged apart from one test of a private helper (see
Changed). Results agree with 0.1.x to the tolerances of
[ADR 0005](docs/adr/0005-parity-two-classes-of-check.md), not to the last bit.

### Added

- A Rust core, `channel_designer._core`, built with maturin as an **abi3 wheel** for Windows, macOS and
  Linux. `pip install channel_designer` needs no compiler (#11). A CI job installs the Windows wheel
  with every compiler off the `PATH` and with source builds refused.
- `RasterTerrain`, `compute_pool_volumes` and `compute_chainage_volumes` read GeoTIFFs with the core's
  own reader: no rasterio, scipy or GDAL needed.
- Python callables still work everywhere they did (terrain samplers, grade, water level, half-width,
  cascade costs, width methods): the core calls back into Python for them.
- This changelog.

### Changed

- Installing from source (a checkout or a git URL) needs a Rust toolchain. Wheels do not.
- `registry`: `register_width_method` and `register_ruleset` still fill the registries in the Python
  modules (`methods._REGISTRY`, `rulesets.RULESETS`), so entry points, `RULESETS[...] = ...` and
  `unregister_width_method` behave as before. On every call that uses a registry the wrapper hands the
  core the registered rulesets as data and the registered width methods as callbacks. The core's own
  registry is never global state that could drift from Python's.
- The Python `PIANC` width method's four callables now run in the core.
- Width-method `options` reach a registered method as floats (an integer option arrives as `3.0`).
- `channel_shape` and `size_channel` read vessel classes from the core's built-in table, which is
  `vessel_designer` v0.2.2 (the pinned release). A vessel class added to `vessel_designer` at run time
  is not seen by them.
- `tests/rules/test_rulesets.py::test_cn_table_a2_holds_only_what_is_read` called a private helper to
  watch which table entries the Python read. The width now comes from the core, so the test calls that
  helper directly. It checks the same thing.
- The core's polygon overlay snaps vertices that lie within 1e-12 of the coordinates' size of an edge
  they should touch, as GEOS does when plain noding fails. Before, such an input (a channel footprint over
  a river polygon that ends on the channel's end edge) panicked.

### Deprecated (removed in 0.3.0)

- Assigning a tunable module constant, such as `loops.TURN_LIMIT_DEG`, `radius_check.RADIUS_TOL`,
  `cut_model.BERM_M` or `bends.E_TOL_M`, no longer changes the result: the constants are fixed in the
  core. The assignment still works and raises a `DeprecationWarning`.
- `RasterTerrain.ds` (the rasterio dataset). It is rebuilt on request, with a `DeprecationWarning`.

### Unchanged, still Python

These stay pure Python in 0.2.0 (the core has no counterpart yet, or they are small helpers over
data classes): `alignment.primitives`, `terrain.profile.line_profile`, the staged design helpers
(`precompute_loa`, `apply_beam`), `volumes.pools.water_level_at`, `grade_at` and `water_level_fn`,
`optimise.lattice.reverse_dijkstra_heuristic` and `FieldScorer`, `optimise.cascade.build_level_table`
(a loop over the core's `section_volumes`) and `LevelTable`, `meanders.summarise_meanders` and
`build_meander_records`, `rules.width_profile`, and `drawings`.

## 0.1.0

First public release: the pure-Python package.
