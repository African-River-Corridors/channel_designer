"""cut_model_grid: `volumes/cut_model_grid.py` (the vectorised twin of the cut models)."""
from __future__ import annotations

import math

import numpy as np

from ..common import Rng, case as _case, frozen, rnd

ENTRY_POINT = "cut_model_grid"
R = "initial case set (spec 0001, ticket 23)"


def _rake(args):
    arr = lambda k, dt=float: np.array(args[k], dtype=dt)  # noqa: E731
    gz2 = arr("gz2")
    # `null` in the ground means unknown: NaN, with `finite2` false
    gz2 = np.where(np.isnan(gz2), np.nan, gz2)
    return arr("tmpl2"), gz2, np.array(args["finite2"], dtype=bool), arr("offs")


def adapter(fn: str, args: dict):
    cg = frozen("volumes.cut_model_grid")
    if fn == "cut_model_a_template":
        tmpl2, gz2, finite2, offs = _rake(args)
        hb = np.array(args["hb"], dtype=float) if isinstance(args["hb"], list) else args["hb"]
        kw = {"wet_slope": args["wet_slope"]} if "wet_slope" in args else {}
        return cg.cut_model_a_template(tmpl2, gz2, finite2, offs, hb, args["grade"], args["wl"], **kw)
    if fn == "cut_model_b_template":
        tmpl2, gz2, finite2, offs = _rake(args)
        hb = np.array(args["hb"], dtype=float) if isinstance(args["hb"], list) else args["hb"]
        kw = {k: args[k] for k in ("wet_slope", "run_m", "step") if k in args}
        return cg.cut_model_b_template(tmpl2, gz2, finite2, offs, hb, args["grade"], args["wl"], **kw)
    if fn == "beyond_works_mask":
        gz2 = np.array(args["gz2"], dtype=float)
        finite2 = np.array(args["finite2"], dtype=bool)
        offs = np.array(args["offs"], dtype=float)
        return cg.beyond_works_mask(np.array(args["comp"], dtype=float), gz2, finite2, offs,
                                    args["o_wl"], args["run_m"], args["step"])
    raise KeyError(f"cut_model_grid has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    DEPTH, HB, WL = 3.25, 11.2, 7.0
    GRADE = WL - DEPTH
    R_WL = 3.0 * DEPTH
    offs = np.arange(-60.0, 61.0, 1.0)
    wall_start = HB + R_WL - 5.0
    wall = np.array([WL + 10.0 if abs(o) >= wall_start else WL - 5.0 for o in offs])
    tmpl0 = GRADE + np.maximum(0.0, np.abs(offs) - HB) / 3.0

    def knoll_row(o_arr):
        o_wl = HB + R_WL
        return np.array([WL + 3.0 if (o > 0 and o_wl - 4 <= o <= o_wl + 6) else
                         (WL + 60.0 if (o > 0 and 60 <= o <= 80) else WL - 4.0) for o in o_arr])

    offs_k = np.arange(-120.0, 121.0, 1.0)
    tmpl_k = GRADE + np.maximum(0.0, np.abs(offs_k) - HB) / 3.0
    knoll = knoll_row(offs_k)
    fin = lambda g: np.isfinite(g)  # noqa: E731

    def rake(tmpl, ground):
        g = np.where(np.isnan(ground), 0.0, ground)
        return dict(tmpl2=tmpl[None, :], gz2=np.where(np.isfinite(ground), ground, 0.0)[None, :],
                    finite2=np.isfinite(ground)[None, :])

    c("a_wall", 1, "cut_model_a_template", dict(**rake(tmpl0, wall), offs=offs, hb=HB, grade=GRADE, wl=WL))
    c("a_two_sections_per_section_hb", 1, "cut_model_a_template", dict(
        tmpl2=np.vstack([tmpl0, tmpl0]), gz2=np.vstack([wall, wall]),
        finite2=np.ones((2, len(offs)), dtype=bool), offs=offs, hb=[HB, HB + 3.0], grade=GRADE, wl=WL))
    c("a_hb_wrong_length_refused", 3, "cut_model_a_template", dict(
        tmpl2=np.vstack([tmpl0, tmpl0]), gz2=np.vstack([wall, wall]),
        finite2=np.ones((2, len(offs)), dtype=bool), offs=offs, hb=[HB] * 3, grade=GRADE, wl=WL))
    c("a_hb_matrix_refused", 3, "cut_model_a_template", dict(
        tmpl2=tmpl0[None, :], gz2=wall[None, :], finite2=np.ones((1, len(offs)), dtype=bool), offs=offs,
        hb=[[HB]], grade=GRADE, wl=WL))
    c("a_wet_slope_2", 1, "cut_model_a_template", dict(**rake(tmpl0, wall), offs=offs, hb=HB, grade=GRADE, wl=WL, wet_slope=2.0))
    c("a_unknown_ground", 1, "cut_model_a_template", dict(
        tmpl2=tmpl0[None, :], gz2=wall[None, :], finite2=(np.arange(len(offs)) % 7 != 0)[None, :],
        offs=offs, hb=HB, grade=GRADE, wl=WL))
    c("a_knoll", 1, "cut_model_a_template", dict(**rake(tmpl_k, knoll), offs=offs_k, hb=HB, grade=GRADE, wl=WL))
    c("b_knoll", 1, "cut_model_b_template", dict(**rake(tmpl_k, knoll), offs=offs_k, hb=HB, grade=GRADE, wl=WL))
    c("b_knoll_explicit_step_and_run", 1, "cut_model_b_template",
      dict(**rake(tmpl_k, knoll), offs=offs_k, hb=HB, grade=GRADE, wl=WL, step=1.0, run_m=30.0))
    c("b_wall_per_section_hb", 1, "cut_model_b_template", dict(
        tmpl2=np.vstack([tmpl0, tmpl0]), gz2=np.vstack([wall, wall]),
        finite2=np.ones((2, len(offs)), dtype=bool), offs=offs, hb=[HB, HB + 2.0], grade=GRADE, wl=WL))
    c("b_symmetric_rake_step_from_unique_offsets", 1, "cut_model_b_template",
      dict(**rake(tmpl_k, knoll), offs=offs_k, hb=HB, grade=GRADE, wl=WL))
    c("b_single_offset_rake", 1, "cut_model_b_template", dict(
        tmpl2=[[GRADE]], gz2=[[WL + 1.0]], finite2=[[True]], offs=[0.0], hb=HB, grade=GRADE, wl=WL))
    c("beyond_mask_knoll", 1, "beyond_works_mask", dict(
        comp=tmpl_k[None, :], gz2=knoll[None, :], finite2=np.ones((1, len(offs_k)), dtype=bool), offs=offs_k,
        o_wl=HB + R_WL, run_m=15.0, step=1.0))
    rng = Rng(23017)
    for i in range(16):
        n_sec = rng.i(1, 5)
        half = rng.r3(30.0, 90.0)
        step = rng.r3(0.5, 2.5)
        o = np.round(np.arange(-half, half + 1e-9, step), 4)
        hb = rng.r3(5.0, 14.0)
        depth = rng.r3(2.0, 4.5)
        wl = rng.r3(5.0, 9.0)
        grade = wl - depth
        tm = np.vstack([grade + np.maximum(0.0, np.abs(o) - hb) / 3.0] * n_sec)
        gz = np.array([[rng.r3(wl - 6.0, wl + 14.0) for _ in o] for _ in range(n_sec)])
        fn_ = np.array([[not rng.p(0.05) for _ in o] for _ in range(n_sec)])
        hbs = [rng.r3(5.0, 14.0) for _ in range(n_sec)] if i % 2 else hb
        c(f"a_sweep_{i:02d}", 2, "cut_model_a_template",
          dict(tmpl2=tm, gz2=gz, finite2=fn_, offs=o, hb=hbs, grade=grade, wl=wl, wet_slope=rng.r3(2.0, 4.0)))
        c(f"b_sweep_{i:02d}", 2, "cut_model_b_template",
          dict(tmpl2=tm, gz2=gz, finite2=fn_, offs=o, hb=hbs, grade=grade, wl=wl, wet_slope=rng.r3(2.0, 4.0),
               run_m=rng.r3(4.0, 30.0)))
    return out
