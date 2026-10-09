# Contributing to channel_designer

Thank you for helping. Bug reports, fixes, worked examples, new rulesets and new features are all welcome.

## Set up

Python 3.12 or later. With [uv](https://docs.astral.sh/uv/):

```
git clone https://github.com/African-River-Corridors/channel_designer
cd channel_designer
uv venv
uv pip install -e '.[dev]'
```

Or with plain pip: `python -m venv .venv && .venv/bin/pip install -e '.[dev]'`.

## Run the tests

```
.venv/bin/python -m pytest
python tools/release_guard.py
```

Both must pass. CI runs them on every push and pull request.

## Rules for the code

- `rules`, `alignment` and `volumes` are calculation code: numpy and shapely only, no file I/O.
  Paths, CRS and DEMs come in as arguments. `tests/test_layer_boundary.py` enforces this.
- Heavy dependencies stay optional: raster I/O lives behind the `[dem]` extra, drawings behind
  `[drawings]`.
- Every new calculation comes with tests: a golden case (a worked number with its source), a property
  (an invariant), and a feasibility check (the checks fire when they should). Tests use synthetic
  geometry built in code — never project data.
- Cite the source of every number: standard, clause or table, and page. Quote coefficients as data,
  not standards prose.
- A project choice is not a standard. Name it as a user choice (for example `BANK_CUT_FRACTION`), and
  never present it as a default from a standard.

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
