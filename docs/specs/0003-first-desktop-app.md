# Spec 0003: First desktop app

Status: **approved by the maintainer, 2026-10-16.** Decisions: ADR 0001 and 0004, and
`docs/design/2026-10-15-rust-port-grill.md` (Q1, Q16–Q27). Builds on spec 0002: the channel design
document and the design session.

## Problem statement

After spec 0002, a channel design can be run from the command line. The users this tool is for are
outside engineers who screen rivers but do not use a terminal (Q1). To them, a list of volumes is
not a design. They need to see the river, the design line options, where the cut is deep, and which
option fails a check, on one screen. The app has one job: **screen a river reach for a vessel and
show what it costs in cut and dredge.**

## Solution

A Tauri desktop app with three panels, in the same layout as the open-vessel-concept-drawings app
(Q17):

- **Left panel: inputs, in stage order.**
  - Data: bank lines, river polygon and DEM, with the option to fetch a Copernicus DEM.
  - Vessel and ruleset, with the resulting channel shape shown under them.
  - Alignment settings, optimise settings and the cut model.
  - Each stage has a status chip: *not run*, *fresh* or *stale*.
- **Main panel.** A **plan map** on a satellite basemap, showing the banks, the river centreline,
  the design line options in distinct colours, tight bends and chainage ticks. Under it, a **long
  section**: bed, water level and pools, design invert and cut depth along chainage. Beside the long
  section, a **cross-section viewer** driven by a station slider that also moves a marker on the map.
- **Bottom panel.** An **option comparison table**: dredge, dry cut, tallest face, minimum radius,
  and radius ok. Beside it, a **checks list**, with failures first.

The app holds no calculation, validation or default logic. Everything comes from the design session
(spec 0002), and a file saved by the app gives the same results with `cdesign run`.

## User stories

### Files
1. As an engineer, I want to start a new channel design, or open an existing `.cdesign`, so that I can
   pick up a study made in the app or on the command line.
2. As an engineer, I want to save to `.cdesign`, and have the file pass `cdesign validate`, so that the
   app and the command line agree.
3. As an engineer, I want a warning before I close a channel design with unsaved changes, so that I
   don't lose work.
4. As an engineer, I want a warning when an opened file's DEM is missing or has changed, so that I know
   the volumes may not hold.

### Data and inputs
5. As an engineer, I want to load bank lines, a river polygon and a DEM by file dialog or drag-and-drop,
   so that loading data is quick.
6. As an engineer, I want WGS84 data accepted, with the UTM zone it was moved to shown, so that I don't
   need a GIS first.
7. As an engineer, I want to fetch a Copernicus DEM for my reach with one button, so that I can start
   with no DEM of my own.
8. As an engineer, I want to choose a ruleset and a vessel class from lists, and see the bottom width,
   R_min and design depth update as I type, so that I see the channel before I run anything.

### Running stages
9. As an engineer, I want to run alignment, optimise and volumes with a Run button per stage, with
   progress and Cancel, so that a long run never locks the app.
10. As an engineer, I want a stage marked stale, and never rerun silently, when I change something it
    depends on, so that the numbers on screen are always ones I ran.

### Viewing and comparing
11. As an engineer, I want the design line options drawn on the satellite map, each in its own colour,
    with tight bends marked, so that I can see where each one goes.
12. As an engineer, I want to pick an option in the table and see it highlighted on the map and in the
    long section, so that I can follow one option across the views.
13. As an engineer, I want a long section of bed, water level, pools, design invert and cut depth, so
    that I can see where the cut is deep.
14. As an engineer, I want a cross-section at any station, picked with a slider or by clicking the map,
    so that I can see the cut where it matters.
15. As an engineer, I want an option that fails a check shown with a red mark and sorted first, with
    its numbers still visible, so that I never mistake a failure for a pass.
16. As an engineer, I want the map to show the DEM hillshade and an "imagery offline" note when there
    is no network, so that the app still works on site.

### Exports
17. As an engineer, I want to export design line options as GeoJSON and the quantity table as CSV,
    carrying the screening label and any failed checks, so that I can share results.

## Implementation decisions

### Modules
- **Design session** (spec 0002). This is the only interface to the engine. The app adds no
  engine-side module.
- **App shell** (a new Tauri crate, `app/src-tauri`, not a default workspace member). It is thin:
  Tauri commands that forward to the design session, plus file dialogs, drag-and-drop, the
  unsaved-changes guard, and running a stage on a worker thread with progress events and Cancel.
- **Front end** (`app/`). Svelte and TypeScript on Vite, as in the OVCD app. It holds display state
  only: the selected option, the station, panel sizes and map view.
  - **The plan map** uses MapLibre GL JS. Geometry is sent to the map in WGS84,
    converted by the session.
  - **The long section** uses uPlot, as OVCD does.
  - **The cross-section** is plain SVG, drawn from the station's profile and cut faces that the
    session returns.

### Basemap (Q20–Q22)
- The basemap is **EOX Sentinel-2 cloudless 2016** (layer `s2cloudless_3857`), from the public WMTS
  endpoint, with no key needed. Its attribution is always visible:
  "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus
  Sentinel data 2016)".
- The 2018–2025 EOX layers are not offered, because their licence is non-commercial.
- **Offline:** when the tiles fail to load, the map shows a hillshade of the channel design's DEM,
  rendered once by the Rust side, with an "imagery offline" note. Imagery is never downloaded for
  offline use in v1.

### Behaviour
- **Sizing recomputes live**, after about a 150 ms pause in typing (Q18). Alignment, optimise and
  volumes run only from their Run buttons. An edit marks the later stages stale; it never reruns
  them.
- **Design lines are view and select only** (Q19). There is no hand editing in v1.
- **A failed check** shows a red mark in the table, and the option sorts first. Its numbers stay
  visible (Q26).
- **A status line** reads "Screening — not for construction". It is always visible, and every export
  carries it (Q27).
- **Units** are SI, as stored, with one display rounding rule per quantity.
- **Platforms:** unsigned pre-release builds for macOS, Windows and Linux on GitHub Releases (Q25).
  Signing is a 1.0 gate and the maintainer's purchase.

## Testing decisions

- **A good test** checks what a user sees or an agent can read: a channel design in; the panels,
  table and exports out.
- **The session is already tested in spec 0002.** This spec adds only the session calls the app
  needs: map geometry in WGS84, long-section series, the cross-section at a station, and the
  hillshade. Each gets Rust tests on the synthetic river.
- **End-to-end smoke tests** on the built app, in CI on Linux through Tauri's WebDriver support:
  1. Open the worked-example file.
  2. Change the vessel and see the channel shape change.
  3. Run alignment, then volumes, and see the table fill.
  4. See a failed option sorted first.
  5. Move the station slider and see the cross-section change.
  6. Save, reopen, and get the same table.
  7. Check that the exports carry the screening label.

  There are no per-component tests.
- **The basemap** has one test that the attribution is visible, and one that the offline hillshade
  appears when tile requests fail.

## Out of scope

- Drawings and DXF (spec 0004). Hand-editing a design line.
- The river scan, imagery analysis and settlements (#12, #13).
- Higher-resolution imagery on a user's own key, and imagery download for offline use.
- Signed installers, auto-update, translations and units other than SI.
- Any change to engineering results.

## Resolved points (maintainer, 2026-10-16)

1. **Map library:** MapLibre GL JS.
2. **First test file:** the synthetic worked example. A real river comes with the open-DEM example (#1).
