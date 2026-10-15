"""Run the parity cases through the frozen Python and write the Reference Results.

    python3 parity/driver.py <out-dir>

Only `cargo xtask regen` (and the oracle guard's reproduction check) should run this. It reads every
`parity/cases/<entry_point>.json`, calls the frozen Python (`channel_designer_frozen`, never the
shipped package) for each case, and writes `<out-dir>/<entry_point>.json`.

Output rules (spec 0001, ADR 0005):
- floats are written as shortest round-trip text (Python's repr);
- NaN and infinity are written as the tagged strings "float:nan", "float:inf" and "float:-inf",
  and the same tags are read back in case inputs;
- a raised ValueError is recorded as {"error": {"type": "ValueError", "message": ...}};
- a normal return is recorded as {"ok": <value>}.
Needs numpy and shapely, as the frozen package does.
"""
from __future__ import annotations

import dataclasses
import enum
import importlib
import json
import math
import sys
from pathlib import Path

import numpy as np

PARITY = Path(__file__).resolve().parent
sys.path.insert(0, str(PARITY / "frozen_python"))

FROZEN = "channel_designer_frozen"
TAGS = {"float:nan": math.nan, "float:inf": math.inf, "float:-inf": -math.inf}


def tag_float(x: float):
    if math.isnan(x):
        return "float:nan"
    if math.isinf(x):
        return "float:inf" if x > 0 else "float:-inf"
    return x


def from_json(value):
    """Read a case input: tagged strings become floats."""
    if isinstance(value, str):
        return TAGS.get(value, value)
    if isinstance(value, list):
        return [from_json(v) for v in value]
    if isinstance(value, dict):
        return {k: from_json(v) for k, v in value.items()}
    return value


def to_json(value):
    """Convert a Python result into plain JSON values, keeping dict and field order."""
    if isinstance(value, (bool, np.bool_)) or value is None or isinstance(value, str):
        return bool(value) if isinstance(value, np.bool_) else value
    if isinstance(value, (int, np.integer)):
        return int(value)
    if isinstance(value, (float, np.floating)):
        return tag_float(float(value))
    if isinstance(value, enum.Enum):
        return to_json(value.value)
    if isinstance(value, np.ndarray):
        return to_json(value.tolist())
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {f.name: to_json(getattr(value, f.name)) for f in dataclasses.fields(value)}
    if isinstance(value, dict):
        return {str(k): to_json(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [to_json(v) for v in value]
    raise TypeError(f"no JSON form for {type(value).__name__}")


def _module(name: str):
    return importlib.import_module(f"{FROZEN}.{name}")


# One adapter per entry point: (function name, args) -> Python result. Args are keyed by the
# Python parameter names, so a case reads like the call it makes.
def cut_model(fn: str, args: dict):
    mod = _module("volumes.cut_model")
    if fn == "cut_model_b_daylight_limit":  # JSON keys are text; the function takes int keys
        args = dict(args, bench={int(k): v for k, v in args["bench"].items()})
    if fn in ("cut_model_a_face", "cut_model_a_bench_gate", "cut_model_b_daylight_limit",
              "section_area"):
        return getattr(mod, fn)(**args)
    if fn == "widened_hb":
        args = dict(args, xy=np.array(args["xy"], dtype=float))
        return mod.widened_hb(**args)
    raise KeyError(f"cut_model has no parity function {fn!r}")


ADAPTERS = {"cut_model": cut_model}


def run_case(entry_point: str, case_input: dict) -> dict:
    adapter = ADAPTERS[entry_point]
    try:
        result = adapter(case_input["fn"], from_json(case_input.get("args", {})))
    except ValueError as exc:
        return {"error": {"type": "ValueError", "message": str(exc)}}
    return {"ok": to_json(result)}


def dumps(doc) -> str:
    return json.dumps(doc, indent=2, ensure_ascii=False, allow_nan=False) + "\n"


def main(out_dir: str) -> int:
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    case_files = sorted((PARITY / "cases").glob("*.json"))
    for stale in out.glob("*.json"):
        if stale.name not in {p.name for p in case_files}:
            stale.unlink()
    for path in case_files:
        cases = json.loads(path.read_text(encoding="utf-8"))
        entry_point = cases["entry_point"]
        if entry_point not in ADAPTERS:
            raise SystemExit(f"{path.name}: no driver adapter for entry point {entry_point!r}")
        results = [{"id": c["id"], "output": run_case(entry_point, c["input"])}
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
