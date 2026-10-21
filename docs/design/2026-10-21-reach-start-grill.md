# Start a study from a river reach: design decisions (grill, 2026-10-21)

The record of a design interview between the maintainer and an agent, for spec 0012 and ADR 0010.
This note keeps every answer, so the spec can be checked against it.

## Why

The first question a river screening asks is: *given this reach, what vessel fits it now, and what
would the next size up need?* OWD cannot answer it. Every user story in specs 0003, 0005 and 0007
starts with bank lines, a river polygon, a DEM and a chosen vessel, and asks what works that vessel
needs. The maintainer wants a front door: pick or trace a reach, get a default for every input, give
a depth profile when there is no bathymetry, then get the answer.

The maintainer's direction: let the user either search for a river and pick the start and end of the
reach, or trace the river roughly on the map. Scripts then make the inputs, with defaults (bank lines
and a DEM included). The user gives a default depth profile when there is no bathymetry.

## What was found first

- **OWD today.** Vessel classes come from `ovcd-core` (pinned by tag, ADR 0001). The CLI and desktop
  fetch a Copernicus GLO-30 DEM (spec 0003 story 7); the web app does not (spec 0011). Spec 0005
  already says "width only, no depth data" when there is no bathymetry. Spec 0007 gives one verdict per
  vessel and lists a class sweep as out of scope.
- **A private river screen** (outside this repo) already answers "what fits" on one reach, from width
  and bend radius only. Its banks start as a hand trace, moved onto the water edge seen in imagery by a
  colour model and a Viterbi edge fit (1 to 3 m claimed on open banks). It refines a seed; it does not
  find banks from nothing. The imagery it uses has terms that do not allow this use in an open tool.
  Sentinel-2 was only a cross-check there: NDWI fails on turbid water and MNDWI is weak. Depth was
  assumed everywhere, with no survey.
- **Open data and licences** (checked 2026-10-21):

  | Data | Licence | Access | Use here |
  |---|---|---|---|
  | OpenStreetMap waterways | ODbL 1.0 | Overpass | Route and name search. Has splits, creeks and canals. |
  | SWORD v17b | CC BY 4.0 | Download per basin | Reaches and nodes (~200 m) with width, water level, slope. Rivers ≥ 30 m. |
  | SWOT via Hydrocron | NASA open data | API, CORS allowed | Water level time series per SWORD reach or node. |
  | JRC Global Surface Water | Copernicus, free, cite Pekel et al. 2016 | 10° GeoTIFF tiles, no CORS | Water occurrence mask, 30 m, 1984–2021. |
  | ESA WorldCover | CC BY 4.0 | COGs on S3, no CORS | 10 m water class, cross-check. |
  | Copernicus GLO-30 | Copernicus DEM licence, commercial use allowed | COGs on S3, no CORS | DEM. Sees the water surface, not the bed. |
  | Sentinel-2 L2A | Copernicus, free and open | Earth Search STAC and COGs, CORS allowed | 10 m imagery for the bank edge. |
  | HydroRIVERS / HydroBASINS | WWF licence, no stand-alone redistribution | Shapefile zips | Not used in v1. A tree: no splits or canals. |
  | Nominatim | ODbL, 1 request/s, no autocomplete | API | Not needed: name search goes through Overpass. |

  "No CORS" means a web page cannot read the file directly. A plain download link still works.

## Decisions

| # | Question | Answer |
|---|---|---|
| Q1 | The example river in public text | The Sassandra, Côte d'Ivoire. No other river, project or client is named. |
| Q2 | What "what fits" answers | The largest vessel class that fits the reach as it is, per rule set; and for each larger class, its constraint stretches (width, radius, depth) and the works they need. Air draught is out: there is no bridge data yet (#13). |
| Q3 | Which surface | All three: CLI, desktop and web. A web user with internet must be able to get a DEM. Analysis must also work with no DEM, from the plan alone. |
| Q4 | Picking the reach | Search a river by name and click start and end, or trace roughly. Both snap to the river network. The route may cross from one river to another and follow a split. A trace with no network line near it stays as drawn, flagged. |
| Q5 | Default depth with no bathymetry | One depth for the reach, plus optional depths at chainages, interpolated, with a season label. The bed is deepest at the centre and rises to zero depth at the banks. |
| Q6 | Defaults against measured data | Every generated input carries its source, date, licence notice and a "default" mark. The app shows a badge. Every verdict and export says "depth assumed, not measured" while the bed is a default. |
| Q7 | The journey 1 reach | The Sassandra from the sea at Sassandra town upstream to the Soubré dam. |
| Q8 | Two levels of answer | Build both. **Plan only** (banks, no DEM): width and radius, labelled "width and radius only, no depth". **Full** (DEM and depth profile): adds depth and the verdict. |
| Q9 | River network | OSM waterways for the route and the name search. The study keeps the route choice, not OSM geometry. SWORD is used wherever it helps (water level, slope, width cross-check). No HydroSHEDS in v1. |
| Q10 | Bank lines | The JRC occurrence mask (water ≥ 75 % of the time, the low-water channel) gives the first banks. They are then moved onto a dry-season Sentinel-2 scene with the edge-fit method. Both in v1. A stretch under about 3 mask pixels wide is flagged as low-confidence width. |
| Q11 | Correcting bank lines | Load your own, redraw a stretch on the map, and a full vertex editor. |
| Q12 | Where the generation code lives | A new pure-Rust crate, `owd-reach`, with network access behind one interface: native HTTP on the CLI and desktop, the browser's `fetch` on the web. ADR 0010. |
| Q13 | The sweep and spec 0007 | Both: a plan-only sweep now, from spec 0005's width check and the radius check; the full sweep with depth and the verdict once spec 0007 lands. |
| Q14 | DEM on the web | No server. The page lists the DEM tiles as plain download links; the user downloads them and drops them back in. The CLI gets a `fetch-dem` command by bounding box. The desktop fetches by itself. |
| Q15 | Which water level | Low water by default: the 10th percentile of SWOT levels per SWORD reach, labelled with its date range. Where SWOT is missing, or the river is under about 100 m wide, the DEM water surface, smoothed to fall downstream. The user can pick another percentile or type a level. |
| Q16 | How the default bed is stored | As a bed model in the document: the depth table, the shape and the season. Not as a new DEM. A survey later replaces it and changes nothing else. |
| Q17 | A data check against a hand-fitted reach | Not run yet. The JRC threshold, the minimum trusted width and the SWOT percentile ship as provisional defaults, marked for calibration. No ticket waits on the check. |
| Q18 | After the answer | Each class row has "Use this vessel". It sets the design's vessel and leads into today's flow (alignment, then volumes). Any class can be chosen, including ones that need works. |
