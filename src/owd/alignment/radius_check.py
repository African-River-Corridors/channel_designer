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

from .. import _rust

MAX_STRIDE_M = 10.0
RADIUS_TOL = 0.02


def default_check_chord(r_min: float) -> float:
    """One vessel length at the package's default R_min = 3 × LOA (PIANC)."""
    return _rust.call("radius_check", "default_check_chord", float, r_min=r_min)


def min_radius_on_chord(line, chord_m: float) -> float:
    """Minimum three-point radius (m) of ``line`` read on a chord of exactly ``chord_m``.

    Returns ``inf`` for a straight line, or a line shorter than the chord (no reading).
    """
    return _rust.call("radius_check", "min_radius_on_chord", float, line=line, chord_m=chord_m)


def min_radius_on_vertices(pts, k: int = 1) -> float:
    """Minimum three-point radius (m) on a polyline's own vertices, arms ``k`` vertices long."""
    return _rust.call("radius_check", "min_radius_on_vertices", float, pts=pts, k=int(k))


def radius_ok(measured_m: float, r_min: float) -> bool:
    """True when ``measured_m`` holds ``r_min`` within the reading tolerance (False on NaN)."""
    return _rust.call("radius_check", "radius_ok", bool, measured_m=measured_m, r_min=r_min)


_rust.core_constants(__name__, "MAX_STRIDE_M", "RADIUS_TOL")
