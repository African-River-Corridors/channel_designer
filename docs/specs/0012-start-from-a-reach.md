# Spec 0012: Start a study from a river reach

Status: **approved by the maintainer, 2026-10-21.** Decisions: `docs/design/2026-10-21-reach-start-grill.md`
(Q1–Q18) and ADR 0010. Builds on spec 0003 (the app), 0005 (navigable water and the width check), 0007
(the verdict and constraint stretches) and 0011 (the web app).

## Problem statement

The first question a river screening asks is: *given this reach, what vessel fits it now, and what
would the next size up need?* OWD cannot answer it.

- Every input must come from the user: bank lines, a river polygon, a DEM, a water level and a vessel.
  A person with only a river name cannot start.
- OWD answers "what works does this vessel need?". The screening question is the inverse: "what fits
  the river as it is?". Spec 0007 gives one verdict per vessel and leaves a sweep over vessels out.
- Most rivers have no bathymetry. A DEM sees the water surface, not the bed. Today the depth checks
  need a bed that nobody has.

## Solution

A front door: **Start from a river.**

1. **Pick the reach.** Type a few letters of a river's name and choose it from a list ("river —
   region, country"). OWD draws the whole river, with Start at its mouth and End at its upstream end.
   Drag Start or End, or click on the map, to cut a reach; or trace it roughly on the map. All snap to
   the OSM waterway graph. The route may cross from one river to another and follow
   a split.
2. **OWD makes every input,** each marked as a default with its source:
   - bank lines, river polygon and centreline from the JRC water mask, moved onto a dry-season
     Sentinel-2 scene;
   - a Copernicus GLO-30 DEM;
   - a low-water level from SWORD and SWOT, or from the DEM water surface where SWOT has none.
3. **The user gives a depth profile** when there is no bathymetry: one depth, optional depths at
   chainages and a season label. OWD stores it as a bed model.
4. **OWD answers "what fits"** over every vessel class, per rule set: the largest class that fits as
   it is, and for each larger class the stretches that stop it and the works they need.
   - **Plan only** (banks, no DEM, no depth): width and radius.
   - **Full** (DEM and depth profile): adds depth and spec 0007's verdict.
5. **"Use this vessel"** sets the design's vessel and leads into today's flow: alignment, then volumes.

It runs on the CLI, the desktop app and the web app. There is no server.

## User stories

### Journey 1: what barge fits the Sassandra now?

An engineer wants to screen the Sassandra in Côte d'Ivoire, from the sea at Sassandra town upstream
to the Soubré dam, and see what barge could use it today. They have no data of their own.

1. As an engineer, I want to type "Sass", choose "Sassandra — region, Côte d'Ivoire" from a list and see
   the whole river drawn on the map with its length, so that I know OWD found the right one and not
   another river with the same name.
2. As an engineer, I want to click a start at the river mouth and an end below the dam, and see the
   route between them along the river with its length, so that I know which water I am screening.
3. As an engineer, I want OWD to make the bank lines, river polygon and centreline for that route,
   so that I can start without a GIS.
4. As an engineer, I want to see each generated input on the map with a "default" badge, its source
   and its date, so that I know what I did not measure.
5. As an engineer, I want stretches too narrow for the water mask to measure flagged as low-confidence
   width, so that I check them before I trust them.
6. As an engineer, I want the plan-only answer at once: for each vessel class, whether it fits by width
   and radius, with the tightest stretches on the map, so that I get a first answer before any DEM.
7. As an engineer, I want OWD to get a DEM and a low-water level for the reach, so that the depth check
   can run.
8. As an engineer with no bathymetry, I want OWD to ask me for a depth (for example 3 m at low water in
   the dry season), so that I can give the one number I have from people who use the river.
9. As an engineer, I want to add depths at known places, such as a shallow bar, so that the profile
   follows what I know.
10. As an engineer, I want the full answer: the largest class that fits as it is, under each rule set,
    so that I can say "this barge fits today".
11. As an engineer, I want each larger class shown with its constraint stretches (too narrow, too tight,
    too shallow), how much each falls short, and the works that follow, so that I see what the next
    size up would need.
12. As an engineer, I want every answer to say "depth assumed, not measured" while the bed is my
    default, so that nobody reads it as a survey.
13. As an engineer, I want to pick any class, including one that needs works, and press "Use this
    vessel", so that I go on to alignment and volumes for it.

### Journey 2: trace a route the network does not draw as one river

An engineer wants a route that leaves one river, follows a creek and joins another.

14. As an engineer, I want to trace the route roughly on the map and have it snap to the waterways it
    follows, so that I do not click every bend.
15. As an engineer, I want the snap to cross from one river to another and to follow the branch I
    traced at a split, so that the route is the one I meant.
16. As an engineer, I want a traced stretch with no waterway near it kept as I drew it, and flagged,
    so that a small creek missing from the map does not stop me.
17. As an engineer, I want to drag a start, an end or a via point and see the route change, so that I
    can correct it quickly.

### Journey 3: correct what OWD made

