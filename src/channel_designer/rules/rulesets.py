"""Channel-sizing rules: named rulesets (``channel_shape``) and width methods (``size_channel``).

Two front doors:

- ``channel_shape(ruleset_id, vessel_standard_id)`` — a named standard (PIANC WG 141
  qualities A/B/C, the Chinese GB 50139 family) plus a vessel class gives the channel shape.
  Rulesets are coefficients as data; add your own with :func:`register_ruleset`.
- ``size_channel(ChannelRuleInputs(...))`` — one-lane channel *geometry* (straight width, min
  turn radius, bend width, required depth, bed level against the pool's minimum water level)
  from a *width method*. The package ships ``"pianc"``; another package can register more —
  see :mod:`channel_designer.rules.methods`.

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

from dataclasses import dataclass, field
from typing import Optional

from vessel_designer.core.checks import Check
from .design import required_depth, under_keel_clearance
from .methods import PIANC_MANEUVER as _PIANC_MANEUVER, get_width_method, width_methods


def rule_sets() -> tuple:
    """Width methods ``size_channel`` accepts (``ChannelRuleInputs.rule_set``): the built-in
    ``"pianc"`` plus any a package has registered. The Chinese standard is reached through the
    named-ruleset front door, ``channel_shape(ruleset_id="CN", ...)``."""
    return width_methods()


# =====================================================================================
# RULESETS — a channel design standard, named. Ruleset x vessel class -> channel shape.
# =====================================================================================
# ONE axis, not two: PIANC's Safety & Ease qualities are part of
# the PIANC ruleset, because the concept does not travel — the Chinese standard has no
# A/B/C tiers at all. Collapsing them into named rulesets avoids inventing a mapping the
# Chinese standard does not contain.
#
# The VESSEL CLASS (see vessel_standards) supplies LOA, beam, draught and formation, so a
# ruleset never needs to ask whether it is sizing for a convoy or a motor vessel — it looks
# that up. Together the two choices fully determine the channel shape. Nothing is typed.
#
# WHERE THE TWO FAMILIES DISAGREE, which is why both are carried:
#   width  PIANC charges for BEAM only (k_w x B). China charges for beam AND LENGTH, via a
#          drift angle: swept = B + L*sin(beta), plus a bank increment. Long convoys cost
#          more under China; short wide ones cost less.
#   bend   PIANC dF_C = c_C*L^2/R, capped <= L. China dB = L^2/(2R + B) which for R >> B is
#          ~0.5*L^2/R, i.e. PIANC's c_C = 0.5. Two independent traditions on the same
#          coefficient — strong evidence c_C ~ 0.5 is the floor, and that the LOA^2/(8R)
#          this module used (c_C = 0.125) never was.
#   depth  PIANC sets a RATIO (1.3 x T). China sets draught PLUS A FIXED CLEARANCE. They
#          diverge with draught: at T = 4.5 m China is ~16% shallower.

from typing import Callable, Tuple as _Tuple


@dataclass(frozen=True)
class Ruleset:
    """One named channel-design standard."""

    ruleset_id: str
    family: str                   # "pianc" | "china"
    label: str
    source: str
    r_mult_convoy: float          # R_min = mult x LOA, pushed convoy
    r_mult_motor: float           # R_min = mult x LOA, self-propelled
    width_mult: float = 0.0       # pianc: bottom width = width_mult x beam
    c_c: float = 0.0              # pianc: bend coefficient
    depth_mult: float = 0.0       # pianc: depth = depth_mult x draught
    bank_mult_convoy: float = 0.0  # china: bank increment, fraction of swept path
    bank_mult_motor: float = 0.0
    depth_add_m: float = 0.0      # china: depth = draught + this
    notes: str = ""


# PIANC WG 141 (2019), free-flowing river, single-lane.
#   width  Table 5.7 (p109). Table 5.7 prints 2.8*B for quality C but carries a 3.0*B
#          minimum "for security reasons" spanning C and B, so C is 3.0, not 2.8.
#   c_c    Table 5.7 fn.3 + Table E.5 (p261). A is read from Table E.5 by VESSEL CLASS;
#          0.6 is Class VIa loaded downstream (short, wide convoys). B follows from A
#          by the bow-thruster reduction in the Table E.5 remarks. C is the field-data
#          floor and the Figure 5.5 single-lane gradient.
#   depth  5.3.4.3 (p108): 1.3 x draught, never below 1.2.
#   R_min  Table 5.7 (p109): 2L / 3L / 4L.
# Chinese GB 50139, as summarised by WG 141 Appendix A.3 (p162-164).
#   width  A.3.2/A.3.3: swept = B + L*sin(beta); bank increment 0.25-0.30 of swept for
#          convoys, 0.34-0.40 for motor vessels. CN uses the permissive end, CN-conservative
#          the strict end — the standard states a range and does not tier it.
#   depth  A.3.2: draught + 0.4-0.5 m (class I) or 0.3-0.4 m (classes II-III).
#   R_min  A.3.2: 3L pushed fleet / 4L cargo vessel, reducible to 2L / 3L where the extra
#          width and driving visibility allow. CN-reduced is that reduction.
RULESETS: dict = {
    "PIANC-A": Ruleset("PIANC-A", "pianc", "PIANC WG 141 — Safety & Ease quality A",
                       "WG 141 Table 5.7 (p109), Table E.5 (p261)",
                       r_mult_convoy=4.0, r_mult_motor=4.0,
                       width_mult=3.4, c_c=0.6, depth_mult=1.3,
                       notes="Highest ease. c_C from Table E.5 for Class VIa laden downstream."),
    "PIANC-B": Ruleset("PIANC-B", "pianc", "PIANC WG 141 — Safety & Ease quality B",
                       "WG 141 Table 5.7 (p109)",
                       r_mult_convoy=3.0, r_mult_motor=3.0,
                       width_mult=3.2, c_c=0.5, depth_mult=1.3,
                       notes="c_C reduced from A by the Table E.5 bow-thruster allowance."),
    "PIANC-C": Ruleset("PIANC-C", "pianc", "PIANC WG 141 — Safety & Ease quality C",
                       "WG 141 Table 5.7 (p109) incl. the 3.0*B security minimum",
                       r_mult_convoy=2.0, r_mult_motor=2.0,
                       width_mult=3.0, c_c=0.5, depth_mult=1.2,
                       notes="Lowest ease. Restricted driving; skilled pilots assumed."),
    "CN": Ruleset("CN", "china", "Chinese GB 50139 — standard",
                  "WG 141 Appendix A.3 (p162-164)",
                  r_mult_convoy=3.0, r_mult_motor=4.0,
                  bank_mult_convoy=0.25, bank_mult_motor=0.34, depth_add_m=0.3,
                  notes="Permissive end of every stated range."),
    "CN-conservative": Ruleset("CN-conservative", "china", "Chinese GB 50139 — conservative",
                               "WG 141 Appendix A.3 (p162-164)",
                               r_mult_convoy=3.0, r_mult_motor=4.0,
                               bank_mult_convoy=0.30, bank_mult_motor=0.40, depth_add_m=0.5,
                               notes="Strict end of every stated range."),
    "CN-reduced": Ruleset("CN-reduced", "china", "Chinese GB 50139 — reduced radius",
                          "WG 141 Appendix A.3 (p162-164)",
                          r_mult_convoy=2.0, r_mult_motor=3.0,
                          bank_mult_convoy=0.30, bank_mult_motor=0.40, depth_add_m=0.5,
                          notes="A USER CHOICE built on GB 50139, not a named Chinese ruleset. "
                                "A.3.2 (p164) permits the radius to drop to 2L convoy / 3L "
                                "motor where the extra width and driving visibility allow; this "
                                "ruleset pairs that with the STRICT end of the width and depth "
                                "ranges. The clause is written for CANAL sections. On a "
                                "canalised river only the cut-throughs are canal, so this "
                                "ruleset may apply to PART of an alignment only. Do not quote a "
                                "whole-river total under it without saying so."),
}

# PIANC (2019) Report No. 141, Design Guidelines for Inland Waterways, Table A.2 (p163): the
# Chinese standard's own single-lane river widths, which include bank clearance and wind.
# ONLY the entries the code reads are kept: those the vessel classes in vessel_designer match
# (``_cn_table_width``; ``test_cn_table_a2_holds_only_what_is_read`` proves it). Key (LOA m,
# beam m) -> width m. For any other vessel the width falls back to the Table 5.1 ratio below.
CN_TABLE_A2 = {
    (223.0, 32.4): 70.0,
    (186.0, 32.4): 70.0,
    (167.0, 21.6): 45.0,
}

# The A.3.2 swept-path formula under-predicts the Table A.2 widths (by roughly a third, measured
# against the published table). A.3.3 says the formula carries bank and passing increments
# only, so wind is not in it. That gap is why the width basis is a published wind-included
# ratio, not the formula.
#
# The width basis is now WG 141 Table 5.1 (p88), row "China River", single-lane W_F/B = 2.3.
# Why this and not the calibrated formula:
#   * it is the STANDARD'S OWN published ratio, not a factor we derived;
#   * Table 5.1's footnote states in WG 141's words that the "Chinese, German and US profile:
#     wind included", which is exactly the gap A.3.3 leaves open (it lists curvature, bank
#     interaction, cross-flow and passing distance — not wind);
#   * it agrees closely with a formula-times-calibration estimate, so it is a change of BASIS,
#     not of number.
# CAUTIONS, both real, both to be quoted with the number:
#   * Table 5.1 is captioned "Canal fairway dimension in existing guidelines"; the China River
#     row is a river row inside a canal comparison. WG 141 prints it there; we did not move it.
#   * W_F is defined (5.2.1, p87) as the net bank-to-bank distance measured AT DESIGN DRAUGHT
#     DEPTH, not at the bed. We apply it as a BOTTOM width, which on a sloped section is the
#     conservative reading (more excavation).
CN_TABLE_5_1_WIDTH_MULT = 2.3

# Chinese depth clearance, WG 141 A.3.2 (p162): depth = draught + a fixed clearance, given as a
# RANGE per class (class I 0.4-0.5 m; classes II-III 0.3-0.4 m). Classes IV and up are NOT
# covered by the clause; this module does not invent a clearance for them.
CN_DEPTH_CLEARANCE_M = {           # class -> (permissive end, strict end)
    "I":   (0.4, 0.5),
    "II":  (0.3, 0.4),
    "III": (0.3, 0.4),
}

# Which end of that range to take. Default: the STRICT end. GB 50139's clearance is an
# ABSOLUTE add-on calibrated on a class II design draught of 2.6 m (Table A.2); at deeper
# draughts a fixed add-on is a shrinking ratio, so the strict end keeps the design inside the
# standard while acknowledging that. A user choice: set "permissive" to take the low end.
CN_DEPTH_CLEARANCE_END = "strict"

# A.3.2 drift angle building the Chinese swept path: 3 deg classes I-V, 2 deg VI-VII.
_CN_DRIFT_DEG_LARGE_BEAM = 2.0
_CN_DRIFT_DEG_SMALL_BEAM = 3.0
_CN_LARGE_BEAM_M = 22.8


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
    spare = max(0.0, (wetted_width_m - top_width_m) / 2.0)
    return spare + bank_cut_fraction * bottom_width_m


def classify_bend(e_m: float, room_m: float, tol_m: float = 1e-9) -> str:
    """Bend class from the offset a minimum-radius curve needs against the room available.

    'S' straight  — the bend needs no offset at all
    'G' gentle    — it fits inside the room; the channel can follow the river
    'T' tight     — it does not; this bend is the optimiser's problem
    """
    if e_m <= tol_m:
        return "S"
    return "G" if e_m <= room_m + tol_m else "T"


def register_ruleset(ruleset: Ruleset, *, replace: bool = False) -> Ruleset:
    """Add a named ruleset for ``channel_shape``.

    A ruleset is coefficients as data. ``family`` must be ``"pianc"`` (width = width_mult x
    beam, bend dF_C = c_c x L²/R, depth = depth_mult x draught) or ``"china"`` (swept path
    plus bank increment, dB = L²/(2R + B), depth = draught + depth_add_m). A method that is
    not coefficients belongs in :mod:`channel_designer.rules.methods` instead.
    """
    if ruleset.family not in ("pianc", "china"):
        raise ValueError(f"ruleset family must be 'pianc' or 'china'; got {ruleset.family!r}")
    if ruleset.ruleset_id in RULESETS and not replace:
        raise ValueError(f"ruleset {ruleset.ruleset_id!r} is already registered; "
                         "pass replace=True to override it")
    RULESETS[ruleset.ruleset_id] = ruleset
    return ruleset


def get_ruleset(ruleset_id: str) -> Ruleset:
    try:
        return RULESETS[ruleset_id]
    except KeyError:
        raise KeyError(f"unknown ruleset {ruleset_id!r}; known: {', '.join(sorted(RULESETS))}")


@dataclass
class ChannelShape:
    """The channel a (ruleset, vessel class) pair requires. Everything here is derived."""

    ruleset_id: str
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
    width_basis: str = ""          # "table" | "formula+calibration" | "rule"
    source: str = ""
    warnings: list = field(default_factory=list)

    def bend_widening_m(self, radius_m: float) -> float:
        return _bend_widening(get_ruleset(self.ruleset_id), self.loa_m, self.beam_m, radius_m)

    def bottom_width_at_m(self, radius_m: float) -> float:
        return self.bottom_width_straight_m + self.bend_widening_m(radius_m)


def _bend_widening(rs: Ruleset, loa_m: float, beam_m: float, radius_m: float) -> float:
    import math as _m
    if radius_m <= 0 or _m.isinf(radius_m):
        return 0.0
    if rs.family == "china":
        return loa_m * loa_m / (2.0 * radius_m + beam_m)      # A.3.3 (p164)
    return min(rs.c_c * loa_m * loa_m / radius_m, loa_m)      # 5.3.3 (p101), cap dF_C <= L


def _cn_table_width(loa_m: float, beam_m: float):
    """Nearest Table A.2 entry, if the vessel plausibly matches one. Else None."""
    best, best_d = None, None
    for (L, B), w in CN_TABLE_A2.items():
        d = abs(L - loa_m) / max(L, 1.0) + abs(B - beam_m) / max(B, 1.0)
        if best_d is None or d < best_d:
            best, best_d = w, d
    # only trust it as a direct read when both dimensions are within 10%
    return best if (best_d is not None and best_d <= 0.20) else None


def _straight_width(rs: Ruleset, loa_m: float, beam_m: float, formation: str):
    """Returns (width_m, basis, warnings)."""
    import math as _m
    warns = []
    if rs.family == "china":
        w_tab = _cn_table_width(loa_m, beam_m)
        beta = _m.radians(_CN_DRIFT_DEG_LARGE_BEAM if beam_m >= _CN_LARGE_BEAM_M
                          else _CN_DRIFT_DEG_SMALL_BEAM)
        swept = beam_m + loa_m * _m.sin(beta)
        bank = rs.bank_mult_motor if formation == "motor_vessel" else rs.bank_mult_convoy
        w_formula = swept * (1.0 + bank)
        if w_tab is not None:
            if abs(w_tab - w_formula) / w_tab > 0.05:
                warns.append(
                    f"Table A.2 gives {w_tab:.0f} m where the A.3.2 formula gives "
                    f"{w_formula:.1f} m; the table includes wind, the formula does not. "
                    "Using the table.")
            return w_tab, "table", warns
        w = CN_TABLE_5_1_WIDTH_MULT * beam_m
        warns.append(
            f"no Table A.2 entry near LOA {loa_m:.0f} m / beam {beam_m:.1f} m, so the "
            f"width is WG 141 Table 5.1's China-River single-lane ratio "
            f"W_F/B = {CN_TABLE_5_1_WIDTH_MULT} ({w:.1f} m), which the table's own "
            f"footnote records as WIND-INCLUDED. The A.3.2 formula gives {w_formula:.1f} m "
            f"and omits wind (A.3.3 lists curvature, bank interaction, cross-flow and "
            f"passing distance only); the implied ratio between them is "
            f"{w / w_formula:.2f}.")
        return w, "table-5.1-ratio", warns
    return rs.width_mult * beam_m, "rule", warns


def channel_shape(ruleset_id: str, vessel_standard_id: str,
                  draught_m: float = None) -> ChannelShape:
    """THE FRONT DOOR: a ruleset plus a vessel class fully determine the channel.

    ``draught_m`` overrides the vessel class's nominal draught — a project's design draught
    is its own decision, so it stays injectable.
    """
    from vessel_designer.core.vessel_standards import get_standard
    rs = get_ruleset(ruleset_id)
    v = get_standard(vessel_standard_id)
    loa = v.loa_m[1]                    # size on the LONG end of the class
    T = v.draught_nominal_m if draught_m is None else draught_m
    r_mult = rs.r_mult_motor if v.formation == "motor_vessel" else rs.r_mult_convoy
    r_min = r_mult * loa
    width, basis, warns = _straight_width(rs, loa, v.beam_m, v.formation)
    if rs.family == "china":
        rng = CN_DEPTH_CLEARANCE_M.get(v.class_name)
        if rng is None:
            # A.3.2 covers classes I-III only. Do not invent a clearance for IV+.
            clearance = rs.depth_add_m
            warns.append(
                f"GB 50139's depth clause (A.3.2, p162) covers classes I-III only; class "
                f"{v.class_name} is outside it, so the depth falls back to the ruleset's "
                f"{clearance} m add-on, which is a user choice, not the standard's.")
        else:
            clearance = rng[1] if CN_DEPTH_CLEARANCE_END == "strict" else rng[0]
            warns.append(
                f"depth = draught + {clearance} m, the {CN_DEPTH_CLEARANCE_END} end of "
                f"GB 50139's class-{v.class_name} range {rng[0]}-{rng[1]} m "
                f"(WG 141 A.3.2, p162, as summarised — we do not hold GB 50139 itself).")
        depth = T + clearance
    else:
        depth = max(rs.depth_mult * T, 1.2 * T)
    # No project floor is applied here: the depth is the ruleset's. Where it falls below
    # PIANC's river minimum ratio of 1.2 x T (WG 141 5.3.4.3, p108) the result says so, but
    # the depth is not changed — a warning, not a hidden correction.
    if depth < 1.2 * T:
        warns.append(
            f"design depth {depth:.2f} m is {1.2 * T - depth:.2f} m BELOW PIANC's river "
            f"minimum depth ratio of 1.2 x draught ({1.2 * T:.2f} m at T {T:.2f} m; "
            f"WG 141 5.3.4.3, p108, which recommends 1.3 x T and says never below 1.2).")
    dW = _bend_widening(rs, loa, v.beam_m, r_min)
    return ChannelShape(
        ruleset_id=rs.ruleset_id, vessel_standard_id=v.standard_id,
        loa_m=loa, beam_m=v.beam_m, draught_m=T, formation=v.formation,
        bottom_width_straight_m=width, min_bend_radius_m=r_min, design_depth_m=depth,
        bend_widening_at_rmin_m=dW, bottom_width_at_rmin_m=width + dW,
        width_basis=basis, warnings=warns,
        source=f"{rs.source}; vessel {v.source}",
    )



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
    # the RULESETS block above. Prefer ``channel_shape`` with a named ruleset for new work.
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
                "the PIANC path silently). Use channel_shape(ruleset_id='CN' | "
                "'CN-conservative' | 'CN-reduced', vessel_standard_id=...) instead.")
        known = rule_sets()
        if self.rule_set not in known:
            raise ValueError(f"rule_set must be one of {known} (register more with "
                             "channel_designer.rules.methods.register_width_method)")
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


def _size(inp: ChannelRuleInputs, ref_radius: float):
    m = get_width_method(inp.rule_set)
    opts = inp.method_options()
    straight = m.straight_width(inp.loa_m, inp.beam_m, opts)
    r_min = m.min_radius(inp.loa_m, opts)
    bend = max(straight, m.bend_width(inp.loa_m, inp.beam_m, opts, ref_radius, ref_radius,
                                      REFERENCE_DEFLECTION_DEG))
    breakdown = dict(m.breakdown(inp.loa_m, inp.beam_m, opts)) if m.breakdown else {}
    breakdown["bend_widening_at_reference_radius"] = round(bend - straight, 2)
    return straight, r_min, bend, breakdown


def width_at_radius(inp: ChannelRuleInputs, radius_m: float) -> float:
    """Required bottom width (m) at a bend of the given radius — the single width
    authority for the alignment solver's width-aware edge cost.

    Straight (radius = inf or <= 0 treated as straight) → the method's straight width.
    Otherwise ``max(straight, bend_width(R1 = R2 = R, 45°))``; for ``"pianc"`` that is
    straight + LOA²/(8R). Monotonic non-increasing in R for every shipped method.
    """
    import math as _math
    m = get_width_method(inp.rule_set)
    opts = inp.method_options()
    straight = m.straight_width(inp.loa_m, inp.beam_m, opts)
    if radius_m <= 0 or _math.isinf(radius_m):
        return straight
    return max(straight, m.bend_width(inp.loa_m, inp.beam_m, opts, radius_m, radius_m,
                                      REFERENCE_DEFLECTION_DEG))


def size_channel(inp: ChannelRuleInputs) -> ChannelRuleResult:
    """One-lane channel geometry under the selected width method (``inp.rule_set``)."""
    ref_radius = inp.reference_bend_radius_m or (3.0 * inp.loa_m)
    straight, r_min, bend, breakdown = _size(inp, ref_radius)

    ukc = under_keel_clearance(inp.draft_m, inp.ukc_fraction, inp.ukc_min_m)
    depth = required_depth(inp.draft_m, inp.ukc_fraction, inp.ukc_min_m)
    bed = inp.min_water_level_m - depth
    top = straight + 2.0 * inp.side_slope_h_per_v * depth

    checks = [
        Check("straight_width_positive", straight > 0, f"straight width {straight:.1f} m > 0"),
        Check("depth_covers_draft", depth >= inp.draft_m,
              f"required depth {depth:.2f} m ≥ draft {inp.draft_m:.2f} m"),
        Check("ref_radius_at_or_above_rule_min", ref_radius >= r_min - 1e-6,
              f"reference bend radius {ref_radius:.0f} m vs rule minimum {r_min:.0f} m "
              f"— bends below the rule minimum need widening / a cut-through", severity="warning"),
    ]
    warnings = []
    if bed < 0.0:
        warnings.append(f"bed level {bed:.2f} masl is below datum — marine/tidal dredge reach "
                        f"(pool at min WL {inp.min_water_level_m:.1f} masl)")

    return ChannelRuleResult(
        rule_set=inp.rule_set, straight_bottom_width_m=straight, min_turn_radius_m=r_min,
        reference_bend_radius_m=ref_radius, bend_bottom_width_m=bend,
        bend_widening_m=bend - straight, required_depth_m=depth, under_keel_clearance_m=ukc,
        min_water_level_m=inp.min_water_level_m, bed_level_m=bed, top_width_straight_m=top,
        width_breakdown=breakdown, checks=checks, warnings=warnings,
    )


# =============================================================================
# Worked example — run: python -m channel_designer.rules.rulesets
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
