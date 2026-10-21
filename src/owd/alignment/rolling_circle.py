"""Rolling-circle meander straightening: make a centreline pass a minimum bend radius.

The algorithm, per side and per pass:

1. Resample the (current) centreline every ``step`` metres.
2. At each sample, place a circle of radius ``min_radius`` tangent to the line on that side.
3. Find the furthest forward point where the circle touches the line again, more than
   ``min_chainage_gap`` along it (default: the larger of ``step`` and 0.25 × ``min_radius``).
4. If there is one, replace the line between the tangent point and that touch with the
   circle arc, and resume from the touch.

The original pipeline runs two passes, each covering the left then the right side.

Pure geometry in a projected CRS (metres). Each spliced circle is returned as a record.

KNOWN_ISSUES
------------
Rejoin kinks. The arc leaves the old line where the circle CROSSES it, not where it is
tangent, so each arc exit carries a short kink. On a vessel-length chord the spliced line
holds R_min; on a short chord the kink reads as a much tighter radius. Measured on
``synthetic_river()`` at R_min 300 m, 10 m step: minimum radius 111 m on a 10 m chord, 288 m
on 25 m, 297 m on 50 m (25 m step: 55 / 165 / 296 m). Read curvature on a chord of about one
vessel length, or smooth the exits first. Kept as is on purpose; a test pins it. See
``docs/known-issues.md``.

Unresolved bends. The circle does not always lift the line to R_min. On a jagged input (a
trace digitised at coarse vertices, with zigzags) more passes do not converge. The result
says so: ``radius_ok``, ``measured_min_radius_m`` and the chord it was read on
(:mod:`owd.alignment.radius_check`). Check ``radius_ok`` before you use the line.
"""
from __future__ import annotations

from .. import _rust

import math
from dataclasses import dataclass, field
from typing import List, Optional, Sequence, Tuple

import numpy as np
from shapely.geometry import LineString

from owd.alignment.radius_check import radius_ok


@dataclass
class RollingCircleResult:
    centreline: LineString
    min_radius_m: float                  # the radius REQUESTED (the circle's), not a measurement
    circles: List[dict] = field(default_factory=list)   # one per arc spliced in
    measured_min_radius_m: float = math.nan   # minimum radius of ``centreline`` on the chord
    radius_check_chord_m: float = math.nan    # the chord it was read on, m
    radius_ok: bool = False                   # measured holds ``min_radius_m`` (2 % reading tol)

    @property
    def n_arcs(self) -> int:
        return len(self.circles)


def primary_line(geom) -> LineString:
    """The single LineString to straighten (longest part of a multi-part line)."""
    return _rust.call("rolling_circle", "primary_line", LineString, geom=geom)


def resample_coordinate_sequence(points: np.ndarray, step: float) -> np.ndarray:
    """Resample the given coordinate array at approximately ``step`` metre spacing."""
    return _rust.call("rolling_circle", "resample_coordinate_sequence", np.ndarray, points=points, step=step)


def resample_linestring(line: LineString, step: float) -> np.ndarray:
    return _rust.call("rolling_circle", "resample_linestring", np.ndarray, line=line, step=step)


def cumulative_chainages(points: np.ndarray) -> np.ndarray:
    return _rust.call("rolling_circle", "cumulative_chainages", np.ndarray, points=points)


def compute_unit_tangent(points: np.ndarray, index: int) -> np.ndarray:
    return _rust.call("rolling_circle", "compute_unit_tangent", np.ndarray, points=points, index=int(index))


def place_circle_at_point(points: np.ndarray, index: int, radius: float,
                          side: str) -> Tuple[np.ndarray, float]:
    """Centre and tangent angle of a circle tangent to the line at ``points[index]``.

    ``side`` is ``"left"`` or ``"right"`` relative to the direction of increasing chainage.
    """
    r = _rust.call("rolling_circle", "place_circle_at_point", points=points, index=int(index), radius=radius, side=side)
    return np.asarray(r["centre"], dtype=float), float(r["tangent_angle"])


def create_arc_between_points(centre: np.ndarray, radius: float, start_point: np.ndarray,
                              end_point: np.ndarray, side: str,
                              num_segments: int = 32) -> np.ndarray:
    """Points along the circle arc from ``start_point`` to ``end_point``, turning to ``side``."""
    return _rust.call("rolling_circle", "create_arc_between_points", np.ndarray, centre=centre, radius=radius,
                      start_point=start_point, end_point=end_point, side=side, num_segments=int(num_segments))


def interpolate_point_on_line(coords: np.ndarray, chainages: np.ndarray, distance: float) -> np.ndarray:
    """The coordinate lying ``distance`` metres along the polyline ``coords``."""
    return _rust.call("rolling_circle", "interpolate_point_on_line", np.ndarray, coords=coords,
                      chainages=chainages, distance=distance)


def replace_segment_with_arc(line: LineString, start_distance: float, end_distance: float,
                             arc_points: np.ndarray) -> LineString:
    """Splice ``arc_points`` into ``line`` between ``start_distance`` and ``end_distance``."""
    return _rust.call("rolling_circle", "replace_segment_with_arc", LineString, line=line,
                      start_distance=start_distance, end_distance=end_distance, arc_points=arc_points)


def find_forward_circle_contact(centreline: LineString, tangent_point: np.ndarray,
                                tangent_chainage: float, centre: np.ndarray, radius: float,
                                min_chainage_gap: float
                                ) -> Optional[Tuple[np.ndarray, np.ndarray, np.ndarray, float]]:
    """Furthest forward touch between the circle and the centreline, or None.

    Returns ``(contact_point, centre, tangent_point, contact_chainage)``.
    """
    r = _rust.call("rolling_circle", "find_forward_circle_contact", centreline=centreline,
                   tangent_point=tangent_point, tangent_chainage=tangent_chainage, centre=centre,
                   radius=radius, min_chainage_gap=min_chainage_gap)
    if r is None:
        return None
    return (np.asarray(r["contact_point"], dtype=float), np.asarray(r["centre"], dtype=float),
            np.asarray(r["tangent_point"], dtype=float), float(r["contact_chainage"]))


def rolling_circle(centreline, min_radius: float, step: float, passes: int = 2,
                   sides: Sequence[str] = ("left", "right"),
                   min_chainage_gap: Optional[float] = None,
                   check_chord_m: Optional[float] = None) -> RollingCircleResult:
    """Straighten ``centreline`` so a circle of ``min_radius`` can roll along both sides.

    ``min_radius`` is usually ``radius_multiple × LOA`` from the rules layer. ``step`` is the
    sampling interval (m) of the sweep; smaller is slower and finer.

    The result is measured, not assumed: ``radius_ok`` is False when the returned line is
    tighter than ``min_radius`` on a chord of ``check_chord_m`` (default ``min_radius / 3``,
    one LOA at R_min = 3 × LOA). Pass the vessel length to read it on one LOA.

    Expects a smooth input line. On a jagged trace (coarse vertices, zigzags) the circle does
    not resolve the zigzags and ``radius_ok`` comes back False: smooth or resample it first.
    """
    r = _rust.call("rolling_circle", "rolling_circle", centreline=centreline, min_radius=min_radius, step=step,
                   passes=int(passes), sides=list(sides), min_chainage_gap=min_chainage_gap,
                   check_chord_m=check_chord_m)
    return _rust.decode(r["result"], RollingCircleResult)
