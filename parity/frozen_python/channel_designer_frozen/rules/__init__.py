"""Channel sizing rules: a named standard plus a vessel class gives the channel shape.

- :mod:`.rulesets` — named rulesets (PIANC-A/B/C, CN family), ``channel_shape``, and the
  ``size_channel`` path.
- :mod:`.methods` — the pluggable width-method registry behind ``size_channel`` and
  ``design.compute`` (built-in ``"pianc"``; add your own with ``register_width_method``).
- :mod:`.design` — per-chainage sizing driven by centreline curvature.
- :mod:`.simple` — one-call straight + single-bend sizing.
- :mod:`.width_profile` — ``width(radius)`` callable for the alignment solver.

Pure Python + numpy. Vessel classes come from ``vessel_designer``.
"""
from .methods import (  # noqa: F401
    WidthMethod,
    get_width_method,
    register_width_method,
    width_methods,
)
from .rulesets import (  # noqa: F401
    RULESETS,
    ChannelRuleInputs,
    ChannelRuleResult,
    ChannelShape,
    Ruleset,
    channel_shape,
    classify_bend,
    get_ruleset,
    lateral_room_m,
    register_ruleset,
    rule_sets,
    size_channel,
    width_at_radius,
)
