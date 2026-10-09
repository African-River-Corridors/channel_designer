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
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import List, Optional, Sequence, Tuple

import numpy as np
from shapely.geometry import GeometryCollection, LineString, MultiLineString, MultiPoint, Point
from shapely.ops import linemerge


@dataclass
class RollingCircleResult:
    centreline: LineString
    min_radius_m: float
    circles: List[dict] = field(default_factory=list)   # one per arc spliced in

    @property
    def n_arcs(self) -> int:
        return len(self.circles)


def primary_line(geom) -> LineString:
    """The single LineString to straighten (longest part of a multi-part line)."""
    if isinstance(geom, LineString):
        return geom
    if isinstance(geom, MultiLineString):
        merged = linemerge(geom)
        if isinstance(merged, LineString):
            return merged
        return max(merged.geoms, key=lambda part: part.length)
    raise ValueError("Centreline geometry must be a LineString or convertible to one.")


def resample_coordinate_sequence(points: np.ndarray, step: float) -> np.ndarray:
    """Resample the given coordinate array at approximately ``step`` metre spacing."""
    if points.shape[0] < 2:
        raise ValueError("Centreline must contain at least two vertices.")
    if step <= 0.0:
        raise ValueError("Resampling step must be positive.")

    seg_lengths = np.linalg.norm(np.diff(points, axis=0), axis=1)
    cumulative = np.concatenate(([0.0], np.cumsum(seg_lengths)))
    total_length = float(cumulative[-1])
    if total_length <= 0.0:
        raise ValueError("Centreline has zero length; cannot resample.")

    n_samples = max(int(math.ceil(total_length / step)) + 1, 3)
    targets = np.linspace(0.0, total_length, n_samples, endpoint=True)
    x = np.interp(targets, cumulative, points[:, 0])
    y = np.interp(targets, cumulative, points[:, 1])
    return np.column_stack((x, y))


def resample_linestring(line: LineString, step: float) -> np.ndarray:
    return resample_coordinate_sequence(np.asarray(line.coords, dtype=float), step)


def cumulative_chainages(points: np.ndarray) -> np.ndarray:
    if points.shape[0] == 0:
        return np.array([], dtype=float)
    if points.shape[0] == 1:
        return np.array([0.0], dtype=float)
    increments = np.linalg.norm(np.diff(points, axis=0), axis=1)
    return np.concatenate(([0.0], np.cumsum(increments)))


def compute_unit_tangent(points: np.ndarray, index: int) -> np.ndarray:
    if points.shape[0] < 2:
        return np.array([0.0, 0.0], dtype=float)
    if index <= 0:
        direction = points[1] - points[0]
    elif index >= points.shape[0] - 1:
        direction = points[-1] - points[-2]
    else:
        direction = points[index + 1] - points[index - 1]
    norm = np.linalg.norm(direction)
    if norm == 0.0:
        return np.array([0.0, 0.0], dtype=float)
    return direction / norm


def place_circle_at_point(points: np.ndarray, index: int, radius: float,
                          side: str) -> Tuple[np.ndarray, float]:
    """Centre and tangent angle of a circle tangent to the line at ``points[index]``.

    ``side`` is ``"left"`` or ``"right"`` relative to the direction of increasing chainage.
    """
    if not (0 <= index < points.shape[0]):
        raise IndexError(f"Point index {index} out of range for {points.shape[0]} samples.")
    if side not in {"left", "right"}:
        raise ValueError(f"Side must be 'left' or 'right'; received {side!r}.")
    if radius <= 0.0:
        raise ValueError("Radius must be positive.")

    tangent = compute_unit_tangent(points, index)
    if np.allclose(tangent, 0.0):
        raise ValueError(f"Cannot compute tangent at index {index}; insufficient variation.")

    normal = np.array([-tangent[1], tangent[0]], dtype=float)
    if side == "right":
        normal *= -1.0
    centre = points[index] + normal * radius
    return centre, math.atan2(tangent[1], tangent[0])


