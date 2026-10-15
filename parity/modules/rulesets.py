"""rulesets: `rules/rulesets.py` (named rulesets, `channel_shape`, `size_channel`)."""
from __future__ import annotations

import dataclasses

from ..common import Rng, case as _case, frozen, to_json
from .methods import toy_method

ENTRY_POINT = "rulesets"
R = "initial case set (spec 0001, ticket 23)"
VESSELS = ["CEMT_IV", "CEMT_Va", "CEMT_Vb", "CEMT_VIa", "CEMT_VIa_motor_NL2011", "CN_I_motor",
           "CN_II_motor", "CN_III_motor", "CN_III_convoy_2ab", "CN_IV_motor", "CN_II_convoy_2ab",
           "CN_I_convoy_2ab"]
RULESET_IDS = ["PIANC-A", "PIANC-B", "PIANC-C", "CN", "CN-conservative", "CN-reduced"]


def _inputs(rs, d: dict):
    return rs.ChannelRuleInputs(**d)


def adapter(fn: str, args: dict):
    rs = frozen("rules.rulesets")
    if fn == "rule_sets":
        return list(rs.rule_sets())
    if fn == "rulesets_table":
        return {"rulesets": rs.RULESETS,
                "cn_table_a2": [[l, b, w] for (l, b), w in rs.CN_TABLE_A2.items()],
                "cn_table_5_1_width_mult": rs.CN_TABLE_5_1_WIDTH_MULT,
                "cn_depth_clearance_m": rs.CN_DEPTH_CLEARANCE_M,
                "cn_depth_clearance_end": rs.CN_DEPTH_CLEARANCE_END,
                "bank_cut_fraction": rs.BANK_CUT_FRACTION,
                "reference_deflection_deg": rs.REFERENCE_DEFLECTION_DEG}
    if fn == "lateral_room_m":
        return rs.lateral_room_m(**args)
    if fn == "classify_bend":
        return rs.classify_bend(**args)
    if fn == "get_ruleset":
        return rs.get_ruleset(**args)
    if fn == "channel_shape":
        return rs.channel_shape(**args)
    if fn == "channel_shape_widths":
        shape = rs.channel_shape(args["ruleset_id"], args["vessel_standard_id"])
        return [{"radius": r, "bend_widening_m": shape.bend_widening_m(r),
                 "bottom_width_at_m": shape.bottom_width_at_m(r)} for r in args["radii"]]
    if fn == "register_ruleset_then_channel_shape":
        spec = dict(args["ruleset"])
        new = rs.Ruleset(**spec)
        saved = dict(rs.RULESETS)
        try:
            rs.register_ruleset(new, replace=args.get("replace", False))
            return rs.channel_shape(new.ruleset_id, args["vessel_standard_id"])
        finally:
            rs.RULESETS.clear()
            rs.RULESETS.update(saved)
    if fn == "size_channel":
        with toy_method(args.get("toy_method")):
            res = rs.size_channel(_inputs(rs, args["inputs"]))
        return {"result": res, "ok": res.ok}
    if fn == "width_at_radius":
        with toy_method(args.get("toy_method")):
            inp = _inputs(rs, args["inputs"])
            return [rs.width_at_radius(inp, r) for r in args["radii"]]
    if fn == "channel_rule_inputs":
        inp = _inputs(rs, args["inputs"])
        return {"inputs": inp, "method_options": inp.method_options()}
    raise KeyError(f"rulesets has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("builtin", 1, "rule_sets", {})
    c("table", 1, "rulesets_table", {})
    # --- from tests/rules/test_rulesets.py: golden and named cases ---
    c("cemt_via_pianc_b", 1, "channel_shape", dict(ruleset_id="PIANC-B", vessel_standard_id="CEMT_VIa"))
    c("cemt_iv_pianc_c_draught_override", 1, "channel_shape",
      dict(ruleset_id="PIANC-C", vessel_standard_id="CEMT_IV", draught_m=3.1))
    c("cn_table_a2_hit", 1, "channel_shape", dict(ruleset_id="CN", vessel_standard_id="CN_I_motor"))
    c("cn_off_table_vessel", 1, "channel_shape", dict(ruleset_id="CN", vessel_standard_id="CN_IV_motor"))
    c("cn_class_outside_depth_clause", 1, "channel_shape",
      dict(ruleset_id="CN-reduced", vessel_standard_id="CN_IV_motor"))
    c("depth_below_pianc_ratio_warns", 1, "channel_shape",
      dict(ruleset_id="CN", vessel_standard_id="CEMT_Va", draught_m=4.5))
    for rid in RULESET_IDS:
        c(f"widths_{rid}", 1, "channel_shape_widths",
          dict(ruleset_id=rid, vessel_standard_id="CEMT_VIa",
               radii=[50.0, 150.0, 330.0, 700.0, 2000.0, 1e7, 0.0, "float:inf", -5.0]))
    for room in [(40.0, 70.0, 60.0), (120.0, 70.0, 60.0), (70.0, 70.0, 60.0)]:
        c(f"room_{room[0]:g}_{room[1]:g}", 1, "lateral_room_m",
          dict(wetted_width_m=room[0], top_width_m=room[1], bottom_width_m=room[2]))
    c("room_custom_fraction", 1, "lateral_room_m",
      dict(wetted_width_m=90.0, top_width_m=70.0, bottom_width_m=60.0, bank_cut_fraction=0.25))
    for e, room in [(0.0, 10.0), (1e-10, 10.0), (5.0, 10.0), (10.0, 10.0), (10.0 + 5e-10, 10.0),
                    (10.1, 10.0), (-1.0, 10.0)]:
        c(f"bend_e{e!r}_room{room!r}", 1, "classify_bend", dict(e_m=e, room_m=room))
    c("bend_loose_tol", 1, "classify_bend", dict(e_m=10.2, room_m=10.0, tol_m=0.25))
    for rid in RULESET_IDS:
        c(f"get_{rid}", 1, "get_ruleset", dict(ruleset_id=rid))
    c("register_then_shape_pianc", 1, "register_ruleset_then_channel_shape", dict(
        vessel_standard_id="CEMT_Va",
        ruleset=dict(ruleset_id="USER-1", family="pianc", label="user", source="demo",
                     r_mult_convoy=2.5, r_mult_motor=2.5, width_mult=3.1, c_c=0.55, depth_mult=1.25)))
    c("register_then_shape_china", 1, "register_ruleset_then_channel_shape", dict(
        vessel_standard_id="CN_II_motor",
        ruleset=dict(ruleset_id="USER-2", family="china", label="user", source="demo",
                     r_mult_convoy=3.5, r_mult_motor=3.5, bank_mult_convoy=0.28,
                     bank_mult_motor=0.36, depth_add_m=0.4)))
    # --- size_channel path ---
    sets = [("A", dict(beam_m=9.5, loa_m=85.0, draft_m=2.5, min_water_level_m=10.0)),
            ("B", dict(beam_m=11.4, loa_m=110.0, draft_m=2.8, min_water_level_m=10.0)),
            ("C_sea_level", dict(beam_m=9.5, loa_m=85.0, draft_m=2.5, min_water_level_m=0.0))]
    for tag, inp in sets:
        c(f"size_{tag}", 1, "size_channel", dict(inputs=inp))
        c(f"width_at_radius_{tag}", 1, "width_at_radius",
          dict(inputs=inp, radii=[100.0, 255.0, 1000.0, "float:inf", 0.0, -3.0]))
    c("size_buildup", 1, "size_channel", dict(inputs=dict(
        sets[0][1], pianc_use_buildup=True, pianc_maneuverability="moderate")))
    c("size_reference_radius_below_min", 1, "size_channel", dict(inputs=dict(
        sets[0][1], reference_bend_radius_m=120.0)))
    c("size_options_override", 1, "size_channel", dict(inputs=dict(
        sets[0][1], options={"width_multiple": 3.3}, pianc_width_multiple=2.1)))
    toy = {"name": "toy", "k_beam": 2.5, "bend_div": 4.0, "rmin_mult": 4.0}
    c("size_toy_method", 1, "size_channel", dict(
        inputs=dict(sets[0][1], rule_set="toy"), toy_method=toy))
    c("width_at_radius_toy_method", 1, "width_at_radius", dict(
        inputs=dict(sets[0][1], rule_set="toy"), radii=[150.0, 600.0, "float:inf"], toy_method=toy))
    c("inputs_method_options", 1, "channel_rule_inputs", dict(inputs=sets[0][1]))
    # --- tier 2: the full grid, and a seeded sweep ---
    for rid in RULESET_IDS:
        for vid in VESSELS:
            c(f"grid_{rid}_{vid}", 2, "channel_shape", dict(ruleset_id=rid, vessel_standard_id=vid))
    rng = Rng(23002)
    for i in range(30):
        inp = dict(beam_m=rng.r3(5.0, 30.0), loa_m=rng.r3(40.0, 250.0), draft_m=rng.r3(1.0, 6.0),
                   min_water_level_m=rng.r3(-2.0, 40.0), side_slope_h_per_v=rng.r3(1.5, 5.0),
                   ukc_fraction=rng.r3(0.05, 0.3), ukc_min_m=rng.r3(0.2, 1.0),
                   pianc_width_multiple=rng.r3(2.0, 4.0), pianc_use_buildup=rng.p(0.4),
                   pianc_maneuverability=("good", "moderate", "poor")[rng.i(0, 2)],
                   pianc_radius_multiple=rng.r3(2.0, 5.0))
        if rng.p(0.5):
            inp["reference_bend_radius_m"] = rng.r3(80.0, 900.0)
        c(f"size_sweep_{i:02d}", 2, "size_channel", dict(inputs=inp))
        c(f"width_sweep_{i:02d}", 2, "width_at_radius",
          dict(inputs=inp, radii=[rng.r3(30.0, 4000.0) for _ in range(4)] + ["float:inf"]))
    for i in range(15):
        c(f"shape_draught_sweep_{i:02d}", 2, "channel_shape",
          dict(ruleset_id=RULESET_IDS[rng.i(0, 5)], vessel_standard_id=VESSELS[rng.i(0, 11)],
               draught_m=rng.r3(0.8, 6.5)))
    for i in range(12):
        c(f"room_sweep_{i:02d}", 2, "lateral_room_m",
          dict(wetted_width_m=rng.r3(10.0, 200.0), top_width_m=rng.r3(10.0, 200.0),
               bottom_width_m=rng.r3(10.0, 150.0), bank_cut_fraction=rng.r3(0.0, 0.8)))
    # --- tier 3: errors ---
    c("unknown_ruleset", 3, "get_ruleset", dict(ruleset_id="nope"))
    c("shape_unknown_ruleset", 3, "channel_shape", dict(ruleset_id="nope", vessel_standard_id="CEMT_IV"))
    c("shape_unknown_vessel", 3, "channel_shape", dict(ruleset_id="CN", vessel_standard_id="nope"))
    c("china_rule_set_not_on_size_path", 3, "size_channel",
      dict(inputs=dict(sets[0][1], rule_set="china")))
    c("unknown_rule_set", 3, "size_channel", dict(inputs=dict(sets[0][1], rule_set="zzz")))
    for name in ("beam_m", "loa_m", "draft_m"):
        c(f"nonpositive_{name}", 3, "size_channel", dict(inputs=dict(sets[0][1], **{name: 0.0})))
    c("bad_maneuverability", 3, "size_channel",
      dict(inputs=dict(sets[0][1], pianc_maneuverability="excellent")))
    c("register_bad_family", 3, "register_ruleset_then_channel_shape", dict(
        vessel_standard_id="CEMT_Va",
        ruleset=dict(ruleset_id="USER-3", family="martian", label="u", source="s",
                     r_mult_convoy=2.0, r_mult_motor=2.0)))
    c("register_duplicate_refused", 3, "register_ruleset_then_channel_shape", dict(
        vessel_standard_id="CEMT_Va",
        ruleset=dict(ruleset_id="PIANC-A", family="pianc", label="u", source="s",
                     r_mult_convoy=2.0, r_mult_motor=2.0)))
    c("register_duplicate_replaced", 1, "register_ruleset_then_channel_shape", dict(
        vessel_standard_id="CEMT_Va", replace=True,
        ruleset=dict(ruleset_id="PIANC-A", family="pianc", label="u", source="s",
                     r_mult_convoy=2.0, r_mult_motor=2.0, width_mult=3.0, c_c=0.4, depth_mult=1.3)))
    return out
