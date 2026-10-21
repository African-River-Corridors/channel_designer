# Spec 0004: Engineering drawings through the Open Technical Drawer

Status: **approved by the maintainer, 2026-10-21.** ADR 0009 holds the decision. The interview record is
`docs/design/2026-10-21-drawings-grill.md`. Every build ticket waits for a tag of
`open-technical-drawer` (OTD); some wait for later OTD releases, as each ticket says.

## Problem statement

A river engineer who screens a river for navigation needs drawings to show and to check the design:
where the channel runs, what it looks like in section, and where to set it out. OWD cannot give them.

- The only sheets are the Python cross-section gallery. It is built on an API that the Drawing Format
  Standard removed in its 1.0 release, it is pinned to an old commit, and it needs an optional extra.
- There is no plan sheet, no typical section and no setting-out table.
- There is no PDF, and the app cannot export a sheet.
- Nothing checks that a drawing follows the standard, or that a drawn number equals a result.

## Solution

OWD draws through **`otd`**, the Drawing Format Standard in Rust, shared with OVCD (ADR 0009). OWD
turns a channel design document into drawing documents; `otd` turns those into normalised content,
SVG, PDF and DXF.

- Four kinds of sheet: **cross-section gallery**, **typical section**, **setting-out table** and
  **plan sheets** (an overview and strips along chainage, over shaded relief or an orthophoto).
- What a sheet shows comes from a **sheet style**: a JSON file with defaults, so no layout choice is
  hard-coded.
- A sheet labels exceptions and governing values, not every station. A sheet with no problems stays
  clean.
- `owd draw` on the CLI, `owd.draw` in Python, and a Sheets view with PDF export in the app.
- Checks in two layers: OTD's conformance suite and layout review; OWD's domain checks and a
  cross-check against the standard's Python reader.
- The Python drawing module and the `[drawings]` extra go. There is one drawing path.

Every ticket ships in the next release.

## User stories

### River engineer

1. As a river engineer, I want a cross-section gallery at one common scale, so that I can compare the cut along the line.
2. As a river engineer, I want the gallery to include the worst station of each constraint stretch and the middle of each bend, so that the hard places are always drawn.
3. As a river engineer, I want to set the section spacing and which extra sections to add, so that the gallery fits the job.
4. As a river engineer, I want a typical section for each reach kind, with bottom width, side slopes, design depth, design water level and under-keel clearance dimensioned, so that I can show the design basis in one picture.
5. As a river engineer, I want each typical-section value to name the Rule that governs it, so that I can quote it with its source.
6. As a river engineer, I want a setting-out table with chainage, easting, northing, bearing and local radius, and a row per bend, so that a surveyor can set out the line.
7. As a river engineer, I want the setting-out table as CSV too, so that I can load it into survey software.
8. As a river engineer, I want a plan overview of the whole line with a key to the strip sheets, so that I can find any place on the line.
9. As a river engineer, I want strip plans along chainage with match lines, so that I can read the line at a working scale.
10. As a river engineer, I want the plan over shaded relief from my DEM, so that I see the ground without any extra data.
11. As a river engineer, I want the plan over my own orthophoto, or over fetched imagery, so that I see the river as it is.
12. As a river engineer, I want the plan to mark constraint stretches, bends tighter than a set limit and infeasible stretches, each with one short label, so that I see the problems and nothing else.
13. As a river engineer, I want every sheet marked "Screening — not for construction", so that nobody builds from it.
14. As a river engineer, I want the PDF and the DXF of every sheet, so that I can issue it and a drafter can take it further.

### Reviewer

15. As a reviewer, I want the sheet style's hash on every sheet, so that I know which rules made it.
16. As a reviewer, I want each label rule to be a named rule with its parameters in a file, so that I can see and change why something is labelled.
17. As a reviewer, I want `owd draw` to fail when a drawn value differs from the result, so that a sheet never shows a wrong number.

### App user

18. As an app user, I want a Sheets view that shows each sheet, so that I can check it without the CLI.
19. As an app user, I want to export the sheets as PDF, so that I can send them.

### Agent and maintainer

20. As an agent, I want `owd draw --document` to print the drawing documents, so that I can check them with the standard's Python tool.
21. As an agent, I want `owd.draw` in Python to give the same sheets as the CLI, so that a notebook and the CLI agree.
22. As a maintainer, I want OWD to use the same drawing crate as OVCD, so that one implementation of the standard serves both.
23. As a maintainer, I want CI to compare `otd`'s normalised content with the standard's Python reader for OWD's drawing documents, so that the two readers cannot drift.

