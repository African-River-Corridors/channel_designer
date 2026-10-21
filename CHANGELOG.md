# Changelog

All notable changes to Open Waterway Designer (OWD), called `channel_designer` before 2026-10-21. The format follows [Keep a Changelog](https://keepachangelog.com/);
versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

- **Renamed to Open Waterway Designer (OWD), spec 0006.** Names only; no result moves. The repo is
  `open-waterway-designer` (GitHub redirects the old URL), the Python import is `owd`, the PyPI name is
  `open-waterway-designer`, the crates are `owd-core`, `owd-format`, `owd-cli`, `owd-py` and `owd-app`, the command
  is `owd`, the file extension is `.owd`, and the schema is `schema/owd.schema.json`. **To move over:**
  change `import channel_designer` to `import owd`; run `owd` instead of `cdesign`; save as `.owd`; declare a
  width-method plugin under the entry-point group `owd.width_methods`. **For one release the old names still
  work:** `import channel_designer` gives a `DeprecationWarning` and the same objects; `cdesign` is an alias of
  `owd` that prints a note on stderr; `.cdesign` files open in the command line and the app, with the same
  contents (`format_version` stays 2); the old entry-point group `channel_designer.width_methods` is still read.
  They go in the release after this one. The desktop app is now "Open Waterway Designer" with a new bundle
  identifier, `org.africanrivercorridors.openwaterwaydesigner`: macOS, Windows and Linux treat it as a new app, so
  its saved window state and recent files do not carry over. The version is not changed by this entry.
- Trademark policy: new `TRADEMARKS.md` says how the Open Waterway Designer (OWD) and ARC names and logos may be used. The Apache licence covers the code; this policy covers the marks. No result moves.
### Design impact

Every pull request is checked for design impact (`tools/design_impact.py`, CI job `design-impact`): fixed
scenarios run on the base branch and on the pull request, and a result that moves by more than 1e-9
(ADR 0005) must have a bullet here, under this heading, saying what moved and why. Add the bullet in the
pull request that moves the result.

- **The water-aware rolling circle in the channel design document (#100, spec 0005).** Format version 2.
  In a new document with a river polygon, the rolling circle is water-aware by default (`use_water`):
  each bend takes the four-rung ladder of #97, and the option carries a `fits_in_water` check (with
  its stretches and the note "width only, no depth data") that travels into `status`, the CSV (a
  new last column, `notes`) and the GeoJSON. Its line can differ from the old rolling circle's
  wherever a bend did not fit the water. Optional `seed_legs` and `fixed` inputs are joined first
  (`join_legs`), with a `join_guard` check. A version 1 document opens as version 2 with
  `use_water: false` recorded, so its results do not move, and fresh stages stay fresh. The
  design-impact scenarios run the Python API, not documents, so they show no change.
- **One radius reading (#95, spec 0005 Q11).** The least-cost re-route now reports
  `measured_min_radius_m`, `radius_check_chord_m` and `radius_ok` read on the chord
  (`min_radius_on_chord`, default R_min / 3), as the rolling circle already did. It used to report
  the reading on its own station vertices, which misses the corners between stations. The
  smoothing and the choice between corridors still read the vertices. What moved: the quickstart
  re-route's `radius_ok` goes from true to false (254.0 m on the vertices, 103.6 m on the 85 m
  chord); its reported chord goes from 200 m (2 × station step) to 85 m. Which way it moves depends
  on the line: a corner between stations reads tighter on the chord, while a smooth excursion can
  read wider on a long chord than on its station vertices. The pool lattice (`LatticeResult`) and
  the pools result (`PoolsResult`) gain the same three fields. Rolling-circle results do not move.
- **The fairway fit is a minimum-curvature corridor solve (#126, ADR 0007).** The elastic band
  inside `fairway_fit` is replaced by the racing line inside navigable water (a convex QP on lateral
  offsets, solved by Clarabel, re-linearised), and `join_legs` builds one corridor per route instead
  of fillets and the join guard. What moved: every result of `fairway_fit`, `fairway_fit_in_water`,
  the water-aware rolling circle's rung 3, `join_legs`, `guard_joins`, `tangent_joins` and
  `optimise_pools_tangent`. Why: on real routes the band stayed in the middle of the water, made
  narrow bends sharper than its input (281 m to 12 m) and left kinks at fixed tails. On the
  synthetic fixtures now: a 200 m bend in 200 m water is taken wide (39-49 % of its bending length
  in the outer third of the navigable width; the centreline has none); the narrow creek is never
  fitted sharper than its input (input 220 m; fitted 271, 301 and 340 m at 40, 46 and 60 m width);
  a fixed tail is met with continuous curvature; a stretch with no navigable width is reported and
  passed without a kink; the open 90° basin's join is one smooth line whose tightest bend is 674 m
  (it was a 639 m fillet with a curvature step at each end). `join_legs` now reports each turning
  join as `FairwayFit` (never `Fillet`) and its `guard_failed` is always empty. API: `FitParams`
  loses the band's settings (`levels`, `length_weight`, `max_jump_spacings`, `admm_rho`,
  `admm_iters`) and gains the corridor's (`spacing_m`, `passes`, `converged_m`, `reach_m`,
  `fold_share`, `reweight_cap`, `free_cap_m`, `pull_weight`); `FitLevel` is gone; `JoinParams` loses
  `r_ceiling_m` and `radius_step_m`. The fixed scenarios of this check do not call these functions
  (#100 wires them into the document and CLI), so no scenario moves.

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
