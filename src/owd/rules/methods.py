"""Width methods — the pluggable registry behind ``size_channel``, ``width_at_radius`` and
``design.compute``.

A *width method* turns a vessel (length overall and beam) into a straight channel width, a
width through a bend, and a design minimum bend radius. The package ships one method,
``"pianc"``. Another package can add its own, for example a consultant's method it may not
publish, without editing this one:

.. code-block:: python

    from owd.rules.methods import WidthMethod, register_width_method

    register_width_method(WidthMethod(
        name="my-method",
        straight_width=lambda loa, beam, opts: 2.5 * beam,
        bend_width=lambda loa, beam, opts, bend_r, design_r, defl: 2.5 * beam + loa**2 / (4 * bend_r),
        min_radius=lambda loa, opts: 4.0 * loa,
        description="my-method: 2.5 x beam, LOA^2/(4R) bend widening",
    ))

or declare it as an entry point, so it is found without an import:

.. code-block:: toml

    [project.entry-points."owd.width_methods"]
    my-method = "my_package.channel:MY_METHOD"     # a WidthMethod instance

Entry points are loaded the first time a name is not found in the registry.

Every callable takes an ``options`` mapping: the method's own parameters, passed through
untouched from ``ChannelRuleInputs.options`` or ``ChannelDesignParams.width_options``.

Contract (the tests hold every registered method to it):

- ``straight_width(loa_m, beam_m, options) -> m`` — bottom width on a straight, > 0.
- ``bend_width(loa_m, beam_m, options, bend_radius_m, design_radius_m, deflection_deg) -> m``
  — bottom width through a bend of radius ``bend_radius_m`` when the design radius is
  ``design_radius_m`` and the bend turns ``deflection_deg``. Callers take
  ``max(straight, bend)``, so a method need not clamp.
- ``min_radius(loa_m, options) -> m`` — the design minimum bend radius.
- ``breakdown(loa_m, beam_m, options) -> dict`` — optional; the width build-up, for reports.
"""
from __future__ import annotations


from dataclasses import dataclass
from typing import Callable, Mapping, Optional

from .. import _rust

ENTRY_POINT_GROUP = "owd.width_methods"
#: The group name before the rename to owd. Read for one release, so a plugin that has not moved still loads.
LEGACY_ENTRY_POINT_GROUP = "channel_designer.width_methods"

Options = Mapping[str, object]


@dataclass(frozen=True)
class WidthMethod:
    """One channel-width method. See the module docstring for the contract."""

    name: str
    straight_width: Callable[[float, float, Options], float]
    bend_width: Callable[[float, float, Options, float, float, float], float]
    min_radius: Callable[[float, Options], float]
    breakdown: Optional[Callable[[float, float, Options], dict]] = None
    description: str = ""


_REGISTRY: dict = {}
_ENTRY_POINTS_LOADED = False


def register_width_method(method: WidthMethod, *, replace: bool = False) -> WidthMethod:
    """Add ``method`` to the registry under ``method.name``.

    Raises ``ValueError`` if the name is taken, unless ``replace=True``. Returns the method,
    so it can be used as a one-line registration at import time.
    """
    if not isinstance(method, WidthMethod):
        raise TypeError("register_width_method needs a WidthMethod")
    if not method.name:
        raise ValueError("a width method needs a name")
    if method.name in _REGISTRY and not replace:
        raise ValueError(f"width method {method.name!r} is already registered; "
                         "pass replace=True to override it")
    _REGISTRY[method.name] = method
    return method


def unregister_width_method(name: str) -> None:
    """Remove a method from the registry. The built-in ``"pianc"`` cannot be removed."""
    if name == "pianc":
        raise ValueError("the built-in 'pianc' method cannot be unregistered")
    _REGISTRY.pop(name, None)


def _load_entry_points() -> None:
    global _ENTRY_POINTS_LOADED
    if _ENTRY_POINTS_LOADED:
        return
    _ENTRY_POINTS_LOADED = True
    from importlib.metadata import entry_points
    for ep in (*entry_points(group=ENTRY_POINT_GROUP), *entry_points(group=LEGACY_ENTRY_POINT_GROUP)):
        if ep.name in _REGISTRY:
            continue
        obj = ep.load()
        method = obj() if callable(obj) and not isinstance(obj, WidthMethod) else obj
        if not isinstance(method, WidthMethod):
            raise TypeError(f"entry point {ep.name!r} in {ENTRY_POINT_GROUP!r} must give a "
                            f"WidthMethod (or a function returning one); got {type(method)!r}")
        if method.name != ep.name:
            raise ValueError(f"entry point {ep.name!r} gives a method named {method.name!r}; "
                             "the two must match")
        _REGISTRY[method.name] = method


def get_width_method(name: str) -> WidthMethod:
    """The registered method called ``name``. Loads entry points on the first miss."""
    if name not in _REGISTRY:
        _load_entry_points()
    try:
        return _REGISTRY[name]
    except KeyError:
        raise ValueError(f"unknown width method {name!r}; registered: "
                         f"{', '.join(sorted(_REGISTRY))}") from None


