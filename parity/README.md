# Parity harness (spec 0001)

The Rust core is a port of the Python core with identical results. This folder is the proof. It is
deleted, with the frozen Python and the hidden `cdesign eval` command, when the port is complete
(ticket 37).

| Path | What it holds | Who writes it |
|---|---|---|
| `cases/<entry_point>.json` | Case inputs, one file per parity entry point | `gen_cases.py`, from `modules/<entry_point>.py` |
| `reference/<entry_point>.json` | Reference Results: the frozen Python's output for every case | `cargo xtask regen` only |
| `tolerance.json` | Every tolerance (ADR 0005), in one place | Never loosened |
| `ported.json` | Entry points the Rust dispatch has ported; each must be fully green | Port tickets add to it |
| `frozen_python/channel_designer_frozen/` | Frozen copy of the Python package (tag `py-reference`), under its own import name | Nobody. After the freeze it takes bug fixes only, each mirrored in the cases |
| `modules/<entry_point>.py` | One per entry point: `adapter(fn, args)` calls the frozen Python, `cases()` lists the cases | People |
| `common.py` | Encoding of floats, geometries and terrain samplers; seeded stream; near-tie rule | People |
| `driver.py` | Runs the cases through the frozen Python | |
| `coverage.py` | Prints the case count per entry point and tier | |

## Entry points

One per module of the frozen Python, named after the module (`cut_model` keeps the name it had
before the others were added; the line profiles are `terrain_profile`). `python3 -m parity.coverage`
prints the case count per entry point and tier.

Rules: `rulesets`, `methods`, `design`, `simple`, `width_profile`. Alignment: `radius_check`,
`loops`, `primitives`, `centreline`, `widths`, `limits`, `meanders`, `bends`, `rolling_circle`,
`route`. Volumes: `cut_model`, `cut_model_extras` (the rest of `cut_model.py`), `cut_model_grid`,
`sections`, `footprint`, `pools`, `raster`. Optimise: `cascade`, `lattice`. Others: `terrain_profile`,
`synthetic`, and `geometry` (the shapely operations the package uses, one function each; no Python
file of its own). Drawings are spec 0004's.

A case calls one public function by name, with its keyword arguments. Where the Python function takes
an object (a `CostGrid`, a `LevelTable`, a `Weir` list), the case carries the data and the module's
adapter builds the object. Where it takes a callable (a terrain sampler, a cost function), the case
carries a spec (see `common.py` and the module docstring). Public functions that only return a
callable, or only wrap another one, are tested through the function that uses them.

## Three tiers

1. **Named cases**, drawn from the Python tests and from the documented worked examples.
2. **A seeded sweep**, written with `random.Random(seed)` and only its `random()`, so the bytes never
   depend on the Python version. Each module has its own seed.
3. **Error cases**: the refusals the package makes (`ValueError`, `KeyError`, `IndexError`,
   `RuntimeError`). A refusal that is numpy's, shapely's, GEOS's or rasterio's own, with its own wording,
   is not provoked: its text is not ours to reproduce. A module with no refusal of its own has no tier 3
   and says why in `coverage.py` (`NO_ERROR_CASES`); a test holds the list to that.

## Case and result shapes

A case: `{"id": "section_area/named/knoll_model_b", "tier": 1, "reason": "...", "input": {"fn": "section_area", "args": {...}}}`.
`fn` names the Python function and `args` are its keyword arguments, by the Python parameter names.
Every case carries the reason it exists or last changed. A removed case moves to the file's `retired`
list, with a reason.

A Reference Result is `{"ok": <value>}`, or `{"error": {"type": "ValueError", "message": "..."}}`
when Python raises. Floats are shortest round-trip text. NaN and infinity are the strings
`"float:nan"`, `"float:inf"` and `"float:-inf"`, in inputs and in results. In a list of heights,
`null` (Python `None`) means "no ground" and reads as NaN. A dict with integer keys uses the keys
as text (`"1"`, `"-1"`). A shapely geometry is `{"geom": "LineString", "coords": [[x, y], ...]}` (also
`Point`, `Polygon` with `exterior` and `holes`, `MultiLineString`, `MultiPolygon`, `MultiPoint`,
`GeometryCollection`; an empty point, multipoint or collection is `{"geom": ..., "empty": true}`).
Polygon results are compared normalised (ADR 0005). A terrain sampler
is a spec: `synthetic_river`, `plane`, `hills`, `valley`, `wrong_shape`. Coordinates in case inputs are
rounded to the millimetre where a module says so (`rnd`): the rounded numbers are the case.

