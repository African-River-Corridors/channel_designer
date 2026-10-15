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

import math
from collections import Counter
from typing import Optional

import numpy as np
from shapely.geometry import LineString, Point

from ..rules.rulesets import BANK_CUT_FRACTION, classify_bend, lateral_room_m

STEP_M = 10.0          # marching step along the natural line
N_PSI = 41             # heading-error states
PSI_MAX_DEG = 12.0     # heading error the corridor search explores
N_Y = 81               # lateral-offset states
E_MAX_M = 300.0        # bisection ceiling; e above this is reported as None (inf)
E_TOL_M = 0.5


def resample(line: LineString, step: float):
    n = int(line.length // step) + 1
    s = np.arange(n) * step
    xy = np.array([line.interpolate(float(d)).coords[0] for d in s])
    return s, xy


def curvature(xy) -> np.ndarray:
    """Signed curvature by 3-point circumradius, positive left. 1/R, so a straight is 0."""
    n = len(xy)
    k = np.zeros(n)
    for i in range(1, n - 1):
        (ax, ay), (bx, by), (cx, cy) = xy[i - 1], xy[i], xy[i + 1]
        cross = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay)
        a = math.hypot(bx - ax, by - ay)
        b = math.hypot(cx - bx, cy - by)
        c = math.hypot(ax - cx, ay - cy)
        if a * b * c < 1e-9:
            continue
        k[i] = 2.0 * cross / (a * b * c)
    k[0], k[-1] = k[1], k[-2]
    return k


def bend_units(s, kappa, smooth_n=5):
    """Split the line at curvature sign changes. A bend unit is one signed lobe.

    Curvature is smoothed first, because raw 3-point curvature on a polyline flips sign on
    noise; the smoothing window is the only tuned number here and it is reported.
    """
    ks = np.convolve(kappa, np.ones(smooth_n) / smooth_n, mode="same")
    sign = np.sign(ks)
    sign[sign == 0] = 1
    idx = [0] + [i for i in range(1, len(sign)) if sign[i] != sign[i - 1]] + [len(sign) - 1]
    out = []
    for a, b in zip(idx[:-1], idx[1:]):
        if b - a < 2:
            continue
        seg = ks[a:b + 1]
        kmax = float(np.max(np.abs(seg)))
        out.append(dict(i0=a, i1=b, s0=float(s[a]), s1=float(s[b]),
                        length_m=float(s[b] - s[a]),
                        sign=int(sign[a]),
                        kappa_max=kmax,
                        r_min_natural_m=(1.0 / kmax) if kmax > 1e-9 else float("inf"),
                        deflection_rad=float(abs(np.sum(seg)) * (s[1] - s[0]))))
    return out


def feasible_within(kappa, ds, half_width, r_min, n_y=N_Y, n_psi=N_PSI):
    """Can any curve with |k| <= 1/r_min traverse this window inside +-half_width?

    Forward reachability over (y, psi) relative to the natural line:
        y'   = psi                     (small-angle lateral drift)
        psi' = k_cmd - k_natural,      |k_cmd| <= 1/r_min
    Both states are discretised; a state is reachable if any predecessor reaches it under
    some admissible k_cmd. Returns True when the reachable set is non-empty at the end.
    """
    if half_width <= 0:
        return bool(np.all(np.abs(kappa) <= 1.0 / r_min + 1e-12))
    ys = np.linspace(-half_width, half_width, n_y)
    psis = np.radians(np.linspace(-PSI_MAX_DEG, PSI_MAX_DEG, n_psi))
    dy, dpsi = ys[1] - ys[0], psis[1] - psis[0]
    k_max = 1.0 / r_min
    reach = np.zeros((n_y, n_psi), dtype=bool)
    reach[np.argmin(np.abs(ys)), np.argmin(np.abs(psis))] = True
    k_cmds = np.linspace(-k_max, k_max, 9)
    for i in range(len(kappa) - 1):
        nxt = np.zeros_like(reach)
        yi, pi = np.nonzero(reach)
        if len(yi) == 0:
            return False
        y0 = ys[yi]
        p0 = psis[pi]
        for kc in k_cmds:
            y1 = y0 + p0 * ds
            p1 = p0 + (kc - kappa[i]) * ds
            j = np.rint((y1 + half_width) / dy).astype(int)
            m = np.rint((p1 + math.radians(PSI_MAX_DEG)) / dpsi).astype(int)
            ok = (j >= 0) & (j < n_y) & (m >= 0) & (m < n_psi)
            nxt[j[ok], m[ok]] = True
        reach = nxt
    return bool(reach.any())


