# Changelog

All notable changes to Open Waterway Designer (OWD), called `channel_designer` before 2026-10-21. The format follows [Keep a Changelog](https://keepachangelog.com/);
versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

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

### Added

- `owd.alignment.fairway.fairway_fit`: the Python wrapper of the corridor solve (poses, must-follow
  segments, any `FitParams` field by name).
- No-cut zones on the least-cost route: `optimise_route(..., no_cut=[NoCutZone(from_m, to_m, bank)],
  bank_line=river_centreline)` and the same on `build_cost_grid`. No dry ground is dug on that bank over
  that chainage (a town, a landholding, a permit that will not come); dredging is still allowed. The bank
  is read on `bank_line` (default: the baseline), so a re-route across the inside of a bend is judged
  against the river, not against itself. The grid gains `protected` (the dry cut on a protected bank per
  cell), the search refuses those cells and skips a corridor with no clear path, and the result reports
  `protected_cut_m3` and `no_cut_ok`. With no zones every call, grid and result is unchanged.

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
