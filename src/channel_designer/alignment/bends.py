"""Bend catalogue and the TIGHT / GENTLE / STRAIGHT classification of a natural river.

Design-independent: it runs on the NATURAL centreline and the river polygon, and sees the
vessel only through (R_min, W_top, W_bottom). No design alignment is needed.

    e(bend)  the smallest lateral corridor half-width about the natural centreline inside
             which SOME curve of radius >= R_min can traverse the bend.
    room     how far the channel may sit off the natural line before it has left the river
             (:func:`channel_designer.rules.rulesets.lateral_room_m`).
    class    e == 0 -> S (straight) | e <= room -> G (gentle, follow it) | else T (tight)

A tight bend is what an alignment optimiser must solve; a gentle one can be followed.

e IS COMPUTED EXACTLY, NOT FROM A CLOSED FORM. The closed form
e = (R_min - R)(sec(phi/2) - 1) has a two-sided bias — it over-calls compound bends and
truncates deflection to the measurement window — so it bounds nothing. Instead a
forward-reachability DP over (lateral offset y, heading error psi) with |curvature| <= 1/R_min
is marched along the natural line, and e is found by bisection on the corridor half-width.
The closed form is kept only as a reported cross-check.
"""
from __future__ import annotations

from .. import _rust


import numpy as np
from shapely.geometry import LineString, Point

from ..rules.rulesets import BANK_CUT_FRACTION

STEP_M = 10.0          # marching step along the natural line
N_PSI = 41             # heading-error states
PSI_MAX_DEG = 12.0     # heading error the corridor search explores
N_Y = 81               # lateral-offset states
E_MAX_M = 300.0        # bisection ceiling; e above this is reported as None (inf)
E_TOL_M = 0.5


def resample(line: LineString, step: float):
    r = _rust.call("bends", "resample", line=line, step=step)
    return np.asarray(r["s"], dtype=float), np.asarray(r["xy"], dtype=float).reshape(-1, 2)


def curvature(xy) -> np.ndarray:
    """Signed curvature by 3-point circumradius, positive left. 1/R, so a straight is 0."""
    return _rust.call("bends", "curvature", np.ndarray, xy=xy)


def bend_units(s, kappa, smooth_n=5):
    """Split the line at curvature sign changes. A bend unit is one signed lobe.

    Curvature is smoothed first, because raw 3-point curvature on a polyline flips sign on
    noise; the smoothing window is the only tuned number here and it is reported.
    """
    return _rust.call("bends", "bend_units", s=s, kappa=kappa, smooth_n=int(smooth_n))


def feasible_within(kappa, ds, half_width, r_min, n_y=N_Y, n_psi=N_PSI):
    """Can any curve with |k| <= 1/r_min traverse this window inside +-half_width?

    Forward reachability over (y, psi) relative to the natural line:
        y'   = psi                     (small-angle lateral drift)
        psi' = k_cmd - k_natural,      |k_cmd| <= 1/r_min
    Both states are discretised; a state is reachable if any predecessor reaches it under
    some admissible k_cmd. Returns True when the reachable set is non-empty at the end.
    """
    return _rust.call("bends", "feasible_within", bool, kappa=kappa, ds=ds, half_width=half_width,
                          r_min=r_min, n_y=int(n_y), n_psi=int(n_psi))


def e_exact(kappa, ds, r_min):
    """Smallest corridor half-width that admits an R_min-feasible curve. Bisection."""
    return _rust.call("bends", "e_exact", float, kappa=kappa, ds=ds, r_min=r_min)


def e_closed_form(r_nat, defl, r_min):
    """The closed form, kept ONLY as a reported cross-check. It bounds nothing."""
    return _rust.call("bends", "e_closed_form", float, r_nat=r_nat, defl=defl, r_min=r_min)


def contiguous_wetted_width(river, centre: Point, nx: float, ny: float,
                            max_half_m: float = 200.0, probe_m: float = 1.0) -> float:
    """Wetted width across ``centre``: walk out each side and stop at the FIRST exit.

    Taking the furthest point still inside the polygon instead would jump islands and
    re-entries and overstate the width, which feeds straight into ``room`` and would
    under-call tight bends.
    """
    return _rust.call("bends", "contiguous_wetted_width", float, river=river, centre=[centre.x, centre.y],
                      nx=nx, ny=ny, max_half_m=max_half_m, probe_m=probe_m)


def classify_bends(natural: LineString, river, r_min: float, top_width: float,
                   bottom_width: float, bank_cut_fraction: float = BANK_CUT_FRACTION,
                   step: float = STEP_M) -> dict:
    """Bend catalogue for ``natural`` against the river polygon ``river``.

    ``bank_cut_fraction`` is a user choice (see ``rules.rulesets.BANK_CUT_FRACTION``).
    Returns ``{"params", "summary", "bends"}``; each bend has its e, room and class.
    """
    return _rust.call("bends", "classify_bends", natural=natural, river=river, r_min=r_min, top_width=top_width,
                      bottom_width=bottom_width, bank_cut_fraction=bank_cut_fraction, step=step)