def e_exact(kappa, ds, r_min):
    """Smallest corridor half-width that admits an R_min-feasible curve. Bisection."""
    if feasible_within(kappa, ds, 0.0, r_min):
        return 0.0
    lo, hi = 0.0, E_MAX_M
    if not feasible_within(kappa, ds, hi, r_min):
        return float("inf")
    while hi - lo > E_TOL_M:
        mid = 0.5 * (lo + hi)
        if feasible_within(kappa, ds, mid, r_min):
            hi = mid
        else:
            lo = mid
    return hi


def e_closed_form(r_nat, defl, r_min):
    """The closed form, kept ONLY as a reported cross-check. It bounds nothing."""
    if not math.isfinite(r_nat) or r_nat >= r_min or defl <= 0:
        return 0.0
    return (r_min - r_nat) * (1.0 / math.cos(min(defl, math.pi * 0.98) / 2.0) - 1.0)


def contiguous_wetted_width(river, centre: Point, nx: float, ny: float,
                            max_half_m: float = 200.0, probe_m: float = 1.0) -> float:
    """Wetted width across ``centre``: walk out each side and stop at the FIRST exit.

    Taking the furthest point still inside the polygon instead would jump islands and
    re-entries and overstate the width, which feeds straight into ``room`` and would
    under-call tight bends.
    """
    half = []
    for sgn in (+1.0, -1.0):
        d = 0.0
        while d < max_half_m:
            if not river.contains(Point(centre.x + sgn * (d + probe_m) * nx,
                                        centre.y + sgn * (d + probe_m) * ny)):
                break
            d += probe_m
        half.append(d)
    return float(sum(half))


def classify_bends(natural: LineString, river, r_min: float, top_width: float,
                   bottom_width: float, bank_cut_fraction: float = BANK_CUT_FRACTION,
                   step: float = STEP_M) -> dict:
    """Bend catalogue for ``natural`` against the river polygon ``river``.

    ``bank_cut_fraction`` is a user choice (see ``rules.rulesets.BANK_CUT_FRACTION``).
    Returns ``{"params", "summary", "bends"}``; each bend has its e, room and class.
    """
    s, xy = resample(natural, step)
    kappa = curvature(xy)
    bends = bend_units(s, kappa)
    rows = []
    for b in bends:
        i0, i1 = b["i0"], b["i1"]
        e = e_exact(kappa[i0:i1 + 1], step, r_min)
        mid = natural.interpolate(0.5 * (b["s0"] + b["s1"]))
        j1 = min(i1, len(xy) - 1)
        th = math.atan2(xy[j1][1] - xy[i0][1], xy[j1][0] - xy[i0][0])
        w = contiguous_wetted_width(river, mid, -math.sin(th), math.cos(th))
        room = lateral_room_m(w if w > 0 else top_width, top_width, bottom_width,
                              bank_cut_fraction)
        klass = classify_bend(e, room) if math.isfinite(e) else "T"
        rows.append(dict(**{k: v for k, v in b.items() if k not in ("i0", "i1")},
                         e_m=(e if math.isfinite(e) else None),
                         e_closed_form_m=e_closed_form(b["r_min_natural_m"],
                                                       b["deflection_rad"], r_min),
                         wetted_width_m=w, room_m=room, klass=klass))
    c = Counter(r["klass"] for r in rows)
    lt = sum(r["length_m"] for r in rows if r["klass"] == "T")
    tot = sum(r["length_m"] for r in rows)
    return dict(
        params=dict(r_min_m=r_min, top_width_m=top_width, bottom_width_m=bottom_width,
                    bank_cut_fraction=bank_cut_fraction, step_m=step,
                    n_y=N_Y, n_psi=N_PSI, psi_max_deg=PSI_MAX_DEG,
                    e_method="exact forward-reachability DP + bisection",
                    room_rule="(w - W_top)/2 + bank_cut_fraction * W_bottom"),
        summary=dict(n_bends=len(rows), n_straight=c["S"], n_gentle=c["G"], n_tight=c["T"],
                     tight_length_m=lt, total_length_m=tot,
                     tight_fraction=lt / tot if tot else 0.0),
        bends=rows)