def create_arc_between_points(centre: np.ndarray, radius: float, start_point: np.ndarray,
                              end_point: np.ndarray, side: str,
                              num_segments: int = 32) -> np.ndarray:
    """Points along the circle arc from ``start_point`` to ``end_point``, turning to ``side``."""
    v_start = start_point - centre
    v_end = end_point - centre
    angle_start = math.atan2(v_start[1], v_start[0])
    angle_end = math.atan2(v_end[1], v_end[0])

    delta = angle_end - angle_start
    delta = (delta + math.pi) % (2 * math.pi) - math.pi
    if side == "left" and delta <= 0:
        delta += 2 * math.pi
    elif side == "right" and delta >= 0:
        delta -= 2 * math.pi

    angles = np.linspace(angle_start, angle_start + delta, num_segments + 2)
    return np.column_stack((centre[0] + radius * np.cos(angles), centre[1] + radius * np.sin(angles)))


def interpolate_point_on_line(coords: np.ndarray, chainages: np.ndarray, distance: float) -> np.ndarray:
    """The coordinate lying ``distance`` metres along the polyline ``coords``."""
    if coords.shape[0] == 0:
        raise ValueError("Cannot interpolate point on an empty coordinate sequence.")
    if distance <= 0.0:
        return coords[0].copy()
    if distance >= float(chainages[-1]):
        return coords[-1].copy()

    idx = int(np.searchsorted(chainages, distance, side="right"))
    if idx == 0:
        return coords[0].copy()
    prev_idx = idx - 1
    prev_chainage = float(chainages[prev_idx])
    next_chainage = float(chainages[idx])
    if math.isclose(next_chainage, prev_chainage):
        return coords[idx].copy()
    ratio = (distance - prev_chainage) / (next_chainage - prev_chainage)
    return coords[prev_idx] + ratio * (coords[idx] - coords[prev_idx])


def replace_segment_with_arc(line: LineString, start_distance: float, end_distance: float,
                             arc_points: np.ndarray) -> LineString:
    """Splice ``arc_points`` into ``line`` between ``start_distance`` and ``end_distance``."""
    if arc_points.shape[0] < 2:
        return line
    coords = np.asarray(line.coords, dtype=float)
    if coords.shape[0] < 2:
        return line

    chainages = cumulative_chainages(coords)
    total_length = float(chainages[-1])
    tol = 1e-9

    start_distance = float(np.clip(start_distance, 0.0, total_length))
    end_distance = float(np.clip(end_distance, 0.0, total_length))
    if end_distance - start_distance <= tol:
        return line

    start_point = interpolate_point_on_line(coords, chainages, start_distance)
    end_point = interpolate_point_on_line(coords, chainages, end_distance)

    prefix: List[np.ndarray] = []
    for idx, ch in enumerate(chainages):
        if ch < start_distance - tol:
            prefix.append(coords[idx])
        else:
            break
    if not prefix:
        prefix.append(coords[0])
    if not np.allclose(prefix[-1], start_point, atol=tol):
        prefix.append(start_point)

    suffix: List[np.ndarray] = []
    for idx in range(chainages.shape[0] - 1, -1, -1):
        if chainages[idx] > end_distance + tol:
            suffix.insert(0, coords[idx])
        else:
            break
    if not suffix:
        suffix.append(coords[-1])
    if not np.allclose(suffix[0], end_point, atol=tol):
        suffix.insert(0, end_point)

    arc_coords = np.asarray(arc_points, dtype=float)
    if not np.allclose(arc_coords[0], start_point, atol=tol):
        arc_coords = np.vstack([start_point, arc_coords])
    if not np.allclose(arc_coords[-1], end_point, atol=tol):
        arc_coords = np.vstack([arc_coords, end_point])

    combined: List[np.ndarray] = []
    for seq in (prefix, arc_coords, suffix):
        for point in seq:
            if not combined or not np.allclose(combined[-1], point, atol=tol):
                combined.append(np.asarray(point, dtype=float))
    if len(combined) < 2:
        return line
    return LineString(np.asarray(combined))


def _gather_points(geom) -> List[Point]:
    if geom.is_empty:
        return []
    if isinstance(geom, Point):
        return [geom]
    if isinstance(geom, (MultiPoint, GeometryCollection)):
        pts: List[Point] = []
        for sub in geom.geoms:
            pts.extend(_gather_points(sub))
        return pts
    if isinstance(geom, LineString):
        coords = list(geom.coords)
        if not coords:
            return []
        if len(coords) == 1:
            return [Point(coords[0])]
        return [Point(coords[0]), Point(coords[-1])]
    if hasattr(geom, "geoms"):
        pts = []
        for sub in geom.geoms:
            pts.extend(_gather_points(sub))
        return pts
    return [geom.representative_point()]


