"""Channel sizing rules: the selected Rule Sets plus a vessel class give the channel shape.

- :mod:`.rulesets` — ``channel_shape`` and ``design_basis`` by Rule Set selection (spec 0007),
  and the ``size_channel`` path.
- :mod:`.methods` — the pluggable width-method registry behind ``size_channel`` and
  ``design.compute`` (built-in ``"pianc"``; add your own with ``register_width_method``). The width
  methods are design assumptions, not verified rules.
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
    ChannelRuleInputs,
    ChannelRuleResult,
    ChannelShape,
    channel_shape,
    classify_bend,
    design_basis,
    lateral_room_m,
    rule_sets,
    size_channel,
    width_at_radius,
)
