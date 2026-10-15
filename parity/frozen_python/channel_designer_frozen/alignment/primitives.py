"""The motion-primitive set for a lattice alignment search — one definition, named values.

A lattice search builds a channel from short arcs of fixed radius plus one straight. Every
parameter that can change the answer lives here, and ``descriptor`` returns them all so a
caller can store them with a result (a provenance key). A parameter that changes an answer
and is not in the key is an input that can change silently.

Two straight lengths exist on purpose: ``STRAIGHT_LEN_M`` (60 m) is the live search value;
``LEGACY_STRAIGHT_LEN_M`` (100 m) reproduces older runs. They are named constants with one
definition each, not a duplicated input.

Bend widening per arc is supplied by the caller (``widen_fn(radius_m) -> extra bottom width``),
for example from :func:`channel_designer.rules.width_profile.make_width_fn` or
:func:`lux_widening` below. The primitive set itself knows no vessel.
"""
from __future__ import annotations

import math
from typing import Callable, Optional

# --- owned parameters ---------------------------------------------------------------
SHARED_RADII = [150, 160, 170, 180, 190, 200, 215, 230, 250, 275, 300,
                340, 400, 500, 650, 850, 1200, 2000]
STRAIGHT_LEN_M = 60.0          # the live search
LEGACY_STRAIGHT_LEN_M = 100.0  # reproduces older runs
NOMINAL_ARC_M = 80.0           # target arc length per primitive (snapped per radius)
STEP_M = 10.0                  # arc sampling step — the quadrature of the swept volume
BUILDER = "primitives.build/1"  # identity recorded in the provenance key


def lux_widening(loa_m: float) -> Callable[[float], float]:
    """Return ``widen(R) = LOA² / (2R)`` (0 for a straight).

    The first-order swept-path allowance of a vessel of length LOA on radius R. A convenient
    default; for a named standard use the ruleset's own bend widening instead.
    """
    def widen(radius_m: float) -> float:
        if not math.isfinite(radius_m) or radius_m <= 0:
            return 0.0
        return loa_m * loa_m / (2.0 * radius_m)
    return widen


def radii_for(r_min_m, radii=None):
    """The finite radii a channel of this minimum bend radius may use."""
    radii = SHARED_RADII if radii is None else radii
    return [float(R) for R in radii if R >= float(r_min_m) - 1e-6]


def build(radii_finite, n_heading, nominal_arc=NOMINAL_ARC_M, step_m=STEP_M,
          straight_len=STRAIGHT_LEN_M, widen_fn: Optional[Callable[[float], float]] = None):
    """Arc primitives for an explicit finite-radius list, plus one straight.

    Each primitive is a dict: ``R`` (inf for the straight), ``sign`` (+1 left, -1 right),
    ``pts`` (local samples ``(s, x, y, heading)``), ``end`` (local end pose), ``arc_len``,
    ``dtheta`` and ``widen`` (extra bottom width from ``widen_fn``; 0 if not given).

    The turn snaps to an integer number of heading bins so forward integration lands exactly
    on bin headings and cannot drift. Sample points start at k=1, i.e. the arc's START point
    is not sampled and its END point is — so the swept volume of an arc is not identical
    when the arc is traversed the other way. That asymmetry is small but real; a search that
    assumes direction symmetry should state it as an assumption.
    """
    widen = widen_fn if widen_fn is not None else (lambda _r: 0.0)
    prims = []
    bin_size = 2 * math.pi / n_heading
    for R in radii_finite:
        kbins = max(1, int(round((nominal_arc / R) / bin_size)))
        dtheta_mag = kbins * bin_size
        arc_len = R * dtheta_mag
        n = max(2, int(round(arc_len / step_m)))
        for sgn in (+1, -1):
            kappa = sgn / R
            pts = []
            for k in range(1, n + 1):
                s = arc_len * k / n
                pts.append((s, math.sin(kappa * s) / kappa,
                            (1 - math.cos(kappa * s)) / kappa, kappa * s))
            xe = math.sin(kappa * arc_len) / kappa
            ye = (1 - math.cos(kappa * arc_len)) / kappa
            prims.append(dict(R=R, sign=sgn, pts=pts, end=(xe, ye, kappa * arc_len),
                              arc_len=arc_len, dtheta=kappa * arc_len,
                              widen=widen(R)))
    n = max(2, int(round(straight_len / step_m)))
    pts = [(straight_len * k / n, straight_len * k / n, 0.0, 0.0) for k in range(1, n + 1)]
    prims.append(dict(R=math.inf, sign=0, pts=pts, end=(straight_len, 0.0, 0.0),
                      arc_len=straight_len, dtheta=0.0,
                      widen=widen(math.inf)))
    return prims


def descriptor(radii_finite, n_heading, nominal_arc=NOMINAL_ARC_M, step_m=STEP_M,
               straight_len=STRAIGHT_LEN_M):
    """Everything about the primitive set that can change an answer — for the key."""
    return {
        "builder": BUILDER,
        "radii": [float(r) for r in radii_finite],
        "n_heading": int(n_heading),
        "nominal_arc_m": float(nominal_arc),
        "step_m": float(step_m),
        "straight_len_m": float(straight_len),
    }
