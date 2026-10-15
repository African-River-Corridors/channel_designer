"""Shared helpers for the parity driver and the case generators.

Encoding rules (spec 0001, ADR 0005), the same in case inputs and in Reference Results:

- floats are shortest round-trip text; NaN and infinity are the strings "float:nan", "float:inf"
  and "float:-inf";
- a shapely geometry is an object with a "geom" key (`encode_geom` / `decode_geom`):
  `{"geom": "LineString", "coords": [[x, y], ...]}`, `{"geom": "Point", "coords": [x, y]}`,
  `{"geom": "Polygon", "exterior": [...], "holes": [[...], ...]}`,
  `{"geom": "MultiLineString", "lines": [[...], ...]}`,
  `{"geom": "MultiPolygon", "polygons": [{"exterior": ..., "holes": ...}, ...]}`;
- a terrain sampler `terrain(xs, ys) -> z` is a spec, `{"kind": ..., ...}` (`terrain_from`):
  "synthetic_river" (the `synthetic_river` keyword arguments), "plane" (`z0`, `gx`, `gy`) and
  "hills" (`base`, `gx`, `gy`, `hills: [[x, y, height, radius], ...]`), "valley" (`x0`, `run`,
  `depth`, optional `gap: [lo, hi]` of no data) and "wrong_shape" (`n`; a broken sampler);
- a dict with integer keys uses the keys as text.

The frozen package is the only Python a driver or generator may import for the code under test.
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
import shapely
from shapely.geometry import (LineString, MultiLineString, MultiPolygon, Point, Polygon)

PARITY = Path(__file__).resolve().parent
sys.path.insert(0, str(PARITY / "frozen_python"))

FROZEN = "channel_designer_frozen"
TAGS = {"float:nan": math.nan, "float:inf": math.inf, "float:-inf": -math.inf}
FINITE_TAGS = {math.inf: "float:inf", -math.inf: "float:-inf"}


def frozen(name: str):
    """A module of the frozen package, e.g. `frozen("volumes.cut_model")`."""
    return importlib.import_module(f"{FROZEN}.{name}")


# --- geometry -------------------------------------------------------------------------------

def _ring(coords) -> list:
    return [[float(x), float(y)] for x, y, *_ in coords]


def _poly_json(p: Polygon) -> dict:
    return {"exterior": _ring(p.exterior.coords), "holes": [_ring(r.coords) for r in p.interiors]}


def encode_geom(g):
    """A shapely geometry as its JSON object."""
    if isinstance(g, LineString):
        return {"geom": "LineString", "coords": _ring(g.coords)}
    if isinstance(g, Point):
        return {"geom": "Point", "coords": [float(g.x), float(g.y)]}
    if isinstance(g, Polygon):
        return {"geom": "Polygon", **_poly_json(g)}
    if isinstance(g, MultiLineString):
        return {"geom": "MultiLineString", "lines": [_ring(p.coords) for p in g.geoms]}
    if isinstance(g, MultiPolygon):
        return {"geom": "MultiPolygon", "polygons": [_poly_json(p) for p in g.geoms]}
    if g.is_empty:
        return {"geom": g.geom_type, "empty": True}
    raise TypeError(f"no JSON form for geometry {g.geom_type}")


def decode_geom(d: dict):
    kind = d["geom"]
    if kind == "LineString":
        return LineString(d["coords"])
    if kind == "Point":
        return Point(d["coords"])
    if kind == "Polygon":
        return Polygon(d["exterior"], d.get("holes", []))
    if kind == "MultiLineString":
        return MultiLineString(d["lines"])
    if kind == "MultiPolygon":
        return MultiPolygon([Polygon(p["exterior"], p.get("holes", [])) for p in d["polygons"]])
    raise ValueError(f"unknown geometry kind {kind!r}")


# --- terrain samplers -----------------------------------------------------------------------

def terrain_from(spec: dict):
    """The sampler `terrain(xs, ys) -> z` that a terrain spec names."""
    kind = spec["kind"]
    if kind == "synthetic_river":
        return frozen("synthetic").synthetic_river(**spec.get("params", {})).terrain
    if kind == "plane":
        z0, gx, gy = spec["z0"], spec["gx"], spec["gy"]

        def plane(xs, ys):
            xs = np.asarray(xs, dtype=float)
            ys = np.asarray(ys, dtype=float)
            return z0 + gx * xs + gy * ys

        return plane
    if kind == "hills":
        base, gx, gy, hills = spec["base"], spec["gx"], spec["gy"], spec["hills"]

        def hills_fn(xs, ys):
            xs = np.asarray(xs, dtype=float)
            ys = np.asarray(ys, dtype=float)
            z = base + gx * xs + gy * ys
            for hx, hy, h, r in hills:
                z = z + h * np.exp(-((xs - hx) ** 2 + (ys - hy) ** 2) / (2.0 * r * r))
            return z

        return hills_fn
    if kind == "wrong_shape":  # a broken sampler: always returns `n` zeros, whatever it is asked
        n = spec["n"]
        return lambda xs, ys: np.zeros(n)
    if kind == "valley":  # z = depth * |x - x0| / run, with NaN where lo < x < hi (a nodata gap)
        x0, run, depth = spec["x0"], spec["run"], spec["depth"]
        gap = spec.get("gap")

        def valley(xs, ys):
            xs = np.asarray(xs, dtype=float)
            z = depth * np.abs(xs - x0) / run
            if gap:
                z = np.where((xs > gap[0]) & (xs < gap[1]), np.nan, z)
            return z

        return valley
    raise ValueError(f"unknown terrain kind {kind!r}")


# --- JSON in and out ------------------------------------------------------------------------

def tag_float(x: float):
    if math.isnan(x):
        return "float:nan"
    if math.isinf(x):
        return "float:inf" if x > 0 else "float:-inf"
    return x


def enc(v):
    """Encode a value for a case input (generator side)."""
    if isinstance(v, (bool, np.bool_)):
        return bool(v)
    if isinstance(v, (float, np.floating)):
        return tag_float(float(v))
    if isinstance(v, (int, np.integer)):
        return int(v)
    if isinstance(v, np.ndarray):
        return enc(v.tolist())
    if isinstance(v, (list, tuple)):
        return [enc(x) for x in v]
    if isinstance(v, dict):
        return {str(k): enc(x) for k, x in v.items()}
    if isinstance(v, shapely.geometry.base.BaseGeometry):
        return encode_geom(v)
    return v


def from_json(value):
    """Read a case input: tagged strings become floats, geometry objects become shapely."""
    if isinstance(value, str):
        return TAGS.get(value, value)
    if isinstance(value, list):
        return [from_json(v) for v in value]
    if isinstance(value, dict):
        if isinstance(value.get("geom"), str):
            return decode_geom(value)
        return {k: from_json(v) for k, v in value.items()}
    return value


def to_json(value):
    """Convert a Python result into plain JSON values, keeping dict and field order."""
    if isinstance(value, (bool, np.bool_)):
        return bool(value)
    if value is None or isinstance(value, str):
        return value
    if isinstance(value, (int, np.integer)):
        return int(value)
    if isinstance(value, (float, np.floating)):
        return tag_float(float(value))
    if isinstance(value, enum.Enum):
        return to_json(value.value)
    if isinstance(value, np.ndarray):
        return to_json(value.tolist())
    if isinstance(value, shapely.geometry.base.BaseGeometry):
        return encode_geom(value)
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {f.name: to_json(getattr(value, f.name)) for f in dataclasses.fields(value)}
    if isinstance(value, dict):
        return {str(k): to_json(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [to_json(v) for v in value]
    raise TypeError(f"no JSON form for {type(value).__name__}")


def _scalar(x) -> bool:
    return not isinstance(x, (list, dict))


def _flat(x) -> bool:
    """A list of scalars, or of short lists of scalars (points): written on one line."""
    return isinstance(x, list) and all(
        _scalar(e) or (isinstance(e, list) and all(_scalar(s) for s in e)) for e in x)


def _write(v, indent: int, out: list) -> None:
    pad = "  " * indent
    if isinstance(v, dict) and v:
        out.append("{\n")
        items = list(v.items())
        for i, (k, x) in enumerate(items):
            out.append(f"{pad}  {json.dumps(k, ensure_ascii=False)}: ")
            _write(x, indent + 1, out)
            out.append(",\n" if i < len(items) - 1 else "\n")
        out.append(f"{pad}}}")
    elif isinstance(v, list) and v and not _flat(v):
        out.append("[\n")
        for i, x in enumerate(v):
            out.append(f"{pad}  ")
            _write(x, indent + 1, out)
            out.append(",\n" if i < len(v) - 1 else "\n")
        out.append(f"{pad}]")
    else:
        out.append(json.dumps(v, ensure_ascii=False, allow_nan=False))


def dumps(doc) -> str:
    """Stable text for a parity file: objects one key per line, lists of numbers on one line."""
    out: list = []
    _write(doc, 0, out)
    return "".join(out) + "\n"


# --- case construction (generator side) -----------------------------------------------------

def case(cid: str, tier: int, fn: str, args: dict, reason: str) -> dict:
    return {"id": cid, "tier": tier, "reason": reason, "input": {"fn": fn, "args": enc(args)}}


def rnd(a, nd: int = 3):
    """Round a coordinate array or list to `nd` decimals (millimetres), to keep case files small.
    The rounded numbers are the case: nothing downstream sees the unrounded ones."""
    return np.round(np.asarray(a, dtype=float), nd)


def frange(lo, hi, step):
    n = int(round((hi - lo) / step))
    return [lo + i * step for i in range(n + 1)]


class Rng:
    """A seeded stream built only on `random.random()`, so the bytes never depend on the Python
    version's higher-level helpers."""

    def __init__(self, seed: int):
        import random
        self._r = random.Random(seed)

    def u(self, a: float, b: float) -> float:
        return a + (b - a) * self._r.random()

    def r3(self, a: float, b: float) -> float:
        return round(self.u(a, b), 3)

    def p(self, prob: float) -> bool:
        return self._r.random() < prob

    def i(self, lo: int, hi: int) -> int:
        """An int in [lo, hi]."""
        return lo + int(self._r.random() * (hi - lo + 1))


