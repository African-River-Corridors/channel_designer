"""Cut Model A/B: the clearance-prism gate, the benched face, Model B termination, and parity
between the scalar functions (the volume integrator) and the vectorised twin (the scorer).

Pure functions only — no DEM needed.
"""
from __future__ import annotations

import math

import numpy as np
import pytest

from channel_designer.volumes import cut_model as cm
from channel_designer.volumes import cut_model_grid as cg

DEPTH = 3.25
HB = 2.8 * 8 / 2.0        # bottom half-width 11.2 m
WL = 7.0
GRADE = WL - DEPTH        # design invert
R_WL = cm.WET_SLOPE_H_PER_V * DEPTH   # 9.75 m horizontal run of the wet batter


def _approx(a, b, tol=1e-6):
    assert abs(a - b) < tol, f"{a} != {b}"


def _xs():
    xs = list(np.arange(-40.0, 40.0 + 0.5, cm.DX_M))
    return xs, [(-1 if o < 0 else 1) for o in xs]


# --- golden -----------------------------------------------------------------------------

def test_flat_bottom():
    _approx(cm.cut_model_a_face(0.0, False, HB, GRADE, WL, DEPTH), GRADE)
    _approx(cm.cut_model_a_face(HB, True, HB, GRADE, WL, DEPTH), GRADE)


def test_wet_batter_is_1v3h():
    o = HB + 3.0
    _approx(cm.cut_model_a_face(o, False, HB, GRADE, WL, DEPTH), GRADE + 1.0)
    _approx(cm.cut_model_a_face(o, True, HB, GRADE, WL, DEPTH), GRADE + 1.0)
    _approx(cm.cut_model_a_face(HB + R_WL, True, HB, GRADE, WL, DEPTH), WL)


def test_no_bench_means_no_cut_above_wl():
    assert math.isinf(cm.cut_model_a_face(HB + R_WL + 5.0, False, HB, GRADE, WL, DEPTH))


def test_bench_is_berm_then_1p5h1v():
    _approx(cm.cut_model_a_face(HB + R_WL + 1.0, True, HB, GRADE, WL, DEPTH), WL)
    _approx(cm.cut_model_a_face(HB + R_WL + cm.BERM_M, True, HB, GRADE, WL, DEPTH), WL)
    _approx(cm.cut_model_a_face(HB + R_WL + cm.BERM_M + 1.5, True, HB, GRADE, WL, DEPTH),
            WL + 1.0)


def test_section_area_of_a_flat_bed_is_the_trapezoid():
    """Ground flat at WL - 0.5 everywhere, all wet: the dredge area is exactly the wet
    trapezoid between the bed and the ground (no bench: ground never above WL)."""
    xs = list(np.arange(-60.0, 60.0 + 0.5, 0.5))
    sw = [(-1 if o < 0 else 1) for o in xs]
    zg = WL - 0.5
    zs = [zg] * len(xs)
    dr, ex, fh = cm.section_area(zs, xs, sw, HB, GRADE, WL, DEPTH, "a", [True] * len(xs), dx=0.5)
    h = zg - GRADE                                   # cut depth on the flat bottom
    want = 2 * HB * h + cm.WET_SLOPE_H_PER_V * h * h  # bottom + two triangles
    assert ex == 0.0 and fh == 0.0
    assert abs(dr - want) / want < 0.01, (dr, want)


# --- feasibility: the gate ---------------------------------------------------------------

def test_prism_gate_triggers_bench_only_above_2m():
    o_wl = HB + R_WL
    xs, sidewhich = _xs()

    def profile(intrude_m):
        wall_start = o_wl - intrude_m
        return [WL + 8.0 if (o > 0 and abs(o) >= wall_start) else (WL - 5.0) for o in xs]

    _d1, e1, _fh1 = cm.section_area(profile(1.0), xs, sidewhich, HB, GRADE, WL, DEPTH, "a",
                                    [False] * len(xs))
    _d5, e5, fh5 = cm.section_area(profile(5.0), xs, sidewhich, HB, GRADE, WL, DEPTH, "a",
                                   [False] * len(xs))
    assert e5 > e1, f"benched section must cut more than the ungated one ({e5} !> {e1})"
    assert fh5 > 0.0


def test_unknown_model_is_refused():
    xs, sw = _xs()
    with pytest.raises(ValueError):
        cm.section_area([WL] * len(xs), xs, sw, HB, GRADE, WL, DEPTH, "v21", [False] * len(xs))


def test_model_ids_normalise():
    assert cm.normalise_model("A") == cm.CUT_MODEL
    assert cm.normalise_model("b") == cm.CUT_MODEL_B
    assert cm.normalise_model("cut_model_b") == cm.CUT_MODEL_B


# --- Model B ------------------------------------------------------------------------------

