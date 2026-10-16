"""The bridge to the Rust core (``channel_designer._core``).

Every public function of the package is a thin wrapper: it names the Rust entry point and function,
encodes its arguments, and decodes the answer into the types the Python API has always returned
(dataclasses, numpy arrays, shapely geometries). Nothing here is part of the public API.

Encoding, both ways (the same rules as the parity harness, ADR 0005):

- a float is a JSON number; NaN and the infinities are the text ``"float:nan"``, ``"float:inf"``
  and ``"float:-inf"``;
- a numpy array is a list; a shapely geometry is ``{"geom": ...}``;
- a dataclass is a mapping of its fields;
- a Python callable is registered with the call (see :class:`Call`) and goes in as
  ``{"kind": "callback", "id": n}``. The core calls back into Python for it. An exception in a
  callable is raised again, unchanged, when the call returns.
"""
from __future__ import annotations

import builtins
import dataclasses
import enum
import json
import math
import types
import typing
import warnings
from typing import Any, Callable, Optional, Union

import numpy as np
import shapely
from shapely.geometry import (GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
                              Point, Polygon)

from . import _core

_TAGS = {"float:nan": math.nan, "float:inf": math.inf, "float:-inf": -math.inf}


# --- geometry -------------------------------------------------------------------------------

def _ring(coords) -> list:
    return [[float(x), float(y)] for x, y, *_ in coords]


def _poly_json(p: Polygon) -> dict:
    return {"exterior": _ring(p.exterior.coords), "holes": [_ring(r.coords) for r in p.interiors]}


def encode_geom(g) -> dict:
    if isinstance(g, LineString):
        return {"geom": "LineString", "coords": _ring(g.coords)}
    if isinstance(g, Point) and g.is_empty:
        return {"geom": "Point", "empty": True}
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
    if isinstance(g, MultiPoint):
        return {"geom": "MultiPoint", "coords": [[float(p.x), float(p.y)] for p in g.geoms]}
    if isinstance(g, GeometryCollection):
        return {"geom": "GeometryCollection", "geoms": [encode_geom(p) for p in g.geoms]}
    raise TypeError(f"no JSON form for geometry {g.geom_type}")


def decode_geom(d: dict):
    kind = d["geom"]
    if d.get("empty"):
        return {"Point": Point, "LineString": LineString, "Polygon": Polygon, "MultiPoint": MultiPoint,
                "MultiLineString": MultiLineString, "MultiPolygon": MultiPolygon,
                "GeometryCollection": GeometryCollection}[kind]()
    if kind == "LineString":
        return LineString(d["coords"])
    if kind == "Point":
        return Point(d["coords"])
    if kind == "Polygon":
        return Polygon(d["exterior"], d.get("holes", []))
    if kind == "MultiLineString":
        return MultiLineString(d["lines"])
    if kind == "MultiPoint":
        return MultiPoint(d["coords"])
    if kind == "GeometryCollection":
        return GeometryCollection([decode_geom(g) for g in d["geoms"]])
    if kind == "MultiPolygon":
        return MultiPolygon([Polygon(p["exterior"], p.get("holes", [])) for p in d["polygons"]])
    raise ValueError(f"unknown geometry kind {kind!r}")


# --- encode ---------------------------------------------------------------------------------

def tag_float(x: float):
    if math.isnan(x):
        return "float:nan"
    if math.isinf(x):
        return "float:inf" if x > 0 else "float:-inf"
    return x


def enc(v):
    """A Python value as the JSON the core reads."""
    if v is None or isinstance(v, str):
        return v
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
    if isinstance(v, enum.Enum):
        return enc(v.value)
    if dataclasses.is_dataclass(v) and not isinstance(v, type):
        return {f.name: enc(getattr(v, f.name)) for f in dataclasses.fields(v)}
    return v


# --- decode ---------------------------------------------------------------------------------

def plain(value):
    """The core's answer with tagged floats read back and geometry objects built."""
    if isinstance(value, str):
        return _TAGS.get(value, value)
    if isinstance(value, list):
        return [plain(v) for v in value]
    if isinstance(value, dict):
        if isinstance(value.get("geom"), str):
            return decode_geom(value)
        return {k: plain(v) for k, v in value.items()}
    return value


