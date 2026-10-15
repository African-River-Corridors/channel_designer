"""Cross-section cut models A and B — the above-waterline cut rule, one definition.

CUT MODEL A. Per side, per station:

  WET, below the waterline
    a ``wet_slope`` H:1V batter from the flat invert up to the WL.

  ABOVE the waterline — a clearance prism plus a binary gate
    Keep the channel top width clear above WL. Then, per side:
      - ground does NOT intrude the prism, or intrudes < ``PRISM_INTRUDE_M`` horizontally
          -> ZERO cut above WL. The natural bank is left exactly as it is, however steep.
      - ground intrudes by more than that
          -> BENCHED cut: a ``BERM_M`` berm at WL, then one equivalent
             ``ABOVE_SLOPE_H_PER_V`` face carried up until it daylights. No vertical faces,
             no height cap.

  The gate is the model's biggest single lever and it is BINARY: whether a section benches
  or not moves its volume far more than where the threshold sits. Treat the branch as the
  modelling decision and test the threshold's sensitivity on your own river.

CUT MODEL B. Model A with ONE change: the transverse integration TERMINATES. A rakes the full
half-width and bills any terrain standing above the rising design face, connected to the
channel or not, so an isolated knoll far out is charged as excavation across an air gap
nobody would dig. B stops each side at the first offset ABOVE THE WATERLINE where the design
face daylights AND STAYS daylit for ``DAYLIGHT_RUN_M``. The sustained run matters: a bare
first-daylight rule would let a one-metre swale end a continuous slope early. Below the WL
nothing changes. B mainly corrects FACE HEIGHT (and drawings); expect it to move the total
volume much less. Compare A and B on your own river.

Every parameter below is an ESTIMATE unless a basis is named. They are listed together
because the model's credibility rests on them. Pure math + numpy; no DEM, no I/O.
"""
from __future__ import annotations

import math

import numpy as np

STATION_M = 25.0        # BASIS: sampling choice
MAXW_M = 200.0          # BASIS: sampling choice — transverse half-width sampled. This rake is
                        # what admits isolated knolls into the Model A volume.
DX_M = 1.0              # BASIS: sampling choice — transverse integration step

CUT_MODEL = "cut_model_a"       # canonical id written to outputs
CUT_MODEL_B = "cut_model_b"
_MODEL_ALIASES = {"cut_model_a": CUT_MODEL, "a": CUT_MODEL,
                  "cut_model_b": CUT_MODEL_B, "b": CUT_MODEL_B}

WET_SLOPE_H_PER_V = 3.0         # BASIS: design input. Wet batter 1V:3H. Pass 2.0 for 1V:2H.
PRISM_CLEAR_M = 5.0             # BASIS: ESTIMATE, "air draft + margin". Air draft is a vessel
                                # property, so this should be derived, not fixed.
PRISM_INTRUDE_M = 2.0           # BASIS: ESTIMATE. The binary bench gate. Test its
                                # sensitivity on your own river.
BERM_M = 2.0                    # BASIS: ESTIMATE — berm width at WL before the benched face.
ABOVE_SLOPE_H_PER_V = 1.5       # BASIS: ESTIMATE — single equivalent benched face, 1.5H:1V.
DAYLIGHT_RUN_M = 15.0           # BASIS: ESTIMATE (Model B only) — the design face must stay
                                # clear of natural ground this far before the works end.

WIDEN_RMAX = 2000.0             # no bend widening beyond this radius (treated as straight)


def normalise_model(name):
    """Canonical model id. Accepts "a", "b" and the long ids."""
    return _MODEL_ALIASES.get(str(name).strip().lower(), str(name).strip().lower())


def local_radii(xs, ys, stride=2):
    """Per-point bend radius (m) via 3-point Menger circumradius, then a short rolling mean
    (+-2 points, ~50 m at 25 m stations). Straight -> inf."""
    n = len(xs); r = np.full(n, np.inf)
    for i in range(n):
        a = max(0, i - stride); c = min(n - 1, i + stride)
        if c - a < 2:
            continue
        ax, ay = xs[a], ys[a]; bx, by = xs[i], ys[i]; cx, cy = xs[c], ys[c]
        area = abs((bx - ax) * (cy - ay) - (cx - ax) * (by - ay)) * 0.5
        if area < 1e-6:
            continue
        ab = math.hypot(bx - ax, by - ay); bc = math.hypot(cx - bx, cy - by); ca = math.hypot(ax - cx, ay - cy)
        r[i] = ab * bc * ca / (4.0 * area)
    out = r.copy()
    for i in range(n):
        w = [v for v in r[max(0, i - 2):i + 3] if np.isfinite(v)]
        if w:
            out[i] = sum(w) / len(w)
    return out


