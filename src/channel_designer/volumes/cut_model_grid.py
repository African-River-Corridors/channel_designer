"""Vectorised twin of the cut models, for many cross-sections at once.

Same rules as :mod:`.cut_model` (and the same constants — imported, never restated), laid out
for an ``(n_sections, n_offsets)`` rake so a search scorer can evaluate thousands of sections
quickly. ``tests/volumes/test_cut_model.py`` proves parity with the scalar functions.

Inputs, all aligned on one offset vector ``offs`` (m, signed):
  ``tmpl2``   (n, m) caller's fairway template (used where |offset| <= hb)
  ``gz2``     (n, m) ground elevation
  ``finite2`` (n, m) True where ``gz2`` is known
  ``hb``      scalar or (n,) bottom half-width
"""
from __future__ import annotations

import numpy as np

from .cut_model import (
    ABOVE_SLOPE_H_PER_V,
    BERM_M,
    DAYLIGHT_RUN_M,
    PRISM_INTRUDE_M,
    WET_SLOPE_H_PER_V,
)


def _hb_column(hb, n):
    """Normalise ``hb`` to broadcast against an ``(n, m)`` rake.

    scalar        -> a Python float
    (n,) vector   -> an ``(n, 1)`` column
    anything else -> ValueError
    """
    a = np.asarray(hb, dtype=float)
    if a.ndim == 0:
        return float(a)
    if a.ndim == 1 and a.shape[0] == n:
        return a[:, None]
    raise ValueError(f"hb must be a scalar or an (n_sections,) vector; got shape {a.shape} "
                     f"for {n} sections")


def cut_model_a_template(tmpl2, gz2, finite2, offs, hb, grade, wl, wet_slope=WET_SLOPE_H_PER_V):
    """Cut Model A design surface for every section. Returns ``(n, m)``; +inf = no cut."""
    n = gz2.shape[0]
    hbc = _hb_column(hb, n)
    ao = np.abs(offs)
    depth = wl - grade
    r_wl = wet_slope * depth
    if isinstance(hbc, float):
        r2 = (ao - hbc)[None, :]
        o_wl = hbc + r_wl
        prism = (ao <= o_wl)[None, :]
        pen = np.where(prism, o_wl - ao, 0.0)
    else:
        r2 = ao[None, :] - hbc
        o_wl = hbc + r_wl
        prism = ao[None, :] <= o_wl
        pen = np.where(prism, o_wl - ao[None, :], 0.0)
    wet_line = grade + np.maximum(0.0, r2) / wet_slope
    in_wet = (r2 > 0) & (r2 <= r_wl)
    in_berm = (r2 > r_wl) & (r2 <= r_wl + BERM_M)
    in_face = r2 > r_wl + BERM_M
    tA = np.where(in_wet, wet_line, np.inf)
    tB = np.where(in_wet, wet_line,
                  np.where(in_berm, wl,
                           np.where(in_face,
                                    wl + (r2 - r_wl - BERM_M) / ABOVE_SLOPE_H_PER_V,
                                    np.inf)))
    left = (offs < 0)[None, :]; right = (offs > 0)[None, :]
    intrude = finite2 & (gz2 > wl) & prism
    pen_left = np.where(intrude & left, pen, 0.0).max(axis=1)
    pen_right = np.where(intrude & right, pen, 0.0).max(axis=1)
    bL = pen_left > PRISM_INTRUDE_M
    bR = pen_right > PRISM_INTRUDE_M
    useB = (bL[:, None] & left) | (bR[:, None] & right)
    comp = np.where(useB, tB, tA)
    fairway = r2 <= 0
    return np.where(fairway, tmpl2, comp)


def beyond_works_mask(comp, gz2, finite2, offs, o_wl, run_m, step):
    """(n, m) True where the offset lies BEYOND the works under Cut Model B."""
    n, m = gz2.shape
    ao = np.abs(offs)
    daylit = (~finite2) | (~np.isfinite(comp)) | (gz2 <= comp)
    outside = np.broadcast_to(ao[None, :] > o_wl, (n, m))
    L = max(1, int(np.ceil(float(run_m) / float(step))))
    beyond = np.zeros((n, m), dtype=bool)
    for sel in (offs > 0, offs < 0):
        idx = np.where(sel)[0]
        if idx.size == 0:
            continue
        order = idx[np.argsort(ao[idx])]
        k = order.size
        d = daylit[:, order]
        cand = d & outside[:, order]
        c = np.concatenate([np.zeros((n, 1), dtype=np.int64),
                            np.cumsum((~d).astype(np.int64), axis=1)], axis=1)
        i0 = np.arange(k)
        hi = i0 + L
        full = hi < k
        win = np.zeros((n, k), dtype=bool)
        win[:, full] = (c[:, hi[full] + 1] - c[:, i0[full]]) == 0
        ok = win & cand
        any_ok = ok.any(axis=1)
        first = np.where(any_ok, ok.argmax(axis=1), k)
        beyond[:, order] = i0[None, :] >= first[:, None]
    return beyond


def cut_model_b_template(tmpl2, gz2, finite2, offs, hb, grade, wl,
                         wet_slope=WET_SLOPE_H_PER_V, run_m=DAYLIGHT_RUN_M, step=None):
    """Cut Model A's template, then close the works at sustained daylight.

    The limit is quantised to the rake step: a search proxy. Quote volumes from the scalar
    integrator (:func:`channel_designer.volumes.sections.section_volumes`).
    """
    comp = cut_model_a_template(tmpl2, gz2, finite2, offs, hb, grade, wl, wet_slope=wet_slope)
    n = gz2.shape[0]
    hbc = _hb_column(hb, n)
    o_wl = hbc + wet_slope * (wl - grade)
    if step is None:
        # Rake step from the distinct |offset| values. (np.unique matters: a symmetric rake
        # repeats every |offset|, and a plain sort-and-diff then returns a step of 0.)
        u = np.unique(np.abs(offs))
        step = float(np.min(np.diff(u))) if u.size > 1 else 1.0
    beyond = beyond_works_mask(comp, gz2, finite2, offs, o_wl, run_m, step)
    return np.where(beyond, np.inf, comp)
