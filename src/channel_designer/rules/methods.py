"""Width methods — the pluggable registry behind ``size_channel``, ``width_at_radius`` and
``design.compute``.

A *width method* turns a vessel (length overall and beam) into a straight channel width, a
width through a bend, and a design minimum bend radius. The package ships one method,
``"pianc"``. Another package can add its own, for example a consultant's method it may not
publish, without editing this one:

.. code-block:: python

    from channel_designer.rules.methods import WidthMethod, register_width_method

    register_width_method(WidthMethod(
        name="my-method",
        straight_width=lambda loa, beam, opts: 2.5 * beam,
        bend_width=lambda loa, beam, opts, bend_r, design_r, defl: 2.5 * beam + loa**2 / (4 * bend_r),
        min_radius=lambda loa, opts: 4.0 * loa,
        description="my-method: 2.5 x beam, LOA^2/(4R) bend widening",
    ))

or declare it as an entry point, so it is found without an import:

.. code-block:: toml

    [project.entry-points."channel_designer.width_methods"]
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

import math
from dataclasses import dataclass
from typing import Callable, Mapping, Optional

ENTRY_POINT_GROUP = "channel_designer.width_methods"

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
    for ep in entry_points(group=ENTRY_POINT_GROUP):
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


def _pianc_opts(options: Options) -> dict:
    o = dict(PIANC_DEFAULTS)
    o.update(options or {})
    if o["maneuverability"] not in PIANC_MANEUVER:
        raise ValueError(f"maneuverability must be one of {tuple(PIANC_MANEUVER)}")
    return o


def _pianc_straight(loa_m: float, beam_m: float, options: Options) -> float:
    o = _pianc_opts(options)
    if not o["use_buildup"]:
        return o["width_multiple"] * beam_m
    w_bm = PIANC_MANEUVER[o["maneuverability"]] * beam_m
    extra = sum(o["extra_widths"].values()) * beam_m
    banks = (o["bank_clearance_outer"] + o["bank_clearance_inner"]) * beam_m
    return w_bm + extra + banks


def _pianc_bend(loa_m: float, beam_m: float, options: Options, bend_radius_m: float,
                design_radius_m: float, deflection_deg: float) -> float:
    straight = _pianc_straight(loa_m, beam_m, options)
    if bend_radius_m <= 0 or math.isinf(bend_radius_m):
        return straight
    return straight + loa_m ** 2 / (8.0 * bend_radius_m)


def _pianc_min_radius(loa_m: float, options: Options) -> float:
    return _pianc_opts(options)["radius_multiple"] * loa_m


def _pianc_breakdown(loa_m: float, beam_m: float, options: Options) -> dict:
    o = _pianc_opts(options)
    B = beam_m
    if not o["use_buildup"]:
        return {"fairway_kw_x_beam": round(o["width_multiple"] * B, 2),
                "width_multiple": o["width_multiple"]}
    return {
        "W_BM_manoeuvring_lane": round(PIANC_MANEUVER[o["maneuverability"]] * B, 2),
        "sum_Wi_additional": round(sum(o["extra_widths"].values()) * B, 2),
        "bank_clearances_Br_Bg": round(
            (o["bank_clearance_outer"] + o["bank_clearance_inner"]) * B, 2),
        "maneuverability": o["maneuverability"],
    }


PIANC = register_width_method(WidthMethod(
    name="pianc",
    straight_width=_pianc_straight,
    bend_width=_pianc_bend,
    min_radius=_pianc_min_radius,
    breakdown=_pianc_breakdown,
    description="PIANC: straight = k_w x beam (or the Report 121 build-up); "
                "bend = straight + LOA^2 / (8R); R_min = k_r x LOA",
))
