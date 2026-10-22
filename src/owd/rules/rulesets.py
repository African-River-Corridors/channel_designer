"""Channel sizing: Rule Sets (``channel_shape``) and width methods (``size_channel``).

Two front doors:

- ``channel_shape(vessel_standard_id, rule_sets=[...], set_inputs={...})`` — the selected Rule
  Sets (``pianc-wg141``, ``gb50139``, or a file of your own) plus a vessel class give the channel
  shape, sized by the shared rules engine (spec 0007): the strictest Rule governs.
- ``size_channel(ChannelRuleInputs(...))`` — one-lane channel *geometry* (straight width, min
  turn radius, bend width, required depth, bed level against the pool's minimum water level)
  from a *width method*. The package ships ``"pianc"``; another package can register more —
  see :mod:`owd.rules.methods`. The width methods, the ``"pianc"`` method with its Report 121
  build-up and ``ChannelDesignParams`` are design assumptions, not verified rules (Report 121 is a
  maritime approach-channel guideline).

``"pianc"`` is the PIANC concept-design path: straight width = ``k_w x beam`` (or the Report
121 build-up W = W_BM + sum(W_i) + W_Br + W_Bg), min radius = ``k_r x LOA``, bend widening
LOA^2/(8R). The coefficients are typical concept-design values for a slow-speed inland reach;
**confirm them against the PIANC report and your own design basis before use**.

Water level / bed
-----------------
``min_water_level_m`` is the pool's minimum water level (masl). The design bed sits at
``bed_level_m = min_water_level_m - required_depth``. A pool at sea level (min WL = 0 masl)
puts the bed *below* datum (a marine/tidal dredge reach) — flagged.

Pure Python (math only) — Pyodide-safe.
"""
from __future__ import annotations

from .. import _rust

from dataclasses import dataclass, field
from typing import Optional

from vessel_designer.core.checks import Check
from .design import required_depth
from .methods import PIANC_MANEUVER as _PIANC_MANEUVER, _core_methods, width_methods


def rule_sets() -> tuple:
    """Width methods ``size_channel`` accepts (``ChannelRuleInputs.rule_set``): the built-in
    ``"pianc"`` plus any a package has registered. The width methods are design assumptions, not
    verified rules. The Chinese standard is reached through the Rule Sets:
    ``channel_shape(..., rule_sets=["gb50139"], set_inputs={"range_end": ...})``."""
    return width_methods()


# =====================================================================================
# BEND TIGHTNESS — does the channel still fit the river through this bend?
# =====================================================================================
# A channel rounding a bend at its minimum radius cannot trace the river's centreline; it
# cuts the corner, so its centreline sits a lateral distance e from the river's. The bend
# is TIGHT when e exceeds the lateral ROOM available, and a tight bend is what the
# alignment optimiser has to solve rather than follow.
#
# WHAT "ROOM" MEANS IS A JUDGEMENT, NOT A GEOMETRIC FACT, and on a narrow river it decides
# most of the answer. Where the channel is about as wide as the river, the genuine spare room
# is close to zero.
#
#   room = w/2                       -> lets the channel CENTRELINE sit at the bank, i.e. half
#                                       the channel outside the river, while still calling the
#                                       bend "gentle". That is a large bank cut hidden in geometry.
#   room = (w - W_top)/2 + b_bank    -> the honest form: the spare room the river actually
#                                       has, plus an EXPLICIT permitted bank cut.
#
# BANK_CUT_FRACTION is a USER CHOICE, not a standard: b_bank = BANK_CUT_FRACTION x channel
# BOTTOM width, i.e. the design channel bottom must still substantially overlap the natural
# bed. It is a fraction rather than metres so it scales with the vessel. 0.40 is an example
# default, not a recommendation; pass your own to ``lateral_room_m``.
BANK_CUT_FRACTION = 0.40