18. As an engineer, I want to load my own bank lines over the generated ones, so that my better data
    wins.
19. As an engineer, I want to redraw one stretch of bank on the map and have it spliced in, so that I
    fix one bad place without redrawing the river.
20. As an engineer, I want to move, add and delete bank vertices, so that I can fix a small error.
21. As an engineer, I want an edited input marked "edited" instead of "default", so that the record
    shows who made each line.
22. As an engineer, I want every stage that depends on a changed input marked stale, never rerun
    silently, so that the answer on screen is always one I ran.
23. As an engineer, I want to override the water level (another percentile, or a typed level), so
    that I can screen a season other than low water.
24. As an engineer, I want a survey, when I get one, to replace the bed model and change nothing else,
    so that I can upgrade the study in place.

### Journey 4: on the web

25. As a web user, I want the same front door in the browser, so that I can screen a river without
    installing anything.
26. As a web user, I want a list of the DEM tiles I need, as download links, and to drop them back in,
    so that I get a DEM with no server between me and the data.
27. As a web user, I want the plan-only answer without a DEM, so that I get a first answer in the
    browser with one small download at most.

### Journey 5: on the command line

28. As a CLI user, I want `owd reach` to make a study from a river name and two points, or from a
    traced line, so that I can script a screening.
29. As a CLI user, I want `owd fetch-dem --bbox` and `owd fits`, so that I can get the DEM and the
    answer in a script.

### Reviewer

30. As a reviewer, I want each input's source, licence notice, date and default mark in the document
    and in every export, so that I can check where every number came from.
31. As a reviewer, I want the provisional defaults (mask threshold, minimum trusted width, water-level
    percentile) named in the document, so that I know which choices are not yet calibrated.

## Implementation decisions

### Modules

- **`owd-reach` (new crate, ADR 0010).** Pure Rust; builds native and wasm. Network access is the
  `Fetch` trait. It holds:
  - **sources:** a registry of sources (URL pattern, licence, required notice);
  - **search:** the river list as the user types (see "River search" below);
  - **route:** OSM waterway graph for the reach's bounding box (Overpass), a shortest route through
    start, via points and end, trace matching, and the whole river of a chosen OSM relation or way,
    from its mouth to its upstream end;
  - **banks:** JRC occurrence mask by window, threshold, the water polygon of the routed channel,
    bank lines and centreline; the low-confidence width flag;
  - **edge fit:** a dry-season, low-cloud Sentinel-2 L2A scene from the Earth Search STAC, a water
    index, and the edge fit along bank normals (a colour likelihood and a Viterbi path, ported from
    earlier work, with a maximum shift and a maximum change per station);
  - **water level:** SWORD reaches and nodes along the route, SWOT levels from Hydrocron, the
    percentile, and the DEM fallback;
  - **DEM tiles:** the list of Copernicus GLO-30 tiles for a bounding box.
- **`owd-format`.** New document parts: the **reach** (start, end, via points, the trace if any, and
  the route as a list of OSM way ids with no geometry), the **provenance** of each input (source id,
  date, notice, mark `default`, `edited` or `user`), the **bed model** (depth table by chainage on the
  river centreline, shape, season) and the **fits** result. The water level stays on the river
  centreline (ADR 0006); the bed model attaches to it the same way.
- **`owd-core`.** The bed: at a point inside the water, bed = water level − depth × shape, where depth
  is interpolated along the river centreline and the shape is a parabola across the water, 1 at the
  centreline and 0 at the banks. Volumes and the depth check read the bed model where the document has
  one, else the DEM, as today. No file, clock or network access.
- **The answer (`fits`).** For each vessel class from `ovcd-core`, in size order, under each selected
  rule set: navigable water (spec 0005), the width check at local radius, and the radius of the river
  centreline. Plan only reports width and radius stretches. Full adds depth from the bed model and the
  spec 0007 verdict and constraint stretches. One row per class: verdict, worst deficit per kind,
  stretch count, and the stretches themselves.
- **App (desktop and web).** A "Start from a river" view: search box, map clicks for start, end and
  via points, a trace tool, the route with its length, the generated inputs with badges, a depth
  profile form, the class table with "Use this vessel", and a bank-line editor (redraw a stretch, and
  move, add and delete vertices). The app computes nothing: every field comes from the session.

### River search (T13 to T15)

- One search box. From 3 letters, and 350 ms after the last key, a list shows the rivers and canals
  whose name starts with or matches the text, one entry per OSM relation or way: "River name —
  region, country", with the length once known. An optional country filter (two-letter code).
- The list comes from **Photon** (photon.komoot.io, OSM data, built for search as you type), with our
  User-Agent and answers cached by request. The Nominatim usage policy forbids type-ahead on its
  public API, so Nominatim is not used for the list. Only `owd_reach::search` knows the provider.
- Choosing an entry reads its waterways from Overpass and routes the main stem. OSM draws a waterway
  in the direction of flow: the **mouth** is the downstream end, and the **upstream end** is the
  source reached by the longest route from the mouth. Start is set to the mouth and End to the
  upstream end; the user drags either, or clicks, to cut a reach. Via points work as before.