def rline(coords, nd: int = 3):
    """A LineString with coordinates rounded to `nd` decimals."""
    return LineString(rnd(coords, nd))


def synth(**kw) -> dict:
    """The synthetic river (frozen code) as rounded shapely geometries: `centreline`, `left_bank`,
    `right_bank` and `polygon`. Keyword arguments are those of `synthetic_river`."""
    river = frozen("synthetic").synthetic_river(**kw)
    ring = rnd(np.array(river.polygon.exterior.coords))
    return {"centreline": rline(river.centreline.coords),
            "left_bank": rline(river.left_bank.coords),
            "right_bank": rline(river.right_bank.coords),
            "polygon": Polygon(ring)}


# --- search outcomes (ADR 0005) -------------------------------------------------------------

#: A case is a NEAR TIE when, at some decision the search made, the runner-up came within this
#: relative gap of the winner: |best - runner_up| <= NEAR_TIE_REL * max(|best|, 1). That is about a
#: thousand times the floating-point noise between two correct implementations (1e-9 relative),
#: so a flipped choice below it is order-of-operations noise, and a flip above it is a real error.
#: It is written into every search case, so a tie is a property of the case, never a judgement
#: made after a failure.
NEAR_TIE_REL = 1e-6


def search_meta(best, runner_up, decisions: int, what: str) -> dict:
    """The record a search case stores: the closest call the search made.

    `best` and `runner_up` are the winner's and runner-up's cost at the decision with the smallest
    gap (`runner_up` is None when no decision had an alternative). `what` says what was compared."""
    if runner_up is None:
        return {"what": what, "decisions": decisions, "best": best, "runner_up": None,
                "gap_rel": None, "near_tie": False, "near_tie_rel": NEAR_TIE_REL}
    gap = abs(best - runner_up) / max(abs(best), 1.0)
    return {"what": what, "decisions": decisions, "best": best, "runner_up": runner_up,
            "gap_rel": gap, "near_tie": bool(gap <= NEAR_TIE_REL), "near_tie_rel": NEAR_TIE_REL}
