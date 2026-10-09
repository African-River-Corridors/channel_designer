# channel_designer

Navigation channel design for inland waterways, as code. Give it a river and a vessel class;
it sizes the channel to a named standard, finds an alignment that the vessel can sail, and
measures the cut and dredge volumes.

> **Screening tool.** Not for construction. No design warranty. Check every number against the
> standard and your own design basis before you rely on it.

## What it does

| Layer | Module | What you get |
|---|---|---|
| **Rules** | `channel_designer.rules` | Channel width, bend widening, minimum bend radius and depth from a named ruleset and a vessel class: PIANC WG 141 qualities A/B/C and the Chinese standard GB 50139 (as summarised in WG 141 Appendix A.3). Every coefficient cites its clause, table and page. Rulesets and width methods are pluggable: another package can add its own (`register_ruleset`, `register_width_method`, or the `channel_designer.width_methods` entry-point group). |
| **Alignment** | `channel_designer.alignment` | Centreline from two bank lines, river width, meander analysis, a tight/gentle/straight bend catalogue (exact reachability, not a closed form), rolling-circle straightening to R_min, channel limits with bend widening, and a least-cost route search that shifts the line sideways where that cuts less ground while holding R_min. |
| **Volumes** | `channel_designer.volumes` | Cut Model A and B cross-sections (wet batter, clearance-prism gate, benched face above the waterline), a station-by-station section integrator, a raster DEM integrator, weir-cascade pool levels, and the plan excavation footprint. Quantities only — m², m³ and face heights. |
| **Optimise** *(optional)* | `channel_designer.optimise` | Weir sites and pool levels by an exact search over (site, level), and the least-cost channel through each pool by an A* search over arcs that never bend tighter than R_min. Costs are weights you supply. Core dependencies only. |
| **Drawings** *(optional)* | `channel_designer.drawings` | Cross-section gallery sheets as SVG (and DXF) with ISO 7200 title blocks, built on [`technical_drawings_for_agents`](https://github.com/African-River-Corridors/technical_drawings_for_agents). |

Vessel classes and dimensions come from
[`vessel_designer`](https://github.com/African-River-Corridors/vessel_designer).

Pricing is out of scope. The package measures quantities; what they cost is your call.

## Install

Python 3.12 or later.

```
pip install "channel_designer @ git+https://github.com/African-River-Corridors/channel_designer"
```

Optional extras:

| Extra | Adds | For |
|---|---|---|
| `[dem]` | rasterio, scipy, affine | reading a raster DEM (`volumes.raster.RasterTerrain`, `compute_chainage_volumes`) |
| `[drawings]` | technical_drawings_for_agents | cross-section sheets (`channel_designer.drawings`) |
| `[dev]` | pytest | running the tests |

For example `pip install "channel_designer[dem,drawings] @ git+https://github.com/African-River-Corridors/channel_designer"`.
Calling a drawing function without `[drawings]` raises an `ImportError` that says what to install.

## Worked example

A synthetic meandering river (built in code — no data files), sized for a CEMT class IV
vessel under PIANC quality B, straightened, re-routed and measured. This is
`examples/quickstart.py`; the tests run it.

```python
from channel_designer.alignment.meanders import find_meanders
from channel_designer.alignment.rolling_circle import rolling_circle
from channel_designer.alignment.route import optimise_route
from channel_designer.rules import channel_shape
from channel_designer.synthetic import synthetic_river
from channel_designer.volumes.cut_model import widened_hb_from_shape
from channel_designer.volumes.sections import section_volumes

river = synthetic_river()
shape = channel_shape("PIANC-B", "CEMT_IV", draught_m=2.5)

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
PIANC-B x CEMT_IV: bottom 30.4 m, R_min 255 m, depth 3.25 m
8 bends tighter than R_min; 16 arcs spliced in
rolling circle: dredge   562,780 m3   dry cut   941,841 m3   tallest face 17.3 m
    least-cost: dredge   513,119 m3   dry cut   771,438 m3   tallest face 13.3 m
```

The numbers describe the synthetic river only.

To use your own river, pass your own geometry in a projected CRS (metres): a centreline or two
bank lines as shapely `LineString`s, the wetted area as a `Polygon`, and a terrain sampler. For a
GeoTIFF DEM in the same CRS:

```
from channel_designer.volumes.raster import RasterTerrain   # needs [dem]
with RasterTerrain("my_dem.tif") as terrain:
    v = section_volumes(line, terrain, river_polygon, water_level, depth, half_width)
```

## Optional: optimise weirs, pool levels and the channel through each pool

`channel_designer.optimise` adds two searches. Both take typed config objects and a terrain
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
- **User choices are named, not hidden:** `BANK_CUT_FRACTION`, the `CN-reduced` ruleset, the
  cut-model parameters and the loop-rejection thresholds are judgements, not standards.

## Known issues

- **Rolling-circle rejoin kinks.** `alignment.rolling_circle` joins each arc to the old line
  where the circle crosses it, not tangentially. On a vessel-length chord (about 50 m) the line
  holds R_min. On a short chord the rejoin point reads below R_min: on the synthetic river at
  R_min 300 m, 111 m on a 10 m chord and 288 m on a 25 m chord. Read curvature on a
  vessel-length chord, or smooth the exits first. Details:
  [docs/known-issues.md](docs/known-issues.md).

## Contributing

Issues and pull requests are welcome — a new national ruleset is a good first contribution. See
[CONTRIBUTING.md](CONTRIBUTING.md) and [ROADMAP.md](ROADMAP.md).

## Licence

Apache-2.0. Copyright 2026 African River Corridors Limited. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).
