"""methods: `rules/methods.py` (the width-method registry and the built-in "pianc" method)."""
from __future__ import annotations

from contextlib import contextmanager

from ..common import case as _case, frozen

ENTRY_POINT = "methods"
R = "initial case set (spec 0001, ticket 23)"


@contextmanager
def toy_method(spec: dict | None):
    """Register a data-defined width method for the length of one case, then remove it.

    straight = k_beam x beam; bend = straight + loa^2 / (bend_div x R); R_min = rmin_mult x loa.
    A Rust port proves a method registered from outside the crate with an equal rule."""
    methods = frozen("rules.methods")
    if spec is None:
        yield None
        return
    m = methods.WidthMethod(
        name=spec["name"],
        straight_width=lambda loa, beam, opts: spec["k_beam"] * beam,
        bend_width=lambda loa, beam, opts, r, rd, d: (
            spec["k_beam"] * beam + loa ** 2 / (spec["bend_div"] * r)),
        min_radius=lambda loa, opts: spec["rmin_mult"] * loa,
        description=spec.get("description", "toy method"),
    )
    methods.register_width_method(m, replace=True)
    try:
        yield m
    finally:
        methods.unregister_width_method(spec["name"])


def adapter(fn: str, args: dict):
    methods = frozen("rules.methods")
    if fn == "width_methods":
        return list(methods.width_methods())
    if fn == "evaluate":
        # One method, every callable, at the given vessel and radii.
        with toy_method(args.get("toy_method")):
            m = methods.get_width_method(args["name"])
            opts = args.get("options", {})
            loa, beam = args["loa"], args["beam"]
            out = {"name": m.name, "description": m.description,
                   "straight": m.straight_width(loa, beam, opts),
                   "min_radius": m.min_radius(loa, opts),
                   "bends": [m.bend_width(loa, beam, opts, r, r, args.get("deflection_deg", 45.0))
                             for r in args["radii"]],
                   "breakdown": m.breakdown(loa, beam, opts) if m.breakdown else None}
            return out
    if fn == "get_width_method":
        m = methods.get_width_method(args["name"])
        return {"name": m.name, "description": m.description}
    if fn == "register_duplicate":
        m = methods.get_width_method(args["name"])
        return methods.register_width_method(m).name
    if fn == "unregister":
        methods.unregister_width_method(args["name"])
        return list(methods.width_methods())
    raise KeyError(f"methods has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("builtin", 1, "width_methods", {})
    base = dict(name="pianc", loa=85.0, beam=9.5, radii=[100.0, 255.0, 1000.0, 1e9, 0.0, "float:inf"])
    c("pianc_default", 1, "evaluate", base)
    c("pianc_k_3", 1, "evaluate", dict(base, options={"width_multiple": 3.0, "radius_multiple": 4.0}))
    c("pianc_buildup_default_maneuver", 1, "evaluate", dict(base, options={"use_buildup": True}))
    for m in ("good", "moderate", "poor"):
        c(f"pianc_buildup_{m}", 1, "evaluate",
          dict(base, loa=110.0, beam=11.4,
               options={"use_buildup": True, "maneuverability": m,
                        "extra_widths": {"wave": 0.3, "cross_current": 0.2},
                        "bank_clearance_outer": 0.4, "bank_clearance_inner": 0.2}))
    c("toy_method", 1, "evaluate",
      dict(name="toy", loa=90.0, beam=10.0, radii=[200.0, 450.0, "float:inf"],
           toy_method={"name": "toy", "k_beam": 2.5, "bend_div": 4.0, "rmin_mult": 4.0}))
    c("get_pianc", 1, "get_width_method", {"name": "pianc"})
    c("duplicate_name_refused", 3, "register_duplicate", {"name": "pianc"})
    c("builtin_cannot_be_unregistered", 3, "unregister", {"name": "pianc"})
    c("unregister_unknown_is_quiet", 1, "unregister", {"name": "no-such-method"})
    c("unknown_method", 3, "get_width_method", {"name": "nope"})
    c("bad_maneuverability", 3, "evaluate", dict(base, options={"maneuverability": "excellent"}))
    c("unknown_method_in_evaluate", 3, "evaluate", dict(base, name="zzz"))
    from ..common import Rng
    rng = Rng(23001)
    for i in range(20):
        opts = {"width_multiple": rng.r3(2.0, 4.0), "radius_multiple": rng.r3(2.0, 5.0),
                "use_buildup": rng.p(0.5),
                "maneuverability": ("good", "moderate", "poor")[rng.i(0, 2)],
                "bank_clearance_outer": rng.r3(0.2, 0.8), "bank_clearance_inner": rng.r3(0.1, 0.5)}
        c(f"sweep_{i:02d}", 2, "evaluate",
          dict(name="pianc", loa=rng.r3(40.0, 200.0), beam=rng.r3(6.0, 30.0), options=opts,
               deflection_deg=rng.r3(10.0, 120.0),
               radii=[rng.r3(50.0, 3000.0) for _ in range(5)] + ["float:inf"]))
    return out
