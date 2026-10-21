# Contributing to Open Waterway Designer

Thank you for helping. Bug reports, fixes, worked examples, new rulesets and new features are all welcome.

## Set up

Python 3.12 or later. With [uv](https://docs.astral.sh/uv/):

```
git clone https://github.com/African-River-Corridors/open-waterway-designer
cd open-waterway-designer
uv venv
uv pip install -e '.[dev]'
```

Or with plain pip: `python -m venv .venv && .venv/bin/pip install -e '.[dev]'`.

## Run the tests

```
.venv/bin/python -m pytest
python tools/release_guard.py
```

Both must pass (installing from source builds the Rust core, so it needs a Rust toolchain: rustup.rs). CI runs them on every push and pull request.

## Rules for the code

- `rules`, `alignment` and `volumes` are calculation code: numpy and shapely only, no file I/O.
  Paths, CRS and DEMs come in as arguments. `tests/test_layer_boundary.py` enforces this.
- Heavy dependencies stay optional: raster I/O lives behind the `[dem]` extra.
- Every new calculation comes with tests: a golden case (a worked number with its source), a property
  (an invariant), and a feasibility check (the checks fire when they should). Tests use synthetic
  geometry built in code — never project data.
- Cite the source of every number: standard, clause or table, and page. Quote coefficients as data,
  not standards prose.
- A project choice is not a standard. Name it as a user choice (for example `BANK_CUT_FRACTION`), and
  never present it as a default from a standard.

## The design-impact check

The calculations run in a Rust core (`crates/`); the Python package is a thin wrapper over it. The port
from the old pure-Python package is finished, and the scaffolding that held the two together (Reference
Results, case files, the frozen Python, the hidden `eval` command, the oracle guard) is gone. The old
package stays reachable as the git tag `py-reference`.

What guards a change now is the **design-impact check**, `design-impact` in CI
(`tools/design_impact.py`). On every pull request it:

1. builds the base branch and your branch;
2. runs fixed scenarios on both: the two worked examples, every ruleset on every vessel class, a size
   and design run, a channel footprint and a Cut Model A run (all synthetic, built in code);
3. compares every number by the ADR 0005 tolerance (1e-9 relative; counts and names exactly).

If nothing moved it says **no design impact** and passes. If a result moved, it lists what moved (scenario,
result, base, your branch, change) and passes only when your pull request adds a bullet under
`## Unreleased` > `### Design impact` in `CHANGELOG.md` saying what moved and why. A moved result with no
entry fails. You can run it yourself:

```
python tools/design_impact.py run --out head.json     # on your branch
python tools/design_impact.py run --out base.json     # on main, in another checkout
python tools/design_impact.py compare base.json head.json --changelog CHANGELOG.md
```

A result that moves is not wrong by itself (a corrected coefficient moves volumes), but it must be
deliberate and written down. Never loosen the tolerance to make a number pass; it lives in one place,
`tools/design_impact.py`, and changing it needs a review.

Other guards: `bash tools/gates.sh` (format, clippy with warnings as errors, Rust tests with Python made
unusable, pytest) and `python tools/release_guard.py` (no embargoed terms or local paths).

## Pull requests

1. Open an issue first for anything bigger than a small fix, so we can agree the approach.
2. Fork, branch from `main`, keep the change focused.
3. Make sure tests and the release guard pass.
4. Open a pull request using the template. Say what changed and how you checked it.

Every pull request from outside the maintainers needs approval from a code owner
(see `.github/CODEOWNERS`) before it merges.

## Licence of contributions

This project is licensed under Apache-2.0. Unless you say otherwise, any contribution you
intentionally submit is licensed under Apache-2.0, as set out in section 5 of the licence.
There is no Contributor Licence Agreement and no Developer Certificate of Origin sign-off to make.

## Conduct

Please follow the [code of conduct](CODE_OF_CONDUCT.md).