def find_forward_circle_contact(centreline: LineString, tangent_point: np.ndarray,
                                tangent_chainage: float, centre: np.ndarray, radius: float,
                                min_chainage_gap: float
                                ) -> Optional[Tuple[np.ndarray, np.ndarray, np.ndarray, float]]:
    """Furthest forward touch between the circle and the centreline, or None.

    Returns ``(contact_point, centre, tangent_point, contact_chainage)``.
    """
    if radius <= 0.0:
        raise ValueError("Radius must be positive.")
    if min_chainage_gap <= 0.0:
        raise ValueError("min_chainage_gap must be positive.")

    circle = Point(float(centre[0]), float(centre[1])).buffer(radius, quad_segs=128)
    intersection = circle.boundary.intersection(centreline)
    if intersection.is_empty:
        return None

    candidates: List[Tuple[float, Point]] = []
    for pt in _gather_points(intersection):
        chainage = float(centreline.project(pt))
        if chainage - tangent_chainage > min_chainage_gap:
            candidates.append((chainage, pt))
    if not candidates:
        return None

    contact_chainage, contact_geom = max(candidates, key=lambda item: item[0])
    contact_point = np.array([contact_geom.x, contact_geom.y], dtype=float)
    return contact_point, np.array(centre, dtype=float), tangent_point, contact_chainage


def rolling_circle(centreline, min_radius: float, step: float, passes: int = 2,
                   sides: Sequence[str] = ("left", "right"),
                   min_chainage_gap: Optional[float] = None) -> RollingCircleResult:
    """Straighten ``centreline`` so a circle of ``min_radius`` can roll along both sides.

    ``min_radius`` is usually ``radius_multiple × LOA`` from the rules layer. ``step`` is the
    sampling interval (m) of the sweep; smaller is slower and finer.
    """
    if min_radius <= 0:
        raise ValueError("min_radius must be positive.")
    if step <= 0:
        raise ValueError("step must be positive.")
    line = LineString(primary_line(centreline))
    gap = max(step, min_radius * 0.25) if min_chainage_gap is None else float(min_chainage_gap)
    circles: List[dict] = []

    def resampled():
        pts = resample_linestring(line, step)
        return pts, cumulative_chainages(pts)

    def start_index(ch: np.ndarray) -> int:
        idx = max(1, int(np.searchsorted(ch, step)))
        if idx >= ch.shape[0] - 1:
            idx = max(1, ch.shape[0] - 2)
        return idx

    for pass_index in range(passes):
        for side in sides:
            samples, chainages = resampled()
            idx = start_index(chainages)
            while idx < samples.shape[0] - 1:
                try:
                    centre, _ = place_circle_at_point(samples, idx, min_radius, side)
                except (ValueError, IndexError):
                    idx += 1
                    continue
                tangent_point = samples[idx]
                tangent_distance = float(line.project(Point(float(tangent_point[0]),
                                                             float(tangent_point[1]))))
                contact = find_forward_circle_contact(line, tangent_point, tangent_distance,
                                                      centre, min_radius, min_chainage_gap=gap)
                if contact is None:
                    idx += 1
                    continue
                contact_point, centre, tangent_point, contact_chainage = contact
                circles.append({
                    "pass": pass_index + 1, "side": side,
                    "centre": (float(centre[0]), float(centre[1])),
                    "radius_m": float(min_radius),
                    "tangent_point": (float(tangent_point[0]), float(tangent_point[1])),
                    "contact_point": (float(contact_point[0]), float(contact_point[1])),
                    "start_chainage_m": tangent_distance,
                    "end_chainage_m": float(contact_chainage),
                })
                arc = create_arc_between_points(centre, min_radius, tangent_point,
                                                contact_point, side)
                line = replace_segment_with_arc(line, tangent_distance, contact_chainage, arc)
                samples, chainages = resampled()
                updated = float(line.project(Point(contact_point[0], contact_point[1])))
                idx = int(np.searchsorted(chainages, updated, side="left"))
                idx = min(idx + 1, samples.shape[0] - 1)
    return RollingCircleResult(centreline=line, min_radius_m=float(min_radius), circles=circles)