## Search cases

Four entry points are search class (ADR 0005): `rolling_circle`, `route`, `cascade`, `lattice`. Each
search case also carries a `search` record, written by the generator when it runs the frozen Python:

```
"search": {"what": "...", "decisions": 21, "best": 5372.1, "runner_up": 5312.7,
           "gap_rel": 0.011, "near_tie": false, "near_tie_rel": 1e-06}
```

It is the closest call the search made. The search is re-run with a tally of the runner-up at every
decision (the contact the rolling circle did not take, the DP predecessor the route did not take,
the arrival the cascade or A* did not take, the next-best ranked cascade, the next corridor, the
next goal state), and `best` and `runner_up` are the pair with the smallest relative gap.

**The flagging threshold** is `NEAR_TIE_REL = 1e-6` (in `common.py`): a case is a near tie when
`|best - runner_up| <= 1e-6 * max(|best|, 1)`. That is about a thousand times the floating-point
noise between two correct implementations. Below it, a different choice in the Rust is
order-of-operations noise, and the exact path is not compared. Above it, a different choice is a
port error. A case is never marked a tie after a failure.

How a search case is judged is in `crates/cd-format/src/search.rs` (ticket 28). A case whose record
says `near_tie: false`, or that has no record, is compared exactly like any other: the same path.
A case with `near_tie: true` is compared on its objective, within `search_objective_rel`, and on its
constraints, which still have to match the Reference Result; the path is not compared. Each search
function names its objective and constraints in `search::view` (rolling circle: the length of the
returned line; `radius_ok`, the measured radius and the chord). `cargo test` prints the near-tie
cases it judged this way, so a tie is reported, never counted as a pass or a failure unseen. A
separate test checks `radius_ok`, the measured radius and the chord on every rolling-circle case,
tie or not.

## What "identical" means

Deterministic floats match within `rel` and `abs` in `tolerance.json` (Python's `math.isclose`
rule). Everything else matches exactly: integers (an integer never matches a float), booleans,
text including messages, list order, object keys. Search outputs and polygons use the other
tolerances in the file, as ADR 0005 describes; the comparators for those arrive with the tickets
that need them. A knife-edge case is nudged or replaced, with the reason recorded on the case. A
tolerance is never loosened.

## Tasks

```
cargo test                    # parity seam: every case of every ported entry point, in-process
cargo xtask regen             # rewrite reference/ from the frozen Python (needs Python + numpy + shapely)
cargo xtask oracle-guard      # what the fast gate runs; see below
cargo xtask mutants           # cargo-mutants on the cd-core code this branch changed
python3 -m parity.gen_cases   # rewrite cases/ (deterministic); then run regen
python3 -m parity.coverage    # case count per entry point and tier
```

`CD_PYTHON` picks the Python for `regen` and `oracle-guard` (default `python3`).
`cargo test` needs no Python. The `cd-cli` test also runs a few cases through `cdesign eval`.

To add or change a case: edit its module in `modules/`, run `gen_cases`, run `cargo xtask regen`, and
give the new or changed case a `reason` (a case that did not change keeps its old one). The same machine writes the same bytes every time, but another machine (macOS against Linux,
or another CPU) rounds the last bits of a float differently, so the oracle guard re-runs the
frozen Python and compares by the parity tolerance, not by bytes.

## Oracle guard

The fast gate fails a pull request that changes `cases/`, `reference/`, `tolerance.json` or
`ported.json`, unless:

- re-running the frozen Python reproduces every committed Reference Result within the parity
  tolerance (`rel` and `abs`; never above their ceilings): floats within it, everything else exact
  (so the change came from `cargo xtask regen`, not a hand edit; a hand edit larger than the
  tolerance fails);
- every new case, and every case whose input or Reference Result changed, carries a new `reason`;
- every removed case is in `retired` with a reason;
- no tolerance is loosened (each has a ceiling written into the guard), and a tightened tolerance
  has a new reason;
- no entry point leaves `ported.json`.

Changing a tolerance or the guard itself stops the build: the builder reports to the coordinator.

## Mutation test

`cargo xtask mutants` runs `cargo-mutants` on the `cd-core` lines this branch changed against
`origin/main`, with every workspace test (so the parity test does the killing), and writes a short
killed / survived / timeout report to `target/mutants-report.md`. It runs after merge and does not
block. A surviving mutant that changes a public output becomes a fix-forward ticket; the fix adds a
parity case, regenerated, with the reason "kills mutant ...".
