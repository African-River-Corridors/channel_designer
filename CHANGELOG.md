# Changelog

All notable changes to Open Waterway Designer (OWD), called `channel_designer` before 2026-10-21. The format follows [Keep a Changelog](https://keepachangelog.com/);
versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

## 0.3.0

- **Long section (spec 0004, T10).** `owd draw` now draws the long section, kind `LS`: the river bed (the ground
  on the design line), the water level and the design bed (the invert) along chainage, with a chainage axis and a
  level axis. The values are the series of the app's long-section view, so the sheet and the app agree. The sheet
  uses the vertical exaggeration of Drawing Format Standard 1.2: levels are drawn larger than chainage by a whole
  factor, the vertical scale is a ratio of the standard, and the sheet prints a vertical scale bar, the words
  "Vertical exaggeration ×E" and a note such as "VERTICAL EXAGGERATION ×100: LEVELS DRAWN AT 1:50, CHAINAGE AT
  1:5000". The title block scale is AS SHOWN. The style's new `long_section` section sets the scales, the bands and
  `vertical_exaggeration`: `"auto"` (the default, the smallest factor whose levels fill half the plot) or a whole
  number of 2 or more; `owd draw --vertical-exaggeration N` overrides it. A line too long for one sheet is split
  over sheets at one scale. `owd draw` checks that every drawn level and chainage equals its result. The `otd` pin
  moves to v0.4.0 (standard 1.2). Only the LS documents use 1.2; no earlier sheet changes.

- **The default water level never rises downstream, across SWOT–DEM joins too (#280).** A DEM-filled stretch was
  made never rising only within itself, so where it met a lower SWOT level just upstream, the level jumped up
  going downstream (about 2.5 m at km 16.4 on the Sassandra). `owd water-level` now makes the whole profile never
  rise downstream, and SWOT outranks the DEM:
  1. A SWOT level above the SWOT level upstream of it is lowered to it (within SWOT's noise, as in a tidal estuary).
  2. A DEM level above the SWOT level upstream of it is lowered to it.
  3. A DEM level below the SWOT level downstream of it is raised to it.

  Each changed level is listed under `adjusted` in the water-level provenance (chainage, source, `lowered` or
  `raised`, the level before and after, and why), and the command counts them.
- **Rolling-circle tangent exits (issue #5).** Each arc the rolling circle splices in now leaves the circle along
  a straight tangent to it, which meets the old line at a vertex with the old line's heading (circle, then
  straight, as in a Dubins path). Up to 0.2.0 the arc ended where the circle crossed the old line, which left a
  short kink at every exit. The arc also has a vertex at least every metre (it had 32 segments whatever its
  length), so it reads its own radius on a short chord. A circle whose straight cannot meet the old line keeps
  the crossing exit. Each circle record gains `exit_point` (where the arc ends) and `tangent_exit` (false on that
  fallback); `contact_point` and `end_chainage_m` are now where the splice joins the old line. On the synthetic
  river at R_min 300 m and a 10 m step, the minimum radius on a 10 m resample goes from 111 m to 295 m
  (`docs/known-issues.md`).
- **Rolling-circle tangent entries (#283).** Each arc now starts at a vertex of the line, on a circle tangent to
  the segment into that vertex, so the heading is continuous at the entry. Before, the circle was placed at the
  resample point, tangent to the direction read on the resample, which was up to 0.9° off the line's own heading.
  From pass 2 on, a circle with no tangent exit is no longer spliced (the roll moves on to the next sample), so
  later passes never add a crossing exit; pass 1 keeps the flagged fallback. On the synthetic river at a 25 m
  rolling step, the minimum radius on a 10 m resample goes from 280 m to 300 m at R_min 300 m, and from 227 m to
  255 m at R_min 255 m (`docs/known-issues.md`).
- **The Sassandra worked example (spec 0012, T12).** `examples/sassandra/run.sh` makes a study of the Sassandra,
  Côte d'Ivoire, from the sea to the Soubré dam, from a river name and two points, with open data fetched at run
  time. It gives the plan-only answer, then the full answer with an assumed 3 m depth, on both plan lines. The docs
  page "Worked example: the Sassandra" gives the result, its labels and notices, the provisional defaults, and the
  limits: no class fits as it is, the mask needs a 30 % threshold here, and the plan line for the radius is still
  to be chosen.
- **Width-method plugins load from the `owd.width_methods` entry-point group only.** The old group
  `channel_designer.width_methods`, read for one release after the rename, is no longer read. **Plugin authors:** declare
  your method under `[project.entry-points."owd.width_methods"]` in your package's `pyproject.toml`.
- **The app's Sheets view and PDF export (spec 0004, T8).** A Sheets button lays a view over the map and sections.
  It lists the drawing sheets of the active design line option and shows each sheet's SVG on white paper,
  with zoom. Export PDFs writes one PDF a sheet: on the desktop into a folder you pick, on the web as
  downloads. The sheets come from the same builder as `owd draw` through the shell (`owd_shell::sheets`), so
  each PDF has the bytes `owd draw --option` writes for the same design and style. The front end only places
  and zooms the SVG. The live long-section and cross-section views are unchanged.

- **The full `fits` answer can read the river's radius on a fairway (spec 0012, opt-in).** `owd fits --plan-line
  fairway` reads the river's own radius on each class's fairway fit (ADR 0007: the smoothest line a channel of
  that class can take in the water) instead of the river centreline. A centreline drawn from bank lines wiggles at
  the scale of its stations, and on the Sassandra that made about a thousand short "tight bend" stretches per class.
  Width, depth, the water level and the reaches still read the river centreline. The default stays `centreline`,
  and a default run writes nothing new; the choice is kept in the answer's settings, so changing it makes the answer
  stale.
- **Point-in-water and water-width tests indexed (spec 0012).** The full `fits` answer on the edge-fitted Sassandra
  (a 181 km centreline, a 77,000-vertex river polygon) took 748 s. It now takes 45 s, with the same answer to the
  last bit. Many points tested against one long river now go through an index: `PolygonIndex` (each ring's segments
  by horizontal strips, the same answers as `contains_xy`) and `WaterIndex` (the cross-section ray walked through a
  grid of the segments, the same answers as `cross_section`). The volumes' wet test, the per-station width check and
  the spec 0007 screening use them, and the screening also chains its line and projects onto the centreline once,
  not once per station.
- **The app's Rule Set picker, and the last of the old names (spec 0007, T6 and T8 part 2).** The inputs panel
  picks one Rule Set or two, with their set inputs (quality A, B or C for `pianc-wg141`; the permissive or strict
  end for `gb50139`), each marked `default` or `typed` in words and a shape. A pick goes through the session and
  sizes at once; the later stages go stale. The app now shows the design basis (each quantity with its governing
  Rule, sets, citation, origin and status), the verdict of each option, and a table of constraint stretches;
  picking a stretch draws it on the map and moves the map to it. Every result says that each Rule is unverified.
  **Removed:** `owd size --ruleset` (use `--rule-set pianc-wg141 --set-input se_quality=B` for the old PIANC-B),
  `Session::set_sizing(ruleset_id, …)` and the old-names `sizing_choices` list; the app commands `set_sizing` gave
  way to `set_rule_sets(ids, inputs)`, `set_vessel(vessel, draught)` and `stretches_geojson`.
  `Session::preview_sizing` takes a selection. `Session::sizing_choices()` now lists `rule_sets` (each with its
  `inputs`), `always` and `vessels`. **JSON key renames:** the sizing results' `ruleset_id` is now `rule_sets`
  (the selection as words, such as `pianc-wg141@0.3.0 (se_quality B)`); in `fits`, `settings.rulesets` is now
  `settings.rule_sets`, `results.rulesets` is now `results.rule_sets`, and each entry's `ruleset_id` is now
  `rule_set`, with a new `selection` (its Rule Set and set input). `owd fits --ruleset` still takes the old names.
  The typical section's notes say "RULE SETS …" for "RULESET …". No number in the examples moved.
- **The fairway fit never returns a line worse than the one it was given (#132).** The fit keeps its best pass:
  the fewest points outside the water, then the widest tightest bend on the chord. The line it starts from is
  now a candidate too. So where the input already lies in navigable water, its tightest bend is never fitted
  sharper. Before, the plain sum of squared curvature could trade a sharper peak for less total bending (up to
  7 % sharper on synthetic creeks). No fixture result moves (the design-impact check), but a fit of such an
  input may now return a line with a wider tightest bend than before.

- **Removed: the old-name shims (spec 0006, T10).** `import channel_designer`, the `cdesign` command and the
  ability to open `.cdesign` files are gone; use `import owd`, `owd` and `.owd`. A `.cdesign` file now gives an
  error that says to rename it to `.owd` (the content is the same).
- **The old `Ruleset` path is gone (spec 0007, T8 part 1).** Channel sizing goes through the Rule Sets only. Removed
  from Rust: `Ruleset`, `RulesetRegistry`, `global_rulesets`, `channel_shape(registry, ruleset_id, …)` and the
  parity-only code; the engine path's numbers are now pinned by a snapshot the old code wrote
  (`crates/owd-core/tests/data/old-ruleset-snapshot.json`, 360 cases, bit for bit). Removed from Python:
  `Ruleset`, `RULESETS`, `register_ruleset`, `get_ruleset` and `CN_TABLE_A2`. **Python API change:**
  `owd.rules.channel_shape(vessel_standard_id, draught_m=None, *, rule_sets=("pianc-wg141",), set_inputs=None,
  reaches=None, rule_set_files=())`; for example `channel_shape("CEMT_IV", 2.5, rule_sets=["pianc-wg141"],
  set_inputs={"se_quality": "B"})` gives the old `channel_shape("PIANC-B", "CEMT_IV", 2.5)`. `ChannelShape.ruleset_id`
  is now `rule_sets`. New: `owd.rules.design_basis(...)`. `owd fits` sizes each old name through its Rule Set
  selection; its output is unchanged. The width methods, the `pianc` method with its Report 121 build-up and
  `ChannelDesignParams` are now documented as design assumptions, not verified rules. GLOSSARY: Rule Set, Rule,
  Measure, Requirement, Governing Rule, Rule Trace, Reach, Design basis, Constraint stretch, Verdict; "Ruleset"
  goes. Kept until the app's Rule Set picker (T6): the old names in the app's list and `owd size --ruleset`.
- **Version 1 and 2 files are refused (spec 0007).** OWD 0.3.0 reads document format 3 only and migrates nothing.
  To rebuild a design: `owd new FILE`, `owd import FILE` with its bank lines, centreline, river polygon and DEM,
  `owd size FILE --rule-set ID --set-input NAME=VALUE --vessel CLASS` (for an old PIANC-B design:
  `--rule-set pianc-wg141 --set-input se_quality=B`; for CN-reduced, first `owd waterway add FILE --from 0
  --waterway canal --claim-reduced-radius`, then `--rule-set gb50139 --set-input range_end=strict`), then
  `owd run FILE`.
- **Edge fit across Sentinel-2 tiles (spec 0012, #264).** A reach longer than one Sentinel-2 tile (about 110 km), or
  one across a tile edge, found no scene and `--edge-fit` refused it. A reach longer than 15 km is now fitted in parts
  of 15 km that overlap by 2 km, each on the clearest dry-season scene that covers it. The shifts are blended across
  each overlap, and the 30 m and 1.5 m per 5 m limits then hold along the whole bank. A part no clear scene covers
  keeps its mask bank, flagged `no-edge`, and the provenance says why. The provenance lists every part with its
  scene, and gives the notice once per year of imagery. The scene search now asks the server for the cloud limit:
  over two years a box could match hundreds of scenes, more than the one page read, and the clearest could be missed.
  A reach of 15 km or less is fitted exactly as before.
- **The full `fits` answer is fast on a long river (spec 0012, #267).** On the Sassandra (a 178 km centreline, 12
  classes) the full answer took 44 minutes. It now takes under 3, with the same answer to the last bit:
  - the classes run in parallel, each on its own copy of the design, and the answer is written back in the same
    order (in the browser they still run one after another);
  - the bed model finds each point's chainage through a grid of the centreline's segments
    (`owd_core::geom::linear::ProjectIndex`) instead of scanning every segment. It gives what the scan gives, to
    the last bit; the volumes read the same bed model, and are faster too.
- **Orthophoto and EOxCloudless imagery on plan sheets (spec 0004, T6).** Plan sheets can now be drawn over
  other backdrops.
  - `owd draw --orthophoto FILE` draws them over the user's orthophoto: an 8 bit RGB GeoTIFF in WGS84 or the
    design's UTM zone, sampled at each pixel centre through OWD's own projection. It is white where the
    file holds no pixel.
  - `owd draw --imagery eox-cloudless-2016` fetches EOxCloudless 2016 through `otd-imagery` (otd v0.3.0).
    No other source or year is offered: from 2018 the mosaics are CC BY-NC-SA 4.0 and need a paid licence
    for commercial use. Every sheet that shows the imagery prints "EOxCloudless https://cloudless.eox.at
    by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)" word for word, and notes
    that the imagery is of 2016 and the river may have moved since.
  - Tiles are cached in `$XDG_CACHE_HOME/owd/imagery` (or `~/.cache/owd/imagery`, or `--imagery-cache`),
    never next to the drawings.
  - A failed or slow fetch (30 s a tile) falls back to shaded relief for that sheet, with a warning, not an
    error.
  - The network crate is only in the command line (owd-format's `imagery` feature), not the web build.
  - Plan notes are now cut at word boundaries to the standard's 120 characters, and the coordinates have a
    note of their own. The backdrop's layer is now `BACKDROP`.

- **Constraint stretches on the drawings (spec 0004, T9).** A new label rule, `constraint-stretch`, reads the
  stretches the screening stores (spec 0007): width, depth or radius short. On the plan each stretch gets one
  band along the line and one short label. Its `fields` choose what the label shows, in order (`kind`,
  `deficit`, `works`, `chainage`), for example "WIDTH SHORT · 3.4 M · BANK CUT · CH 0+325.3 TO 0+625.5". A new
  extra-section rule, `constraint-worst`, adds the worst station of each stretch to the cross-section
  gallery, once. Both rules are in the default style; take them out to remove the bands, labels and
  sections. Infeasible stretches keep their own rule. `owd draw` checks that each stretch has one band and
  one label, and that the label's deficit and chainages equal the stretch's. A design with no stretch shows
  none. The gallery of a design with stretches now has more sections by default.

- **Layout review in `owd draw` (spec 0004, T7).** Each sheet now goes through `otd`'s layout review before it
  is written. An error fails `owd draw` (and `owd.draw`) and names the sheet and the finding, for example
  "sheet OWD-…-XS-001: layout review: error `text-overlap` in viewport xs-1: texts "A" and "B" overlap by
  1.50 mm". The errors are `text-overlap`, `text-print-size` and `outside-frame`. A warning (`text-on-hatch`) is
  printed and the sheet is written. The `otd` pin moves to v0.3.0. The plan sheets now place their texts clear
  of each other and of the north arrow and scale bar; a grid or tick label with no room is left out. Every
  sheet of the fixture designs passes with no error.

- **Plan sheets with shaded relief (spec 0004, T5).** `owd draw` now makes plan sheets, kind `PL`. First an
  overview of the whole line at a fitted scale (the map ratios allowed), with the strip sheets outlined as a
  key. Then strips along chainage at the style's scale (default 1:5000 on A1), with match lines that name the
  next sheet. Each sheet is drawn over shaded relief of the design DEM, from `otd` 0.2.0. It shows the water
  edge (the river bank lines of the design), the design line with chainage ticks, the channel footprint (toe
  and top of cut at the stations of the volumes stage), the gallery's section lines `S1`, `S2`, ..., a UTM
  grid, a north arrow and a scale bar. Labels mark exceptions only. The two first label rules are
  `bend-tighter-than` (parameter `k`, against the ruleset's least bend radius, on the bend rows of the
  setting-out table) and `infeasible-stretch` (the stored stretches of `checks.fits_in_water`). Leave a rule
  out of `plan.label_rules` and its labels go. Strips are north up, as the standard draws an image only in a
  viewport with no rotation. `owd draw` checks that each match line and section line crosses the line at its
  result. The `otd` pin moves from v0.1.0 to v0.2.0 (Drawing Format Standard 1.1). No earlier sheet changes.

- **Start from a river, in the app (spec 0012, T10).** A "Start from a river" view on the desktop and the web: a
  river name, a start, an end and via points (typed, or picked on the map), the route with its length, then
  "Make the inputs": banks from the JRC water mask, moved onto Sentinel-2 (optional), and the default water level
  from SWORD and SWOT. Each step is its own call, with progress and Cancel (a cancelled step changes nothing). Each
  input shows who made it in a word and a shape (◇ default, ✎ edited, ■ user) with its source and date, and every
  licence notice is listed on the map. The JRC and DEM tiles the reach needs are download links; on the web the
  JRC tiles are given back by hand (their host does not let a web page read them), and Sentinel-2, SWORD and
  Hydrocron are read from the page's worker. Then a depth (one depth, depths at chainages, a season), "Answer
  what fits" with the class table and each class's stretches on the map, and "Use this vessel", which sets the
  sizing and returns to the stages. A DEM loaded by hand is now marked `user` in the provenance. A water level the
  DEM alone gave was recorded with the date `DEM` and no notice, which the schema refuses: it is now dated the day
  it was made and carries the DEM's notice.
- **What fits, with depth (spec 0012, T9).** With a bed model, a water level and its DEM, `owd fits` gives the full
  answer: each vessel class is sized under the ruleset, walked along the river centreline with the bed model as the
  bed, and screened by the spec 0007 path itself. Each class row then carries the verdict (`navigable-as-is`,
  `navigable-with-works` or `not-navigable`) and the constraint stretches (width, radius, and now depth), each with
  its governing Rule, what it needs and finds, the works, and the dredge and dry cut inside it. "Largest class that
  fits as it is" counts depth too. Every row says "depth assumed, not measured". The answer goes stale when the bed
  model, the water level or the DEM changes. Without a DEM or a bed model the answer is plan only, exactly as before.
- **Default water level from SWORD and SWOT (spec 0012, T6).** New `owd water-level FILE` sets the river's low-water
  level along its centreline, as the water level profile every stage already reads. The SWORD v17c reaches near the
  river are read from the database's shapefile release on Zenodo (only the basin the river is in, found from the
  shapefile headers in the zip by range request). Each reach's SWOT levels come from Hydrocron (good and suspect
  quality); the 10th percentile is the reach's level, labelled with the dates used. Where a reach has fewer than 3
  observations, is under 100 m wide, or there is no reach, the DEM water surface on the centreline is used, made to
  never rise downstream (SWORD's distance to the outlet, else the DEM, says which way that is). `--percentile` and
  `--min-swot-width` change the provisional defaults, and the level is then marked `user`; a typed level is still
  `owd import --water-level`. SWOT levels are heights above EGM2008, the datum of the Copernicus DEM, so nothing is
  converted for it; for a DEM in ellipsoidal heights (`--dem-datum ellipsoid`) each observation adds its geoid height.
  The provenance names the conversion, the reaches, the dates and the stretches the DEM filled. The `sword` source is
  now v17c on Zenodo, which a web page can read.
- **Proofed checks for the channel Rules (spec 0007, T7).** New `crates/owd-core/tests/rules_proofs.rs`: property tests
  on random vessels and radii (the width at a bend never rises as the radius rises; every width is above zero; the
  `capped-widening` widening never exceeds L; quality A ≥ B ≥ C on every quantity; adding a set never makes a design
  basis value smaller), boundary cases for every evaluated Rule of `pianc-wg141` and `gb50139` built from the Rule's
  own data, and the WG 141 Middle Rhine worked example (5.3.3, p104) as a golden case with its known divergence: the
  example's totals add up against 3.0 B, not the 3.2 B it quotes. No result moves.

- **Edge fit of the banks on Sentinel-2 (spec 0012, T4).** `owd reach ... --banks --edge-fit` then moves the mask
  banks onto the water edge seen in a dry-season, low-cloud Sentinel-2 L2A scene, found in the Earth Search STAC
  (the two years up to the study date, the dry-season months for its latitude, 20 % cloud or less: `--max-cloud`,
  `--stac`). Only small windows of five bands are read, by range request, from the scene's Cloud Optimized
  GeoTIFFs (DEFLATE and 16-bit pixels are now read too). Along each bank normal (±40 m) a colour model learnt
  from the mask's own water and land scores every pixel, and a Viterbi path picks the edge: at most 30 m from
  the mask bank and at most 1.5 m change per 5 m. Where the scene shows no clear edge (cloud, shadow, muddy water)
  the mask bank is kept and the stretch is flagged `no-edge`; where the water runs on past 30 m the bank moves by
  30 m and the stretch is flagged `at-shift-limit`; a fitted width under 3 pixels (30 m) is flagged `narrow`. The
  inputs record the scene id and date, "Contains modified Copernicus Sentinel data [year]" and the JRC notice of
  the seed, and name the limits provisional. On the recorded Sassandra estuary window the mask's 75 % line lies
  inside the dry-season water, and most bank stations move out by metres.
- **PIANC WG 141 sizing on the shared rules engine (spec 0007, T1).** OWD now pins `arc-rules` (the rules engine
  shared with open-vessel-concept-drawings) at its tag `v0.2.4`. The WG 141 coefficients are a Rule Set file,
  `pianc-wg141` version 0.3.0: twelve Rules (bottom width, minimum radius, design depth and bend widening for
  qualities A, B and C), each citing its WG 141 clause or table and page, origin `guideline`, all `unverified`.
  The set loads through the same loader a user's file takes. `channel_shape` with PIANC-A, PIANC-B or PIANC-C now
  sizes through the engine and gives the same numbers bit for bit. New: `owd_core::rules::engine`, and
  `docs/rules/codification.md` (how a Rule is written, cited and verified).
- **GB 50139 sizing on the rules engine (spec 0007, T2).** New Rule Set `gb50139` version 0.3.0: GB 50139 as WG 141
  Appendix A.3 summarises it, origin `national-practice`, every Rule `unverified`, with the `range_end` input
  (`permissive`, `strict`). New formula shapes `gb-straight-width`, `gb-widening` and `plus-clearance`; the A.3.2 swept
  path is a `note` Rule. `channel_shape` with CN or CN-conservative now sizes through the engine and gives the same
  numbers and warnings bit for bit. The range end changes only the class IV-and-up depth and the swept-path note
  (finding #250).
- **Rule Set selection in the document; format version 3 (spec 0007, T3).** The sizing settings now store the
  selected Rule Sets (id, version, content hash, and `default` or `typed`) and the set inputs, instead of `ruleset_id`.
  A new design selects `pianc-wg141` with `se_quality` C, marked `default`. The `owd` set (OWD's own checks) is always
  selected. Two sets at once: every Rule applies and the strictest governs each quantity. **A version 1 or 2 file is
  refused** with a message to rebuild the design; there is no migration. The examples are rebuilt as version 3 with the
  same numbers. A changed Rule Set (another hash) makes the sizing stage and every later stage stale. New CLI:
  `owd size --rule-set ID --set-input NAME=VALUE --rule-set-file FILE`, `owd rules`, `owd rules show ID`,
  `owd rules --measures`, and `owd run --rule-set-file`. `owd size --ruleset PIANC-B` still works and maps to its set.
  The warnings of the sizing now come from the Rule Trace of the sized channel (the same text as before).
- **Reaches of the river and per-reach Rules (spec 0007, T4).** `owd waterway add|list|remove` marks a reach of the
  river by chainage on its centreline as `canal`, `impounded-river` or `free-flowing-river`, with an optional
  reduced-radius claim (canal reaches only). Chainage not marked is a free-flowing river with no claim. The radius
  Rules now test each station (`station.radius`), and a station takes the inputs of its reach through the engine's
  object-scoped inputs. `gb50139` gains the reduced radius (2L pushed convoy, 3L motor vessel) as Rules that apply
  only on a canal reach with the claim. The alignment takes one R_min: the largest governing R_min of any reach. One
  claimed canal reach over the whole line with `gb50139` strict gives the old CN-reduced result bit for bit. The
  command is `owd waterway`, not `owd reach` as spec 0007 says, because `owd reach` already starts a study from a
  river reach (spec 0012).
- **Screening outputs: verdict, design basis, constraint stretches, `owd trace` (spec 0007, T5).** The sizing
  results now hold the **design basis**: each quantity (bottom width, minimum radius per waterway kind, design depth,
  widening at R_min) with its governing Rule, sets, citation, origin, status and the other Rules of its Requirement.
  The volumes stage now stores, for each design line option, a **verdict** (`navigable-as-is`, `navigable-with-works`
  or `not-navigable`, with the blocking Rules not evaluated and the screening notice) and its **constraint
  stretches**: runs of stations where the river's own width, depth or centreline radius falls short of a governing
  limit, with the governing Rule, the required and found values at the worst station, the deficit, the works
  (`bank-cut`, `dredge`, `cut-through`) and the dredge and dry cut inside. Spec 0005's infeasible stretches join them
  as `infeasible`. `owd trace FILE [--option ID] [--json]` rebuilds the full Rule Trace from the document and refuses a
  stored set hash the engine does not hold; it is never stored. `owd export --csv --stretches` and
  `owd export --geojson --stretches` export the stretches with the notice; the CSV head holds the design basis and the
  verdicts. No number of an earlier stage moves.

- **A DEM for a reach (spec 0012, T5).** `owd fetch-dem DOC --bbox WEST,SOUTH,EAST,NORTH` fetches the Copernicus
  GLO-30 tiles for a box, and a study started with `owd reach` that has no banks yet fetches for its reach. The
  DEM is marked `default` in the document, with the Copernicus notice. A tile the source does not hold (much of the
  sea) is now named, with every other missing tile, and nothing is changed; before, the first missing tile stopped
  the run. For the web, `owd-reach` lists the tiles as plain download links and makes one DEM from the files the
  user drops back in (the bucket sends no CORS header, and there is no proxy); the result equals the native fetch.
- **Bank-line editor, desktop and web (spec 0012, T11).** "Edit banks" on the plan map: move, add and delete
  vertices, or redraw one stretch (it replaces the bank between the two nearest vertices). A bank always stays
  one line: an edit that would cross the other bank, or the bank itself, is refused with a message and changes
  nothing. Each edit can be undone. Bank lines you load are marked `user`; an edited line, and the river
  polygon and centreline the session rebuilds from the two banks, are marked `edited`. Every later stage goes
  stale and nothing reruns by itself. New in `owd-format`: `Session::edit_bank`; `AppState::load_inputs` now takes
  the date for the provenance record.
- **Banks from the JRC water mask (spec 0012, T3).** `owd reach ... --banks` also makes the left and right bank,
  the river polygon and the centreline from the JRC Global Surface Water occurrence layer, in the study's UTM
  zone, marked `default` with the JRC notice. OWD reads only the few tiles of the 10 degree JRC file it needs,
  by range request (or the tile files you give with `--jrc-tile`, which is how the web build reads them). Water
  is occurrence of 75 % or more, and a stretch under 3 mask pixels (90 m) wide, or with no water, is recorded in
  the document's new `low_confidence_width` for you to check. Both numbers are provisional defaults, named so in
  the document (`--mask-threshold` changes the first). The 75 % mask misses narrow, forest-shaded reaches: the
  Sassandra above its estuary shows as thin or absent, and `owd reach` then says so and suggests a lower
  threshold or your own bank lines.
- **`owd reach` (spec 0012, T2).** `owd reach FILE --river NAME --start LON,LAT --end LON,LAT [--via LON,LAT]`
  or `owd reach FILE --trace LINE.geojson` starts a new study from a river reach. OWD reads the OSM waterways
  around the points through Overpass and finds the shortest route along them. The route crosses from one river
  to another, and a via point picks the branch at a split. A trace snaps to the waterways it follows; a stretch
  with no waterway near it stays as drawn and is flagged. The document keeps only the OSM way ids and your own
  points (no OSM geometry), and carries the OpenStreetMap notice. New optional `reach` field in the document.
- **Start from a river: the `owd-reach` crate (spec 0012, T1).** New pure-Rust crate for inputs made from open
  data. It holds the `Fetch` interface (one place for network access: native HTTP on the CLI, the browser's
  `fetch` on the web) and a source registry with each source's licence, notice and whether a browser can read
  it. A document can now record a `provenance` for each input (source, date, notice, mark `default`, `edited`
  or `user`); GeoJSON and CSV exports carry the notices. A document without provenance gives the same results
  and the same exports as before.
- **Engineering drawings through the Open Technical Drawer (spec 0004, ticket 1).** New `owd draw FILE --out DIR`
  draws the cross-section gallery of a design line option: sections every 500 m and at the middle of each tight
  bend, one scale for all, on as many A3 sheets as needed, with the ground, the design cut face, the water level,
  the dredge and the dry cut hatched, and their areas. Each sheet has the standard's ISO 7200 title block, the
  drawing number `OWD-<design>-XS-<nnn>`, the status `concept` and the line "Screening — not for construction".
  For each sheet it writes the drawing document, the normalised content, SVG, PDF and DXF. `owd draw FILE
  --document` prints the drawing documents; `owd draw run FILE` reads one with the drawing crate. `owd.draw(...)`
  in Python gives the same files, byte for byte. Drawing is done by [`otd`](https://github.com/African-River-Corridors/open-technical-drawer)
  (the Drawing Format Standard in Rust, pinned by tag `v0.1.0`), and OWD checks that every drawn value equals the
  design's result: a sheet with a wrong number fails the command and names the sheet and the check. CI compares
  `otd`'s normalised content with the standard's Python reader for OWD's own drawing documents.
  **Removed:** `owd.drawings` and the `[drawings]` extra (they drew through an API that the Drawing Format Standard
  dropped in 1.0). Use `owd.draw` or `owd draw`.
- **Sheet style (spec 0004, ticket 2).** What a sheet shows is now data: a JSON sheet style with a JSON Schema
  (`schema/owd-style.schema.json`) and a default built into every binary (`owd-default.style.json`): sheet size,
  scales to try, optional layers, the section spacing and the extra-section rules, and the layout of a panel.
  `owd draw --style FILE` lays the members of a file over the default; `--section-spacing`, `--sheet-size` and
  `--scale` override single values (`owd.draw(style=..., overrides=...)` in Python). An invalid style is refused
  with the field named. The SHA-256 of the style after the overrides is printed on every sheet.
- **Setting-out table (spec 0004, ticket 4).** `owd draw` now also draws the setting-out table (kind `SO`) and
  writes it as one CSV: a row per station at the style's spacing (`setting_out.spacing_m`, 100 m by default) with
  chainage, easting, northing, bearing and the local radius on the radius-check chord, and a row per bend with start
  and end chainage, least radius and deflection. Coordinates are in the design's UTM zone to 0.001 m, and the zone
  and datum are on every sheet. The design line is a polyline, so there are no intersection-point rows. A long table
  splits over sheets.
- **Typical section (spec 0004, ticket 3).** `owd draw` now also draws the typical section (kind `TS`): the channel
  in section with its bottom width, design depth and side slope, the design water level, the design vessel for scale,
  and a detail that dimensions the under-keel clearance at a scale where it can be read. Each value has a note that
  names its source: the ruleset for the bottom width, depth and clearance, the volumes stage for the side slope, the
  water profile for the level. Levels are relative to the channel invert. `--sheets xs,ts` picks the kinds.
- **Bed model from a depth profile (spec 0012, T7).** `owd bed` takes the depth of a river that has no survey:
  one depth (`--depth`), or a table of depths along the river centreline (`--table`), and a season label.
  The volumes then read the bed inside the water as the water level less the depth, shaped as a parabola
  across the water (deepest at the centreline, level with the water at the banks). Outside the water the DEM
  is read as before. The depth is assumed, not measured: the volumes, the quantity table, the sections and
  the CSV and GeoJSON exports all say so. A survey replaces the bed model (`owd bed --clear`) and nothing else
  in the document changes. The pool search (`optimise`) still reads the DEM. Documents without a bed model
  give the same results.
- **`owd fits` (spec 0012, T8).** For a river with banks, `owd fits` says which vessel classes fit as it is.
  It checks every vessel class, in size order, under each ruleset (`--ruleset`, default PIANC-B): the water
  against the bend-widened bottom width, and the river's radius against R_min. Each class that does not fit
  lists its stretches, with what the rule needs, what the river has and the shortfall. The largest class that
  fits is named per ruleset. Every output says "width and radius only, no depth". The answer is stored in the
  document's new `fits` part. It shows as stale when the banks or the centreline change, in `owd status` too,
  and it never reruns by itself (`owd run` leaves it). Documents without a `fits` part give the same results.
- **Web app (spec 0011).** The desktop app's front end now also runs in a browser, over a WebAssembly build
  of the same Rust core, at `africanrivercorridors.com/owd/app`. Nothing you load is uploaded. It opens a
  bundled example, or `.owd` files, GeoJSON or GeoPackage banks and a GeoTIFF DEM from your computer; Save and
  Export download. A run that you cancel is dropped and the design is as it was. Fetching a DEM is not offered
  in the browser. The desktop app is unchanged. Results equal the native ones: `tools/web_parity.mjs` compares
  the wasm and native documents by the ADR 0005 tolerance in CI. New crates: `owd-shell` (the app state, moved
  out of the Tauri crate) and `owd-web` (the wasm session); `proj4rs` now builds without its C geodesic code
  (OWD uses UTM only), and the pool search timer uses `web-time` on wasm.
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

### Fixed

- The fairway fit's corridor QP is scaled before it is solved. On a seed with a sharp corner at the default 10 m
  spacing, the interior-point solver stalled on pass 1 (InsufficientProgress) and the fit returned the lightly
  smoothed seed (a 90 deg dog-leg in 800 m water: R 84 m, `radius_ok` false). Scaled, the same case reads 735 m.
- **The water-aware rolling circle is fast on a long river.** On the 140 km Sassandra reach (`pianc-wg141` B, `CEMT_IV`), `owd align --method rolling-circle` took 45 minutes in a release build of main and now takes 22 seconds, with the same document to the last byte. Four quadratic or whole-line steps are gone: the splice rebuilt the line's end by inserting each vertex at the front; GEOS's buffer depth rule (`geom::depth::region`, used by every navigable-water buffer) counted each piece's ray against every other piece; the roll read every chainage, and every circle contact, by a scan of the whole line; and the tangent entry and exit summed the line's chainages again for every sample. Each is replaced by an exact index or by reusing what was already computed, and each has a test that compares it with the old scan.
- **Long section: the axes end at the data, and the DXF model windows no longer overlap.** The level axis runs from the grid line just below the lowest level to the one just above the highest, and the chainage axis from the sheet's first station to its last, never past the line's end. The key viewport's model window now sits apart from the long section's, so `owd draw --sheets ls` no longer warns under clause 15.4.9.
- **Start from a river keeps its steps and route.** After Fetch DEM, which reads the study again from its file, or after opening a saved study, the panel dropped back to step 1 and lost the route. A reloaded study now keeps what the panel held; an opened one draws its route again from OpenStreetMap (from the river's OSM id, or from the user's own points), and shows its steps and inputs from the document. The document never holds the route's geometry, as before. (#318)
- **`owd reach` fails on broken banks (#360).** It gave `"ok": true` when the banks or the centreline were many times the route's length, or when the banks crossed. It now checks the banks it writes (the edge-fitted ones when there are any): each bank and the centreline must be from 0.5 to 1.5 times the route's length (`owd_reach::sanity`, provisional screening limits), and the two banks must not cross. A failed check is a `warning:` line, `"ok": false` with a `warnings` list in the JSON, and exit code 1; the study is still written, to check by eye with `owd qa-overlay`.
- **No feasible line is no longer a design (#366).** When every bend of the water-aware rolling circle was infeasible, `owd align --method rolling-circle` kept the fallback line with `radius_ok: true`, and `owd volumes` costed it as a design. The option now has a `feasible` check (`ok: false`, note `no feasible line`), which every export, the table and the report list as the failed check `no_feasible_line`. Its screening verdict is `not-navigable`. The volumes stage keeps its numbers but flags the row (`no_feasible_line: true`, note `no feasible line`), `owd volumes` marks it `FAILS no_feasible_line: not a design` and exits 1, the report shows "no feasible line" in place of its quantities, and `owd align` says so and exits 1. An infeasible bend now always leaves a stretch on the final line, even when the final line's own checks find none there.
- **A failed option's summary says it failed (#367).** `owd align`, `owd optimise` and `owd volumes` wrote `"ok": true` in their JSON summary even when an option failed a check (a re-route that could not hold R_min, for one); only the exit code said otherwise. The JSON now gives `"ok": false` and the `failed_checks` (`owd volumes` gives `"ok": false` when any option fails), and the words say "the option is saved, marked failed" with the checks. The re-route still does not always hold R_min: its option is kept, marked failed, as before.
- **The README shows how to load your own DEM and a fixed water level (#370).** A new section, "Your own river, DEM and water level", gives the route: `owd new`, then `owd import --dem … --water-level …` with the river (there is no `owd new --dem`), then `owd bed` and `owd fits`. A CLI test runs the same commands.
- **A native request that never answers no longer hangs the reach (#372).** The shared HTTP agent (#375) now has time limits (`reach_fetch::LIMITS`, provisional): 30 s to connect, 300 s to the response head, and 30 min for the body. They are long enough for the largest honest answer: the Overpass queries ask the server for at most 120 s, and a whole JRC tile at 0.5 MB/s takes under 15 min. A timeout is a failed fetch that names the host ("… timed out"), so the Overpass search asks once more and then moves to its next mirror, as it does on a 502. Before, a mirror that accepted the connection and never answered held `owd reach` for hours. The desktop uses the same agent.
- **A whole river's centreline no longer tangles (#373).** On the whole Sassandra run the centreline came out 178.7 km for a 144 km route: 6.7 km of it lay in a 600 m box with 93 self-crossings. Two faults, both now fixed. A bank had gaps, straight chords of 1.75 to 11 km across a bend (and past the other bank), and the mid-line moved its stations halfway to them. A bank segment far longer than the rest (20 times the bank's median segment, and at least 1.5 km) is now a gap, not a bank: a normal that meets only a gap meets no bank, so the checks flag and repair that station and the mid-line keeps it in place. And the mid-line could loop at tight bends and poor stretches. Its loops are now cut, as the banks' are (up to 2000 stations). Offline, on the recorded whole-river banks (every 4th vertex), the centreline is 1.05 times its 134 km seed; without the two fixes, 1.67.
- **One HTTP agent for every native request (#375).** `owd reach`, `owd qa-overlay`, `owd fetch-dem` and the desktop sent each request through `ureq::get`, which makes a new agent each time, so no connection was reused. A long reach opened a new TCP and TLS connection for every range read, and could run out of local ports (`os error 49`), sooner with two reaches at once. Every native request now goes through one agent for the process (`reach_fetch::agent`), so reads to one host reuse a pooled connection: 50 range reads to a local server now make at most 2 connections, not 50.
- **The re-route opens a bend tighter than R_min (#381).** On a baseline tighter than R_min (a meandering river centreline), the re-route's Laplacian passes on the offsets could not reach R_min: their fixed point is a constant offset, which is the baseline's own shape. Every corridor of the sweep then failed, and the option was kept marked failed (#377). Where those passes fall short, a second stage now smooths the built line itself: each station moves along its normal, within the corridor and short of its centre of curvature (`FOLD_FRACTION` 0.9), with the ends tapered by the room they may use. It stops once R_min holds. A line that already holds R_min is not touched, and the second stage is dropped if it would cut more of a protected bank. On a 3 km sine meander whose tightest bend is 285 m, R_min 600 m now holds in a 200 m corridor (it read 299 m); a 20 m corridor still fails, marked failed.
- **Desktop smoke test for the river search.** The test cleared the search box with empty text, which the desktop WebDriver refuses, and clicked "Start from a river" twice, which closed the view for the next test. It now clears the box with an input event and opens the view only when it is closed. (#308)
- **Desktop smoke test for progress.** The test clicked "Give a JRC tile" in the moment between the route's result arriving and the panel enabling its buttons again, so the click was lost. It now waits for the status block to go first. (#320)
- **Desktop smoke test for the JRC download.** The progress test gave the JRC tile by hand, and a tile given by hand stays given for the next study, so the next test found it given before its download. The progress test now gets the tile by download too. (#321)
- **Web smoke test: the report engine's hosts.** The bundle's host scan now knows the hosts the report engine names in its bundled data (citation styles, syntax definitions, XML vocabularies), kept as SHA-256 hashes because some are personal sites. They are text: the engine opens no network. A new host still fails the scan.

### Added

- `owd qa-overlay DOC --png FILE [--kmz FILE] [--from-km A --to-km B] [--scene ID]` (spec 0013 T5): the
  bank lines, the centreline and the low-confidence stretches over the Sentinel-2 true-colour image of
  the scenes the banks came from, with a legend of the scenes and notices, and a KMZ for Google Earth
  (the image placed by its four corners, the lines as KML). Up to 30 km across at a time; small windows
  are drawn at up to 4 picture pixels per 10 m. The drawing is `owd_reach::overlay` (pure, so the app
  can use it too, #295).
- The Sentinel-2 water grid (`owd_reach::water`, spec 0013 T2, ADR 0011): water and land from a
  scene's MNDWI (water at 0.0 or more, provisional), with the scene classification masking cloud,
  shadow, cirrus and no data; scenes ranked by the share of the river corridor they see clear, up to
  three filling each other's gaps, and the JRC mask filling what none sees, each pixel saying where it
  came from. `sentinel2::read_clear` reads a scene's classification alone to rank it; `read_blue` and
  `Scene::blue` give the blue band for the true-colour picture. Nothing uses the grid yet (T3 does).
- `owd.alignment.fairway.fairway_fit`: the Python wrapper of the corridor solve (poses, must-follow
  segments, any `FitParams` field by name).
- No-cut zones on the least-cost route: `optimise_route(..., no_cut=[NoCutZone(from_m, to_m, bank)],
  bank_line=river_centreline)` and the same on `build_cost_grid`. No dry ground is dug on that bank over
  that chainage (a town, a landholding, a permit that will not come); dredging is still allowed. The bank
  is read on `bank_line` (default: the baseline), so a re-route across the inside of a bend is judged
  against the river, not against itself. The grid gains `protected` (the dry cut on a protected bank per
  cell), the search refuses those cells and skips a corridor with no clear path, and the result reports
  `protected_cut_m3` and `no_cut_ok`. With no zones every call, grid and result is unchanged.
- `owd reach --seed s2|jrc` (default `s2`) and `owd_reach::seed` (`banks_from_s2`, `record_s2_banks`): the provenance of seeded banks names each part's scenes with the share of the corridor each saw clear and its water split, the JRC agreement, and the provisional values. `--max-cloud` and `--stac` now apply to the seed as well as the edge fit. `tools/make_s2_window_fixture.py` records the Sassandra km 148 to 152 window (`examples/reach/s2win_*`).
- **The QA overlay in the app (#295, spec 0013 T7).** "Start from a river" has a "Check the banks on the image…" button once the banks are made, with an optional stretch (from and to km along the centreline). It draws the banks, the centreline and the low-confidence stretches over the Sentinel-2 true colour of the scenes the banks came from and saves a PNG with its legend and a KMZ for Google Earth: to a folder on the desktop, as two downloads on the web (drawn in the worker, the scenes read from their COGs). It is the same drawing as `owd qa-overlay` (`owd_reach::overlay::render_document`), so the app and the CLI give the same bytes. The recorded lower-Sassandra scene gains its blue band (`examples/reach/s2_sassandra_B02.tif`, `tools/make_s2_blue_fixture.py`).
- **River search with a list to choose from.** `owd reach --search TEXT [--country CC]` lists the rivers and canals whose name starts with or matches the text, each as "river — region, country" with its OSM id, so two rivers with the same name stay apart. The list comes from Photon (OSM data, built for search as you type; the Nominatim policy forbids type-ahead). Spec 0012 gains T13 to T15. (#306)
- **A whole river from a search hit.** `owd reach FILE --osm-id relation/N` (or `way/N`, as `--search` lists it) reads the river's main stem from Overpass, finds its mouth (the downstream end) and its upstream end, and routes between them. `--start`, `--end` and `--via` cut a reach out of the same river. The route's provenance records the OSM id. (#307)
- **App: find a river as you type.** In "Start from a river" (desktop and web), typing 3 letters lists the matching rivers and canals as "river — region, country", with an optional country filter and the OSM attribution. Choosing one draws the whole river, sets Start at its mouth and End at its upstream end, and shows its length. Drag Start or End on the map, or pick them, to cut a reach out of the same river with no new request. If the search (Photon) fails, the app says so and Enter asks Nominatim once (`owd reach --search TEXT --nominatim` on the CLI). The search server is a setting (`OWD_PHOTON_URL`, `?photon=URL` on the web). (#308)
- **Overpass mirrors.** When overpass-api.de fails (an HTTP error such as 502 or 429, a timeout, or a busy page instead of JSON), OWD asks it once more, then the public mirrors overpass.kumi.systems and overpass.private.coffee in order. `owd reach` says which server failed and which answered, and the route's provenance records the server used. The server list is a setting: `--overpass URL,URL` or `OWD_OVERPASS_URLS`. (#319)
- **Progress for long steps.** Finding the route, reading a river, making the inputs, the water level and the DEM fetch now show a status line in the app: the step, a progress bar, what it is doing now (for example "reading Sentinel-2 imagery (request 37)", "SWOT levels for SWORD reach 3 of 12", "fetching DEM tile 5 of 18"), the time it has taken, and Cancel. Cancel stops the work before its next request and leaves the study as it was. The CLI prints the same messages on stderr. (#320)
- **The desktop fetches the JRC water-mask tiles itself.** Make the inputs downloads any JRC occurrence tile the reach needs into the study's folder (`jrc/`), with its licence notice beside it and the download's progress on the status line, then reads it from there. Give a JRC tile stays for offline use; the web app reads the tiles from the network as before. (#321)
- **Each study saves itself.** On the desktop, a new study from a river is saved at once in its own folder in the projects folder (`~/Documents/Open Waterway Designer/`, or `OWD_PROJECTS_DIR`), named for the minute it started and its river, in local time: for example `2026-10-22 1435 Rhine`, with a suffix when the name is taken. Cutting the reach again out of the same river keeps the folder. The document and every download go inside it, each reach step saves it, and Fetch DEM never asks to save first. Save as still works; the web app keeps its browser storage. (#322)
- **`owd report`: the screening report (spec 0014).** A new pure crate, `owd-report`, makes the screening report of a design as PDF, HTML and Word through the Open Report Designer v0.1.0 (pinned by tag): document control, the screening notice on every page, a "Data and limits" box, the data sources, the verdicts, the constraint stretches, the design basis and the works. Every number is read from the design.
- **Screening report: figures and the rule-trace appendix (spec 0014).** The key map (the plan overview sheet) and the long section, each the SVG `owd draw` makes for its sheet, and Appendix A with the Rule Trace: the channel and reach rows in full, and for the chosen option every station row that does not pass, the passing rows counted per Rule, and each trace's hash.
- **`owd report --kind quantities`: the quantity schedule (spec 0014).** For each design line option: the volumes stage's totals and the tallest cut face with its chainage, and the dredge, dry cut and tallest face inside each constraint stretch, as PDF, HTML and Word.
- **`owd report --kind sections`: the section pack (spec 0014).** The typical section, every cross-section gallery sheet (its drawing number in the caption) and the setting-out table with its bends, taken from the sheets and the setting-out CSV `owd draw` makes, as PDF, HTML and Word.
- **`owd report --kind run`: the run report, and `--kind all` (spec 0014).** For testers and developers: what asked for the report, the versions (OWD, and the git tags of ORD, otd and arc-rules, each Rule Set with its hash), each stage with its status, engine, dependency hash and settings, each input's hash, the DEM's recorded and given hashes, and the hashes of the document, the rule traces and every sheet. `--kind all --format all` writes the four reports in three formats.
- **App: the Report button (spec 0014).** In the Sheets view, on the desktop and the web: pick the kind (or all four), the format (PDF, HTML, Word or all three), the revision, the status and who prepared it. The reports come from the same builder as `owd report`, through the shell, so the files have the same bytes. The web build's WebAssembly grows from 7.0 MB to 47.1 MB (2.4 MB to 14.4 MB gzipped) with the report engine and its fonts.
- **Reports in the docs and the Sassandra example (spec 0014).** A Reports page (`docs/reports.md`, on the website), a Reports section in the README, and step 7 of `examples/sassandra/run.sh`, which writes the four reports of the study as PDF, HTML and Word.
- **Spec 0015: custom vessel.** A study can hold its own vessel (length, beam, draught, formation), sized and reported like a catalogue class; every class in the shared catalogue becomes pickable; the file format becomes 0.4.
- **The full vessel catalogue.** OWD moves to `ovcd-core` v0.2.4, the release `arc-rules` already comes from, so it links one OVCD version. Every class of the shared catalogue with a length, a beam and a draught can be picked in `owd size --vessel` and the app (for example `CEMT-VIb-motor`, 140 × 15 m). The app's picker groups the classes by family and has a search box. A class that cannot size a channel is refused with a message that names what it lacks. `owd fits` checks the same 12 classes as before by default; `owd fits --all` checks every class. Studies that hold an old id (`CEMT_Va`) open with their stages as they were and are saved with the catalogue id (`CEMT-Va`). Under GB 50139 alone, a class with no GB class, or a coupled unit or lone barge, is refused in words. (#356)
- **A custom vessel (#357, spec 0015 T1).** `owd size DOC --custom-vessel NAME --loa .. --beam .. --draught .. --formation motor|convoy` sizes the channel for a vessel of your own, with `--abreast` and `--in-line` for a convoy and `--gb-class`, `--air-draught` and `--deadweight` as options. The study keeps it in `stages.sizing.settings.custom_vessel`, with `vessel_standard_id` null; later commands read it from there, and `--vessel ID` replaces it. A zero, negative or non-numeric dimension, a beam not less than the length, an empty name or a convoy with no barge counts is refused, naming the field; a vessel outside every catalogue class, or with a length-to-beam ratio below 2.5 or above 15, is a warning. GB 50139 reads the typed GB class (without one, its width and depth are "not evaluated"), and a custom vessel is always a special design vessel (Appendix A). Every output labels it `Custom: <name>, L × B × T`: the `size` and `fits` output, the rule trace, the typical section, the reports' design basis, the reports' Data and limits box (with any warning) and the exports. `owd fits` adds it as one more row after the classes.
- **Custom vessel… in the app (#358, spec 0015 T2).** The vessel picker's last entry, **Custom vessel…**, shows the custom vessel's fields, filled from the class that was selected: its name with " (custom)", its length, beam, draught and formation. Barges abreast and in line show for a convoy, the GB 50139 class only when GB 50139 is selected, and the air draught and deadweight under "More". A refused value shows next to its field and the Size button stays off until it is fixed; a warning shows under the form and does not block. The channel shape names the vessel by its label. The desktop and the web share one path through the shell (`AppState::set_custom_vessel`, `check_custom_vessel`). In "What fits", the custom vessel's row shows its label, and "Use this vessel" works for it too.
- **Spec 0016: the largest vessel the river allows.** A smooth reference line that belongs to no vessel, and `owd envelope`, a sweep of single hulls for the largest vessel per reach and per kilometre.
- **Overpass mirrors in the app.** The panel says which Overpass server failed and which answered, under the route's length. The server list is a setting on the desktop (`OWD_OVERPASS_URLS`) and on the web page (`?overpass=URL,URL`). (#319)
- **Spec 0014: reports.** The plan for four reports (screening, quantity schedule, section pack, run report) as PDF, HTML and Word through the Open Report Designer v0.1.0, from `owd report` and an app button; ADR 0012 puts them in their own crate.

### Design impact

Every pull request is checked for design impact (`tools/design_impact.py`, CI job `design-impact`): fixed
scenarios run on the base branch and on the pull request, and a result that moves by more than 1e-9
(ADR 0005) must have a bullet here, under this heading, saying what moved and why. Add the bullet in the
pull request that moves the result.

- **Generated banks are checked and repaired, and the centreline is their mid-line (#285, spec 0013
  T1).** `owd reach --banks` and the edge fit now run the bank checks (each bank on its own side, no
  outward jump past 2 station steps, a width within 0.5 to 2 times the running median of its neighbours,
  no crossing) and store the centreline as the mid-line of the final banks. The bank editor rebuilds the
  centreline by the same mid-line (it paired the banks by their share of length, which drifts where one
  bank is longer). What moved: every generated bank line where a check failed (cut back to its
  neighbours and flagged as a `repaired` low-confidence stretch, with the checks that failed), every
  generated and every edited centreline, and the widths and stretch chainages read on them. On the
  recorded lower-Sassandra window the edge fit's summed shift goes from 25,709 m to 20,358 m and its
  bank stations from (410, 536) to (308, 434), because the mask banks and the centreline it starts from
  are now checked and rebuilt; the window's lagoon mouth is cut as an outward excursion. A narrowing is
  never widened: a sudden narrowing is flagged (`width-narrow`) and kept. The design-impact scenarios do
  not use generated banks or the editor, so they show no change. New: `owd check-banks DOC` runs the
  checks on any document's banks and changes nothing.

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
- **Rolling-circle tangent exits (issue #5).** What moved: every rolled line (rolling circle, with or without
  water), and so everything built on one. Why: the crossing exit left a kink at each arc exit that read as a
  much tighter radius on a short chord. Which way, in the quickstart (R_min 255 m): the rolled line's measured
  minimum radius rises from 253.2 to 254.4 m, it gets one more arc (17, was 16) and 2,514 vertices (was 702, from
  the arc's vertex every metre), and it is 0.11 m shorter. The re-route, which starts from the rolled line, saves
  2.440 % (was 2.444 %) and its minimum radius rises from 254.0 to 254.8 m. Volumes move by less than 0.05 %:
  least-cost dredge 512,936 m3 (was 513,163), rolling-circle total 1,504,641 m3 (was 1,504,620).
- **Rolling-circle tangent entries (#283).** What moved: every rolled line again, and so everything built on one.
  Why: each arc now starts at a vertex, tangent to the segment into it, not at the resample point tangent to the
  resampled direction; and a later-pass circle with no tangent exit is not spliced. Which way, in the quickstart
  (R_min 255 m): the rolled line's measured minimum radius rises from 254.37 to 254.97 m, it has 16 arcs (was 17)
  and 2,625 vertices (was 2,514), and it is 0.17 m longer. The re-route saves 2.441 % (was 2.440 %); its minimum
  radius falls from 254.76 to 254.50 m, still within the 2 % reading tolerance. Volumes move by less than 0.15 %:
  rolling-circle total 1,506,028 m3 (was 1,504,641), least-cost total 1,289,766 m3 (was 1,289,958).
- **`gb50139` channels move to GB 50139-2014 (#250).** Every result sized under `gb50139` (the old `CN`, `CN-conservative` and `CN-reduced`) can move; PIANC results do not. Default draught, before → after:
  - **Width.** Typical vessels keep their Table 3.0.2-1 width, except CN_IV_motor, which now matches its row: 24.84 → 30 m. Special design vessels move from 2.3 × B to the A.0.2 formula. Permissive / strict: CN_I_motor 37.26 → 35.57 / 38.11 m; CN_II_motor 34.04 → 32.78 / 35.12 m; CN_III_motor 24.84 → 25.62 / 27.45 m; CEMT_IV 21.85 → 23.43 / 25.11 m; CEMT_Va 26.22 → 28.82 / 30.88 m; CEMT_Vb 26.22 → 31.62 / 33.73 m; CEMT_VIa 52.44 → 42.84 / 45.69 m (narrower); CEMT_VIa_motor_NL2011 39.10 → 40.43 / 43.32 m.
  - **Radius.** The table radius governs where it is larger: CN_IV_motor 270 → 330 m, CN_I_convoy_2ab 669 → 670 m, CN_II_convoy_2ab 558 → 560 m. `CN-reduced` loses the 2004 reduced radius (for example CN_III_motor 255 → 340 m) and now equals `CN-conservative`.
  - **Depth.** Permissive now takes the low end of each Table A.0.1 range, 0.1 m less for classes I–V (for example CN_I_convoy_2ab 4.0 → 3.9 m). Class IV strict drops from T + 0.5 m to T + 0.3 m (CN_IV_motor 2.1 → 1.9 m). CEMT vessels take their class by tonnage (CEMT_Va, Vb, VIa: 3.8 → 3.9 m permissive; CEMT_IV strict 3.0 → 2.9 m).
  - **Widening.** It follows the radius: CN_IV_motor 8.27 → 6.79 m at R_min, with small moves where R_min moved.
- **Bank lines are seeded from Sentinel-2 water by default (#287, spec 0013 T3, ADR 0011).** `owd reach --banks` and the app's "Make the inputs" trace the banks on the water of dry-season Sentinel-2 scenes: MNDWI with the 20 m SWIR sharpened to 10 m by the near-infrared, split per scene into water and land by Otsu's method over the river corridor (within -0.3 to +0.2), the scene classification masking cloud and shadow, up to three scenes per 5 km part ranked by how much of the corridor each sees clear, and the JRC mask filling what no scene sees (flagged `jrc-fallback`). What moved: every generated bank line, centreline, polygon and width, and the stretches on them. On the recorded Sassandra window of km 148 to 152 the median width goes from 112 m (the JRC mask alone at 30 %) to 199 m, against 219 m of open water measured on the scene. The bank checks of T1 change too, from what real reaches showed: the banks are read along the route moved onto the water by a running median of the casts' offsets; a side fault is now two banks out of order, not a bank on the wrong side of the reference (a reference off a narrow river widened it); an outward excursion over 250 m (a bay, a tributary mouth, a cut across an island) is cut; the checks run up to three rounds until one repairs nothing; a run of jumps that touches either end of the banks is kept; and a crossing left after the rounds is flagged `crossing` and named by the command instead of stopping the run. `--seed jrc` keeps the JRC seed. The design-impact scenarios do not use generated banks, so they show no change.
- **Smoothed joins in the edge fit (#288, spec 0013 T4).** Where a long reach is fitted in parts, the parts' shifts are blended over each overlap along half a cosine instead of a straight ramp, so the blend has no kink where a part begins or ends; the step limit still runs along the whole bank after the blend. A repair of the fitted banks (T1) that would change the bank by more than the fit's 1.5 m per 5 m station is widened, taking in good stations either side, until its straight line keeps to that limit, and the stations it took in are flagged with the repair. What moved: edge-fitted banks, centrelines and widths in part overlaps and around repairs after the fit. The mask banks and the Sentinel-2 seed (traced in one pass over all parts) are not affected. The design-impact scenarios do not use generated banks, so they show no change.
- **The bank checks on a whole river (#290, spec 0013 T6).** The first run of the Sassandra worked example on the Sentinel-2 seed showed three faults that a short window did not, now fixed. Each round of the checks read the banks along the last round's centreline, so a round that went wrong made the next one's reference worse: the edge-fitted centreline came out at 1,731 km for a 144 km river, with 26,000 `crossing` flags. Every round now reads along the original seed. A repair was spliced into the bank at the nearest vertex found over the whole bank, which on a long bank could match a vertex far along, broke the order and rebuilt the whole bank from its offsets; the search now walks forward from the last splice, and a run it cannot place is left as it is (and stays flagged). That search was also what made the run take 88 minutes. A mid-line station now moves at most about one river width in a pass (at least 200 m): where the banks of a tight bend looped or crossed, stations jumped to a bank a kilometre off and the centreline came out 2.5 times the river's length; and an offset bank's loops are now cut over 512 stations, not 64. And a bank that runs on past either end of the centreline (a cast that found other water at the first or last station) is trimmed there, beyond the river's width at that end, since no check can see it. What moved: generated banks, centrelines, widths and stretches wherever these happened, and the regenerated worked example. The design-impact scenarios do not use generated banks, so they show no change.
- **CEMT Va's default draught: 3.5 m → 2.65 m (shallower).** The shared vessel catalogue (`ovcd-core` v0.2.4) gives class Va a draught range of 2.5–2.8 m; v0.2.2 had 2.5–4.5 m. With no draught given, the design draught is the middle of the range, so a channel sized for CEMT Va with no draught override now has a smaller design depth (and anything that follows from it). A study that gives its own draught does not move, and the other 11 classes do not move: their figures are the same. The old `CEMT_VIa_motor_NL2011` is the catalogue's `RWS-M12`; its design length (135 m), beam and draught are unchanged. (#356)
- **An option with no feasible line has no volumes (#366).** What moved: a water-aware rolling-circle option whose every bend is infeasible now fails the new `no_feasible_line` check, its screening verdict is `not-navigable`, and its volumes row is flagged `no_feasible_line` (the numbers stay, so the app's table still fills; the report shows "no feasible line" in place of them). An infeasible bend whose fit stretch the final line's checks missed now shows that stretch, so such an option can gain `fits_in_water` stretches. Why: the fallback line breaks the rules least but is not a design; costing it unflagged showed a bad answer as a design. Options with at least one bend that fits do not move.
- **Generated centrelines and banks (#373).** What moved: where a bank generated by `owd reach` has a gap (a segment over 20 times its median and at least 1.5 km), the stations facing it are flagged and repaired, and the mid-line keeps them in place; where the mid-line looped, the loop is cut. So the banks, the centreline, the widths and the low-confidence stretches move there, and the centreline gets shorter. Why: a gap is not a bank, and a loop is not part of the river. Banks with no gap and a centreline with no loop do not move. The design-impact scenarios do not use generated banks, so they show no change.
- **Re-routes that failed R_min can now hold it (#381).** What moved: a re-route (or a lattice pool re-route) whose line ended below R_min after the passes on the offsets now runs a second stage on the line itself, so its line, offsets, smoothing passes, minimum radius, `radius_ok` and saving can change, towards a line that holds R_min within the corridor. Why: the first stage cannot open a bend tighter than R_min. Re-routes that already held R_min do not move.
- **The Sassandra example's answers moved with the spec 0013 banks (#373).** What moved, per class under PIANC-B: width stretches went from 194–649 to 49–368, and radius stretches on the fairway from about 150 to 3–25 (on the centreline, from about 1,000 to 572–837). The depth stretches did not move, apart from CEMT-Va, whose design depth is now 3.44 m. Every class is still `navigable-with-works`, and none fits as it is. Why: the banks are seeded from Sentinel-2 water and repaired (spec 0013), and the centreline no longer loops (#394): 150.7 km instead of 178.7 km. This is the example's documentation only. No code moved.
- **The full `fits` answer reads the river's radius on the fairway (ADR 0013).** What moved: in a full answer (DEM, bed model and water level), each class's radius stretches, `worst_radius_deficit_m`, `stretch_count`, `fits`, the verdict and `largest_fitting` can change, because the river's radius is now read on each class's fairway fit, not on the river centreline. Which way: fewer and shorter radius stretches where the centreline wiggles inside wide water, so a class may now fit as it is where it did not (on the whole Sassandra, CEMT-Va: 315 radius stretches against 1,058). Width and depth stretches do not move. A plan-only answer does not move, except that it now records `plan_line: "centreline"` and a note. A full answer also gains `plan_line` on each row, and `settings.plan_line` is always written. Why: the centreline is drawn from the bank lines, and each wiggle of it read as a tight bend that is not in the river. Answers already stored keep the centreline until `owd fits` runs again.

### Changed

- **`gb50139` now reads GB 50139-2014 itself (#250).** The set was WG 141 Appendix A.3's summary of the 2004 edition; it is now the 2014 standard, still version `0.3.0` (unreleased sets change in place) with a new content hash, so a design that selects it goes stale. A vessel whose class, length and beam match a row of Table 3.0.2-1 takes that row's single-line width and bend radius (all 20 rows). Any other vessel is a special design vessel (3.0.7) and takes the A.0.2 width, the swept path B + L sin β plus a clearance on each side, β by channel class. The radius is the larger of 3 L or 4 L (3.0.5(4)) and the table's. The depth is draught plus the Table A.0.1 clearance, with `range_end` choosing the end of each range. The 2004 reduced radius on a claimed canal is gone, so `CN-reduced` now equals `CN-conservative`. The 2.3 × B fallback (WG 141's own Table 5.1) and the 20% match are gone. The bend widening L² / (2R + B) stays, cited to WG 141 as a guideline, because 2014 gives no formula. The channel class comes from a Chinese vessel, or for a CEMT vessel by tonnage: the smallest GB class (Table 3.0.1) whose design tonnage covers the top of its CEMT range (ECMT Resolution 92/2); above 3000 t it is class I and a special design vessel. A new set input, `gb_class` (`I` to `VII`), overrides it, and the trace says which way the class was found. A vessel with no class and no tonnage gets no GB width or depth: its Rules show as blocking Rules not evaluated. A note says that GB 50139 expects a double-line channel. `width_basis` is now `table`, `appendix-a` or `rule`.
- **Long section: the `auto` vertical exaggeration is now the largest that fits.** It was the smallest allowed factor whose levels filled half the plot height. It is now the largest allowed whole-number factor whose vertical scale is a ratio of the standard and whose level axis fits within `auto_max_fill` (0.8) of the plot height. The style key `long_section.auto_min_fill` is now `long_section.auto_max_fill`, and the long section's default room above and below the levels is 0.5 m (was 3 m and 4 m). A fixed factor in the style or `--vertical-exaggeration N` still overrides `auto`.
- **No hint box over the map.** The box "Load bank lines and a river polygon, and run alignment…" covered the map and the route. It is gone; the empty Options panel now says what to do in one line. (#323)
- **Reports: the Data and limits box says more (spec 0014).** The "Reach" line gives the centreline's length beside the route's and how many times longer it is, so banks that wander show on the first page. The "Rule Sets" line names the sets the fits answer used and marks a sizing selection that has not run. `owd report --help` names all four reports. The Sassandra example's four reports are in `docs/examples/sassandra-reports/`.
- **File format 0.4 (#357, spec 0015).** `format_version` is now the string `"0.4"`. A file with the number `3` (format 0.3) opens, upgrades in memory and is saved as 0.4, with nothing else changed and its stages as fresh as they were. A format newer than 0.4 is refused with "made by a newer OWD". Until format 1.0, which waits for an external review, a format change raises the minor number. The worked example is regenerated (only its `format_version` moves).
- **The Sassandra worked example is regenerated on the spec 0013 seed (#373).** `docs/examples/sassandra.md` and its four report PDFs now come from a whole-river run of `examples/sassandra/run.sh` on main `7609e40` (2026-10-22 to 23). The centreline is 150.7 km for the 144 km route (1.05 times), and the page states the 23 `crossing` repairs and the 24 places where the banks still cross. `run.sh` now goes on when `owd reach` exits 1 with the `banks_cross` warning only (the study is written). Any other failure still stops it.
- **Changelog fragments (contributors).** A pull request no longer edits `CHANGELOG.md`. It adds one file,
  `changelog.d/<pr>.<kind>.md`, so two open pull requests never conflict over the changelog.
  `tools/changelog.py release <version>` moves the fragments and any `## Unreleased` entries under the new
  version at release time; `tools/changelog.py check` checks them. The design-impact check accepts a
  `changelog.d/<pr>.design-impact.md` fragment as well as a bullet under `## Unreleased` > `### Design impact`.
- **CI runs the slow non-required jobs only when their inputs change (contributors).** On a pull request,
  the app, web, bundle, wheel and website jobs run only when the files they build from change; main runs
  them as before. CodeQL can move to a push to main and a weekly run. A failed scheduled run opens an issue.
  The required checks `test` and `release-guard` still run on every pull request.
- **The required `test` check runs in parallel parts (contributors).** CI runs the gate as `gate-lint`,
  three `gate-rust` partitions and `gate-python` at once; the `test` check passes only when all of them
  pass. `tools/gates.sh` takes `GATES_STEPS` and `GATES_PARTITION` to run one part, and with neither it
  runs the whole gate, as before.
- **`owd fits` reads the river's radius on the fairway by default (ADR 0013).** The full answer now fits each class's fairway (ADR 0007) in the water and reads the river's radius on it, in the command line, the desktop app and the web app. `--plan-line centreline` keeps the old line. The answer says which line it read: `Plan line:` in the table, `plan_line` in the JSON (the answer and each row), and `plan_line_note` when it fell back to the centreline. A plan-only answer reads the centreline and says that the fairway runs only for the full answer; a class whose fairway fit cannot run reads the centreline, with the reason. The reports' "Data and limits" box shows the note too.
- **A smaller web build.** `tools/build_web.sh` builds with a new `wasm` profile (link-time optimisation in one unit, symbol names stripped): the WebAssembly goes from 47.1 MB to 35.5 MB (gzip 14.4 MB to 12.9 MB), and the design runs no slower in the browser. Native builds keep `release`.

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
