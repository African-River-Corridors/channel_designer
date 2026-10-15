---
status: accepted
---
# A Rust core with a CLI, a desktop app and a thin Python wrapper

channel_designer moves from a Python package to a Rust core, so that outside users get one
signed binary with no Python, GDAL or compiler to install (#11). The heavy searches (pool lattice,
least-cost re-route, DEM integration) also get faster, and the tool shares one stack with
open-vessel-concept-drawings (OVCD). The Rust core is the only engine. A CLI and a Tauri desktop
app are two views of it, and the Python package becomes a thin PyO3 wrapper over it, so existing
scripts keep working.

## Considered options

- **Rust hot paths behind Python (PyO3 only).** Rejected: it speeds things up but leaves the
  install problem, which is the main reason for the change.
- **A new repository.** Rejected: the issues, roadmap, users and the `channel_designer` package
  name stay in this one. This repository becomes a Cargo workspace: `cd-core`, `cd-format`,
  `cd-cli`, `cd-py` and `app/`. The private history repository is retired.

## Consequences

- Vessel classes and the `Check` type come from OVCD's `ovcd-core`, and drawings from
  `ovcd-draw`. Both are git dependencies pinned to a tag, never a branch.
