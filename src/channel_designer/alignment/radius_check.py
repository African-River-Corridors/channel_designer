"""Measured minimum radius of a design line, read on a stated chord.

Every step that straightens a line to R_min reports what it achieved, not what it aimed for:
``radius_ok``, the measured minimum radius, and the chord it was read on. This module is the
one measurement they share.

THE MEASUREMENT. Resample the line at a stride of at most ``MAX_STRIDE_M`` that divides half
the chord into ``k`` equal steps. At every sample ``i`` with ``k`` samples each side, take the
three-point circumradius of ``(p[i-k], p[i], p[i+k])``: points half a chord apart along the
line. The minimum over all samples is the measured minimum radius.

WHY A CHORD. The three-point radius depends on how far apart the points are. On a short chord
the rolling-circle exit kinks and polyline corners read as much tighter radii than a vessel
sees (``docs/known-issues.md``). Read on a chord of about one vessel length. A polyline whose
vertices are ``ds`` apart only resolves its own curvature on a chord of at least ``2 × ds``.

``radius_ok`` allows ``RADIUS_TOL`` (2 %) below R_min for arc discretisation, resampling and
the rolling-circle exit kink (about 1 % on a 100 m chord at R_min 300 m). It is a reading
tolerance, not a design relaxation.
"""
from __future__ import annotations

import math

import numpy as np

MAX_STRIDE_M = 10.0
RADIUS_TOL = 0.02


def default_check_chord(r_min: float) -> float:
    """One vessel length at the package's default R_min = 3 × LOA (PIANC)."""
    return float(r_min) / 3.0


def _resample(coords: np.ndarray, spacing: float) -> np.ndarray:
    seg = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cum = np.concatenate(([0.0], np.cumsum(seg)))
    total = float(cum[-1])
    n = max(int(math.ceil(total / spacing)) + 1, 3)
    t = np.linspace(0.0, total, n)
    return np.column_stack((np.interp(t, cum, coords[:, 0]), np.interp(t, cum, coords[:, 1])))


def min_radius_on_chord(line, chord_m: float) -> float:
    """Minimum three-point radius (m) of ``line`` read on a chord of ``chord_m``.

    Returns ``inf`` for a straight line or a line shorter than the chord.
    """
    if chord_m <= 0.0:
        raise ValueError("chord_m must be positive.")
    coords = np.asarray(line.coords, dtype=float)
    half = chord_m / 2.0
    k = max(1, int(math.ceil(half / MAX_STRIDE_M)))
    pts = _resample(coords, half / k)
    if pts.shape[0] < 2 * k + 1:
        return math.inf
    A, B, C = pts[:-2 * k], pts[k:-k], pts[2 * k:]
    a = np.hypot(*(B - C).T)
    b = np.hypot(*(A - C).T)
    c = np.hypot(*(A - B).T)
    area = 0.5 * np.abs((B[:, 0] - A[:, 0]) * (C[:, 1] - A[:, 1])
                        - (C[:, 0] - A[:, 0]) * (B[:, 1] - A[:, 1]))
    with np.errstate(divide="ignore", invalid="ignore"):
        R = np.where(area > 1e-9, a * b * c / (4.0 * area), np.inf)
    return float(np.min(R))


def radius_ok(measured_m: float, r_min: float) -> bool:
    """True when ``measured_m`` holds ``r_min`` within the reading tolerance."""
    return bool(measured_m >= (1.0 - RADIUS_TOL) * r_min)
