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

from .. import _rust

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
    return _rust.call("cut_model_extras", "normalise_model", names=[str(name)])[0]


def local_radii(xs, ys, stride=2):
    """Per-point bend radius (m) via 3-point Menger circumradius, then a short rolling mean
    (+-2 points, ~50 m at 25 m stations). Straight -> inf."""
    return _rust.call("cut_model_extras", "local_radii", np.ndarray, xs=xs, ys=ys, stride=int(stride))


def widened_hb(xy, loa, beam, radius_multiple, width_mult, c_c, rmax=WIDEN_RMAX):
    """Per-station channel bottom HALF-width including WG 141 bend widening.

    base = width_mult*beam/2; dF_C = c_c*LOA^2 / max(R, R_min), capped at <= LOA (WG 141
    5.3.3, p101); half of dF_C is added per side. ``width_mult`` and ``c_c`` come from a
    ruleset (``rules.rulesets.RULESETS[...]``) — there is no default here on purpose.
    """
    return _rust.call("cut_model", "widened_hb", np.ndarray, xy=xy, loa=loa, beam=beam,
                      radius_multiple=radius_multiple, width_mult=width_mult, c_c=c_c, rmax=rmax)


def widened_hb_from_shape(xy, shape, rmax=WIDEN_RMAX):
    """Per-station bottom half-width from a :class:`rules.rulesets.ChannelShape`."""
    from ..rules.rulesets import RULESETS
    return _rust.call("cut_model_extras", "widened_hb_from_shape", np.ndarray, xy=xy,
                      ruleset_id=shape.ruleset_id, vessel_standard_id=shape.vessel_standard_id, rmax=rmax,
                      host_rulesets=[_rust.enc(r) for r in RULESETS.values()])


def cut_model_a_face(o, side_bench, hb, grade, wl, depth, wet_slope=WET_SLOPE_H_PER_V):
    """Design cut-face elevation at offset ``o`` (>= 0, from the centreline).

    Below WL: ``wet_slope`` H:1V batter from the flat invert to the WL.
    Above WL (gate decided per side by :func:`cut_model_a_bench_gate` -> ``side_bench``):
      False -> +inf (no cut above WL; natural bank left as-is);
      True  -> berm at WL, then a single 1.5H:1V face to daylight (no height cap).
    """
    return _rust.call("cut_model", "cut_model_a_face", float, o=o, side_bench=bool(side_bench), hb=hb, grade=grade,
                      wl=wl, depth=depth, wet_slope=wet_slope)


def cut_model_a_bench_gate(offsets, terrain, hb, wl, depth, sidewhich=None,
                           wet_slope=WET_SLOPE_H_PER_V):
    """Per-side clearance-prism gate for ONE transverse profile. Returns {1: bool, -1: bool}.

    Does natural ground intrude the channel-top prism (|offset| <= o_wl, terrain above WL)
    by more than ``PRISM_INTRUDE_M`` horizontally? o_wl tracks ``wet_slope``. The single
    source of the above-WL bench decision — used by the volume integrator AND the drawings.
    """
    r = _rust.call("cut_model", "cut_model_a_bench_gate", offsets=offsets, terrain=terrain, hb=hb, wl=wl,
                   depth=depth, sidewhich=sidewhich, wet_slope=wet_slope)
    return {1: bool(r["1"]), -1: bool(r["-1"])}


def cut_model_b_daylight_limit(offsets, terrain, hb, grade, wl, depth, bench,
                               sidewhich=None, wet_slope=WET_SLOPE_H_PER_V, run_m=None):
    """CUT MODEL B ONLY. Per side, the offset at which the works STOP. {1: lim, -1: lim}.

    Walking outward, the first offset ABOVE THE WATERLINE where the design face stands clear
    of natural ground AND stays clear for ``run_m`` metres; ``inf`` if it never does inside
    the rake. A DEM gap counts as daylit: a hole in the raster must not RE-OPEN the works.
    """
    r = _rust.call("cut_model", "cut_model_b_daylight_limit", offsets=offsets, terrain=terrain, hb=hb,
                   grade=grade, wl=wl, depth=depth, bench={str(k): bool(v) for k, v in bench.items()},
                   sidewhich=sidewhich, wet_slope=wet_slope, run_m=run_m)
    return {1: float(r["1"]), -1: float(r["-1"])}


def cut_model_a_section_face(offsets, terrain, hb, grade, wl, depth,
                             wet_slope=WET_SLOPE_H_PER_V, model=CUT_MODEL, run_m=None):
    """Per-offset cut-face elevation for ONE transverse profile (for drawings).

    Reuses the gate + face — the SAME functions the volume model uses. Returns a list aligned
    to ``offsets``; None where there is no cut face (no cut above WL, or beyond the Model B
    limit). Passing the model matters: a drawing that hatches a wedge the volume model no
    longer takes is exactly the drawing-vs-model drift this function exists to prevent.
    """
    return _rust.call("cut_model_extras", "cut_model_a_section_face", offsets=offsets, terrain=terrain, hb=hb,
                      grade=grade, wl=wl, depth=depth, wet_slope=wet_slope, model=model, run_m=run_m)


def section_area(zs, xs, sidewhich, hb, grade, wl, depth, model, wet,
                 wet_slope=WET_SLOPE_H_PER_V, dx=DX_M, run_m=None):
    """(dredge_area, excav_area, max_face_h) for one transverse profile.

    ``zs`` terrain per offset (None/NaN where unknown); ``xs`` signed offsets at spacing ``dx``;
    ``wet[k]`` True inside the present river (dredge) else dry (excavation).
    ``max_face_h`` is the tallest cut face ABOVE the waterline, m.
    """
    r = _rust.call("cut_model", "section_area", zs=zs, xs=xs, sidewhich=sidewhich, hb=hb, grade=grade, wl=wl,
                   depth=depth, model=model, wet=[bool(w) for w in wet], wet_slope=wet_slope, dx=dx, run_m=run_m)
    return float(r[0]), float(r[1]), float(r[2])


_rust.core_constants(__name__, "STATION_M", "MAXW_M", "DX_M", "WET_SLOPE_H_PER_V", "PRISM_CLEAR_M",
                     "PRISM_INTRUDE_M", "BERM_M", "ABOVE_SLOPE_H_PER_V", "DAYLIGHT_RUN_M", "WIDEN_RMAX")
