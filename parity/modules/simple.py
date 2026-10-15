"""simple: `rules/simple.py` (one-call straight and single-bend sizing)."""
from __future__ import annotations

from ..common import Rng, case as _case, frozen
from .methods import toy_method

ENTRY_POINT = "simple"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    sm = frozen("rules.simple")
    if fn == "size_channel":
        with toy_method(args.get("toy_method")):
            inp = sm.PIANCInputs(**args["inputs"])
            return sm.size_channel(inp, args.get("depth_m"))
    raise KeyError(f"simple has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, args):
        out.append(_case(f"size_channel/{cid}", tier, "size_channel", args, R))

    c("beam_9_5", 1, dict(inputs=dict(beam_m=9.5, loa_m=85.0, draft_m=2.5)))
    c("width_scales_with_beam", 1, dict(inputs=dict(beam_m=19.0, loa_m=85.0, draft_m=2.5)))
    c("bend_widening", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0, draft_m=2.0,
                                           width_multiple_straight=3.0, radius_multiple=3.0)))
    c("from_depth_no_draft", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0), depth_m=4.0))
    c("draft_and_depth", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0, draft_m=2.0), depth_m=3.5))
    c("fractional_ukc", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0, draft_m=6.0,
                                            ukc_fraction=0.2, ukc_min_m=0.3)))
    c("pianc_buildup_options", 1, dict(inputs=dict(
        beam_m=11.4, loa_m=110.0, draft_m=2.8, width_options={"use_buildup": True,
                                                              "maneuverability": "good"})))
    c("pianc_options_beat_field", 1, dict(inputs=dict(
        beam_m=11.4, loa_m=110.0, draft_m=2.8, width_multiple_straight=2.0,
        width_options={"width_multiple": 3.4})))
    c("passing_places", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0, draft_m=2.0,
                                            passing_place_spacing_m=1500.0,
                                            passing_place_length_multiple=6.5)))
    c("zero_depth", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0), depth_m=0.0))
    c("negative_depth", 1, dict(inputs=dict(beam_m=10.0, loa_m=100.0, draft_m=2.0), depth_m=-2.0))
    toy = {"name": "toy", "k_beam": 2.5, "bend_div": 4.0, "rmin_mult": 4.0}
    c("toy_method", 1, dict(inputs=dict(beam_m=10.0, loa_m=90.0, draft_m=2.2, width_method="toy"),
                            toy_method=toy))
    rng = Rng(23004)
    for i in range(30):
        inp = dict(beam_m=rng.r3(4.0, 30.0), loa_m=rng.r3(30.0, 250.0), draft_m=rng.r3(0.0, 6.0),
                   width_multiple_straight=rng.r3(2.0, 4.0), radius_multiple=rng.r3(2.0, 5.0),
                   ukc_fraction=rng.r3(0.02, 0.3), ukc_min_m=rng.r3(0.1, 1.2),
                   side_slope_h_per_v=rng.r3(1.5, 5.0))
        depth = rng.r3(0.5, 9.0) if rng.p(0.5) else None
        c(f"sweep_{i:02d}", 2, dict(inputs=inp, depth_m=depth))
    c("unknown_method", 3, dict(inputs=dict(beam_m=10.0, loa_m=100.0, width_method="nope")))
    c("bad_maneuverability", 3, dict(inputs=dict(
        beam_m=10.0, loa_m=100.0, width_options={"maneuverability": "excellent"})))
    return out
