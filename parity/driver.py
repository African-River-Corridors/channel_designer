"""Run the parity cases through the frozen Python and write the Reference Results.

    python3 parity/driver.py <out-dir>

Only `cargo xtask regen` (and the oracle guard's reproduction check) should run this. It reads every
`parity/cases/<entry_point>.json`, calls the frozen Python (`channel_designer_frozen`, never the
shipped package) through the entry point's adapter in `parity/modules/`, and writes
`<out-dir>/<entry_point>.json`. Encoding rules are in `parity/common.py`.

A raised ValueError, KeyError, IndexError or RuntimeError is recorded as {"error": {"type": "ValueError", "message": ...}}; a normal
return is recorded as {"ok": <value>}. Needs numpy and shapely, as the frozen package does.
"""
from __future__ import annotations

import importlib
import json
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    __package__ = "parity"

from .common import PARITY, dumps, from_json, tag_float, to_json  # noqa: E402,F401
from .modules import registry  # noqa: E402

FROZEN = "channel_designer_frozen"


def _module(name: str):
    return importlib.import_module(f"{FROZEN}.{name}")


def run_case(entry_point: str, case_input: dict, modules=None) -> dict:
    mod = (modules or registry())[entry_point]
    try:
        result = mod.adapter(case_input["fn"], from_json(case_input.get("args", {})))
    except (ValueError, KeyError, IndexError, RuntimeError) as exc:  # the only refusals the package raises
        return {"error": {"type": type(exc).__name__, "message": str(exc)}}
    return {"ok": to_json(result)}


def main(out_dir: str) -> int:
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    modules = registry()
    case_files = sorted((PARITY / "cases").glob("*.json"))
    for stale in out.glob("*.json"):
        if stale.name not in {p.name for p in case_files}:
            stale.unlink()
    for path in case_files:
        cases = json.loads(path.read_text(encoding="utf-8"))
        entry_point = cases["entry_point"]
        if entry_point not in modules:
            raise SystemExit(f"{path.name}: no module for entry point {entry_point!r}")
        results = [{"id": c["id"], "output": run_case(entry_point, c["input"], modules)}
                   for c in cases["cases"]]
        doc = {"entry_point": entry_point,
               "generated_by": "cargo xtask regen (frozen Python); never edit by hand",
               "results": results}
        (out / path.name).write_text(dumps(doc), encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1]))
