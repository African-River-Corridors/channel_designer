# Spec 0013: Bank extraction you can trust

Status: **approved by the maintainer, 2026-10-22,** after the Sassandra worked example (spec 0012 T12,
#279). Decisions: ADR 0011 (the water seed is Sentinel-2). Builds on spec 0012 (start from a reach) and
ADR 0010 (reach inputs in `owd-reach`). Reworks spec 0012 T3 (#228) and T4 (#229), and the tiled edge
fit (#264, #270).

## Problem statement

Spec 0012 makes bank lines from the JRC water mask and moves them onto Sentinel-2. On the Sassandra
this goes wrong. In some stretches the banks are wrong by a whole river width. The river radius and the
fairway fit (ADR 0007) then sit on bad banks, and the choice of the default plan line (#278) waits on
better ones.

Diagnosis on km 148 to 152 of the Sassandra worked example, against scene `S2B_29NQG_20250126_0_L2A`:

1. **The seed misses the river.** The open water is about 200 to 250 m wide. The JRC mask banks (30 %
   occurrence) give a median width of 17 m, and as little as 2 m. JRC occurrence is an average over
   1984 to 2021. A reservoir filled late in that period, or water that is there only in some seasons,
   falls under the threshold.
2. **The edge fit cannot recover.** It only refines: a bank moves at most 30 m from the seed. Where the
   seed is a river width wrong, the fit gives `no-edge` (km 142.9 to 153.8 almost throughout) or
   `at-shift-limit`.
3. **Nothing checks the banks.** Banks trace into inlets and tributary mouths: there are spikes, and 5
   of 80 stations have both banks on one side of the river. No check keeps each bank on its own side,
   finds a crossing, or asks whether a width is plausible next to its neighbours.
4. **The centreline is stale.** It is not rebuilt from the final banks. It is up to 83 m off their
   mid-line.
5. **Joins are not smoothed.** Bank parts (one per scene) and per-station shifts are joined as they
   come.
6. **The scene is picked by its overall cloud.** A scene with 5 % cloud can have its 5 % on the river.

## Solution

Make the water mask from the scene itself, trace the banks from it, check and repair them, and rebuild
the centreline from the final banks every time a bank changes. Show the result over the image so a
person can check it by eye.

1. **A Sentinel-2 water mask is the seed** (ADR 0011). Per part of the reach, rank the dry-season scenes
   by how much of the river corridor they see clear, not by their overall cloud. Classify each clear
   pixel as water or land by a water index (MNDWI). The scene classification (SCL) masks cloud, cloud
   shadow, cirrus and no data. Where the best scene is not clear, the next one fills in, up to three.
   Where no scene is clear, the JRC mask fills in, and the stretch is flagged. JRC also serves as a
   cross-check: the study records how far the two agree.
2. **Banks are traced from that mask** with the spec 0012 ray cast, on the routed channel only.
3. **Hard checks, and recorded repairs.** Each bank stays on its own side of the centreline. The banks
   never cross each other or themselves. The width is plausible against its neighbours. A bank that
   jumps sideways more than a limit between stations (a spike, or a run into an inlet or a tributary
   mouth) is cut back to the line its neighbours give. Every repaired station joins a low-confidence
   stretch of kind `repaired`, so nobody reads a repair as a measurement.
4. **The centreline comes from the final banks,** as their mid-line. It is rebuilt after every change to
   a bank: after tracing, after the repairs, after the edge fit, and after a hand edit in the app. No
   step stores a centreline from an earlier step.
5. **Joins are smoothed:** between parts and between scenes, and across repaired stretches, inside the
   edge fit's step limit (1.5 m per 5 m).
6. **The edge fit stays a refiner,** now on a seed that is already on the water. Its limits do not
   change.
7. **A QA overlay.** `owd qa-overlay` draws the banks, the centreline and the low-confidence stretches
   over the Sentinel-2 true-colour image of the scene(s) used, as a PNG and as a KMZ for Google Earth.

## User stories

1. As an engineer screening a river with a reservoir on it, I want the banks to follow the water I can
   see on a recent image, so that a lake filled after 2000 is not read as a 17 m channel.
2. As an engineer, I want a cloudy stretch filled from another clear scene before the JRC mask, so that
   one cloud does not cost me the bank.
3. As an engineer, I want the scene chosen by how clear it is over my river, so that a clear scene with
   a cloud on the river loses to one that sees it.
4. As an engineer, I want each bank kept on its own side of the river, and the banks never crossing, so
   that a width is never negative or measured across one bank.
5. As an engineer, I want a bank that runs into an inlet or a tributary mouth cut back, so that one
   creek does not read as a 600 m wide river.
6. As an engineer, I want every repaired stretch flagged as low-confidence, with what was repaired, so
   that I check it before I trust it.
7. As an engineer, I want the centreline always to be the middle of the banks on screen, so that the
   river radius and the fairway fit sit on the river I see.
8. As an engineer, I want a bank I edit by hand to give a new centreline by the same rule, so that the
   editor and the generator never disagree.
9. As an engineer, I want no step or kink where two scenes or two parts meet, so that a join does not
   read as a tight bend.
10. As a reviewer, I want one picture of the banks, the centreline and the flagged stretches over the
    true-colour image, as a PNG and a KMZ, so that I can check a reach by eye in a few minutes.
11. As a reviewer, I want the study to record which scenes made the mask, the water-index threshold and
    the share of the corridor each scene saw clear, so that I can repeat the result.
12. As a reviewer, I want the study to record how far the Sentinel-2 mask and the JRC mask agree, so
    that I see where the two sources tell a different story.

## Implementation decisions

### Modules

- **`owd-reach::water` (new): the water grid.** A raster in the study's UTM zone, 10 m pixels, three
  states per pixel: water, land, unknown. Each pixel records its source: the index of the scene that
  classified it, or JRC. Built by:
  - `sentinel2`: reads the blue band (B02) too, for the true-colour overlay; computes MNDWI =
    (green − SWIR) / (green + SWIR), with SWIR (B11) on the 10 m grid;
  - a scene ranking per part: the clear share over a corridor around the route (the route buffered
    by the largest half-width), from the SCL band, read at 20 m; the least-cloud tie-break stays;
  - a composite: the best scene classifies every pixel it sees clear; the next scene fills the pixels
    still unknown; at most three scenes per part;
  - the JRC fallback: a pixel no scene sees is water when its occurrence is at or above the threshold,
    and its source is JRC.
- **`owd-reach::banks`.** The ray cast reads a `WaterSource` trait (is this point water, land or
  unknown, and from what source) instead of the JRC occurrence directly. The JRC mask and the water
  grid both implement it, so the JRC-only path (`--seed jrc`) stays. The cast starts from the water
  connected to the route, so a lake beside the river is not read as river.
- **`owd-reach::bankcheck` (new): checks and repairs,** on any pair of banks (mask, edge fit, or the
  user's own lines run through `owd check-banks`, see below). In this order:
  1. **Own side.** Along the centreline's normal, the left bank must lie at a positive offset and the
     right bank at a negative one. A station that fails is repaired.
  2. **Lateral jump.** Per bank, the offset from the centreline may change by at most `max_jump_m`
     between stations (default 3 × the station step: a bank at 20 m stations may run off at 72°). A
     station past the limit, and the run after it until the bank comes back within the limit of the
     line before the run, is repaired. This cuts spikes, inlets and tributary mouths.
  3. **Width plausibility.** The width at a station must lie between 0.5 and 2 times the running
     median of the widths over `width_window_m` (default 500 m) either side. A station outside is
     repaired, on the side that moved most from its own running median.
  4. **No crossing.** After the repairs, neither bank crosses itself or the other. A crossing left
     means the check fails, loudly (an error), not a silent pass.

  A repair replaces the station's offset by a straight interpolation between the nearest good stations
  either side, then the join smoothing (below) applies. Every repaired station joins a low-confidence
  stretch of kind `repaired`, which records which checks failed (`side`, `jump`, `width`).
- **`owd-format::design::midline` (new): the centreline from the banks.** One function, used by the
  bank generator, the edge fit and the bank editor (`bank_edit.rs` today pairs the banks by arc-length
  share, which drifts where one bank is longer). It starts from a seed line, then three times: at
  stations every `station_m`, cast the normal both ways to the first crossing with each bank, and move
  the station to the midpoint. A light smoothing goes between iterations, not after the last one, so the
  stored line is a mid-line. Where a normal finds no bank on one side, the station keeps its place and
  the stretch is flagged.
- **Join smoothing.** The parts and scenes are blended by a cosine weight over the overlap (it replaces
  the linear chainage weight), and the slope-limited envelope that already holds the edge fit's step
  limit runs once along the whole bank after the blend and after the repairs. The step limit itself
  (1.5 m per 5 m) does not change.
- **The pipeline,** in `owd reach --banks` and the app's "Make the inputs":
  water grid → banks → checks and repairs → mid-line → edge fit (optional) → checks and repairs →
  mid-line → record. The JRC cross-check runs on the same stations: the share of stations where the
  JRC width at the threshold is within 50 % of the Sentinel-2 width.
- **`owd-format`.** The provenance of the bank inputs gains the seed (`sentinel2-l2a` or `jrc-gsw`), the
  scenes of the mask per part with their corridor clear share, the MNDWI threshold, the JRC agreement
  and the check limits, each named provisional. The low-confidence stretches gain the kinds
  `repaired` (with the checks that failed) and `jrc-fallback`. Old files read as before.
- **QA overlay: `owd-reach::overlay` (new) and `owd qa-overlay`.** Pure Rust: from a document and the
  scene windows, an RGBA image (true colour, 2 % to 98 % stretch), with the left bank, the right bank,
  the centreline and the low-confidence stretches drawn on it in the OWD colours, and a legend with
  the scene ids and the notices. It encodes the PNG, and a KMZ (a KML ground overlay with its
  `LatLonBox`, plus the banks, centreline and stretches as KML lines, zipped). The CLI writes the files:
  `owd qa-overlay DOC --png out.png [--kmz out.kmz] [--from-km A --to-km B]`. The app offers the same
  export. On the web, the scene windows come from the COGs, which allow CORS.
- **`owd check-banks DOC`.** Runs the checks on the document's banks, whatever made them, and prints
  the failures; it changes nothing. It is the tool a person uses on their own lines.

### Defaults (provisional until calibrated against a surveyed reach)

| Default | Value |
|---|---|
| Seed | Sentinel-2 water grid; JRC only where no scene sees the pixel |
| Water index | MNDWI ≥ 0.0 is water |
| Scenes per part | up to 3, ranked by clear share of the river corridor |
| Masked SCL classes | 0 (no data), 3 (cloud shadow), 8 and 9 (cloud), 10 (cirrus) |
| JRC fallback threshold | 75 % occurrence (as spec 0012) |
| Lateral jump limit | 3 × the station step, per station |
| Width plausibility | 0.5 to 2 × the running median over ±500 m |
| Mid-line | 3 iterations, stations every 20 m |
| Edge fit | unchanged: ±40 m search, at most 30 m shift, at most 1.5 m change per 5 m |

The MNDWI threshold of 0.0 is the textbook split (Xu 2006, Int. J. Remote Sensing 27(14)). On the
Sassandra scene the MNDWI histogram has two clear modes, at about −0.55 (land) and +0.35 (water), with
the trough near 0.

### Licences and notices

No new source. The overlay carries "Contains modified Copernicus Sentinel data [year]" for each scene
year, and the JRC notice where the JRC fallback was used.

## Testing decisions

- **A good test** checks what a user or an agent can read: banks, a centreline and stretches out of a
  mask or a scene.
- **No network in `test`.** New fixtures are small windows of real open data under `examples/reach/`.
- **Synthetic, with known answers:**
  - a straight channel 200 m wide with a 60 m inlet on one bank: the inlet is cut, the stretch is
    flagged `repaired (jump)`, and the width away from it is 200 m within one pixel;
  - banks swapped at a few stations: the swap is repaired, flagged `repaired (side)`, no station keeps
    both banks on one side;
  - a cloud over part of the channel in scene 1 and not in scene 2: the bank there comes from scene 2,
    not from JRC;
  - a cloud in every scene: the JRC fills in, flagged `jrc-fallback`;
  - two scenes, one with less overall cloud but a cloud on the river: the other is ranked first;
  - two parts with a 3 m offset between their fits: no step over the 1.5 m per 5 m limit after the join.
- **Properties:** the mid-line of any two non-crossing banks lies between them, and its offset from
  the banks' midpoint (half of |left offset − right offset| along its normal) is under 1 m on smooth
  synthetic banks; after the checks, no station has both banks on one side and no bank crosses.
- **The Sassandra window (acceptance).** A recorded fixture of km 148 to 152 (scene
  `S2B_29NQG_20250126_0_L2A`, bands B02, B03, B04, B08, B11 and SCL, and the JRC window, with the
  route). From it:
  - the median width is within 20 % of the open water measured on the scene by hand (transects on the
    true-colour image, recorded in the fixture's README with their method, `Label: sourced`);
  - no station has both banks on one side;
  - the centreline's offset from the banks' mid-line is at most 5 m at the 95th percentile and 10 m at
    most.
- **Design impact.** The synthetic design-impact scenarios do not use generated banks, so the check
  should report none. Generated banks do move: each ticket that moves them adds a `### Design impact`
  bullet, and the last ticket regenerates `docs/examples/sassandra.md`.

## Tickets

Each is a sub-issue of the spec issue (#284). Blocking uses native links.

1. **T1 (#285): the mid-line, and checks and repairs on any banks.** `owd-format::design::midline` (also used
   by the bank editor), `owd-reach::bankcheck`, the `repaired` stretch kind, `owd check-banks`; run
   after the JRC banks and after the edge fit in today's pipeline.
2. **T2 (#286): the Sentinel-2 water grid.** The blue band, MNDWI, SCL, scene ranking by corridor clear share,
   the composite of up to three scenes, the JRC fill and its flag.
3. **T3 (#287): banks from the water grid, and the Sentinel-2 seed by default.** The `WaterSource` trait, the
   connected-water start, `--seed s2|jrc` on `owd reach`, the app, the provenance and the JRC
   cross-check. Blocked by T1 and T2.
4. **T4 (#288): smoothed joins.** The cosine blend over overlaps, and the step limit along the whole bank after
   the blend and the repairs. Blocked by T3.
5. **T5 (#289): the QA overlay.** `owd-reach::overlay`, `owd qa-overlay` (PNG and KMZ), and the app's export.
   Blocked by T2.
6. **T6 (#290): the Sassandra acceptance.** The km 148 to 152 fixture and its hand-measured widths, the
   acceptance test, the regenerated worked example and design-impact entries, and the QA overlay of the
   window in the PR body. Blocked by T4 and T5.

## Out of scope

- Choosing the default plan line (#278). This spec gives it better banks to decide on.
- A learned water classifier, SAR (Sentinel-1) for cloud, or any imagery with non-open terms.
- Calibrating the defaults against a surveyed reach.
- A medial axis of the whole water polygon (islands, braids). The mid-line follows one channel, the
  routed one.
- Bridges, rapids and rock (#12, #13).
