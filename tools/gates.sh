#!/usr/bin/env bash
# Fast gate (spec 0001): one fail-closed script, run by the `test` CI job and by `cargo xtask gates`.
#
#   bash tools/gates.sh
#
# Fail-closed means: a missing tool fails the job before any step runs, any failing step stops the
# job, and the job fails unless every step ran. There is no skip switch.
#
# Steps:
#   1. cargo fmt --check
#   2. clippy, warnings as errors (includes the banned-maths rule: crates/cd-core/clippy.toml and
#      crates/cd-format/clippy.toml, which hold the same list)
#   3. cargo test, with Python made unusable, to prove the Rust loop needs no Python
#   4. pytest against the shipped package
#
# `cd-py` (the Python extension) is excluded from step 3 as well: PyO3's build script runs Python to find
# the interpreter, which step 3 forbids. It has no Rust tests; clippy (step 2) and the Wheels workflow build it.
#
# The desktop app (`cd-app`, spec 0003) is excluded from steps 2 and 3: it needs Node and the Tauri
# system libraries, and its own CI jobs (`app-linux`, `app-e2e`) build and test it.
#
# CD_PYTHON picks the Python for step 4 (default python3).
set -euo pipefail

fail() { echo "gates: FAIL: $*" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || fail "required tool '$1' is not on PATH"
}

# --- Tool check: every external tool the script calls, before anything runs. --------------------
# A missing tool must fail here, not make a step run inert: without `dirname` the `cd` below lands
# in `/`; without `mktemp` or `chmod` the no-Python shims are never made. Add a `need` line for any
# new tool (tests/test_gates_fail_closed.py lists them too).
need dirname
cd "$(dirname "$0")/.."
PY="${CD_PYTHON:-python3}"
EXPECTED_STEPS=4
steps_run=0

need git
need mktemp
need chmod
need cat
need env
need cargo
need rustc
need "$PY"
cargo fmt --version >/dev/null 2>&1 || fail "rustfmt is not installed (rustup component add rustfmt)"
cargo clippy --version >/dev/null 2>&1 || fail "clippy is not installed (rustup component add clippy)"
"$PY" -c "import pytest" 2>/dev/null || fail "pytest is not importable by $PY (pip install -e '.[dev]')"

# Note: "$@" runs inside `||`, so bash ignores `set -e` in it. A step that is a shell function
# must return non-zero on every failure itself (see cargo_test_without_python).
step() {
  local name="$1"; shift
  local start=$SECONDS
  echo "::group::gates: $name"
  "$@" || { echo "::endgroup::"; fail "$name"; }
  echo "::endgroup::"
  steps_run=$((steps_run + 1))
  echo "gates: ok: $name ($((SECONDS - start)) s)"
}

# cargo test with every python on PATH replaced by a shim that records the call and fails.
cargo_test_without_python() {
  local shims marker py
  shims="$(mktemp -d)"
  marker="$shims/python-was-called"
  for py in python python3 "python3.12" "python3.13" "python3.14"; do
    printf '#!/bin/sh\necho "$0 $*" >> "%s"\necho "gates: cargo test tried to run Python" >&2\nexit 97\n' \
      "$marker" > "$shims/$py"
    chmod +x "$shims/$py"
  done
  # Explicit `|| return 1`: `step` calls this function inside `||`, where bash ignores `set -e`,
  # so without it a failing cargo test would fall through and the step would report ok.
  PATH="$shims:$PATH" CD_PYTHON="$shims/python3" cargo test --workspace --exclude cd-app --exclude cd-py --all-features || return 1
  if [ -e "$marker" ]; then
    echo "gates: cargo test called Python:" >&2
    cat "$marker" >&2
    return 1
  fi
}

step "cargo fmt --check" cargo fmt --all --check
step "clippy -D warnings" cargo clippy --workspace --exclude cd-app --all-targets --all-features -- -D warnings
step "cargo test (no Python)" cargo_test_without_python
step "pytest" "$PY" -m pytest -q

[ "$steps_run" -eq "$EXPECTED_STEPS" ] || fail "only $steps_run of $EXPECTED_STEPS steps ran"
echo "gates: all $EXPECTED_STEPS steps passed in $SECONDS s"
