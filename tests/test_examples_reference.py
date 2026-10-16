"""The committed reference for the two worked examples is what the frozen Python prints."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_examples_reference_is_what_the_frozen_python_prints():
    done = subprocess.run([sys.executable, "-m", "parity.examples", "--check"], cwd=ROOT,
                          capture_output=True, text=True)
    assert done.returncode == 0, done.stderr
