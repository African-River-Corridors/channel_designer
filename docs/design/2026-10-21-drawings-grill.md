# Engineering drawings: design decisions (grill, 2026-10-21)

The record of a design interview between the maintainer and an agent, for ADR 0009 and spec 0004.
This note keeps every answer, so the spec can be checked against it.

## Facts found before the interview

- OVCD's drawing crate `ovcd-draw` implements the Drawing Format Standard (the public
  `technical_drawings_for_agents` repository) in Rust: drawing document to normalised content, then
  SVG, PDF and DXF. It has no dependency on `ovcd-core` or `ovcd-format`, so it is already
  domain-free. It passes the standard's conformance suite, vendored at a pinned commit (OVCD spec 0003).
- That pin is standard 1.0. The standard's main branch has 1.1 (unreleased): the `image` entity
  (clause 9.10), the colour print profile and map ratios to 1:10 000 000. A river plan sheet needs all
  three.
- OWD draws today in two places. `owd.drawings.sections` (Python) makes the cross-section gallery on
  the standard's old Python API, which the standard's 1.0 clean break removed; it is pinned to an old
  commit. The app has its own live long-section and cross-section views in Svelte.
- **The flag.** Clause 7.3.3 of the standard: a viewport has one scale for both axes. A long section
  with vertical exaggeration cannot be drawn under the standard today.

## Decisions

| # | Question | Answer |
|---|---|---|
| Q1 | Sheets in the first build | Cross-section gallery, typical section, setting-out table, plan sheets. The long section waits for Q4. |
| Q2 | How OWD gets the drawing crate | `ovcd-draw` moves to its own public repository and gets a new name. |
| Q3 | Standard version | 1.1. The crate moves to standard 1.1 when it moves repository. |
| Q4 | Long section without exaggeration | Left out until the standard allows a vertically exaggerated viewport. A request goes to the standard. The app keeps its own long-section view until then. |
| Q5 | The Python section gallery | Deleted. Python calls the Rust drawing path through `owd-py`. The `[drawings]` extra goes. One drawing path. |
| Q6 | Outputs and where they show | `owd draw` writes the drawing document, normalised content, SVG, PDF and DXF. Python gets the same through the binding. The app gets a Sheets view with PDF export. The app's live long and cross sections stay. |
| Q7 | Names | Repository `African-River-Corridors/open-technical-drawer` (OTD). Crate `otd`, with the library API of `ovcd-draw`. No CLI of its own yet. |
| Q8 | How the move happens | The new repository is made from `crates/ovcd-draw` with its history, moved to standard 1.1 and tagged. OVCD pins the tag and deletes its copy. The OVCD coordinator does this. OWD's drawing tickets wait for the tag. |
| Q9 | Plan sheet layout | One overview sheet at a fitted scale (map ratios allowed), plus strip sheets along chainage with match lines and a key on the overview. Strips default to 1:5000 on A1. |
| Q10 | Plan background | Shaded relief from the DEM, and orthophotos too. Image handling belongs in OTD, not in each tool. |
| Q11 | Title block and status | The standard's ISO 7200 template. Fields from the document: design, river, design line option, sheet title, scale, revision, date. Status `concept` with the line "Screening — not for construction". Drawing numbers `OWD-<design>-<kind>-<nnn>`, kind PL, XS, TS or SO. |
| Q12 | Who checks a drawing | OTD checks format and layout: its conformance suite, and layout rules (a linter). OWD checks its domain: drawn values equal the document's results. OWD's CI also runs its drawing documents through the standard's Python `normalise` and through `otd`, and the normalised content must be equal. |
| Q13 | Rules engine results on drawings | Yes, later, blocked by spec 0007's screening outputs. What is added must be context-aware: only what a reader needs to review the design. |
| Q14 | OTD's image module | Takes a raster grid in model coordinates (a DEM or an RGB orthophoto), clips it to the viewport, makes shaded relief from a DEM, encodes a JPEG and returns the `image` entity with its bytes. It opens no file; the caller reads the GeoTIFF. |
| Q15 | Where the orthophoto comes from | Both: a user's GeoTIFF (WGS84 or the design's UTM zone), and imagery tiles fetched online. Fetching lives in OTD, in a crate apart from the core. |
| Q16 | OTD's layout review | OTD gets a review of normalised content (text overlap, smallest text at print size, text clear of hatches, entities inside the frame). An OTD coordinator owns the repository; OWD and OVCD give it tickets. `owd draw` runs OTD's review, then OWD's domain checks. |
| Q17 | Plan sheet content | Always: background, water edge at the design water level, design line with chainage ticks, footprint, section key, UTM grid, north arrow, scale bar. Where they apply: constraint stretches, bends tighter than a set limit, infeasible stretches. Label exceptions and governing values, never every station. Keep it configurable; do not hard-code design choices that may change. |
| Q18 | Which cross sections | Default: a set spacing (500 m), plus the worst station of each constraint stretch, plus the middle of each bend. Configurable. One common scale. |
| Q19 | Typical section | One per reach kind: bottom width, side slopes, design depth, design water level, under-keel clearance, each dimensioned from geometry. A note names the governing Rule once spec 0007 lands; until then, the ruleset. |
| Q20 | Setting-out table | On the sheet as lines and text, and as CSV. Coordinates in the design's UTM zone to 0.001 m, zone and datum in the header. |
| Q21 | Imagery source | Esri World Imagery. |
| Q22 | What configurable means | A sheet style: a JSON file with built-in defaults (`owd-default.style.json`). It sets layers, named label rules with parameters, chainage tick spacing by scale, section spacing and extra sections, sheet size and scales. No threshold lives only in code. The style's content hash goes in the title block. CLI flags override single values. |
| Q23 | Release | Every ticket goes in the next release. The spec names no version number; the maintainer says when the version moves. |

## Notes recorded with the answers

- **Esri terms (Q21).** Esri World Imagery needs an access key, and its terms limit offline caching and
  reuse. The user supplies the key; no key is in this public repository. The sheet prints the
  attribution the terms ask for. This is recorded as a risk in spec 0004, not reopened.
- **Setting-out table (Q20), found after the interview.** OWD's design line is a polyline from the
  fairway fit and the rolling circle (ADR 0007), not a chain of intersection points with arcs. So there
  are no IPs to tabulate. The spec reads Q20 as: one row per station at a set spacing (chainage, E, N,
  bearing, local radius) and one row per bend (start and end chainage, least radius, deflection), from
  results the core already gives. A reduction of the line to intersection points and arcs is a later
  spec.
