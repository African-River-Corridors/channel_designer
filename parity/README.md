# Parity harness (spec 0001)

The Rust core is a port of the Python core with identical results. This folder is the proof. It is
deleted, with the frozen Python and the hidden `cdesign eval` command, when the port is complete
(ticket 37).

| Path | What it holds | Who writes it |
|---|---|---|
| `cases/<entry_point>.json` | Case inputs, one file per parity entry point | `gen_cases.py` (named cases, a seeded sweep, error cases) |
| `reference/<entry_point>.json` | Reference Results: the frozen Python's output for every case | `cargo xtask regen` only |
| `tolerance.json` | Every tolerance (ADR 0005), in one place | Never loosened |
| `ported.json` | Entry points the Rust dispatch has ported; each must be fully green | Port tickets add to it |
| `frozen_python/channel_designer_frozen/` | Frozen copy of the Python package (tag `py-reference`), under its own import name | Nobody. After the freeze it takes bug fixes only, each mirrored in the cases |
| `driver.py` | Runs the cases through the frozen Python | |

## Case and result shapes

A case: `{"id": "section_area/named/knoll_model_b", "tier": 1, "reason": "...", "input": {"fn": "section_area", "args": {...}}}`.
`fn` names the Python function and `args` are its keyword arguments, by the Python parameter names.
Tier 1 is a named case, tier 2 a seeded sweep point, tier 3 an error case. Every case carries the
reason it exists or last changed. A removed case moves to the file's `retired` list, with a reason.

A Reference Result is `{"ok": <value>}`, or `{"error": {"type": "ValueError", "message": "..."}}`
when Python raises. Floats are shortest round-trip text. NaN and infinity are the strings
`"float:nan"`, `"float:inf"` and `"float:-inf"`, in inputs and in results. In a list of heights,
`null` (Python `None`) means "no ground" and reads as NaN. A dict with integer keys uses the keys
as text (`"1"`, `"-1"`).

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
python3 parity/gen_cases.py   # rewrite cases/ (deterministic); then run regen
```

`CD_PYTHON` picks the Python for `regen` and `oracle-guard` (default `python3`).
`cargo test` needs no Python. The `cd-cli` test also runs a few cases through `cdesign eval`.

To add or change a case: edit `gen_cases.py`, run it, run `cargo xtask regen`, and give the new
or changed case a `reason`. Reference Results come out byte for byte the same on any machine with
the same numpy and shapely, so the oracle guard re-runs the frozen Python to check them.

## Oracle guard

The fast gate fails a pull request that changes `cases/`, `reference/`, `tolerance.json` or
`ported.json`, unless:

- re-running the frozen Python reproduces every committed Reference Result byte for byte (so the
  change came from `cargo xtask regen`, not a hand edit);
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
