"""The fast gate fails closed: a missing tool fails the job before any step runs.

Spec 0001: one fail-closed gates script; a missing tool or a skipped step fails the job. Risk
stopped: CI goes green because a check silently did not run.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATES = ROOT / "tools" / "gates.sh"


def _path_with_only(tmp_path: Path, tools: list[str], stubs: tuple[str, ...] = ()) -> str:
    """A PATH that holds only the named tools (symlinked from the real PATH), plus do-nothing
    stubs that only need to exist for the tool check."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for tool in tools:
        found = shutil.which(tool)
        assert found, f"test needs {tool} on PATH"
        (bin_dir / tool).symlink_to(found)
    for stub in stubs:
        (bin_dir / stub).write_text("#!/bin/sh\nexit 0\n")
        (bin_dir / stub).chmod(0o755)
    return str(bin_dir)


def _gates(env_path: str, **extra) -> subprocess.CompletedProcess:
    env = {"PATH": env_path, "HOME": os.environ.get("HOME", "/tmp"), **extra}
    bash = shutil.which("bash")
    return subprocess.run([bash, str(GATES)], cwd=ROOT, env=env, capture_output=True, text=True)


def test_gates_fail_when_cargo_is_missing(tmp_path):
    path = _path_with_only(tmp_path, ["dirname", "git", "mktemp", "chmod", "cat", "env"])
    result = _gates(path, CD_PYTHON=sys.executable)
    assert result.returncode != 0
    assert "required tool 'cargo' is not on PATH" in result.stderr
    assert "gates: ok:" not in result.stdout


def test_gates_fail_when_python_is_missing(tmp_path):
    path = _path_with_only(tmp_path, ["dirname", "git", "mktemp", "chmod", "cat", "env"], stubs=("cargo", "rustc"))
    result = _gates(path, CD_PYTHON="no-such-python")
    assert result.returncode != 0
    assert "required tool 'no-such-python' is not on PATH" in result.stderr
    assert "gates: ok:" not in result.stdout



def test_gates_fail_when_cargo_test_fails(tmp_path):
    """A failing `cargo test` must fail its own step, not pass it and leave a later step to catch
    it. Regression: `step` runs the function inside `||`, where bash ignores `set -e`."""
    path = _path_with_only(tmp_path, ["dirname", "git", "mktemp", "chmod", "cat", "env"])
    bin_dir = Path(path)
    # cargo passes every call except `cargo nextest run`.
    (bin_dir / "cargo").write_text(
        '#!/bin/sh\nif [ "$1" = nextest ] && [ "$2" = run ]; then echo "test result: FAILED" >&2; exit 101; fi\nexit 0\n'
    )
    (bin_dir / "rustc").write_text("#!/bin/sh\nexit 0\n")
    for stub in ("cargo", "rustc"):
        (bin_dir / stub).chmod(0o755)
    result = _gates(path, CD_PYTHON=sys.executable)
    assert result.returncode != 0
    assert "gates: FAIL: cargo test (no Python)" in result.stderr
    assert "gates: ok: cargo test (no Python)" not in result.stdout
    assert "gates: ok: clippy -D warnings" in result.stdout


# --- the gate in parts (GATES_STEPS, GATES_PARTITION), as the CI `gate-*` jobs run it --------------

def _recording_cargo(tmp_path):
    """A PATH whose cargo passes every call and records its arguments."""
    path = _path_with_only(tmp_path, ["dirname", "git", "mktemp", "chmod", "cat", "env"])
    bin_dir = Path(path)
    log = tmp_path / "cargo-calls"
    (bin_dir / "cargo").write_text(f'#!/bin/sh\necho "$*" >> "{log}"\nexit 0\n')
    (bin_dir / "rustc").write_text("#!/bin/sh\nexit 0\n")
    for stub in ("cargo", "rustc"):
        (bin_dir / stub).chmod(0o755)
    return path, log


def test_an_unknown_step_name_fails_before_any_step_runs(tmp_path):
    path, _ = _recording_cargo(tmp_path)
    result = _gates(path, CD_PYTHON=sys.executable, GATES_STEPS="fmt,lint")
    assert result.returncode != 0
    assert "unknown step 'lint'" in result.stderr
    assert "gates: ok:" not in result.stdout


def test_a_bad_partition_fails_before_any_step_runs(tmp_path):
    path, _ = _recording_cargo(tmp_path)
    for bad in ("4/3", "0/3", "two/3", "1"):
        result = _gates(path, CD_PYTHON=sys.executable, GATES_STEPS="rust", GATES_PARTITION=bad)
        assert result.returncode != 0 and "GATES_PARTITION must be k/N" in result.stderr, bad
        assert "gates: ok:" not in result.stdout


def test_a_part_runs_only_its_steps_and_counts_them(tmp_path):
    path, log = _recording_cargo(tmp_path)
    result = _gates(path, CD_PYTHON=sys.executable, GATES_STEPS="fmt,clippy")
    assert result.returncode == 0, result.stderr
    assert "gates: all 2 steps passed" in result.stdout
    calls = log.read_text().splitlines()
    assert any(c.startswith("fmt --all --check") for c in calls)
    assert not any(c.startswith(("nextest run", "test --doc")) for c in calls)


def test_a_rust_partition_passes_it_to_nextest_and_runs_the_doctests_once(tmp_path):
    for k, doc in ((1, True), (2, False), (3, False)):
        (tmp_path / str(k)).mkdir()
        path, log = _recording_cargo(tmp_path / str(k))
        result = _gates(path, CD_PYTHON=sys.executable, GATES_STEPS="rust", GATES_PARTITION=f"{k}/3")
        assert result.returncode == 0, result.stderr
        calls = log.read_text().splitlines()
        runs = [c for c in calls if c.startswith("nextest run")]
        assert len(runs) == 1 and runs[0].endswith(f"--partition hash:{k}/3")
        assert any(c.startswith("test --doc") for c in calls) == doc


def test_without_a_partition_nextest_runs_every_test_and_all_steps_is_the_default(tmp_path):
    path, log = _recording_cargo(tmp_path)
    result = _gates(path, CD_PYTHON=sys.executable, GATES_STEPS="rust")
    assert result.returncode == 0, result.stderr
    assert "--partition" not in log.read_text()
    text = GATES.read_text()
    assert 'ALL_STEPS="fmt,clippy,rust,pytest,oldnames"' in text and 'STEPS="${GATES_STEPS:-$ALL_STEPS}"' in text
