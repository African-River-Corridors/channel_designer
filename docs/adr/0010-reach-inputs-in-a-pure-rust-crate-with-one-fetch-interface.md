---
status: accepted
---
# Reach inputs are made in a pure-Rust crate, with network access behind one interface

Spec 0012 makes every input of a study from open data: a route on the OSM waterway graph, bank lines
from the JRC water mask moved onto Sentinel-2, a Copernicus DEM, and a water level from SWORD and
SWOT. This code lives in a new crate, `owd-reach`. It is pure Rust, and it builds for native and for
`wasm32-unknown-unknown`.

## Decision

- **`owd-reach` holds all the generation logic:** routing, mask to banks, the edge fit, the water
  level, the bed model's defaults and the provenance of each input. `owd-core` stays free of file,
  clock and network access, as ADR 0009 keeps it.
- **Network access is one interface,** a `Fetch` trait (get a URL, get a byte range). The CLI and
  desktop give it a native HTTP client. The web gives it the browser's `fetch`, run in the session's
  Worker. The logic never knows which.
- **A source the browser cannot read** (no CORS: the Copernicus DEM, the JRC mask, WorldCover) is
  never proxied. On the web, `owd-reach` returns the list of files the user must download, and reads
  them once dropped in. No server sees a study.
- **Every source has one entry** in a source registry: its URL pattern, its licence and the notice
  it requires. Every generated input records the entry, the date and the default mark, so the
  notice travels into every export.
- **No Python and no GDAL,** as ADR 0001 and ADR 0003 decide. Methods from earlier Python work are
  ported, not called.

## Considered

- **Python scripts** (the earlier edge fit is Python). Rejected: the web app cannot run them, and the
  core has no Python since ADR 0001.
- **Inside `owd-format`.** Rejected: it would bring network code into the document crate and into
  every build that reads a file.
- **A small proxy server** for the sources without CORS. Rejected by the maintainer: no server.

## Consequences

- The web and native paths run the same code, so the web parity check (spec 0011) can cover
  generation on recorded fixtures.
- Raster work on large tiles (JRC is 10° by 10°) must read by window, or in the browser it runs out
  of memory. Each source reader proves this on a fixture.
- A new source is a registry entry and a reader. Its licence is checked before it lands.
