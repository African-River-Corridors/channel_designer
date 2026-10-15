"""Measured minimum radius of a design line, read on a stated chord.

Every step that straightens a line to R_min reports what it achieved, not what it aimed for:
``radius_ok``, the measured minimum radius, and the chord it was read on. This module holds
the two readings they use.

ON A DENSE LINE (``min_radius_on_chord``, used by the rolling circle). Resample the line at an
exact stride ``h`` of at most ``MAX_STRIDE_M`` that divides half the chord into ``k`` steps,
once forward from the start and once backward from the end, so both ends are read and every
reading is on the exact chord. At every sample ``i`` take the three-point circumradius of
``(p[i-k], p[i], p[i+k])``: points half a chord apart along the line. The minimum over both
passes is the measured minimum radius.

ON A POLYLINE'S OWN VERTICES (``min_radius_on_vertices``, used by the route). The route line
has a vertex per station; those vertices are the design. Take the three-point circumradius of
``(v[i-k], v[i], v[i+k])`` at every vertex. The chord is ``2 × k × vertex spacing``. Reading
between vertices instead would read the straight segments' corners: a polyline with every
vertex on an R circle reads about ``R·cos(ds / 2R)`` on a resample, 3-6 % low at typical
spacings, and would fail a line that holds R exactly.

WHY A CHORD. The three-point radius depends on how far apart the points are. On a short chord
the rolling-circle exit kinks read as much tighter radii than a vessel sees
(``docs/known-issues.md``). Read on a chord of about one vessel length.

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


def _three_point_min(pts: np.ndarray, k: int) -> float:
    """Minimum circumradius of ``(pts[i-k], pts[i], pts[i+k])`` over all ``i``; inf if none."""
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


def _resample_exact(coords: np.ndarray, h: float) -> np.ndarray:
    """Points every exactly ``h`` metres from the start; the remainder (< h) is dropped."""
    seg = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cum = np.concatenate(([0.0], np.cumsum(seg)))
    t = np.arange(0.0, float(cum[-1]) + 1e-9, h)
    return np.column_stack((np.interp(t, cum, coords[:, 0]), np.interp(t, cum, coords[:, 1])))


def min_radius_on_chord(line, chord_m: float) -> float:
    """Minimum three-point radius (m) of ``line`` read on a chord of exactly ``chord_m``.

    Returns ``inf`` for a straight line, or a line shorter than the chord (no reading).
    """
    if not (math.isfinite(chord_m) and chord_m > 0.0):
        raise ValueError("chord_m must be finite and positive.")
    coords = np.asarray(line.coords, dtype=float)
    half = chord_m / 2.0
    k = max(1, int(math.ceil(half / MAX_STRIDE_M)))
    h = half / k
    forward = _three_point_min(_resample_exact(coords, h), k)
    backward = _three_point_min(_resample_exact(coords[::-1], h), k)
    return min(forward, backward)


def min_radius_on_vertices(pts, k: int = 1) -> float:
    """Minimum three-point radius (m) on a polyline's own vertices, arms ``k`` vertices long."""
    if k < 1:
        raise ValueError("k must be at least 1.")
    return _three_point_min(np.asarray(pts, dtype=float), int(k))


def radius_ok(measured_m: float, r_min: float) -> bool:
    """True when ``measured_m`` holds ``r_min`` within the reading tolerance (False on NaN)."""
    return bool(measured_m >= (1.0 - RADIUS_TOL) * r_min)
