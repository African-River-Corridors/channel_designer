"""Thin bridge: channel bottom width as a function of bend radius, sourced from the
single width authority (:func:`channel_designer.rules.rulesets.width_at_radius`).

No width math is duplicated here — this module only wires inputs through and returns a
callable the alignment solver can use as a width-aware edge cost.
"""
from __future__ import annotations

from typing import Callable

from .rulesets import ChannelRuleInputs, width_at_radius


def make_width_fn(
    loa_m: float,
    beam_m: float,
    draft_m: float,
    rule_set: str = "pianc",
    **rule_overrides,
) -> Callable[[float], float]:
    """Return ``width(radius_m) -> bottom width (m)`` delegating to
    ``channel_rules.width_at_radius``. ``rule_overrides`` are passed straight through to
    ``ChannelRuleInputs`` (e.g. ``pianc_maneuverability``, ``reference_bend_radius_m``,
    ``options`` for a registered method,
    or ``min_water_level_m`` if the caller needs a non-default pool level — width_at_radius
    does not use it, so it defaults to 0.0 here).

    The returned callable also exposes ``.half_width(radius_m)`` (= width/2, the ``hb``
    the FaithfulScorer template uses) as a convenience attribute.
    """
    rule_overrides.setdefault("min_water_level_m", 0.0)
    inputs = ChannelRuleInputs(
        beam_m=beam_m, loa_m=loa_m, draft_m=draft_m, rule_set=rule_set, **rule_overrides
    )

    def width(radius_m: float) -> float:
        return width_at_radius(inputs, radius_m)

    def half_width(radius_m: float) -> float:
        return width(radius_m) / 2.0

    width.half_width = half_width
    return width


def width_params_dict(
    loa_m: float,
    beam_m: float,
    draft_m: float,
    rule_set: str = "pianc",
    **rule_overrides,
) -> dict:
    """Provenance dict for artifact metadata (``alignment_key.width_params``)."""
    rule_overrides.setdefault("min_water_level_m", 0.0)
    inputs = ChannelRuleInputs(
        beam_m=beam_m, loa_m=loa_m, draft_m=draft_m, rule_set=rule_set, **rule_overrides
    )
    straight_bottom_m = width_at_radius(inputs, float("inf"))
    from .methods import get_width_method
    widening_coeff_note = get_width_method(rule_set).description
    return {
        "rule_set": rule_set,
        "straight_bottom_m": straight_bottom_m,
        "widening_coeff_note": widening_coeff_note,
        "loa_m": loa_m,
        "beam_m": beam_m,
        "draft_m": draft_m,
    }
