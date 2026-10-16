"""The committed reference for the two worked examples is what the frozen Python prints."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_examples_reference_is_what_the_frozen_python_prints():
    done = subprocess.run([sys.executable, "-m", "parity.examples", "--check"], cwd=ROOT,
                          capture_output=True, text=True)
    assert done.returncode == 0, done.stderr


def _reference():
    import json
    return json.loads((ROOT / "parity" / "examples" / "reference.json").read_text())


def test_check_ignores_last_bits_but_not_a_real_change():
    """The check compares by the ADR 0005 tolerance: last-bit noise passes, a corrupted number fails."""
    sys.path.insert(0, str(ROOT))
    from parity.examples import differences
    ref = _reference()
    # Last-bit noise: one ulp up on every float, as another platform's libm can print.
    import copy
    import math

    def jiggle(x):
        if isinstance(x, float) and math.isfinite(x):
            return math.nextafter(x, math.inf)
        if isinstance(x, list):
            return [jiggle(v) for v in x]
        if isinstance(x, dict):
            return {k: jiggle(v) for k, v in x.items()}
        return x

    assert differences(ref, jiggle(copy.deepcopy(ref))) == []
    # A corrupted example number, by far more than the tolerance (1e-9 relative).
    bad = copy.deepcopy(ref)
    bad["quickstart"]["volumes"]["least-cost"]["dredge_m3"] *= 1.000001
    diffs = differences(ref, bad)
    assert len(diffs) == 1 and "dredge_m3" in diffs[0]
    # A changed printed line, a changed count and a changed key fail too.
    bad2 = copy.deepcopy(ref)
    bad2["optimise_pools"]["stdout"][0] += " "
    bad2["quickstart"]["n_arcs"] += 1
    assert len(differences(ref, bad2)) == 2