- The document records the chosen OSM id with the reach, and the search source with its date in the
  provenance, beside Overpass.
- CLI: `owd reach --search TEXT [--country CC]` lists the entries with their OSM ids; `owd reach FILE
  --osm-id relation/N` makes the study from the whole river, and `--start`, `--end` and `--via` cut it.

### Defaults (provisional until calibrated against a surveyed reach)

| Default | Value |
|---|---|
| Water mask | JRC occurrence ≥ 75 % |
| Low-confidence width | under 3 mask pixels (90 m on JRC) before the edge fit; under 3 pixels (30 m) after |
| Edge fit | ±40 m search, at most 30 m shift, at most 1.5 m change per 5 m |
| Water level | SWOT 10th percentile per SWORD reach; DEM water surface where SWOT is missing or the river is under 100 m wide |
| Depth | none: the user must give one for the full answer |
| Bed shape | parabola across the water |

### Licences and notices

Every notice is shown on the map and carried in every export while its input is used: "© OpenStreetMap
contributors" for the route and the river search; the Copernicus DEM notice and disclaimer; "Contains modified Copernicus
Sentinel data [year]"; "Source: EC JRC/Google" and Pekel et al. 2016; SWORD (CC BY 4.0) and SWOT
(NASA). OSM geometry is never stored in the document: the route is kept as way ids, and the bank
lines come from the water mask and imagery.

### The web

- Sources that allow CORS (Overpass, Earth Search, the Sentinel-2 COGs, Hydrocron) are read from the
  page through the Worker.
- Sources that do not (the Copernicus DEM, the JRC mask) are offered as a list of plain download
  links. The user drops the files in. So the plan-only answer on the web needs the JRC tile (or
  tiles) for the reach, and no DEM.
- Nothing is sent to a server of ours.

## Testing decisions

- **A good test** checks what a user or an agent can read: a reach in, a document and an answer out.
- **No network in `test`.** Every source reader is tested on recorded fixtures (small windows of real
  open data, with their notices, under `examples/reach/`). A live check against each source runs in a
  separate, non-required CI job.
- **Known answers on synthetic fixtures:** a straight channel with one narrow throat, one shallow bar
  and one tight bend gives known stretches for each class, as in spec 0007.
- **Routing:** fixtures with a split, a river-to-river crossing and a gap in the network give the
  expected route and the expected "as drawn" flag.
- **The bed model:** a property test that the bed is never above the water level and meets the water
  level at the banks.
- **Parity** (spec 0011): the web session and native give the same document for each fixture.
- **Design impact:** existing fixtures carry no bed model and give the same results. The check
  reports none.

## Tickets

Each is a sub-issue of the spec issue (#225). Blocking uses native links.

1. **T1 (#226): `owd-reach` crate, `Fetch` interface, source registry, provenance in the document.** ADR
   0010 in code.
2. **T2 (#227): route.** OSM name search, waterway graph, routing with splits and crossings, trace matching,
   `owd reach`. Blocked by T1.
3. **T3 (#228): banks from the water mask.** JRC by window, threshold, polygon, banks, centreline,
   low-confidence width. Blocked by T2.
4. **T4 (#229): edge fit on Sentinel-2.** Blocked by T3.
5. **T5 (#230): DEM for the reach.** `owd fetch-dem --bbox`, the desktop fetch from the reach, the web tile
   list. Blocked by T1.
6. **T6 (#231): default water level** from SWORD and SWOT, with the DEM fallback. Blocked by T2 and T5.
7. **T7 (#232): bed model.** Blocked by T1.
8. **T8 (#233): plan-only answer and `owd fits`.** Blocked by T1.
9. **T9 (#234): full answer** with depth and the spec 0007 verdict. Blocked by T7, T8 and spec 0007's
   screening outputs (0007 T5, #163).
10. **T10 (#235): app: Start from a river,** desktop and web. Blocked by T2 to T8.
11. **T11 (#236): app: bank-line editor.** Blocked by T1.
12. **T12 (#237): the Sassandra worked example,** end to end, plan only and full. Its PR closes #1. Blocked by T4,
    T6, T9 and T10.
13. **T13 (#306): river search with a list to choose from,** `owd_reach::search` (Photon) and `owd reach
    --search`. Added after the maintainer's test of T10.
14. **T14 (#307): the whole river from a search hit,** mouth and upstream end found, `owd reach --osm-id`.
    Blocked by T13.
15. **T15 (#308): app: the search dropdown, the whole-river route, and dragging Start and End,** desktop
    and web. Blocked by T13 and T14.

## Out of scope

- Air draught and bridges; rapids, rock and towns (#12, #13).
- A vessel that is not an `ovcd-core` class (a free beam, length and draught).
- Calibrating the defaults against a surveyed reach. It is planned; until then they are marked
  provisional.
- HydroSHEDS, and any imagery with non-open terms.
- Nominatim for the river list: its policy forbids type-ahead (see "River search").
- A server, accounts or saved studies online.
- Survey import formats beyond what OWD reads today.