## Implementation decisions

### The drawing crate

- OWD pins `otd` by git tag of `African-River-Corridors/open-technical-drawer`. OWD adds no drawing
  logic of its own: no sheet layout, scale search, text widths, hatches, dimensions or output writers.
- OTD's backlog, which OWD tickets wait on (the OTD coordinator sets the tag numbers):
  1. **The move.** `ovcd-draw` with its history, renamed `otd`, on standard 1.1. OVCD pins it and drops its
     copy. All of OWD's tickets wait for this.
  2. **Images.** From a raster grid in model coordinates: clip to a viewport, shaded relief from a DEM,
     JPEG, and the `image` entity with its bytes. No file access in the core.
  3. **Fetched imagery.** A crate apart from the core that fetches and caches imagery tiles for a box
     (Esri World Imagery first, with the user's key) and returns a raster grid for item 2.
  4. **Layout review.** A review of normalised content: text overlap, smallest text at print size, text
     clear of hatches, entities inside the frame.
  5. **Vertical exaggeration**, once the standard allows it (clause 7.3.3). A request to the standard.

### The drawing builder

- A module in `owd-format`. It reads the design document, its stage results and a sheet style, and
  returns drawing documents. It computes no new engineering value: every drawn number is a result the
  document already holds. If a value is missing, the ticket stops and reports.
- Sheets use the standard's ISO 7200 title block. Fields: design name, river, design line option,
  sheet title, scale, revision, date and the sheet style's hash. Status `concept`, with the line
  "Screening — not for construction".
- Drawing numbers: `OWD-<design>-<kind>-<nnn>`, where kind is `PL` (plan), `XS` (cross sections), `TS`
  (typical section) or `SO` (setting out).
- Coordinates are in the design's UTM zone (ADR 0003). Each sheet that shows coordinates states the zone
  and datum.
- Layers come from the standard's layer table. Colour uses the standard's colour print profile only on
  sheets that hold an image.

### The sheet style

- A JSON file with a JSON Schema in `owd-format`. The default, `owd-default.style.json`, is built into
  every binary. A user's file loads through the same path.
- It sets: sheet size and orientation per sheet kind; scales (a list to try, or fixed); layers on or
  off; chainage tick spacing by scale; section spacing and the extra-section rules; strip length and
  overlap; and the **label rules**.
- A label rule is named and has parameters, for example `bend-tighter-than` with `k` (label a bend
  whose least radius is below `k × R_min`), or `constraint-stretch` with the fields its label shows.
  No threshold lives only in code.
- CLI flags override single values (for example `--section-spacing 250`). The hash on the sheet is of
  the style after the overrides.

### The sheets

- **Cross-section gallery.** Sections at the style's spacing (default 500 m), plus the extra sections the
  style names (defaults: the worst station of each constraint stretch once spec 0007 gives them, and the
  middle of each bend). One common scale for all sections, packed onto as many sheets as needed. Each
  shows ground, design section, water level, dredge and dry cut hatched, and their areas.
- **Typical section.** One per reach kind (one in all until spec 0007's reaches land). Bottom width, side
  slopes, design depth, design water level and under-keel clearance, each a dimension computed from
  geometry. A note under each value names its governing Rule once spec 0007 lands, and the ruleset
  until then.
- **Setting-out table.** One row per station at the style's spacing: chainage, E, N, bearing, local
  radius on the chord. One row per bend: start and end chainage, least radius, deflection. Drawn as
  lines and text, split over sheets as needed. Also written as CSV. Coordinates to 0.001 m. OWD's design
  line is a polyline, not intersection points and arcs (ADR 0007), so the table has no IP rows; that
  reduction is a later spec.
- **Plan sheets.** An overview of the whole line at a fitted scale (map ratios allowed), with the strip
  outlines as a key. Strips along chainage, default 1:5000 on A1, with match lines at chainages.
  Always shown: background, water edge at the design water level, design line with chainage ticks,
  channel footprint (toe and top of cut), section lines keyed to the gallery, a UTM grid, north arrow
  and scale bar. Shown only where the style's label rules find something: bends tighter than the limit,
  infeasible stretches, and (later ticket) constraint stretches.
- **Background.** Shaded relief from the design's DEM by default. A user's orthophoto GeoTIFF (WGS84 or
  the design's UTM zone, reprojected by OWD's existing code) or fetched imagery when asked. OWD reads
  the files; OTD makes the image. Fetched imagery prints its attribution on the sheet.
- **Long section.** Not in this build: it needs vertical exaggeration. The app keeps its own view.

### CLI

- `owd draw <file> --out <dir>` writes, for each sheet, the drawing document, the normalised content, SVG,
  PDF and DXF; and the setting-out CSV. Flags: `--option <name>`, `--sheets <kinds>`, `--style <file>`,
  single-value overrides, `--orthophoto <geotiff>`, `--imagery esri` (key from the environment).
- `owd draw --document` prints the drawing documents only.
- `owd draw` runs OTD's layout review (when the pinned `otd` has it), then OWD's domain checks. Any
  error fails the command with the sheet and the check named.

### Python

- `owd.draw(...)` through `owd-py` gives the same outputs as the CLI.
- `owd.drawings`, `src/owd/drawings/sections.py` and the `[drawings]` extra go.

### App

- A Sheets view lists the sheets of the active design line option and shows each SVG. An export button
  writes the PDFs. The app computes nothing: the sheets come from the same builder through the shell.
- The app's live long-section and cross-section views stay as they are.

## Testing decisions

- **A good test goes through a public seam:** `owd draw --document` and `--json`, the drawing builder's
  public function, or `owd.draw`. Never a private layout detail.
- **Domain checks.** For each fixture design, every drawn value equals the document's result: chainages,
  coordinates and radii in the setting-out table (to its stated precision), section areas, typical
  section dimensions, chainages of match lines and section lines.
- **Schema.** Every drawing document validates against the standard's schema; every sheet style
  validates against its schema.
- **Cross-check.** In CI, OWD's fixture drawing documents go through the standard's Python `normalise`
  (pinned commit) and through `otd`. The normalised content must be equal. The plain Rust tests need no
  Python.
- **Label rules.** Synthetic fixtures (a straight line with one tight bend, one infeasible stretch, and a
  clean line) give known labels: the clean line gives none.
- **Style.** The same design and style give byte-identical drawing documents on every run and platform.
  Changing one parameter changes the hash on the sheet.
- **App.** One smoke test sees the Sheets view show a sheet.
- OTD's conformance suite and output tests run in OTD, not here.
- Prior art: OVCD spec 0003's cross-implementation check; this repo's fixture designs.

## Out of scope

- Any change to the standard. Requests go to its repository.
- The long section, until the standard allows vertical exaggeration.
- Intersection-point setting out (a later spec).
- Drawings of structures (locks, weirs, bank protection).
- Editing the sheet style in the app.
- Imagery sources other than Esri World Imagery.

## Tickets

Each build ticket is blocked by an OTD tag. That block is stated in each ticket, because a native link
cannot cross repos.

1. **`otd` and the cross-section gallery.** Pin `otd`; the drawing builder; title block, numbers and
   status; `owd draw` and `--document`; the gallery; `owd.draw` in Python; remove the Python module and
   the extra; the Python cross-check in CI.
2. **Sheet style.** The style file, schema, default, loader, overrides and hash; the gallery reads its
   spacing and extra sections from it. Blocked by 1.
3. **Typical section.** Blocked by 2.
4. **Setting-out table and CSV.** Blocked by 2.
5. **Plan sheets.** Overview, strips, match lines, key; shaded relief; bend and infeasible label rules.
   Blocked by 2 and an OTD release with images.
6. **Orthophoto and fetched imagery.** Blocked by 5 and an OTD release with fetched imagery.
7. **Layout review in `owd draw`.** Blocked by 1 and an OTD release with the review.
8. **App Sheets view and PDF export.** Blocked by 3, 4 and 5.
9. **Constraint stretches on drawings.** On the plan and as extra gallery sections, as label rules.
   Blocked by 5 and spec 0007's screening outputs (#163).
10. **Long section.** Blocked by 2, a standard version with vertical exaggeration, and an OTD release
    that has it.

## Risks

- **OTD is not yet a repository.** Every ticket waits on it. If the move changes the library API,
  ticket 1 adapts to it; it does not pin `ovcd-draw` from OVCD instead.
- **Imagery terms.** Esri World Imagery needs a key, and its terms limit offline caching and reuse. The
  user supplies the key, OWD stores none, and the sheet prints the attribution. If the terms forbid a
  use, the shaded relief and the user's orthophoto still work.
- **Sheets look busy.** The label rules label exceptions only, and every rule can be turned off.
- **A drawn number drifts from a result.** The domain checks fail `owd draw`.
- **Two Python readers.** The cross-check pins the standard's commit; updating it is a deliberate change.