def widened_hb(xy, loa, beam, radius_multiple, width_mult, c_c, rmax=WIDEN_RMAX):
    """Per-station channel bottom HALF-width including WG 141 bend widening.

    base = width_mult*beam/2; dF_C = c_c*LOA^2 / max(R, R_min), capped at <= LOA (WG 141
    5.3.3, p101); half of dF_C is added per side. ``width_mult`` and ``c_c`` come from a
    ruleset (``rules.rulesets.RULESETS[...]``) — there is no default here on purpose.
    """
    base = width_mult * beam / 2.0
    rmin = radius_multiple * loa
    R = local_radii(xy[:, 0], xy[:, 1])
    hb = np.full(len(R), base)
    for i, Ri in enumerate(R):
        if np.isfinite(Ri) and 0 < Ri < rmax:
            dfc = min(c_c * loa * loa / max(Ri, rmin), loa)
            hb[i] = base + 0.5 * dfc
    return hb


def widened_hb_from_shape(xy, shape, rmax=WIDEN_RMAX):
    """Per-station bottom half-width from a :class:`rules.rulesets.ChannelShape`."""
    hb = shape.bottom_width_straight_m / 2.0
    R = local_radii(xy[:, 0], xy[:, 1])
    rmin = shape.min_bend_radius_m
    return np.array([hb + 0.5 * shape.bend_widening_m(max(Ri, rmin))
                     if np.isfinite(Ri) and 0 < Ri < rmax else hb for Ri in R])


def cut_model_a_face(o, side_bench, hb, grade, wl, depth, wet_slope=WET_SLOPE_H_PER_V):
    """Design cut-face elevation at offset ``o`` (>= 0, from the centreline).

    Below WL: ``wet_slope`` H:1V batter from the flat invert to the WL.
    Above WL (gate decided per side by :func:`cut_model_a_bench_gate` -> ``side_bench``):
      False -> +inf (no cut above WL; natural bank left as-is);
      True  -> berm at WL, then a single 1.5H:1V face to daylight (no height cap).
    """
    if o <= hb:
        return grade                                   # flat bottom
    r = o - hb
    r_wl = wet_slope * depth                            # wet batter reaches WL here
    if r <= r_wl:
        return grade + r / wet_slope                    # submerged wet batter
    if not side_bench:
        return math.inf                                # zero cut above WL
    r2 = r - r_wl                                       # horizontal beyond the WL point
    if r2 <= BERM_M:
        return wl                                       # berm at WL
    return wl + (r2 - BERM_M) / ABOVE_SLOPE_H_PER_V   # benched face to daylight


def cut_model_a_bench_gate(offsets, terrain, hb, wl, depth, sidewhich=None,
                           wet_slope=WET_SLOPE_H_PER_V):
    """Per-side clearance-prism gate for ONE transverse profile. Returns {1: bool, -1: bool}.

    Does natural ground intrude the channel-top prism (|offset| <= o_wl, terrain above WL)
    by more than ``PRISM_INTRUDE_M`` horizontally? o_wl tracks ``wet_slope``. The single
    source of the above-WL bench decision — used by the volume integrator AND the drawings.
    """
    o_wl = hb + wet_slope * depth                      # channel top half-width at WL
    if sidewhich is None:
        sidewhich = [(-1 if o < 0 else 1) for o in offsets]
    bench = {1: False, -1: False}
    for s in (1, -1):
        min_o = None
        for k in range(len(offsets)):
            if sidewhich[k] != s:
                continue
            z = terrain[k]
            if z is None or not np.isfinite(z):
                continue
            o = abs(offsets[k])
            if o <= o_wl and z > wl:                    # terrain above WL inside the prism
                if min_o is None or o < min_o:
                    min_o = o
        intrusion = 0.0 if min_o is None else (o_wl - min_o)
        bench[s] = intrusion > PRISM_INTRUDE_M
    return bench


