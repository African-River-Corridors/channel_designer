"""Simple one-call channel sizing: straight width and a single design bend.

A thin layer over a width method (:mod:`.methods`, default ``"pianc"``) and
:mod:`.design` (depth/UKC). For curvature-driven
widening along a real centreline use :func:`channel_designer.rules.design.compute`; for a
named standard and vessel class use :func:`channel_designer.rules.rulesets.channel_shape`.
"""
from __future__ import annotations

from .. import _rust

from dataclasses import dataclass, field

# Re-exported from the canonical module (single source of truth).
from .design import (  # noqa: F401
    under_keel_clearance,
    required_depth,
    max_draft_for_depth,
)
from .methods import _core_methods


@dataclass
class PIANCInputs:
    beam_m: float
    loa_m: float                          # barge length (drives bends; excludes pusher)
    draft_m: float = 0.0                  # laden draft (0 → size from a given depth instead)
    width_multiple_straight: float = 2.5  # one-lane bottom width = k_w × beam
    radius_multiple: float = 4.0          # min bend radius = k_r × LOA
    ukc_fraction: float = 0.15            # under-keel clearance as fraction of draft
    ukc_min_m: float = 0.5                # minimum absolute UKC
    side_slope_h_per_v: float = 3.0
    passing_place_spacing_m: float = 2000.0      # one passing place every ~2 km
    passing_place_length_multiple: float = 5.0   # passing-place length = k × LOA
    width_method: str = "pianc"           # a registered width method (see .methods)
    width_options: dict = field(default_factory=dict)   # the method's own parameters


@dataclass
class PIANCChannel:
    bottom_width_straight_m: float
    bottom_width_bend_m: float
    bend_widening_m: float
    min_bend_radius_m: float
    required_depth_m: float          # depth implied by the given draft (0 if draft not given)
    max_draft_for_depth_m: float     # laden draft a given channel depth allows
    top_width_straight_m: float      # at the design depth (bottom + 2 × slope × depth)
    design_depth_m: float
    passing_place_spacing_m: float
    passing_place_length_m: float


def size_channel(inp: PIANCInputs, depth_m: float | None = None) -> PIANCChannel:
    """Compute one-lane channel geometry for a barge with a width method.

    Default (``"pianc"``): straight bottom width = ``width_multiple_straight x beam``; bend
    width = straight + LOA²/(8R) at the design radius R = radius_multiple × LOA. For
    curvature-driven widening along a real surveyed centreline, use
    :func:`channel_designer.rules.design.compute`.
    """
    c = _rust.Call()
    r = _rust.call("simple", "size_channel", _call=c, inputs=inp, depth_m=depth_m,
                   **_core_methods(c, inp.width_method))
    return _rust.decode(r, PIANCChannel)