def _knoll_profile():
    """Ground intrudes the prism on the + side (so it benches), daylights, then a detached
    knoll 60-80 m out stands above the rising face."""
    xs = list(np.arange(-120.0, 120.0 + 0.5, 1.0))
    o_wl = HB + R_WL
    zs = []
    for o in xs:
        if o > 0 and o_wl - 4 <= o <= o_wl + 6:
            zs.append(WL + 3.0)              # bank intruding the prism
        elif o > 0 and 60 <= o <= 80:
            zs.append(WL + 60.0)             # detached knoll
        else:
            zs.append(WL - 4.0)
    return xs, [(-1 if o < 0 else 1) for o in xs], zs


def test_model_b_drops_the_detached_knoll():
    xs, sw, zs = _knoll_profile()
    wet = [False] * len(xs)
    _da, ea, fha = cm.section_area(zs, xs, sw, HB, GRADE, WL, DEPTH, "a", wet)
    _db, eb, fhb = cm.section_area(zs, xs, sw, HB, GRADE, WL, DEPTH, "b", wet)
    assert eb < ea, "Model B must not bill the knoll across the air gap"
    assert fhb < fha, "the tallest face must come down when the knoll is dropped"


def test_model_b_never_changes_below_wl():
    """All ground below WL -> A and B agree exactly."""
    xs, sw = _xs()
    zs = [WL - 1.0] * len(xs)
    wet = [True] * len(xs)
    a = cm.section_area(zs, xs, sw, HB, GRADE, WL, DEPTH, "a", wet)
    b = cm.section_area(zs, xs, sw, HB, GRADE, WL, DEPTH, "b", wet)
    assert a == b


# --- parity: scalar vs vectorised twin ----------------------------------------------------

def _wall_section():
    offs = np.arange(-60.0, 60.0 + 1e-6, 1.0)
    wall_start = HB + R_WL - 5.0
    gz = np.array([WL + 10.0 if abs(o) >= wall_start else WL - 5.0 for o in offs])
    return offs, gz


def test_vectorised_template_matches_scalar_face():
    offs, gz = _wall_section()
    gz2 = gz[None, :]
    tmpl0 = GRADE + np.maximum(0.0, np.abs(offs) - HB) / cm.WET_SLOPE_H_PER_V
    got = cg.cut_model_a_template(tmpl0[None, :], gz2, np.isfinite(gz2), offs, HB, GRADE, WL)[0]
    for j, o in enumerate(offs):
        if abs(o) <= HB:
            continue
        want = cm.cut_model_a_face(abs(o), True, HB, GRADE, WL, DEPTH)
        if math.isinf(want):
            assert math.isinf(got[j])
        else:
            _approx(float(got[j]), want)


def test_vectorised_template_accepts_per_section_hb():
    offs, gz = _wall_section()
    gz2 = np.vstack([gz, gz])
    tmpl = np.vstack([GRADE + np.maximum(0.0, np.abs(offs) - HB) / 3.0] * 2)
    a = cg.cut_model_a_template(tmpl, gz2, np.isfinite(gz2), offs, HB, GRADE, WL)
    b = cg.cut_model_a_template(tmpl, gz2, np.isfinite(gz2), offs, np.array([HB, HB]), GRADE, WL)
    assert np.array_equal(a, b)
    with pytest.raises(ValueError):
        cg.cut_model_a_template(tmpl, gz2, np.isfinite(gz2), offs, np.array([HB] * 3), GRADE, WL)


def test_vectorised_model_b_matches_scalar_limit():
    xs, sw, zs = _knoll_profile()
    offs = np.array(xs)
    gz2 = np.array(zs)[None, :]
    tmpl = (GRADE + np.maximum(0.0, np.abs(offs) - HB) / 3.0)[None, :]
    got = cg.cut_model_b_template(tmpl, gz2, np.isfinite(gz2), offs, HB, GRADE, WL)[0]
    want = cm.cut_model_a_section_face(xs, zs, HB, GRADE, WL, DEPTH, model="b")
    # Compare the CUT, not the face: at the exact run start the ground is already below the
    # face (that is what "daylit" means), so the twins may disagree on whether to draw the
    # face there while agreeing that nothing is cut.
    for g, w, o, z in zip(got, want, xs, zs):
        if abs(o) <= HB:
            continue
        cut_grid = max(0.0, z - g) if np.isfinite(g) else 0.0
        cut_scalar = max(0.0, z - w) if w is not None else 0.0
        assert abs(cut_grid - cut_scalar) < 0.01, (o, cut_grid, cut_scalar)
    # and the knoll is out of the works in both
    assert all(not np.isfinite(g) for g, o in zip(got, xs) if 60 <= o <= 80)


def test_widened_hb_adds_half_the_bend_widening():
    """A circle of radius 400 m sampled every 25 m: hb = base + 0.5 * c_c * L^2 / R."""
    R, loa, beam = 400.0, 80.0, 10.0
    t = np.arange(0, 2 * math.pi, 25.0 / R)
    xy = np.column_stack([R * np.cos(t), R * np.sin(t)])
    hb = cm.widened_hb(xy, loa, beam, radius_multiple=3.0, width_mult=3.2, c_c=0.5)
    want = 3.2 * beam / 2 + 0.5 * 0.5 * loa * loa / R
    assert np.allclose(hb[5:-5], want, rtol=1e-3)
