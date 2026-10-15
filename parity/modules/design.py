"""design: `rules/design.py` (per-chainage sizing driven by centreline curvature)."""
from __future__ import annotations

import math

import numpy as np

from ..common import Rng, case as _case, frozen, frange, rnd
from .methods import toy_method

ENTRY_POINT = "design"
R = "initial case set (spec 0001, ticket 23)"


def _params(d: dict | None):
    ds = frozen("rules.design")
    return ds.ChannelDesignParams(**(d or {}))


def _pts(p):
    return None if p is None else np.array(p, dtype=float)


def _result_json(res):
    # `sections()` is the same numbers again, row by row, so it is left out; `to_dict(40)` keeps the
    # rounding and the downsampling under test.
    return {"result": res, "summary": res.summary(), "dict": res.to_dict(40), "ok": res.ok}


def adapter(fn: str, args: dict):
    ds = frozen("rules.design")
    if fn in ("under_keel_clearance", "required_depth", "max_draft_for_depth"):
        return getattr(ds, fn)(**args)
    if fn == "resolve_design_depth":
        return ds.resolve_design_depth(_params(args.get("params")))
    if fn == "extra_width":
        return [ds.extra_width(r, args["total_loa"], _params(args.get("params")))
                for r in args["radii"]]
    if fn == "min_bend_radius":
        return ds.min_bend_radius(args["total_loa"], _params(args.get("params")))
    if fn == "compute":
        with toy_method(args.get("toy_method")):
            inp = ds.ChannelDesignInputs(total_loa=args["total_loa"], beam=args["beam"],
                                         centreline_points=_pts(args.get("centreline_points")),
                                         params=_params(args.get("params")))
            return _result_json(ds.compute(inp))
    if fn == "compute_scenario_grid":
        with toy_method(args.get("toy_method")):
            return ds.compute_scenario_grid(
                _pts(args.get("centreline_points")), args["loa_values"], args["beam_values"],
                params=_params(args["params"]) if args.get("params") is not None else None,
                n_samples=args.get("n_samples", 160))
    if fn == "stage_profiles":
        # Stage 0 and 1 on their own: the curvature profile and the LOA profile.
        params = _params(args.get("params"))
        curv = ds._compute_curvature(_pts(args.get("centreline_points")), params)
        loa = ds.precompute_loa(curv, args["total_loa"], params)
        return {"curvature": curv, "loa": loa}
    raise KeyError(f"design has no parity function {fn!r}")


def arc(radius, sweep_deg=90.0, spacing=10.0):
    sweep = math.radians(sweep_deg)
    n = max(int(radius * sweep / spacing) + 1, 5)
    theta = np.linspace(0.0, sweep, n)
    return np.column_stack((radius * np.cos(theta), radius * np.sin(theta)))


def line(length=4000.0, spacing=10.0):
    n = int(length / spacing) + 1
    x = np.linspace(0.0, length, n)
    return np.column_stack((x, np.zeros_like(x)))


def sine(length=3000.0, wavelength=1000.0, amp=150.0, step=25.0):
    x = np.arange(0.0, length + step, step)
    return np.column_stack([x, amp * np.sin(2 * math.pi * x / wavelength)])


def s_bend(r=400.0, spacing=10.0, turn_deg=70.0):
    """A left arc then a right arc of equal radius, joined with no kink (two discrete bends)."""
    n = max(int(r * math.radians(turn_deg) / spacing), 3)
    dth = spacing / r
    x = y = h = 0.0
    pts = [(x, y)]
    for i in range(2 * n):
        h += dth if i < n else -dth
        x += spacing * math.cos(h)
        y += spacing * math.sin(h)
        pts.append((x, y))
    return np.array(pts)