def width_methods() -> tuple:
    """Names of every registered method, entry points included."""
    _load_entry_points()
    return tuple(sorted(_REGISTRY))


# =====================================================================================
# The built-in method: "pianc"
# =====================================================================================
# Default path: one-lane fairway = width_multiple x beam, bend widening LOA^2/(8R),
# R_min = radius_multiple x LOA. NOTE the LOA^2/(8R) widening (c_C = 0.125) is BELOW
# WG 141's own floor of c_C ~ 0.5 — see the RULESETS block in ``rulesets``. Prefer
# ``channel_shape`` with a named ruleset for new work.
#
# ``use_buildup=True`` switches the straight width to the PIANC Report 121 concept-design
# build-up W = W_BM + sum(W_i) + W_Br + W_Bg, coefficients as fractions of beam. Those are
# typical concept-design values for a slow-speed inland reach; confirm them against the
# report before use.

PIANC_MANEUVER = {"good": 1.3, "moderate": 1.5, "poor": 1.8}   # W_BM = n x B

PIANC_DEFAULTS = {
    "width_multiple": 2.8,          # one-lane fairway = k_w x beam (default path)
    "radius_multiple": 3.0,         # min bend radius = k_r x LOA
    "use_buildup": False,           # True -> Report 121 W_BM + sum(W_i) + banks
    "maneuverability": "poor",      # good / moderate / poor -> W_BM = 1.3 / 1.5 / 1.8 B
    # additional widths sum(W_i), each a fraction of beam (slow-speed inland defaults)
    "extra_widths": {
        "prevailing_wind": 0.1, "cross_current": 0.4, "longitudinal_current": 0.0,
        "wave": 0.0, "aids_to_navigation": 0.1, "bottom_surface": 0.1,
        "depth_to_draft": 0.2, "cargo_hazard": 0.0,
    },
    "bank_clearance_outer": 0.5,    # W_Br, fraction of beam (steep / hard bank)
    "bank_clearance_inner": 0.3,    # W_Bg, fraction of beam (sloping bank)
}


def _pianc(what: str, loa_m: float, beam_m: float, options: Options, radius_m: float = 1.0,
           deflection_deg: float = 45.0):
    """One of the built-in method's callables, run by the Rust core (the method itself lives there)."""
    r = _rust.call("methods", "evaluate", name="pianc", loa=loa_m, beam=beam_m, options=dict(options or {}),
                   radii=[radius_m], deflection_deg=deflection_deg)
    if what == "bend":
        return r["bends"][0]
    if what == "breakdown":
        return r["breakdown"]
    return r[what if what == "straight" else "min_radius"]


def _pianc_straight(loa_m: float, beam_m: float, options: Options) -> float:
    return _pianc("straight", loa_m, beam_m, options)


def _pianc_bend(loa_m: float, beam_m: float, options: Options, bend_radius_m: float,
                design_radius_m: float, deflection_deg: float) -> float:
    return _pianc("bend", loa_m, beam_m, options, bend_radius_m, deflection_deg)


def _pianc_min_radius(loa_m: float, options: Options) -> float:
    return _pianc("min_radius", loa_m, 0.0, options)


def _pianc_breakdown(loa_m: float, beam_m: float, options: Options) -> dict:
    return _pianc("breakdown", loa_m, beam_m, options)


PIANC = register_width_method(WidthMethod(
    name="pianc",
    straight_width=_pianc_straight,
    bend_width=_pianc_bend,
    min_radius=_pianc_min_radius,
    breakdown=_pianc_breakdown,
    description="PIANC: straight = k_w x beam (or the Report 121 build-up); "
                "bend = straight + LOA^2 / (8R); R_min = k_r x LOA",
))


def _core_methods(call, name: Optional[str] = None) -> dict:
    """The registry as the Rust core needs it for one call: every method but the built-in
    ``"pianc"`` (the core has its own), each as a callback into this process.

    ``name`` is the method the call is about; asking for it here loads entry points on a miss and
    raises the usual ``ValueError`` for an unknown one, exactly as before.
    """
    if name is not None:
        get_width_method(name)
    out = []
    for m in list(_REGISTRY.values()):
        if m is PIANC:
            continue
        out.append({"name": m.name, "description": m.description, "breakdown": m.breakdown is not None,
                    "id": call.json(_handler(m))})
    return {"host_methods": out}


def _handler(m: WidthMethod):
    def handle(req):
        req = _rust.plain(req)
        op, opts = req["op"], req.get("options", {})
        if op == "straight":
            return m.straight_width(req["loa"], req["beam"], opts)
        if op == "bend":
            return m.bend_width(req["loa"], req["beam"], opts, req["bend_radius"], req["design_radius"],
                                req["deflection_deg"])
        if op == "min_radius":
            return m.min_radius(req["loa"], opts)
        return m.breakdown(req["loa"], req["beam"], opts)
    return handle
