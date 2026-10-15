"""Write the parity case inputs, `parity/cases/<entry_point>.json`.

    python3 parity/gen_cases.py

Three tiers per entry point (spec 0001): tier 1 named cases drawn from the Python tests, tier 2 a
seeded sweep, tier 3 error cases. Each entry point's cases come from `cases()` in its module in
`parity/modules/`. The output is deterministic: sweeps use a seeded stream built on `random()`
only, so running this twice gives the same bytes. Case inputs are data once committed; the
oracle guard stops a change to one unless it carries a new reason.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    __package__ = "parity"

from .common import PARITY, dumps  # noqa: E402
from .modules import registry  # noqa: E402


def build(entry_point: str, mod) -> dict:
    path = PARITY / "cases" / f"{entry_point}.json"
    previous, retired = {}, []
    if path.exists():
        old = json.loads(path.read_text(encoding="utf-8"))
        previous = {c["id"]: c for c in old["cases"]}
        retired = old.get("retired", [])
    cases = mod.cases()
    searches = getattr(mod, "SEARCH_FNS", ())
    for c in cases:  # a search case stores its closest call, so a near tie is known up front
        if c["input"]["fn"] in searches and c["tier"] != 3:
            c["search"] = mod.search_meta_fn(c["input"]["fn"], c["input"]["args"])
    for c in cases:  # a case that did not change keeps its recorded reason
        old_case = previous.get(c["id"])
        if old_case and old_case["input"] == c["input"]:
            c["reason"] = old_case["reason"]
    ids = [c["id"] for c in cases]
    assert len(ids) == len(set(ids)), f"{entry_point}: duplicate case id"
    for cid in previous:
        if cid not in ids and cid not in {r["id"] for r in retired}:
            raise SystemExit(f"{entry_point}: case {cid} dropped; add it to `retired` "
                             "with a reason in the module")
    return {"entry_point": entry_point, "cases": cases, "retired": retired}


def main() -> int:
    out_dir = PARITY / "cases"
    out_dir.mkdir(exist_ok=True)
    for entry_point, mod in registry().items():
        doc = build(entry_point, mod)
        (out_dir / f"{entry_point}.json").write_text(dumps(doc), encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