def _well_conditioned(pts, limit=1e5):
    """True when no 3-point radius on `pts` (as the centreline code resamples them) is both finite and
    above `limit` m. Radii past that are noise on almost collinear points."""
    ds = frozen("rules.design")
    for step in (10.0, 15.0, 20.0, 25.0, 30.0, 40.0):
        rp = ds._resample_points(np.asarray(pts, dtype=float), step)
        r = ds._compute_curvature_radius(rp)
        if np.any(np.isfinite(r) & (r > limit)):
            return False
    return True


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    P = dict(width_multiple=2.8, radius_multiple=3.0, side_slope=4.0, draft_m=1.75,
             extra_width_coefficient=0.6, max_extra_width_radius=2000.0, radius_sample_length=10.0)
    # --- named: the worked example in tests/rules/test_design.py ---
    for d, ukc_f, ukc_m in [(1.75, 0.15, 0.5), (4.0, 0.15, 0.5), (3.0, 0.2, 0.9), (0.0, 0.15, 0.5)]:
        c(f"ukc_d{d:g}", 1, "under_keel_clearance",
          dict(draft_m=d, ukc_fraction=ukc_f, ukc_min_m=ukc_m))
        c(f"required_d{d:g}", 1, "required_depth",
          dict(draft_m=d, ukc_fraction=ukc_f, ukc_min_m=ukc_m))
    for depth in (2.25, 6.0, 0.4, 0.0, -1.0, 4.0, 10.0):
        c(f"max_draft_depth{depth:g}", 1, "max_draft_for_depth", dict(depth_m=depth))
    c("max_draft_abs_branch", 1, "max_draft_for_depth",
      dict(depth_m=2.4, ukc_fraction=0.05, ukc_min_m=0.8))
    c("resolve_default", 1, "resolve_design_depth", dict(params=P))
    c("resolve_override", 1, "resolve_design_depth", dict(params=dict(P, design_depth_m=3.5)))
    c("extra_width_bands", 1, "extra_width",
      dict(total_loa=160.0, params=P,
           radii=[300.0, 480.0, 600.0, 900.0, 1400.0, 1999.0, 2000.0, 5000.0, "float:inf", 0.0, -4.0]))
    c("extra_width_zero_coefficient", 1, "extra_width",
      dict(total_loa=160.0, params=dict(P, extra_width_coefficient=0.0), radii=[500.0, 800.0]))
    c("extra_width_one_band", 1, "extra_width",
      dict(total_loa=160.0, params=dict(P, n_bands=1), radii=[500.0, 900.0, 1990.0]))
    c("min_radius", 1, "min_bend_radius", dict(total_loa=160.0, params=P))
    c("min_radius_k4", 1, "min_bend_radius", dict(total_loa=110.0, params=dict(P, radius_multiple=4.0)))
    c("straight_none", 1, "compute", dict(total_loa=160.0, beam=28.0, params=P))
    c("straight_line", 1, "compute", dict(total_loa=160.0, beam=28.0, params=P, centreline_points=line()))
    for r_, sweep in [(1500.0, 90.0), (800.0, 90.0), (600.0, 120.0), (300.0, 90.0), (20.0, 90.0)]:
        c(f"arc_r{r_:g}", 1, "compute",
          dict(total_loa=160.0, beam=28.0, params=P, centreline_points=arc(r_, sweep)))
    c("arc_r1500_no_edge_margin", 1, "compute",
      dict(total_loa=160.0, beam=28.0, params=dict(P, edge_margin_m=0.0),
           centreline_points=arc(1500.0, 120.0)))
    c("sine", 1, "compute", dict(total_loa=120.0, beam=12.0, params=P, centreline_points=sine()))
    c("s_bend", 1, "compute", dict(total_loa=100.0, beam=10.0, params=P, centreline_points=s_bend()))
    z = sine()
    z3 = np.column_stack([z, 5.0 + 0.002 * z[:, 0]])
    c("with_z_column", 1, "compute", dict(total_loa=120.0, beam=12.0, params=P, centreline_points=z3))
    c("depth_override_too_shallow", 1, "compute",
      dict(total_loa=160.0, beam=28.0, params=dict(P, design_depth_m=1.5)))
    c("two_points_is_straight", 1, "compute",
      dict(total_loa=160.0, beam=28.0, params=P, centreline_points=[[0.0, 0.0], [100.0, 0.0]]))
    c("cut_through_pianc_method", 1, "compute",
      dict(total_loa=200.0, beam=6.0, params=dict(P, width_method="pianc"),
           centreline_points=arc(20.0)))
    c("pianc_method_sine", 1, "compute",
      dict(total_loa=120.0, beam=12.0,
           params=dict(P, width_method="pianc", width_options={"width_multiple": 3.0}),
           centreline_points=sine()))
    toy = {"name": "toy", "k_beam": 2.5, "bend_div": 4.0, "rmin_mult": 4.0}
    c("toy_method_s_bend", 1, "compute",
      dict(total_loa=100.0, beam=10.0, params=dict(P, width_method="toy"),
           centreline_points=s_bend(), toy_method=toy))
    c("stage_profiles_sine", 1, "stage_profiles",
      dict(total_loa=120.0, params=P, centreline_points=sine()))
    c("stage_profiles_straight", 1, "stage_profiles", dict(total_loa=120.0, params=P))
    c("grid_arc", 1, "compute_scenario_grid",
      dict(centreline_points=arc(1000.0), loa_values=[120.0, 160.0], beam_values=[16.0, 28.0], params=P,
           n_samples=30))
    c("grid_default_params_fractional_keys", 1, "compute_scenario_grid",
      dict(centreline_points=sine(), loa_values=[80.5, 100.0], beam_values=[9.25, 12.0],
           params=None))
    c("grid_no_downsample", 1, "compute_scenario_grid",
      dict(centreline_points=arc(700.0, 60.0), loa_values=[90.0], beam_values=[11.0],
           params=P, n_samples=None))
    # --- tier 2: seeded sweep ---
    rng = Rng(23003)
    for i in range(14):
        # A centreline with a nearly straight stretch (a sine at its inflection) gives a 3-point radius
        # of 1e7 or more, computed from points that are almost collinear: the last bits of the platform's
        # arithmetic then move it by 1e-8 relative. Such a case is drawn again.
        for attempt in range(8):
            kind = i % 4
            if kind == 0:
                pts = arc(rng.r3(150.0, 2500.0), rng.r3(20.0, 160.0), rng.r3(15.0, 40.0))
            elif kind == 1:
                pts = sine(rng.r3(1500.0, 3500.0), rng.r3(600.0, 2000.0), rng.r3(40.0, 300.0),
                           rng.r3(25.0, 60.0))
            elif kind == 2:
                base = sine(2500.0, rng.r3(500.0, 1200.0), rng.r3(50.0, 200.0), 25.0)
                pts = np.column_stack([base, rng.r3(0.0, 20.0) + 0.001 * base[:, 0]])
            else:
                pts = s_bend(rng.r3(150.0, 900.0), rng.r3(15.0, 40.0))
            pts = rnd(pts, 4)
            if _well_conditioned(pts):
                break
        params = dict(width_multiple=rng.r3(2.0, 4.0), radius_multiple=rng.r3(2.0, 5.0),
                      side_slope=rng.r3(1.0, 5.0), draft_m=rng.r3(1.0, 4.0),
                      extra_width_coefficient=rng.r3(0.1, 1.0),
                      max_extra_width_radius=rng.r3(800.0, 3000.0), n_bands=rng.i(1, 6),
                      radius_sample_length=rng.r3(10.0, 40.0), rolling_window_m=rng.r3(20.0, 120.0),
                      edge_margin_m=rng.r3(0.0, 200.0), reference_level_m=rng.r3(-1.0, 10.0))
        if i % 3 == 0:
            params["width_method"] = "pianc"
            params["width_options"] = {"width_multiple": rng.r3(2.4, 3.6),
                                       "use_buildup": rng.p(0.3)}
        if i % 5 == 0:
            params["design_depth_m"] = rng.r3(1.0, 5.0)
        c(f"compute_sweep_{i:02d}", 2, "compute",
          dict(total_loa=rng.r3(50.0, 220.0), beam=rng.r3(6.0, 30.0), params=params,
               centreline_points=pts))
    for i in range(10):
        c(f"extra_width_sweep_{i:02d}", 2, "extra_width",
          dict(total_loa=rng.r3(60.0, 200.0),
               params=dict(P, n_bands=rng.i(1, 7), extra_width_coefficient=rng.r3(0.05, 1.0),
                           max_extra_width_radius=rng.r3(600.0, 3500.0),
                           radius_multiple=rng.r3(2.0, 5.0)),
               radii=[rng.r3(50.0, 4000.0) for _ in range(8)]))
    for i in range(8):
        c(f"grid_sweep_{i:02d}", 2, "compute_scenario_grid",
          dict(centreline_points=arc(rng.r3(200.0, 1800.0), rng.r3(30.0, 130.0)),
               loa_values=[rng.r3(60.0, 180.0), rng.r3(60.0, 180.0)],
               beam_values=[rng.r3(8.0, 30.0), rng.r3(8.0, 30.0)],
               params=dict(P, radius_multiple=rng.r3(2.0, 5.0)), n_samples=rng.i(10, 40)))
    for i in range(8):
        d = rng.r3(0.2, 12.0)
        c(f"depth_sweep_{i:02d}", 2, "max_draft_for_depth",
          dict(depth_m=d, ukc_fraction=rng.r3(0.02, 0.3), ukc_min_m=rng.r3(0.2, 1.2)))
    # --- tier 3: errors ---
    for name, kw in [("width_multiple", {"width_multiple": 0.0}), ("radius_multiple", {"radius_multiple": -1.0}),
                     ("side_slope", {"side_slope": -0.1}), ("radius_sample_length", {"radius_sample_length": 0.0}),
                     ("max_extra_width_radius", {"max_extra_width_radius": 0.0}), ("n_bands", {"n_bands": 0}),
                     ("design_depth", {"design_depth_m": 0.0}), ("width_method", {"width_method": "nope"})]:
        c(f"params_bad_{name}", 3, "resolve_design_depth", dict(params=dict(P, **kw)))
    c("inputs_bad_loa", 3, "compute", dict(total_loa=0.0, beam=10.0, params=P))
    c("inputs_bad_beam", 3, "compute", dict(total_loa=100.0, beam=-1.0, params=P))
    c("grid_bad_method", 3, "compute_scenario_grid",
      dict(centreline_points=arc(500.0), loa_values=[100.0], beam_values=[10.0],
           params=dict(P, width_method="nope")))
    return out
