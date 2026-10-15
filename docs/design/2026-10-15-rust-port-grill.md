# Rust port, CLI chain and first desktop app: design decisions (grill, 2026-10-15)

The record of a design interview between Niall O Tuathail and the Systems Architect agent. The
hard-to-reverse decisions are in `docs/adr/0001`–`0005`, and the terms in `GLOSSARY.md`. This
note keeps every answer, so the specs (`docs/specs/0001`–`0004`) can be written from it.

## Purpose

| # | Question | Decision |
|---|---|---|
| Q1 | Who is the desktop app for | Outside users of the open-source tool. Its one job: screen a river reach for a vessel and show the cut and dredge it needs. |
| Q2 | Why Rust | Distribution (one binary, no Python, GDAL or compiler), speed (lattice, DP re-route, DEM integration), and one stack with OVCD. ADR 0001. |
| Q3 | Fate of the Python | As OVCD: frozen as the reference during the port, then a thin PyO3 wrapper over the Rust core. ADR 0001. |
| Q4 | What "CLI chain" means | Subcommands that each read a channel design and write a richer one. Pipes allowed for small steps; a recipe runner is a thin wrapper. The CLI and app share one document. ADR 0004. |

## Conversion

| # | Question | Decision |
|---|---|---|
| Q5 | #9 and #10 before the freeze | Fix in Python first, then freeze. ADR 0002. |
| Q6 | The shared document | `.cdesign`, JSON with a schema: inputs plus each stage's results and input hashes (stale detection). DEM by path and hash; small geometry embedded. ADR 0004. |
| Q7 | Geodata input | Pure-Rust readers: GeoTIFF, GeoJSON, GeoPackage. WGS84 reprojected to UTM automatically. No GDAL. ADR 0003. |
| Q8 | Repository | This public repo becomes a Cargo workspace: `cd-core`, `cd-format`, `cd-cli`, `cd-py`, `app/`. The private history repo is retired. ADR 0001. |
| Q9 | OVCD crates | Git dependencies on `ovcd-core` (vessel classes, `Check`) and `ovcd-draw`, pinned to a tag. ADR 0001. |
| Q10 | Spec order | 0001 core port (rules, alignment, volumes, **optimise**) · 0002 `.cdesign` + CLI · 0003 first desktop app · 0004 drawings via `ovcd-draw`. |
| Q11 | Names of the lines | River centreline, design line, design line option, alignment (activity). "Route" retired as a noun. `GLOSSARY.md`. |
| Q12 | Freeze point | Tag `py-reference` once PR #16, the #9/#10 fixes and in-flight Python branches land. Python takes bug fixes only after that, each mirrored in the parity cases. #15 closed as superseded by spec 0004. |
| Q13 | Parity for searches | Two classes of check. ADR 0005. |
| Q14 | Real-DEM tests | Synthetic terrain in parity; a few GeoTIFF fixtures (one Copernicus clip) checked against rasterio to 1e-6 m. ADR 0005. |
| Q15 | Build budget | One Sonnet builder per spec, merging on green gates; mutation testing after merge. Coordinator stops at the `.cdesign` schema and before any public release. Stop and report at 30% of the weekly quota per spec. |

## First desktop app (spec 0003)

| # | Question | Decision |
|---|---|---|
| Q16 | Scope of v1 | Design a reach from the user's own data: load banks and DEM, choose ruleset and vessel, make design line options, compare, save. The first-pass river scan (#12, #13) is a later spec. |
| Q17 | Layout | Three panels as OVCD. Left: inputs in stage order with a status chip per stage (not run / fresh / stale). Main: plan map (banks, centreline, design line options, tight bends, chainage) **plus a long section and a cross-section viewer** (station slider linked to a map marker). Bottom: option comparison table and checks, failures first. |
| Q18 | Recompute | Sizing live (about a 150 ms pause). Alignment, volumes and optimise run from a Run button per stage, with progress and Cancel. Later stages go stale, never rerun silently. |
| Q19 | Editing the design line | View and choose between options only in v1. Hand editing (with live radius check and a "hand-edited" flag) is the first v2 candidate. |
| Q20 | Basemap | **Satellite imagery basemap from the start.** Provider: see Q21. |
| Q21 | Imagery provider | **Free only: EOX Sentinel-2 cloudless 2016** (layer `s2cloudless_3857`, public WMTS, no key, CC BY 4.0, about 10 m). Show the attribution "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)" on the map. Esri was considered and dropped: it needs an API key and is billed past 2M tiles a month, and a key built into an open-source app can be extracted. The 2018–2025 EOX layers are CC BY-NC-SA and stay out. Higher-resolution providers on the user's own key may come later. A short note to EOX (cloudless@eox.at) to confirm that an open app may use the public endpoint is still to be sent. |
| Q22 | Offline on site | Fall back to the DEM hillshade with an "imagery offline" note. Downloading imagery for a reach waits on the provider's caching terms. |
| Q23 | Exports in v1 | Design line options as GeoJSON (source CRS and WGS84), the quantity table as CSV, the `.cdesign`. DXF and sheets come with spec 0004. |
| Q24 | Python wrapper API | Keep today's functions and signatures through 0.x, so existing scripts keep working; deprecate with one release's notice. |
| Q25 | Installers | Unsigned pre-releases on GitHub Releases (macOS, Windows, Linux) until 1.0. Signing (Apple Developer account, Windows certificate) gates 1.0 and is a purchase decision for the maintainers. |
| Q26 | Failed checks | Show the option with a red check, failures sorted first; "fails: R_min" carried in the table and on every export. |
| Q27 | Screening label | A small "Screening — not for construction" status line in the app and on every export. |
