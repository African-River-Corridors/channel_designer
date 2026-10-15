"""width_profile: `rules/width_profile.py` (width as a function of bend radius)."""
from __future__ import annotations

from ..common import Rng, case as _case, frozen
from .methods import toy_method

ENTRY_POINT = "width_profile"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    wp = frozen("rules.width_profile")
    if fn == "make_width_fn":
        with toy_method(args.get("toy_method")):
            f = wp.make_width_fn(args["loa_m"], args["beam_m"], args["draft_m"],
                                 args.get("rule_set", "pianc"), **args.get("overrides", {}))
            return [{"radius": r, "width": f(r), "half_width": f.half_width(r)}
                    for r in args["radii"]]
    if fn == "width_params_dict":
        with toy_method(args.get("toy_method")):
            return wp.width_params_dict(args["loa_m"], args["beam_m"], args["draft_m"],
                                        args.get("rule_set", "pianc"), **args.get("overrides", {}))
    raise KeyError(f"width_profile has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    radii = [60.0, 150.0, 240.0, 480.0, 1000.0, 5000.0, "float:inf", 0.0, -10.0]
    c("loa80_beam12", 1, "make_width_fn", dict(loa_m=80.0, beam_m=12.0, draft_m=2.5, radii=radii))
    c("loa60_beam10", 1, "make_width_fn", dict(loa_m=60.0, beam_m=10.0, draft_m=2.0, radii=radii))
    c("buildup_override", 1, "make_width_fn", dict(
        loa_m=80.0, beam_m=12.0, draft_m=2.5, radii=radii,
        overrides=dict(pianc_use_buildup=True, pianc_maneuverability="good")))
    c("options_override", 1, "make_width_fn", dict(
        loa_m=80.0, beam_m=12.0, draft_m=2.5, radii=radii, overrides=dict(options={"width_multiple": 3.1})))
    toy = {"name": "toy", "k_beam": 2.5, "bend_div": 4.0, "rmin_mult": 4.0}
    c("toy_method", 1, "make_width_fn", dict(
        loa_m=80.0, beam_m=12.0, draft_m=2.5, rule_set="toy", radii=radii, toy_method=toy))
    c("params_dict_pianc", 1, "width_params_dict", dict(loa_m=80.0, beam_m=12.0, draft_m=2.5))
    c("params_dict_toy", 1, "width_params_dict",
      dict(loa_m=80.0, beam_m=12.0, draft_m=2.5, rule_set="toy", toy_method=toy))
    rng = Rng(23005)
    for i in range(25):
        c(f"sweep_{i:02d}", 2, "make_width_fn",
          dict(loa_m=rng.r3(30.0, 250.0), beam_m=rng.r3(5.0, 30.0), draft_m=rng.r3(1.0, 6.0),
               radii=[rng.r3(40.0, 4000.0) for _ in range(6)] + ["float:inf"],
               overrides=dict(pianc_width_multiple=rng.r3(2.0, 4.0),
                              pianc_use_buildup=rng.p(0.3))))
    c("unknown_rule_set", 3, "make_width_fn", dict(loa_m=80.0, beam_m=12.0, draft_m=2.5,
                                                   rule_set="nope", radii=[100.0]))
    c("china_rule_set", 3, "make_width_fn", dict(loa_m=80.0, beam_m=12.0, draft_m=2.5,
                                                 rule_set="china", radii=[100.0]))
    c("nonpositive_beam", 3, "make_width_fn", dict(loa_m=80.0, beam_m=0.0, draft_m=2.5, radii=[100.0]))
    c("params_dict_unknown", 3, "width_params_dict",
      dict(loa_m=80.0, beam_m=12.0, draft_m=2.5, rule_set="nope"))
    return out
