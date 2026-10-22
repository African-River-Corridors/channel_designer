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
#   2. clippy, warnings as errors (includes the banned-maths rule: crates/owd-core/clippy.toml and
#      crates/owd-format/clippy.toml, which hold the same list)
#   3. cargo nextest (profile `pr`, .config/nextest.toml) and `cargo test --doc`, with Python made
#      unusable, to prove the Rust loop needs no Python. The slowest tests run nightly instead
#      (docs/agents/slow-tests.md); `GATES_NEXTEST_PROFILE=nightly` runs every test.
#   4. pytest against the shipped package
#   5. the leftover-names check (tools/old_names.py, spec 0006): no old name of the tool outside the
#      allowlist
#
# `owd-py` (the Python extension) is excluded from step 3 as well: PyO3's build script runs Python to find
# the interpreter, which step 3 forbids. It has no Rust tests; clippy (step 2) and the Wheels workflow build it.
#
# The web session (`owd-web`, spec 0011) is excluded from steps 2 and 3: it is built for wasm32, which this
# fast gate does not do. The `web-parity` and `web-app` CI jobs build, lint and test it.
#
# The desktop app (`owd-app`, spec 0003) is excluded from steps 2 and 3: it needs Node and the Tauri
# system libraries, and its own CI jobs (`app-linux`, `app-e2e`) build and test it.
#
# CD_PYTHON picks the Python for step 4 (default python3).
#
# CI runs the gate in parallel parts (the `gate-*` jobs in .github/workflows/ci.yml; one job named `test`
# needs them all). Two settings pick a part; with neither set, the script runs every step, as before:
#   GATES_STEPS      the steps to run, by name, separated by commas: fmt, clippy, rust, pytest, oldnames.
#                    An unknown name fails. The script fails unless every named step ran.
#   GATES_PARTITION  k/N: step 3 runs only the k-th of N hash partitions of the Rust tests
#                    (`cargo nextest run --partition hash:k/N`). The doctests run in partition 1 only.
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
ALL_STEPS="fmt,clippy,rust,pytest,oldnames"
STEPS="${GATES_STEPS:-$ALL_STEPS}"
PARTITION="${GATES_PARTITION:-}"
EXPECTED_STEPS=0
steps_run=0
for s in ${STEPS//,/ }; do
  case ",$ALL_STEPS," in
    *",$s,"*) EXPECTED_STEPS=$((EXPECTED_STEPS + 1)) ;;
    *) fail "unknown step '$s' in GATES_STEPS (known: $ALL_STEPS)" ;;
  esac
done
[ "$EXPECTED_STEPS" -gt 0 ] || fail "GATES_STEPS names no step"
wants() { case ",$STEPS," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }
if [ -n "$PARTITION" ]; then
  [[ "$PARTITION" =~ ^([1-9][0-9]*)/([1-9][0-9]*)$ ]] && [ "${BASH_REMATCH[1]}" -le "${BASH_REMATCH[2]}" ] \
    || fail "GATES_PARTITION must be k/N with 1 <= k <= N, not '$PARTITION'"
fi

need git
need mktemp
need chmod
need cat
need env
need cargo
need rustc
need "$PY"
if wants fmt; then cargo fmt --version >/dev/null 2>&1 || fail "rustfmt is not installed (rustup component add rustfmt)"; fi
if wants clippy; then cargo clippy --version >/dev/null 2>&1 || fail "clippy is not installed (rustup component add clippy)"; fi
if wants rust; then cargo nextest --version >/dev/null 2>&1 || fail "cargo-nextest is not installed (cargo install cargo-nextest --locked)"; fi
if wants pytest; then "$PY" -c "import pytest" 2>/dev/null || fail "pytest is not importable by $PY (pip install -e '.[dev]')"; fi

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
  local part=()
  [ -n "$PARTITION" ] && part=(--partition "hash:$PARTITION")
  PATH="$shims:$PATH" CD_PYTHON="$shims/python3" cargo nextest run --workspace --exclude owd-app --exclude owd-py --exclude owd-web \
    --all-features --profile "${GATES_NEXTEST_PROFILE:-pr}" ${part[@]+"${part[@]}"} || return 1
  # nextest does not run doctests. They run once: with no partition, or in partition 1.
  if [ -z "$PARTITION" ] || [ "${PARTITION%%/*}" = 1 ]; then
    PATH="$shims:$PATH" CD_PYTHON="$shims/python3" cargo test --doc --workspace --exclude owd-app --exclude owd-py --exclude owd-web \
      --all-features || return 1
  fi
  if [ -e "$marker" ]; then
    echo "gates: cargo test called Python:" >&2
    cat "$marker" >&2
    return 1
  fi
}

if wants fmt; then step "cargo fmt --check" cargo fmt --all --check; fi
if wants clippy; then
  step "clippy -D warnings" cargo clippy --workspace --exclude owd-app --exclude owd-web --all-targets --all-features -- -D warnings
fi
if wants rust; then step "cargo test (no Python)${PARTITION:+, partition $PARTITION}" cargo_test_without_python; fi
if wants pytest; then step "pytest" "$PY" -m pytest -q; fi
if wants oldnames; then step "old names" "$PY" tools/old_names.py; fi

[ "$steps_run" -eq "$EXPECTED_STEPS" ] || fail "only $steps_run of $EXPECTED_STEPS steps ran"
echo "gates: all $EXPECTED_STEPS steps passed in $SECONDS s ($STEPS${PARTITION:+, partition $PARTITION})"