def cut_model_b_daylight_limit(offsets, terrain, hb, grade, wl, depth, bench,
                               sidewhich=None, wet_slope=WET_SLOPE_H_PER_V, run_m=None):
    """CUT MODEL B ONLY. Per side, the offset at which the works STOP. {1: lim, -1: lim}.

    Walking outward, the first offset ABOVE THE WATERLINE where the design face stands clear
    of natural ground AND stays clear for ``run_m`` metres; ``inf`` if it never does inside
    the rake. A DEM gap counts as daylit: a hole in the raster must not RE-OPEN the works.
    """
    if sidewhich is None:
        sidewhich = [(-1 if o < 0 else 1) for o in offsets]
    if run_m is None:
        run_m = DAYLIGHT_RUN_M
    o_wl = hb + wet_slope * depth
    limits = {1: math.inf, -1: math.inf}
    for s in (1, -1):
        idx = sorted([k for k in range(len(offsets)) if sidewhich[k] == s],
                     key=lambda k: abs(offsets[k]))
        run_start = None
        for k in idx:
            o = abs(offsets[k])
            if o <= o_wl:                      # inside the prism: never terminates
                continue
            z = terrain[k]
            f = cut_model_a_face(o, bench[s], hb, grade, wl, depth, wet_slope=wet_slope)
            daylit = (z is None or not np.isfinite(z) or not np.isfinite(f) or z <= f)
            if not daylit:
                run_start = None
                continue
            if run_start is None:
                run_start = o
            if o - run_start >= run_m:
                limits[s] = run_start
                break
    return limits


def cut_model_a_section_face(offsets, terrain, hb, grade, wl, depth,
                             wet_slope=WET_SLOPE_H_PER_V, model=CUT_MODEL, run_m=None):
    """Per-offset cut-face elevation for ONE transverse profile (for drawings).

    Reuses the gate + face — the SAME functions the volume model uses. Returns a list aligned
    to ``offsets``; None where there is no cut face (no cut above WL, or beyond the Model B
    limit). Passing the model matters: a drawing that hatches a wedge the volume model no
    longer takes is exactly the drawing-vs-model drift this function exists to prevent.
    """
    bench = cut_model_a_bench_gate(offsets, terrain, hb, wl, depth, wet_slope=wet_slope)
    limits = (cut_model_b_daylight_limit(offsets, terrain, hb, grade, wl, depth, bench,
                                         wet_slope=wet_slope, run_m=run_m)
              if normalise_model(model) == CUT_MODEL_B else None)
    out = []
    for off in offsets:
        s = -1 if off < 0 else 1
        if limits is not None and abs(off) > limits[s]:
            out.append(None); continue
        f = cut_model_a_face(abs(off), bench[s], hb, grade, wl, depth, wet_slope=wet_slope)
        out.append(None if not np.isfinite(f) else round(float(f), 2))
    return out


def section_area(zs, xs, sidewhich, hb, grade, wl, depth, model, wet,
                 wet_slope=WET_SLOPE_H_PER_V, dx=DX_M, run_m=None):
    """(dredge_area, excav_area, max_face_h) for one transverse profile.

    ``zs`` terrain per offset (None/NaN where unknown); ``xs`` signed offsets at spacing ``dx``;
    ``wet[k]`` True inside the present river (dredge) else dry (excavation).
    ``max_face_h`` is the tallest cut face ABOVE the waterline, m.
    """
    mdl = normalise_model(model)
    if mdl not in (CUT_MODEL, CUT_MODEL_B):
        raise ValueError(f"unknown cut model {model!r}. Known: {CUT_MODEL!r} (A) and "
                         f"{CUT_MODEL_B!r} (B).")
    bench = cut_model_a_bench_gate(xs, zs, hb, wl, depth, sidewhich, wet_slope=wet_slope)
    limits = (cut_model_b_daylight_limit(xs, zs, hb, grade, wl, depth, bench, sidewhich,
                                         wet_slope=wet_slope, run_m=run_m)
              if mdl == CUT_MODEL_B else None)
    dredge = excav = 0.0
    max_face_h = 0.0
    for k in range(len(xs)):
        z = zs[k]
        if z is None or not np.isfinite(z):
            continue
        o = abs(xs[k]); s = sidewhich[k]
        if limits is not None and o > limits[s]:
            continue
        f = cut_model_a_face(o, bench[s], hb, grade, wl, depth, wet_slope=wet_slope)
        if not np.isfinite(f):
            continue
        cut = z - f
        if cut <= 0:
            continue
        a = cut * dx
        if wet[k]:
            dredge += a
        else:
            excav += a
        if z > wl and (z - wl) > max_face_h:
            max_face_h = z - wl
    return dredge, excav, max_face_h