def lateral_room_m(wetted_width_m: float, top_width_m: float, bottom_width_m: float,
                   bank_cut_fraction: float = BANK_CUT_FRACTION) -> float:
    """Lateral room per side before a bend counts as TIGHT.

    room = (w - W_top)/2 + bank_cut_fraction * W_bottom

    The first term is the river's genuine spare room (often ~0, and negative where the
    channel is wider than the river — clamped at 0, since a channel wider than the river has
    no spare room, it does not have negative room). The second is the permitted bank cut.
    """
    return _rust.call("rulesets", "lateral_room_m", float, wetted_width_m=wetted_width_m,
                      top_width_m=top_width_m, bottom_width_m=bottom_width_m,
                      bank_cut_fraction=bank_cut_fraction)


def classify_bend(e_m: float, room_m: float, tol_m: float = 1e-9) -> str:
    """Bend class from the offset a minimum-radius curve needs against the room available.

    'S' straight  — the bend needs no offset at all
    'G' gentle    — it fits inside the room; the channel can follow the river
    'T' tight     — it does not; this bend is the optimiser's problem
    """
    return _rust.call("rulesets", "classify_bend", str, e_m=e_m, room_m=room_m, tol_m=tol_m)


def _selection(rule_sets, set_inputs, reaches, rule_set_files) -> dict:
    """The selection a call names (spec 0007), as the Rust core reads it."""
    sel = {"rule_sets": list(rule_sets), "set_inputs": dict(set_inputs or {})}
    if reaches:
        sel["reaches"] = [dict(r) for r in reaches]
    if rule_set_files:
        sel["rule_set_files"] = list(rule_set_files)
    return sel


@dataclass
class ChannelShape:
    """The channel a selection of Rule Sets and a vessel class require. Everything here is derived."""

    rule_sets: str                 # the selected sets, ``id@version`` joined with `` + ``
    vessel_standard_id: str
    loa_m: float
    beam_m: float
    draught_m: float
    formation: str
    bottom_width_straight_m: float
    min_bend_radius_m: float
    design_depth_m: float
    bend_widening_at_rmin_m: float
    bottom_width_at_rmin_m: float
    width_basis: str = ""          # "table" | "table-5.1-ratio" | "rule"
    source: str = ""
    warnings: list = field(default_factory=list)
    selection: dict = field(default_factory=dict)   # what sized it, for the widening at a radius

    def bend_widening_m(self, radius_m: float) -> float:
        rows = _rust.call("rulesets", "channel_shape_widths", list, radii=[radius_m],
                          vessel_standard_id=self.vessel_standard_id, draught_m=self.draught_m,
                          **self.selection)
        return _rust.plain(rows[0]["bend_widening_m"])

    def bottom_width_at_m(self, radius_m: float) -> float:
        return self.bottom_width_straight_m + self.bend_widening_m(radius_m)


def channel_shape(vessel_standard_id: str, draught_m: float = None, *,
                  rule_sets=("pianc-wg141",), set_inputs: Optional[dict] = None,
                  reaches=None, rule_set_files=()) -> ChannelShape:
    """THE FRONT DOOR: the selected Rule Sets plus a vessel class fully determine the channel.

    ``rule_sets`` are Rule Set ids or ``id@version`` (``owd rules`` lists them): every Rule of every
    selected set applies and the strictest governs (spec 0007). ``set_inputs`` gives their inputs,
    such as ``{"se_quality": "B"}`` or ``{"range_end": "strict"}``; ``pianc-wg141`` takes quality C
    when none is given. ``reaches`` is a list of ``{"waterway", "reduced_radius_claim"}`` for a
    line over one reach kind, such as a claimed canal for the reduced GB 50139 radius.
    ``rule_set_files`` are Rule Set file texts loaded first. The ``owd`` checks are always selected.

    ``draught_m`` overrides the vessel class's nominal draught: a project's design draught is its
    own decision, so it stays injectable.
    """
    sel = _selection(rule_sets, set_inputs, reaches, rule_set_files)
    args = {"vessel_standard_id": vessel_standard_id, **sel}
    if draught_m is not None:
        args["draught_m"] = draught_m
    shape = _rust.call("rulesets", "channel_shape", ChannelShape, **args)
    shape.selection = sel
    return shape


