# Spec 0002: The channel design document, the CLI chain and WGS84 input

Status: **approved by the maintainer, 2026-10-16.** Decisions: `GLOSSARY.md`, ADR 0001,
0003 and 0004, and `docs/design/2026-10-15-rust-port-grill.md` (Q4, Q6, Q7, Q21–Q23, Q26, Q27, T4).
Builds on spec 0001 (#20): the Rust core with Python parity.

## Problem statement

After spec 0001 the engine is a Rust library and a Python package. A river engineer who does not
write code still cannot run a screening study. There is no file that holds a study, no command to
run one, and every input must already be in a projected metric CRS. Most first rivers arrive as a
WGS84 trace and a geographic DEM such as Copernicus GLO-30. The desktop app (spec 0003) also needs
one place that holds a study and runs its stages, so that the app and the command line give the same
answers.

## Solution

1. **A channel design document** (`.cdesign`, JSON, with a published JSON Schema; ADR 0004). It holds
   the inputs, each stage's settings and results, and per stage a hash of what it depended on, so an
   edit marks the later stages **stale**. Small geometry is embedded as GeoJSON. The DEM is referenced
   by path and content hash, never embedded.
2. **A design session** in `cd-format`: the one interface to a channel design. The CLI and the app
   both use it. It holds no file, clock, network or screen access.
3. **The `cdesign` CLI chain.** Each subcommand reads a channel design, runs one stage and writes the
   richer document back:
   `new → import → size → align → optimise → volumes`, plus `status`, `validate`, `run`, `export`,
   `schema` and `fetch-dem`.
4. **WGS84 input** (ADR 0003). WGS84 vectors are reprojected to the right UTM zone on import. A
   geographic DEM is sampled where it is, with no resampling to a new grid.
5. **`cdesign fetch-dem`** downloads Copernicus GLO-30 tiles for a channel design's extent (T4).
   It is the only network code, and it lives in the CLI, never in the core.

This spec changes no engineering results. Every stage calls the core that spec 0001 proved
against the Python.

## User stories

1. As a river engineer, I want to start a channel design with one command and a title, so that I
   have one file for the study.
2. As a river engineer, I want to import two bank lines, a river polygon and a DEM from GeoJSON,
   GeoPackage or GeoTIFF, so that I can use the data I have.
3. As a river engineer, I want WGS84 input reprojected to the right UTM zone for me, and the zone
   recorded in the file, so that I don't have to run a GIS first.
4. As a river engineer, I want a clear message naming the CRS I gave and what to convert it to when
   the tool cannot use it, so that I know how to fix my data.
5. As a river engineer, I want to fetch a Copernicus GLO-30 DEM for my reach with one command, so that
   I can start without a DEM of my own.
6. As a river engineer, I want to choose a ruleset, a vessel class (or my own vessel dimensions) and a
   draught, and see the channel shape at once, so that I know the channel I am designing.
7. As a river engineer, I want to generate design line options by rolling circle, least-cost re-route
   and the pool lattice, each kept as a separate option, so that I can compare them.
8. As a river engineer, I want dredge, dry cut, tallest face and minimum radius for every option, so
   that I can compare them on quantities.
9. As a river engineer, I want an option that fails a check (for example "fails: R_min") to keep its
   numbers but be marked as failing everywhere, including in exports, so that I never quote it as a
   pass.
10. As a river engineer, I want a stage to show as stale when something it depends on has changed,
    and never be rerun without my asking, so that the numbers in the file are always ones I chose to run.
11. As a river engineer, I want `cdesign run` to rerun every stale stage in order, so that I can bring
    a file up to date in one command.
12. As a river engineer, I want to export design line options as GeoJSON (in the project CRS and in
    WGS84) and the quantity table as CSV, so that I can use them in a GIS or a spreadsheet.
13. As a river engineer, I want every export to carry "Screening — not for construction", so that a
    file that leaves my hands says what it is.
14. As a river engineer, I want a warning when I open a file whose DEM has changed or moved since its
    hash was recorded, so that I know its volumes may no longer hold.
15. As an agent or a script, I want every command to take `--json` and print a machine-readable
    result, and small steps to read and write `-` for stdin and stdout, so that I can chain commands.
16. As an agent, I want `cdesign schema` to print the document's JSON Schema, so that I can write or
    check a file without reading the source.
17. As a maintainer, I want a saved file to give the same results when it is run again, so that a
    file is a reproducible record.

## Implementation decisions

### The channel design document (`.cdesign`)

- JSON, UTF-8, with a top-level `format_version` (starts at 1) and `engine_version`. A JSON Schema
  (2020-12) is published in the repo and printed by `cdesign schema`.
- **Project:** title, created date, the project CRS (an EPSG code; WGS84 UTM in v1), and the source
  CRS each input came in.
- **Inputs:** left and right bank lines, or a river centreline; the river polygon (all GeoJSON in the
  project CRS); the DEM reference: path relative to the document, SHA-256 of the file, its CRS and
  nodata value; the **water level**.
- **Water level** (ADR 0006): a profile along the river centreline, a table of points
  (river chainage in metres, level in metres), read linearly between points and held flat beyond the
  first and last. Two points at one chainage make a step, as at a weir. It is set by hand or import
  (a constant, a straight fall, or a table; `cdesign import --water-level/--water-slope/--water-table`).
  A design line's station takes its level by projecting onto the river centreline and reading the
  profile at that chainage, so every stage (the re-route, optimise and volumes) sees the same water,
  and two design lines through the same place see the same level. When the optimise stage has run, its
  pool levels (a step at each weir) are the profile the volumes stage reads; the results record which.
  Editing the weirs marks the volumes stale. The document's `format_version` stays 1: `inputs.water_level`
  is an optional addition, and a document made before it, with a start level and a fall in the stage
  settings, is converted on open to a two-point profile; the stages that held those settings then show
  as stale, because their results came from the old model.
- **Stages,** in order: `sizing`, `alignment`, `optimise`, `volumes`. Each stage holds its settings,
  its results, the hash of everything it depended on (inputs, earlier stages' results, settings),
  and the engine version that ran it. A stage's status is worked out on load, never stored:
  *not run*, *fresh* or *stale*.
- **Design line options:** the alignment and optimise stages add options. Each option has an id, a
  name, the method and settings that made it, its line, and its checks (`radius_ok`, measured minimum
  radius, the chord it was read on). The volumes stage gives each option its quantities.
- Numbers are stored in SI units, as the core works in them. Floats are written as shortest
  round-trip text, so a saved file reloads bit for bit.

### The design session (in `cd-format`)

The one interface between the engine and its users, whether the CLI or the app. It takes and returns
strings and values. The callers do all file, network and clock access.

- *new*: an empty channel design with a title and defaults.
- *open*: parse and validate a `.cdesign` text, and report each stage's status and any DEM hash
  mismatch. The caller supplies the DEM bytes.
- *edit*: set inputs and settings. Each edit returns the new state, and marks the stages it touches
  stale.
- *run stage*: run one stage. It reports progress, and the caller can cancel it. A stage whose
  inputs are stale refuses to run and names the stage to run first.
- *status*, *validate*, *schema*, *save* (the document text) and *export* (GeoJSON and CSV text).

### The CLI (`cdesign`, crate `cd-cli`)

- **Subcommands:**
  - `new`, `import`, `size`, `align --method rolling-circle|reroute`, `optimise`, `volumes`
  - `run`, `status`, `validate`
  - `export --geojson|--csv`, `schema`, `fetch-dem`
- Each stage subcommand reads a file, runs its stage and writes the file in place, unless `-o` is
  given. `-` reads from stdin or writes to stdout.
- `--json` prints a machine-readable result.
- Exit codes are 0 for success, 1 for a failed check or a stale-stage refusal, and 2 for a usage or
  input error.
- The hidden `eval` command from spec 0001 is removed in #37, not here.

### Geodata (ADR 0003)

- **Readers:** GeoJSON and GeoPackage vectors; GeoTIFF DEMs (the reader from spec 0001 #33). All are
  pure Rust, except that GeoPackage needs SQLite, which is compiled into the binary. No system
  library is needed at run time.
- **CRS rules for v1:**
  - WGS84 (EPSG:4326) is reprojected to WGS84 / UTM. The zone and hemisphere come from the centroid
    of the bank lines, and the chosen EPSG is recorded.
  - WGS84 / UTM (EPSG:326xx and 327xx) is used as it is.
  - Anything else is refused, with a message naming the CRS and what to convert to.
- **A geographic DEM is not warped.** The terrain sampler converts each query point from the project
  CRS to the DEM's CRS and samples there (bilinear, as the core does). The DEM file stays untouched.
- **Reprojection** uses a maintained pure-Rust library, for example `proj4rs`; the build ticket
  confirms the choice. It is tested against PROJ's published test points to 1 mm.

### `cdesign fetch-dem`

- Downloads the Copernicus GLO-30 tiles that cover the river polygon's extent plus a margin, from
  the public open-data mirror. It writes them next to the document and records their hashes and
  licence notice.
- It is the only command that uses the network. It refuses to overwrite a file whose hash differs
  from the one recorded.
- FABDEM is not offered, because its licence is non-commercial.

### Checks and labels

- A failed check stays with its option through results, `status` and exports: a column in the CSV,
  and a property in the GeoJSON (Q26).
- Every export carries "Screening — not for construction": a CSV header line, and a GeoJSON
  property (Q27).

## Testing decisions

- **A good test** puts a channel design in and checks the document, messages and exports that come
  out, not how the session stores state.
- **Session tests in Rust:**
  - For both worked examples, running every stage through the session gives the same results as
    the core functions called directly in spec 0001's end-to-end test.
  - Save, then open, gives the same document.
  - Each edit marks exactly the expected stages stale.
  - A stale stage refuses to run.
  - A failed check appears in every export.
- **CLI tests:** each subcommand on a small synthetic river. The full chain `new → … → volumes`
  gives the same file as `cdesign run` on the same inputs. `schema` equals the published schema.
- **Geodata:**
  - Reprojection matches PROJ's test points to 1 mm.
  - A WGS84 synthetic river imported and run gives the same volumes, to 1e-6 relative, as the same
    river given in UTM.
  - A geographic DEM sampled through the converter matches rasterio's sampling of a warped copy at
    test points, within the warp's own interpolation error.
- **`fetch-dem`** is tested against a recorded response, with no network in CI. One manual check
  downloads a real tile.

## Out of scope

- The desktop app (spec 0003). Drawings and DXF (spec 0004).
- Shapefile, KML and other formats; CRSs other than WGS84 and WGS84 / UTM; reprojecting a DEM to a
  new grid.
- Hand-editing a design line; settlement detection and the river scan (#12, #13).
- Any change to engineering results.

## Resolved points (maintainer, 2026-10-16)

1. **GeoPackage:** yes, with SQLite compiled into the binary.
2. **CRS scope for v1:** WGS84 and WGS84 / UTM only. Other CRSs are refused with a message.
