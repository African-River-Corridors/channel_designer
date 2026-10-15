"""Print the case count per entry point and tier, as a Markdown table.

    python3 -m parity.coverage

Reads the committed `parity/cases/*.json`. Used for the coverage line in pull requests and checked by
`tests/test_parity_cases.py`: every entry point has tiers 1 and 2, and tier 3 unless
`NO_ERROR_CASES` says why the module has no refusal to test."""
from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

PARITY = Path(__file__).resolve().parent

#: Entry points with no tier 3, and why. A module that raises nothing of its own has no error case to
#: write; a case that provokes a refusal from numpy, shapely, rasterio or GEOS would pin their wording.
NO_ERROR_CASES = {
    "loops": "no function raises: every input gives a verdict",
    "primitives": "no function raises on its own; bad inputs (zero bins, zero radius) divide by zero",
    "bends": "no function raises: an infeasible bend is a result (`e = inf`), not an error",
    "raster": "no refusal of its own; a corridor off the raster raises rasterio's WindowError",
    "synthetic": "no function raises",
}


def load() -> dict:
    return {p.stem: json.loads(p.read_text(encoding="utf-8")) for p in sorted((PARITY / "cases").glob("*.json"))}


def table() -> str:
    docs = load()
    rows = ["| Entry point | Tier 1 | Tier 2 | Tier 3 | Total | Search cases | Near ties |", "|---|---|---|---|---|---|---|"]
    tot = Counter()
    for ep, doc in docs.items():
        n = Counter(c["tier"] for c in doc["cases"])
        s = [c for c in doc["cases"] if "search" in c]
        ties = sum(1 for c in s if c["search"]["near_tie"])
        rows.append(f"| `{ep}` | {n[1]} | {n[2]} | {n[3]} | {len(doc['cases'])} | {len(s)} | {ties} |")
        tot.update(n)
        tot["search"] += len(s)
        tot["ties"] += ties
    rows.append(f"| **{len(docs)} entry points** | {tot[1]} | {tot[2]} | {tot[3]} | {tot[1] + tot[2] + tot[3]} "
                f"| {tot['search']} | {tot['ties']} |")
    return "\n".join(rows)


if __name__ == "__main__":
    sys.stdout.write(table() + "\n")