def decode(value, tp=Any):
    """``value`` (the core's JSON) as the Python type ``tp``."""
    if tp is Any or tp is None and value is None:
        return plain(value)
    if tp is type(None):
        return None
    if isinstance(tp, str):
        raise TypeError(f"unresolved annotation {tp!r}")
    origin = typing.get_origin(tp)
    if origin in (Union, types.UnionType):
        if value is None:
            return None
        args = [a for a in typing.get_args(tp) if a is not type(None)]
        if len(args) == 1:
            return decode(value, args[0])
        return plain(value)
    if value is None:
        return None
    if tp is float:
        return float(_TAGS.get(value, value)) if isinstance(value, str) else float(value)
    if tp is int:
        return int(value)
    if tp is bool:
        return bool(value)
    if tp is str:
        return value
    if tp is np.ndarray:
        return np.asarray(plain(value), dtype=float)
    if isinstance(tp, type) and issubclass(tp, shapely.geometry.base.BaseGeometry):
        return plain(value)
    if isinstance(tp, type) and issubclass(tp, enum.Enum):
        return tp(value)
    if isinstance(tp, type) and dataclasses.is_dataclass(tp):
        hints = typing.get_type_hints(tp)
        return tp(**{f.name: decode(value[f.name], hints[f.name])
                     for f in dataclasses.fields(tp) if f.init and f.name in value})
    if origin in (list, typing.List):
        (a,) = typing.get_args(tp) or (Any,)
        return [decode(v, a) for v in value]
    if origin in (tuple, typing.Tuple):
        args = typing.get_args(tp)
        if len(args) == 2 and args[1] is Ellipsis:
            return tuple(decode(v, args[0]) for v in value)
        if args:
            return tuple(decode(v, a) for v, a in zip(value, args))
        return tuple(plain(v) for v in value)
    if origin in (dict, typing.Dict):
        args = typing.get_args(tp)
        vt = args[1] if len(args) == 2 else Any
        return {k: decode(v, vt) for k, v in value.items()}
    return plain(value)


# --- callables ------------------------------------------------------------------------------

class Call:
    """One call into the core. Callables are registered here and go in as callback specs."""

    def __init__(self):
        self.callbacks: list = []

    def _add(self, fn) -> dict:
        self.callbacks.append(fn)
        return {"kind": "callback", "id": len(self.callbacks) - 1}

    def terrain(self, f) -> dict:
        """``f(xs, ys) -> z``: numpy arrays in and out."""
        def run(xs, ys):
            z = np.asarray(f(np.asarray(xs, dtype=float), np.asarray(ys, dtype=float)), dtype=float)
            return ("a", z.ravel().tolist())
        return self._add(run)

    def scalar(self, f) -> dict:
        """``f(x) -> y`` on floats."""
        return self._add(lambda x: ("f", float(f(x))))

    def reach(self, f) -> dict:
        """``f(a, b, level) -> weight``."""
        return self._add(lambda a, b, c: ("f", float(f(a, b, c))))

    def stations(self, f) -> dict:
        """``f(xy) -> values`` for an (n, 2) array of stations."""
        def run(xs, ys):
            xy = np.column_stack([np.asarray(xs, dtype=float), np.asarray(ys, dtype=float)])
            return ("a", np.asarray(f(xy), dtype=float).ravel().tolist())
        return self._add(run)

    def text(self, f) -> dict:
        """``f(x) -> str``."""
        return self._add(lambda x: ("s", str(f(x))))

    def json(self, f) -> int:
        """``f(mapping) -> mapping or float``; the id, for the registries to name."""
        def run(text):
            out = f(json.loads(text))
            return ("j", json.dumps(enc(out)))
        return self._add(run)["id"]


class RustError(RuntimeError):
    """The core could not run the call (a bug or a malformed call; never a user refusal)."""


def run(entry_point: str, fn: str, args: dict, call: Optional[Call] = None):
    """Run one call and return the raw ``ok`` value; a refusal raises its Python exception."""
    payload = json.dumps({"fn": fn, "args": {k: enc(v) for k, v in args.items() if v is not _OMIT}},
                         allow_nan=False)
    out = json.loads(_core.eval(entry_point, payload, call.callbacks if call else None))
    if "error" in out:
        err = out["error"]
        cls = getattr(builtins, err.get("type", "ValueError"), ValueError)
        if not (isinstance(cls, type) and issubclass(cls, Exception)):
            cls = ValueError
        raise cls(err.get("message", ""))
    return out["ok"]


#: An argument the caller did not give: left out, so the core uses its default.
_OMIT = object()
OMIT = _OMIT


def call(entry_point: str, fn: str, returns=Any, *, _call: Optional[Call] = None, **args):
    """Run one call and decode the answer as ``returns``."""
    return decode(run(entry_point, fn, args, _call), returns)


def opts(**kw) -> dict:
    """The keyword arguments the caller really gave: those that are not :data:`OMIT`."""
    return {k: v for k, v in kw.items() if v is not _OMIT}


# --- tunable constants ----------------------------------------------------------------------

class _ReadOnlyConstants(types.ModuleType):
    """A module whose tunable constants live in the core. Setting one used to change the Python
    behaviour; it no longer can. The assignment still works (nothing breaks) but warns."""

    _CORE_CONSTANTS: tuple = ()

    def __setattr__(self, name, value):
        if name in type(self)._CORE_CONSTANTS and name in self.__dict__ and self.__dict__[name] != value:
            warnings.warn(
                f"{self.__name__}.{name} is now fixed in the Rust core (channel_designer 0.2.0): "
                "assigning it has no effect and will raise an error in 0.3.0.",
                DeprecationWarning, stacklevel=2)
        super().__setattr__(name, value)


def core_constants(module_name: str, *names: str) -> None:
    """Mark ``names`` of the module ``module_name`` as constants of the core."""
    import sys
    mod = sys.modules[module_name]
    mod.__class__ = type("_Module", (_ReadOnlyConstants,), {"_CORE_CONSTANTS": tuple(names)})
