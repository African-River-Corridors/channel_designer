"""The README / quickstart example runs and says what the README says it does."""
import importlib.util
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _load(path):
    spec = importlib.util.spec_from_file_location("quickstart", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_quickstart_runs_and_the_least_cost_line_cuts_less():
    shape, rolled, route, results = _load(ROOT / "examples" / "quickstart.py").main()
    assert results["least-cost"].total_m3 < results["rolling circle"].total_m3


def test_readme_example_is_the_quickstart():
    """The README's code block is the quickstart's main body, so it cannot drift."""
    readme = (ROOT / "README.md").read_text()
    block = re.search(r"```python\n(.*?)```", readme, re.S).group(1)
    quick = (ROOT / "examples" / "quickstart.py").read_text()
    for line in block.strip().splitlines():
        if line.strip() and not line.lstrip().startswith("#"):
            assert line.strip() in quick, f"README line not in quickstart: {line!r}"


def test_optimise_example_runs_and_cuts_less_dry_ground():
    best, out, results = _load(ROOT / "examples" / "optimise_pools.py").main()
    assert best.weirs and out.n_replaced == len(best.pools())
    assert results["optimised"].excav_m3 < results["straight"].excav_m3


def test_readme_optimiser_example_is_the_example_file():
    readme = (ROOT / "README.md").read_text()
    blocks = re.findall(r"```python\n(.*?)```", readme, re.S)
    block = next(b for b in blocks if "search_cascade" in b)
    src = (ROOT / "examples" / "optimise_pools.py").read_text()
    for line in block.strip().splitlines():
        if line.strip() and not line.lstrip().startswith("#"):
            assert line.strip() in src, f"README line not in optimise_pools.py: {line!r}"
