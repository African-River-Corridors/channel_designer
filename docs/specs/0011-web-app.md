# Spec 0011: The OWD web app

Status: **approved to build by the maintainer, 2026-10-21.** Going live is a separate step that the
maintainer approves (see "Go live").

## Problem statement

OWD runs only as a desktop app, a command line and a Python package. A person who wants to try it must
build it first. The sibling tool OVCD runs in the browser at `africanrivercorridors.com/ovcd/app`. OWD
needs the same at `africanrivercorridors.com/owd/app`: the same Svelte front end, over the same Rust
core compiled to WebAssembly. No server sees a design.

## Solution

Copy the OVCD pattern (its issues 245 and 246).

- **Shared session crate.** `app/src-tauri/src/state.rs` and `run.rs` hold the app's state and the
  stage runner. They have no Tauri types. They move, unchanged, to `crates/owd-shell`. The Tauri app
  uses that crate. The tests move with the code.
- **Wasm crate.** `crates/owd-web` (cdylib) is a thin `wasm-bindgen` layer over `owd_shell::AppState`.
  One `Session` object, one method per Tauri command that needs no dialog. Values go in and out as the
  same JSON the Tauri commands return. Errors are thrown as plain strings. The `wasm-bindgen` crate is
  pinned with `=x.y.z`; `tools/build_web.sh` checks that the CLI has the same version.
- **Transport split.** `app/src/lib/api.ts` keeps its exports. Under it, `app/src/lib/platform/` holds
  two adapters: Tauri `invoke`, and the web adapter over a Web Worker that holds the wasm session. The
  build picks one at compile time (`__OWD_WEB__`), so the desktop bundle has no WebAssembly and the web
  bundle has no Tauri code. `npm run build:web` builds the web bundle with `app/vite.web.config.ts`.
- **Parity.** `tools/web_parity.mjs` runs every web fixture through the wasm session and through native
  `owd run`, and compares the two documents by the ADR 0005 tolerance. Never bytes.
- **CI.** Two new jobs, `web-parity` and `web-app`. Neither is a required check. Neither runs in the
  `test` job. The `wasm-bindgen` CLI is cached.
- **Publish.** `website/build.py --app-dist app/dist-web` puts the page at `website/app.html` (served
  as `/owd/app`) and its files under `website/app-files/` (served as `/owd/app-files/`), as spec 0010
  fixed. The website workflow builds the wasm and the app first.

## What was found first (feasibility)

Checked on 2026-10-21 with `cargo build --target wasm32-unknown-unknown`.

| Part | Result | What we do |
|---|---|---|
| `owd-core` (Dubins joins, `clarabel` QP solver, libm maths) | Builds as it is. `clarabel` is pure Rust and needs no threads. No `rayon` anywhere. | Nothing. |
| `owd-format`: GeoTIFF (`tiff`, deflate, LZW), PNG, `sha2`, `serde_json` | Builds. | Nothing. |
| `owd-format`: `proj4rs` | Links fail: its default `aeqd` feature compiles C geodesic code that needs libc maths. OWD uses only UTM. | `default-features = false`. Native results do not change. |
| `owd-format`: `rusqlite` (GeoPackage reader) | Builds on wasm through `sqlite-wasm-rs`. It compiles C, so the build needs a clang that targets wasm32 (Ubuntu's does). | Keep. Tested in the parity run. |
| `std::time::Instant` in the pool lattice search | Compiles, then **panics at run time** on `wasm32-unknown-unknown`. | Use `web-time` on wasm only. |
| Cancel (`run.rs` unwinds out of the stage) | Wasm is built with `panic=abort`: unwinding is not possible. | Run the session in a Worker. Cancel ends the Worker and restores the design (see below). |
| `std::fs`, `SystemTime` in the CLI seam | Compile; fail only if called. The web session never calls them. | Nothing. |
| `fetch_dem` (network, writes files beside the saved design) | Native only. | Off on the web (see below). |

## Web behaviour

Same screen, same results. Where a desktop dialog or file write was, the browser has its own way.

- **Open, Load input, Load DEM.** The browser file picker. A DEM is a GeoTIFF the person picks. An `.owd`
  file names its DEM by path; on the web the person opens the `.owd`, then loads the DEM. The hash check
  and its warning are the desktop's.
- **Save, Save as, Export.** A download.
- **Bundled example.** An "Open the example" button loads a small synthetic river and its DEM from
  `/owd/app-files/examples/`. A visitor with no data can run a design at once.
- **Fetch DEM** is not offered on the web. It needs a saved folder and a network path that is not
  proved for a browser. A person loads a DEM instead.
- **Run in a Worker.** The page stays alive during a long stage. Progress is the number of terrain reads,
  as on the desktop. Cancel ends the Worker and starts a new one from the design as it was before the
  run, so the rule "a cancelled run leaves the design as it was" holds.
- **Last design.** The page keeps the last design in `localStorage` (guarded: storage may be blocked) so
  a reload brings it back. The DEM is not kept.
- **Desktop-only items** (the unsaved-changes guard on window close, the native dialogs) have a browser
  form: `beforeunload`, `window.confirm`.

## Parity

For each fixture, the wasm session opens the document and its DEM, runs every stale stage, and gives the
document text. Native `owd run` does the same on the same files. The two documents are compared by the
ADR 0005 tolerance (1e-9 relative and absolute for numbers, equality for everything else). Search
outputs follow ADR 0005 the same way they do against Python. Fixtures live in `examples/web/`: the
synthetic river as it ships, and the same river with its DEM and with water levels, so sizing,
alignment, optimise and volumes each run.

## Tickets

Each is a sub-issue of the spec issue (#211). Blocking uses native links.

1. **T1 (#212): owd-shell and owd-web.** Extract the shared crate. The wasm crate, the pin, `build_web.sh`,
   the `proj4rs` and `web-time` fixes, the wasm build in `tools/gates.sh`.
2. **T2 (#213): transport split.** Platform adapters, Worker, `vite.web.config.ts`, `build:web`, the web UI
   differences above. Blocked by T1.
3. **T3 (#214): parity.** `tools/web_parity.mjs` and the fixtures. Blocked by T1.
4. **T4 (#215): web smoke test.** `app/e2e/run-web.sh`, `web-server.mjs`, `web-smoke.test.mjs` in headless Chrome.
   Blocked by T2 and T3.
5. **T5 (#216): CI jobs.** `web-parity` and `web-app`. Blocked by T3 and T4.
6. **T6 (#217): publish in the website build (go live).** Blocked by T2 and T5.

## Go live

T6 is the one change that publishes. When it merges, `africanrivercorridors.com/owd/app` is live. The
maintainer approves that merge. T6 also edits the home page: a "Web app" link, and the sentences that
say there is no web app and that terrain and design stay on the computer, so every claim stays true of
the code.

## Out of scope

- Fetching a DEM in the browser.
- Drawings and PDF in the browser (spec 0004 decides their web form).
- Saving a design to a server, accounts, sharing.
- Any change to results. The design-impact check stays clean.
- `app/src/tokens.css`.

## Acceptance

- `cargo build --target wasm32-unknown-unknown -p owd-web` builds; the pinned versions are equal.
- Parity passes for every fixture.
- The desktop build, tests and e2e are unchanged.
- The smoke test in headless Chrome loads a fixture, runs it, changes an input and sees a result move.
- `test` is not slower and no new check is required.
