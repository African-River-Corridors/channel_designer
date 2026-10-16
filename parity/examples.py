"""The two worked examples, run through the FROZEN Python, to the reference file the Rust is held to.

    python -m parity.examples            # rewrite parity/examples/reference.json
    python -m parity.examples --check    # fail if the committed file is not what the frozen Python prints,
                                         # within the ADR 0005 tolerance (parity/tolerance.json)

`examples/quickstart.py` and `examples/optimise_pools.py` are run as they stand, with the import
name `channel_designer` swapped for `channel_designer_frozen` (the shipped package will become a
thin wrapper over the Rust core in ticket 35, so it cannot be the oracle). The reference holds:

- `stdout`: every line the example prints, as text;
- `lines`: each design line the example builds, as vertices;
- `volumes`: dredge, dry cut and tallest face of each line, unrounded;
- the numbers the example reads off its other results (shape, counts, weirs).

The Rust test `crates/cd-format/tests/examples_end_to_end.rs` rebuilds both examples from the Rust
core and compares with this file.
"""
from __future__ import annotations

import contextlib
import io
import json
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    __package__ = "parity"

from .common import PARITY, dumps, tag_float  # noqa: E402,F401

ROOT = PARITY.parent
REFERENCE = PARITY / "examples" / "reference.json"


def _tolerance() -> tuple:
    t = json.loads((PARITY / "tolerance.json").read_text(encoding="utf-8"))
    return t["rel"], t["abs"]


def differences(expected, actual, path: str = "$", tol=None) -> list:
    """Where `actual` differs from `expected`: floats by the ADR 0005 tolerance (Python's
    `math.isclose(rel_tol=rel, abs_tol=abs)`, the rule the Rust comparator and the oracle guard use),
    everything else exactly. The frozen Python prints different last bits on different platforms, so
    a byte comparison would fail on a platform other than the one that wrote the file."""
    import math
    rel, abs_ = tol or _tolerance()
    out: list = []
    if isinstance(expected, bool) or isinstance(actual, bool):
        if expected != actual or type(expected) is not type(actual):
            out.append(f"{path}: expected {expected!r}, got {actual!r}")
    elif isinstance(expected, float) and isinstance(actual, float):
        if not math.isclose(expected, actual, rel_tol=rel, abs_tol=abs_):
            out.append(f"{path}: expected {expected!r}, got {actual!r}")
    elif isinstance(expected, dict) and isinstance(actual, dict):
        for k in sorted(set(expected) | set(actual)):
            if k not in expected or k not in actual:
                out.append(f"{path}.{k}: present in only one")
            else:
                out += differences(expected[k], actual[k], f"{path}.{k}", (rel, abs_))
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            out.append(f"{path}: expected {len(expected)} items, got {len(actual)}")
        else:
            for i, (e, a) in enumerate(zip(expected, actual)):
                out += differences(e, a, f"{path}[{i}]", (rel, abs_))
    elif expected != actual or type(expected) is not type(actual):
        out.append(f"{path}: expected {expected!r}, got {actual!r}")
    return out


def _run_example(name: str):
    """Run `examples/<name>.py` against the frozen package; return (main's result, stdout lines)."""
    src = (ROOT / "examples" / f"{name}.py").read_text(encoding="utf-8")
    src = src.replace("channel_designer", "channel_designer_frozen")
    ns: dict = {"__name__": f"example_{name}"}
    exec(compile(src, f"examples/{name}.py (frozen)", "exec"), ns)
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        result = ns["main"]()
    return result, buf.getvalue().splitlines()


def _coords(line) -> list:
    return [[float(x), float(y)] for x, y in line.coords]


def _vol(v) -> dict:
    return {"dredge_m3": v.dredge_m3, "excav_m3": v.excav_m3, "total_m3": v.total_m3,
            "max_face_h_m": v.max_face_h_m, "n_stations": len(v.stations)}


def build() -> dict:
    shape, rolled, route, results = None, None, None, None
    (shape, rolled, route, results), out = _run_example("quickstart")
    quick = {
        "stdout": out,
        "shape": {"bottom_width_straight_m": shape.bottom_width_straight_m,
                  "min_bend_radius_m": shape.min_bend_radius_m,
                  "design_depth_m": shape.design_depth_m},
        "n_arcs": rolled.n_arcs,
        "lines": {"rolling circle": _coords(rolled.centreline), "least-cost": _coords(route.line)},
        "volumes": {k: _vol(v) for k, v in results.items()},
    }
    (best, pools_out, results2), out2 = _run_example("optimise_pools")
    opt = {
        "stdout": out2,
        "weirs": [{"chainage_m": w.chainage_m, "pool_level_m": w.pool_level_m} for w in best.weirs],
        "pools": [list(p) for p in best.pools()],
        "n_replaced": pools_out.n_replaced,
        "lines": {"optimised": _coords(pools_out.line)},
        "volumes": {k: _vol(v) for k, v in results2.items()},
    }
    return {"generated_by": "python -m parity.examples (frozen Python); never edit by hand",
            "quickstart": quick, "optimise_pools": opt}


def main(argv: list) -> int:
    doc = build()
    text = dumps(doc)
    if "--check" in argv:
        if not REFERENCE.exists():
            print("parity/examples/reference.json is missing; run `python -m parity.examples`",
                  file=sys.stderr)
            return 1
        diffs = differences(json.loads(REFERENCE.read_text()), json.loads(text))
        if diffs:
            print("parity/examples/reference.json is not what the frozen Python prints "
                  "(beyond the ADR 0005 tolerance); run `python -m parity.examples`:\n  "
                  + "\n  ".join(diffs[:10]), file=sys.stderr)
            return 1
        return 0
    REFERENCE.parent.mkdir(parents=True, exist_ok=True)
    REFERENCE.write_text(text, encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