def design_basis(vessel_standard_id: str, draught_m: float = None, *,
                 rule_sets=("pianc-wg141",), set_inputs: Optional[dict] = None,
                 reaches=None, rule_set_files=()) -> list:
    """The design basis of the channel ``channel_shape`` sizes (spec 0007): one row per quantity,
    each with the Rule that governs it, its sets, citation, origin and verification status. Every
    Rule is unverified until a second reader checks it against its source."""
    args = {"vessel_standard_id": vessel_standard_id,
            **_selection(rule_sets, set_inputs, reaches, rule_set_files)}
    if draught_m is not None:
        args["draught_m"] = draught_m
    return _rust.plain(_rust.call("rulesets", "design_basis", list, **args))


@dataclass
class ChannelRuleInputs:
    """Vessel + reach inputs for ``size_channel``.

    ``rule_set`` names a width method (:func:`rule_sets`). The ``pianc_*`` fields are the
    built-in method's parameters; ``options`` carries a registered method's own parameters
    and is passed to it untouched (for ``"pianc"`` it overrides the ``pianc_*`` fields).
    """

    beam_m: float
    loa_m: float                         # convoy length overall (barge + pusher) — drives bends
    draft_m: float
    min_water_level_m: float             # pool minimum water level (masl); pool 0 = 0.0
    rule_set: str = "pianc"

    side_slope_h_per_v: float = 3.0
    ukc_fraction: float = 0.15
    ukc_min_m: float = 0.5
    reference_bend_radius_m: Optional[float] = None   # evaluate bend width here (default 3·LOA)

    # --- built-in "pianc" method parameters ---
    # Default path: one-lane fairway = 2.8 × beam + bend widening LOA²/(8R), R_min = 3 × LOA.
    # NOTE this LOA²/(8R) widening (c_C = 0.125) is BELOW WG 141's own floor of c_C ≈ 0.5 — see
    # the Rule Sets (spec 0007). Prefer ``channel_shape`` with a Rule Set for new work.
    # The Report-121 concept-design build-up (W_BM + ΣWᵢ + bank clearances) is retained behind
    # ``pianc_use_buildup=True`` for later coefficient confirmation against the source PDFs.
    pianc_width_multiple: float = 2.8            # one-lane fairway = k_w × beam (default path)
    pianc_use_buildup: bool = False              # True -> Report-121 W_BM + ΣWᵢ + banks instead
    pianc_maneuverability: str = "poor"          # good/moderate/poor -> W_BM = 1.3/1.5/1.8 B
    pianc_radius_multiple: float = 3.0           # PIANC inland min bend radius = k·LOA
    # additional widths ΣWᵢ, each a fraction of beam (slow-speed inland defaults):
    pianc_extra_widths: dict = field(default_factory=lambda: {
        "prevailing_wind": 0.1, "cross_current": 0.4, "longitudinal_current": 0.0,
        "wave": 0.0, "aids_to_navigation": 0.1, "bottom_surface": 0.1,
        "depth_to_draft": 0.2, "cargo_hazard": 0.0,
    })
    pianc_bank_clearance_outer: float = 0.5      # W_Br, fraction of beam (steep/hard bank)
    pianc_bank_clearance_inner: float = 0.3      # W_Bg, fraction of beam (sloping bank)

    # --- a registered width method's own parameters ---
    options: dict = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.rule_set == "china":
            raise ValueError(
                "rule_set='china' is not implemented on the size_channel path (it used to run "
                "the PIANC path silently). Use channel_shape(vessel_standard_id=..., "
                "rule_sets=['gb50139'], set_inputs={'range_end': ...}) instead.")
        known = rule_sets()
        if self.rule_set not in known:
            raise ValueError(f"rule_set must be one of {known} (register more with "
                             "owd.rules.methods.register_width_method)")
        for name in ("beam_m", "loa_m", "draft_m"):
            if getattr(self, name) <= 0:
                raise ValueError(f"{name} must be > 0")
        if self.pianc_maneuverability not in _PIANC_MANEUVER:
            raise ValueError(f"pianc_maneuverability must be one of {tuple(_PIANC_MANEUVER)}")

    def method_options(self) -> dict:
        """The options passed to the width method."""
        if self.rule_set != "pianc":
            return dict(self.options)
        o = {
            "width_multiple": self.pianc_width_multiple,
            "use_buildup": self.pianc_use_buildup,
            "maneuverability": self.pianc_maneuverability,
            "radius_multiple": self.pianc_radius_multiple,
            "extra_widths": dict(self.pianc_extra_widths),
            "bank_clearance_outer": self.pianc_bank_clearance_outer,
            "bank_clearance_inner": self.pianc_bank_clearance_inner,
        }
        o.update(self.options)
        return o


