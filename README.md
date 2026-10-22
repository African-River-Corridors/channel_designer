# Open Waterway Designer (OWD)

Navigation channel design for inland waterways, as code. Give it a river and a vessel class;
it sizes the channel to a named standard, finds an alignment that the vessel can sail, and
measures the cut and dredge volumes. Formerly `channel_designer`.

> **Screening tool.** Not for construction. No design warranty. Check every number against the
> standard and your own design basis before you rely on it.

## What it does

| Layer | Module | What you get |
|---|---|---|
| **Rules** | `owd.rules` | Channel width, bend widening, minimum bend radius and depth from the selected Rule Sets and a vessel class, on the rules engine shared with open-vessel-concept-drawings: `pianc-wg141` (qualities A, B, C) and `gb50139` (the Chinese standard GB 50139-2014). Every Rule cites its clause, table and page, and is unverified until a second reader checks it. Select more than one set and the strictest Rule governs; load a Rule Set file of your own at run time (`owd rules`, `owd size --rule-set-file`). Width methods stay pluggable (`register_width_method`, or the `owd.width_methods` entry-point group); they are design assumptions, not verified rules. |
| **Alignment** | `owd.alignment` | Centreline from two bank lines, river width, meander analysis, a tight/gentle/straight bend catalogue (exact reachability, not a closed form), rolling-circle straightening to R_min, channel limits with bend widening, and a least-cost route search that shifts the line sideways where that cuts less ground while holding R_min. |
| **Volumes** | `owd.volumes` | Cut Model A and B cross-sections (wet batter, clearance-prism gate, benched face above the waterline), a station-by-station section integrator, a raster DEM integrator, weir-cascade pool levels, and the plan excavation footprint. Quantities only — m², m³ and face heights. |
| **Terrain** | `owd.terrain` | Long profile of any line over a terrain sampler: chainage, levels, ascent and descent. Pure; it reads no files and fetches nothing. |
| **Optimise** *(optional)* | `owd.optimise` | Weir sites and pool levels by an exact search over (site, level), and the least-cost channel through each pool by an A* search over arcs that never bend tighter than R_min. Costs are weights you supply. Core dependencies only. |
| **Drawings** | `owd.draw` | Engineering drawings of a design line option, drawn by the Rust core through [`otd`](https://github.com/African-River-Corridors/open-technical-drawer), the [Drawing Format Standard](https://github.com/African-River-Corridors/technical_drawings_for_agents) in Rust: the cross-section gallery and the typical section (ISO 7200 title block, one scale, dredge and dry cut hatched, areas, dimensions), as drawing documents, SVG, PDF and DXF. The same sheets as `owd draw`. Every sheet says "Screening — not for construction". |

Vessel classes and dimensions come from
[`vessel_designer`](https://github.com/African-River-Corridors/open-vessel-concept-drawings).

Pricing is out of scope. The package measures quantities; what they cost is your call.

## Install

Python 3.12 or later.

```
pip install open-waterway-designer
```

The PyPI release is still to come ([#2](https://github.com/African-River-Corridors/open-waterway-designer/issues/2)); until it ships, install from git as shown below.

From 0.2.0 the calculations run in a Rust core (`owd._core`) that ships inside the
package. Each release has one **abi3 wheel** per platform (Windows, macOS, Linux), so
`pip install` needs **no compiler and no Rust**. The same wheel runs on every Python from 3.12.
CI proves it: the `Wheels` workflow installs the wheel on a Windows machine with no compiler on
the `PATH`, and refuses any source build (`--only-binary :all:`).

Installing from source (a checkout or the git URL) builds the core with
[maturin](https://www.maturin.rs/), so it **does** need a Rust toolchain ([rustup.rs](https://rustup.rs)):

```
pip install "open-waterway-designer @ git+https://github.com/African-River-Corridors/open-waterway-designer"
```

Your scripts do not change. The 0.2.0 functions and classes have the names, arguments and results of
0.1.x; see [CHANGELOG.md](CHANGELOG.md) for the few things that moved, and what is deprecated.

### Your own river, DEM and water level

A study starts with `owd new`, which makes an empty design. The river, the DEM and the water level all go in
with `owd import`. There is no `owd new --dem`.

```
owd new study.owd --title "My river"
owd import study.owd --polygon river.geojson --centreline centre.geojson --dem lidar.tif --water-level 38
owd bed study.owd --depth 3 --season dry
owd fits study.owd
```

- **The river.** Give the river polygon and the centreline, or the two bank lines (`--left-bank`, `--right-bank`).
  GeoJSON or a GeoPackage layer (`FILE#LAYER`), in WGS84 or WGS84 / UTM.
- **The DEM.** A GeoTIFF in the river's CRS or in WGS84. The design keeps its relative path and its SHA-256, never a
  copy, so keep the file next to the design. Without a DEM of your own, `owd fetch-dem` gets Copernicus GLO-30.
- **The water level.** `--water-level 38` is one level for the whole river, in metres. Add `--water-slope` for a
  level that falls along the river, or give `--water-table FILE` (lines of `chainage_m,level_m`). Without one,
  `owd water-level` sets a default from SWORD and SWOT.
- **The depth.** `owd bed` gives an assumed depth where there is no survey. With the DEM, the water level and a
  depth, `owd fits` gives the full answer (width, radius and depth).

Then size and design for the vessel you choose (below): `owd size`, `owd align`, `owd volumes`.

### Channel rules

`owd size` sizes the channel to the Rule Sets you select (`--rule-set pianc-wg141 --set-input se_quality=B`); with
two sets the strictest Rule governs. `owd waterway` marks canal and river reaches, so a Rule can apply on part of
the river. After the volumes stage each design line option has a verdict and its constraint stretches
(`owd export --csv --stretches`), and `owd trace` prints the full Rule Trace. Size for any class of the shared vessel
catalogue (`--vessel CEMT-VIb-motor`), or for a vessel of your own: `owd size design.owd --custom-vessel "My barge"
--loa 140 --beam 29 --draught 4.7 --formation motor` (a convoy also takes `--abreast` and `--in-line`; GB 50139 reads
`--gb-class`). Every output labels it `Custom: My barge, 140 × 29 × 4.7 m`, and `owd fits` adds it as one more row.
Designs are document format 0.4; a format 0.3 file (the number 3) opens and is saved as 0.4, and an older file is
refused and must be rebuilt. See [docs/rules/channel-rules.md](docs/rules/channel-rules.md).

### Drawings

After the volumes stage, `owd draw design.owd --out sheets/` writes, for each sheet, the drawing document, the
normalised content, SVG, PDF and DXF. `owd draw design.owd --document` prints the drawing documents only. In Python,
`owd.draw("design.owd", out="sheets/")` gives the same files, byte for byte. Sheets are drawn by
[`otd`](https://github.com/African-River-Corridors/open-technical-drawer), the Drawing Format Standard in Rust. Every
sheet is marked "Screening — not for construction".

### Reports

`owd report design.owd --kind all --format all -o reports/` writes four reports, each as PDF, HTML and Word, through
the [Open Report Designer](https://github.com/African-River-Corridors/open-report-designer): the screening report
(with the Rule Trace as an appendix), the quantity schedule, the section pack and a run report for testers. Every
report has document control, the screening notice on every page and a "Data and limits" box. The app's Sheets view
has a **Report…** button that makes the same files. See [docs/reports.md](docs/reports.md).

Optional extras:

| Extra | Adds | For |
|---|---|---|
| `[dem]` | rasterio, scipy, affine | nothing the package needs any more: the core reads GeoTIFFs itself. Kept so existing install commands work. |
| `[dev]` | pytest | running the tests |

The `[drawings]` extra is gone: drawings are part of the package now (spec 0004).

## Worked example

A synthetic meandering river (built in code — no data files), sized for a CEMT class IV
vessel under PIANC quality B, straightened, re-routed and measured. This is
`examples/quickstart.py`; the tests run it.

```python
from owd.alignment.meanders import find_meanders
from owd.alignment.rolling_circle import rolling_circle
from owd.alignment.route import optimise_route
from owd.rules import channel_shape
from owd.synthetic import synthetic_river
from owd.volumes.cut_model import widened_hb_from_shape
from owd.volumes.sections import section_volumes

river = synthetic_river()
shape = channel_shape("CEMT_IV", draught_m=2.5, rule_sets=["pianc-wg141"],
                      set_inputs={"se_quality": "B"})

tight = find_meanders(river.centreline, max_radius=shape.min_bend_radius_m)
rolled = rolling_circle(river.centreline, shape.min_bend_radius_m, step=10.0)

route = optimise_route(
    rolled.centreline, river.terrain, river.polygon,
    grade=lambda s: river.water_level(s) - shape.design_depth_m,
    bottom_width=shape.bottom_width_straight_m, depth=shape.design_depth_m,
    side_slope=3.0, r_min=shape.min_bend_radius_m, corridor_sweep=(40, 80, 120))

def hb(xy):
    return widened_hb_from_shape(xy, shape)

for name, line in (("rolling circle", rolled.centreline), ("least-cost", route.line)):
    v = section_volumes(line, river.terrain, river.polygon, river.water_level,
                        shape.design_depth_m, hb, model="b", maxw_m=150.0)
```

Output:

```
pianc-wg141@0.3.0 + owd@0.3.0 x CEMT_IV: bottom 30.4 m, R_min 255 m, depth 3.25 m
8 bends tighter than R_min; 16 arcs spliced in
rolling circle: dredge   562,780 m3   dry cut   941,841 m3   tallest face 17.3 m
    least-cost: dredge   513,163 m3   dry cut   776,725 m3   tallest face 13.3 m
```

The numbers describe the synthetic river only.

To use your own river, pass your own geometry in a projected CRS (metres): a centreline or two
bank lines as shapely `LineString`s, the wetted area as a `Polygon`, and a terrain sampler. For a
GeoTIFF DEM in the same CRS:

```
from owd.volumes.raster import RasterTerrain
with RasterTerrain("my_dem.tif") as terrain:
    v = section_volumes(line, terrain, river_polygon, water_level, depth, half_width)
```

## Optional: optimise weirs, pool levels and the channel through each pool

`owd.optimise` adds two searches. Both take typed config objects and a terrain
sampler, and both minimise weights you supply, never prices:

- **Cascade** (`search_cascade`) — where the weirs go and how high each pool sits. An exact
  shortest path over (candidate site, pool level). The reach cost is yours; the usual one is
  earthworks per level from `build_level_table` + `earthworks_reach_cost`.
- **Lattice** (`optimise_pools`) — the channel through each pool, pinned to your design line at
  both ends. A* over left/right arcs at R_min and wider, so R_min holds by construction.

This is `examples/optimise_pools.py`; the tests run it.

```python
cfg = CascadeConfig(candidate_sites_m=[500, 1000, 1500, 2000, 2500], lifts_m=(0.5, 1.0),
                    terminal_m=3000.0, terminal_min_level_m=0.5, min_spacing_m=1000.0)
table = build_level_table(line, river.terrain, river.polygon, cascade_levels(cfg),
                          depth, half_width, model="b", maxw_m=120.0)
best = search_cascade(cfg, earthworks_reach_cost(table, wet_weight=1.0, dry_weight=1.5),
                      structure_cost=lambda lift: 1.0e5 + 2.0e5 * lift).best

spec = SectionSpec(half_width=half_width, side_slope_h_per_v=3.0, dry_weight=1.5,
                   rake_half_m=80.0, rake_step_m=5.0, ds_step_m=10.0)
lattice = LatticeConfig(r_min_m=r_min, pitch_m=20.0, n_heading=24, radius_multiples=(1, 2, 4))
out = optimise_pools(line, river.terrain, river.polygon, best.pools(), depth_m=depth,
                     section=spec, cfg=lattice, corridor_half_m=180.0, cell_m=5.0)
```

Output, re-measured with Cut Model B (a straight design line across a gently meandering
synthetic river):

```
weirs: 1000 m -> pool 0.5 m
  straight: dredge    21,988 m3   dry cut   934,659 m3
 optimised: dredge   141,201 m3   dry cut   423,598 m3
```

The search cost is a search objective. Re-measure the winner with `volumes.sections` before
you quote a quantity. The lattice is coarse by design; a finer one (`pitch_m`, `n_heading`,
more `radius_multiples`) follows the river more closely and runs longer.

## Conventions

- **Units:** metres, m², m³. **CRS:** any projected metric CRS; reprojection is the caller's job.
- **Offsets:** signed, − left bank, + right bank, looking downstream→upstream along the line.
- **Checks are data:** rules return `Check(name, ok, message, severity)` lists, not buried `if`s.
- **User choices are named, not hidden:** `BANK_CUT_FRACTION`, the reduced-radius claim on a canal reach, the
  cut-model parameters and the loop-rejection thresholds are judgements, not standards.

## Known issues

- **Rolling-circle rejoin kinks.** `alignment.rolling_circle` joins each arc to the old line
  where the circle crosses it, not tangentially. On a vessel-length chord (about 50 m) the line
  holds R_min. On a short chord the rejoin point reads below R_min: on the synthetic river at
  R_min 300 m, 111 m on a 10 m chord and 288 m on a 25 m chord. Read curvature on a
  vessel-length chord, or smooth the exits first. Details:
  [docs/known-issues.md](docs/known-issues.md).
- **R_min is reported, not guaranteed.** `rolling_circle` and `optimise_route` can return a
  design line tighter than R_min (a jagged input line; a baseline tighter than R_min). Both
  results carry `radius_ok`, `measured_min_radius_m` and `radius_check_chord_m`. Check
  `radius_ok` before you use the line.

## Contributing

Issues and pull requests are welcome — a new national Rule Set is a good first contribution
([docs/rules/codification.md](docs/rules/codification.md)). See
[CONTRIBUTING.md](CONTRIBUTING.md) and [ROADMAP.md](ROADMAP.md).

## Licence

Apache-2.0. Copyright 2026 African River Corridors Limited. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).
