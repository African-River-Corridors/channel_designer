"""primitives: `alignment/primitives.py` (the motion-primitive set for the lattice search)."""
from __future__ import annotations

from ..common import Rng, case as _case, frozen

ENTRY_POINT = "primitives"
R = "initial case set (spec 0001, ticket 23)"


def _widen(spec):
    p = frozen("alignment.primitives")
    if spec is None or spec == "none":
        return None
    if spec["kind"] == "lux":
        return p.lux_widening(spec["loa_m"])
    raise ValueError(f"unknown widen spec {spec!r}")


def adapter(fn: str, args: dict):
    p = frozen("alignment.primitives")
    if fn == "constants":
        return {"SHARED_RADII": p.SHARED_RADII, "STRAIGHT_LEN_M": p.STRAIGHT_LEN_M,
                "LEGACY_STRAIGHT_LEN_M": p.LEGACY_STRAIGHT_LEN_M, "NOMINAL_ARC_M": p.NOMINAL_ARC_M,
                "STEP_M": p.STEP_M, "BUILDER": p.BUILDER}
    if fn == "lux_widening":
        f = p.lux_widening(args["loa_m"])
        return [f(r) for r in args["radii"]]
    if fn == "radii_for":
        return p.radii_for(args["r_min_m"], args.get("radii"))
    if fn == "build":
        kw = {k: v for k, v in args.items() if k not in ("radii_finite", "n_heading", "widen")}
        return p.build(args["radii_finite"], args["n_heading"], widen_fn=_widen(args.get("widen")), **kw)
    if fn == "descriptor":
        kw = {k: v for k, v in args.items() if k not in ("radii_finite", "n_heading")}
        return p.descriptor(args["radii_finite"], args["n_heading"], **kw)
    raise KeyError(f"primitives has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    c("constants", 1, "constants", {})
    c("lux_loa85", 1, "lux_widening",
      dict(loa_m=85.0, radii=[100.0, 255.0, 1000.0, "float:inf", 0.0, -3.0, "float:nan"]))
    for r_min in (0.0, 150.0, 151.0, 275.0, 275.0000001, 2000.0, 2001.0):
        c(f"radii_for_{r_min!r}", 1, "radii_for", dict(r_min_m=r_min))
    c("radii_for_custom", 1, "radii_for", dict(r_min_m=200.0, radii=[100, 200, 300.5, 150.25]))
    c("radii_for_empty", 1, "radii_for", dict(r_min_m=3000.0))
    c("build_default_72", 1, "build", dict(radii_finite=[300.0, 500.0], n_heading=72))
    c("build_lux", 1, "build", dict(radii_finite=[250.0, 400.0, 2000.0], n_heading=36,
                                    widen={"kind": "lux", "loa_m": 85.0}))
    c("build_legacy_straight", 1, "build", dict(radii_finite=[300.0], n_heading=72, straight_len=100.0))
    c("build_custom_arc_and_step", 1, "build",
      dict(radii_finite=[150.0, 340.0], n_heading=48, nominal_arc=120.0, step_m=7.5))
    c("build_all_shared_radii", 1, "build", dict(
        radii_finite=[150.0, 160.0, 170.0, 180.0, 190.0, 200.0, 215.0, 230.0, 250.0, 275.0, 300.0,
                      340.0, 400.0, 500.0, 650.0, 850.0, 1200.0, 2000.0], n_heading=72))
    c("build_no_radii", 1, "build", dict(radii_finite=[], n_heading=72))
    c("descriptor_default", 1, "descriptor", dict(radii_finite=[300.0, 500.0], n_heading=72))
    c("descriptor_custom", 1, "descriptor", dict(radii_finite=[300], n_heading=36, nominal_arc=90.0,
                                                step_m=5.0, straight_len=100.0))
    rng = Rng(23008)
    for i in range(8):
        radii = sorted({float(rng.i(100, 2200)) for _ in range(rng.i(1, 6))})
        c(f"build_sweep_{i:02d}", 2, "build",
          dict(radii_finite=radii, n_heading=rng.i(8, 120), nominal_arc=rng.r3(30.0, 200.0),
               step_m=rng.r3(2.0, 20.0), straight_len=rng.r3(20.0, 150.0),
               widen={"kind": "lux", "loa_m": rng.r3(40.0, 200.0)} if i % 2 else None))
        c(f"descriptor_sweep_{i:02d}", 2, "descriptor",
          dict(radii_finite=radii, n_heading=rng.i(8, 120), nominal_arc=rng.r3(30.0, 200.0),
               step_m=rng.r3(2.0, 20.0), straight_len=rng.r3(20.0, 150.0)))
    for i in range(10):
        c(f"radii_for_sweep_{i:02d}", 2, "radii_for", dict(r_min_m=rng.r3(0.0, 2500.0)))
        c(f"lux_sweep_{i:02d}", 2, "lux_widening",
          dict(loa_m=rng.r3(30.0, 250.0), radii=[rng.r3(50.0, 3000.0) for _ in range(5)]))
    return out