@dataclass
class ChannelRuleResult:
    rule_set: str
    straight_bottom_width_m: float
    min_turn_radius_m: float             # the rule's nominal minimum bend radius
    reference_bend_radius_m: float
    bend_bottom_width_m: float           # required bottom width through the reference bend
    bend_widening_m: float               # bend width − straight width
    required_depth_m: float
    under_keel_clearance_m: float
    min_water_level_m: float
    bed_level_m: float                   # = min_water_level − required_depth
    top_width_straight_m: float          # bottom + 2·slope·depth
    width_breakdown: dict = field(default_factory=dict)
    checks: list = field(default_factory=list)
    warnings: list = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return all(c.ok for c in self.checks if c.severity != "warning")


# The deflection a single reference bend is assumed to turn, for methods whose bend width
# depends on it. The built-in "pianc" method ignores it.
REFERENCE_DEFLECTION_DEG = 45.0


def width_at_radius(inp: ChannelRuleInputs, radius_m: float) -> float:
    """Required bottom width (m) at a bend of the given radius — the single width
    authority for the alignment solver's width-aware edge cost.

    Straight (radius = inf or <= 0 treated as straight) → the method's straight width.
    Otherwise ``max(straight, bend_width(R1 = R2 = R, 45°))``; for ``"pianc"`` that is
    straight + LOA²/(8R). Monotonic non-increasing in R for every shipped method.
    """
    c = _rust.Call()
    return _rust.call("rulesets", "width_at_radius", _call=c, inputs=inp, radii=[radius_m],
                      **_core_methods(c, inp.rule_set))[0]


def size_channel(inp: ChannelRuleInputs) -> ChannelRuleResult:
    """One-lane channel geometry under the selected width method (``inp.rule_set``)."""
    c = _rust.Call()
    r = _rust.call("rulesets", "size_channel", _call=c, inputs=inp, **_core_methods(c, inp.rule_set))
    res = _rust.plain(r["result"])
    res["checks"] = [Check(**ck) for ck in res["checks"]]
    return ChannelRuleResult(**res)


# =============================================================================
# Worked example — run: python -m owd.rules.rulesets
# =============================================================================

def _print_case(title, base_kwargs):
    print(f"\n{'='*74}\n{title}\n{'='*74}")
    for rs in rule_sets():
        r = size_channel(ChannelRuleInputs(rule_set=rs, **base_kwargs))
        print(f"\n  [{rs.upper()}]  straight bottom {r.straight_bottom_width_m:6.1f} m   "
              f"R_min {r.min_turn_radius_m:7.0f} m   bend width {r.bend_bottom_width_m:6.1f} m "
              f"(+{r.bend_widening_m:4.1f})")
        print(f"        depth {r.required_depth_m:.2f} m (UKC {r.under_keel_clearance_m:.2f})   "
              f"bed {r.bed_level_m:+.2f} masl (WL {r.min_water_level_m:.1f})   "
              f"top width {r.top_width_straight_m:.1f} m")
        print(f"        breakdown: {r.width_breakdown}")
        for w in r.warnings:
            print(f"        ⚠ {w}")


if __name__ == "__main__":
    # Illustrative inputs: a CEMT class IV motor vessel and a class Va vessel.
    _print_case("SET A — beam 9.5, LOA 85, draft 2.5 m, pool WL 10.0 masl",
                dict(beam_m=9.5, loa_m=85.0, draft_m=2.5, min_water_level_m=10.0))
    _print_case("SET B — beam 11.4, LOA 110, draft 2.8 m, pool WL 10.0 masl",
                dict(beam_m=11.4, loa_m=110.0, draft_m=2.8, min_water_level_m=10.0))
    _print_case("SET C — beam 9.5, LOA 85, draft 2.5 m, pool at SEA LEVEL (min WL 0.0 masl)",
                dict(beam_m=9.5, loa_m=85.0, draft_m=2.5, min_water_level_m=0.0))
    print("\nok")
