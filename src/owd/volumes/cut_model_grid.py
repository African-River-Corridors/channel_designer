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

from .. import _rust

import numpy as np

from .cut_model import DAYLIGHT_RUN_M, WET_SLOPE_H_PER_V


def _hb_arg(hb, n):
    """``hb`` as the core reads it: a number, or one per section. Anything else is refused."""
    a = np.asarray(hb, dtype=float)
    if a.ndim == 0:
        return float(a)
    if a.ndim == 1 and a.shape[0] == n:
        return a
    raise ValueError(f"hb must be a scalar or an (n_sections,) vector; got shape {a.shape} "
                     f"for {n} sections")


def cut_model_a_template(tmpl2, gz2, finite2, offs, hb, grade, wl, wet_slope=WET_SLOPE_H_PER_V):
    """Cut Model A design surface for every section. Returns ``(n, m)``; +inf = no cut."""
    return _rust.call("cut_model_grid", "cut_model_a_template", np.ndarray, tmpl2=tmpl2, gz2=gz2,
                      finite2=np.asarray(finite2, dtype=bool), offs=offs, hb=_hb_arg(hb, gz2.shape[0]),
                      grade=grade, wl=wl, wet_slope=wet_slope)


def beyond_works_mask(comp, gz2, finite2, offs, o_wl, run_m, step):
    """(n, m) True where the offset lies BEYOND the works under Cut Model B."""
    r = _rust.call("cut_model_grid", "beyond_works_mask", comp=comp, gz2=gz2,
                   finite2=np.asarray(finite2, dtype=bool), offs=offs, o_wl=o_wl, run_m=run_m, step=step)
    return np.asarray(r, dtype=bool)


def cut_model_b_template(tmpl2, gz2, finite2, offs, hb, grade, wl,
                         wet_slope=WET_SLOPE_H_PER_V, run_m=DAYLIGHT_RUN_M, step=None):
    """Cut Model A's template, then close the works at sustained daylight.

    The limit is quantised to the rake step: a search proxy. Quote volumes from the scalar
    integrator (:func:`owd.volumes.sections.section_volumes`).
    """
    return _rust.call("cut_model_grid", "cut_model_b_template", np.ndarray, tmpl2=tmpl2, gz2=gz2,
                      finite2=np.asarray(finite2, dtype=bool), offs=offs, hb=_hb_arg(hb, gz2.shape[0]),
                      grade=grade, wl=wl, wet_slope=wet_slope, run_m=run_m, step=step)
