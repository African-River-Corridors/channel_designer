"""River width measured perpendicular to the centreline, bank to bank.

At each station a cross-line is cast along the local normal and intersected with the left and
right bank lines. Pure geometry in a projected CRS (metres). Narrow-span polygons and KMZ
export from the original pipeline are not included.
"""
from __future__ import annotations

import math
from typing import Iterator, List, Optional, Tuple

import numpy as np
from shapely.geometry import LineString, Point


def resample_distances(total_length: float, spacing: float) -> np.ndarray:
    if spacing <= 0.0:
        raise ValueError("Sampling spacing must be positive.")
    steps = max(int(math.ceil(total_length / spacing)) + 1, 2)
    distances = np.linspace(0.0, total_length, steps, endpoint=True)
    distances[-1] = total_length
    return distances


def measure_widths(
    centreline: LineString,
    left_bank: LineString,
    right_bank: LineString,
    spacing: float,
    cross_span: float,
) -> Tuple[List[dict], int]:
    """Bank-to-bank width every ``spacing`` metres.

    ``cross_span`` is the half-length of each cross-line (m) — make it wider than half the
    widest river reach. Returns ``(records, failures)``; each record has ``chainage_m``,
    ``width_m``, ``geometry`` (the bank-to-bank segment) and the two bank points.
    """
    if cross_span <= 0.0:
        raise ValueError("Cross-span must be positive.")

    total_length = centreline.length
    distances = resample_distances(total_length, spacing)
    results: List[dict] = []
    failures = 0

    for idx, chainage in enumerate(distances):
        point = centreline.interpolate(chainage)
        tangent = _estimate_tangent(centreline, chainage, spacing)
        if tangent is None:
            failures += 1
            continue
        nx, ny = _normal_from_tangent(*tangent)
        offset_x, offset_y = nx * cross_span, ny * cross_span
        left_endpoint = Point(point.x + offset_x, point.y + offset_y)
        right_endpoint = Point(point.x - offset_x, point.y - offset_y)
        cross_line = LineString([left_endpoint, right_endpoint])

        left_pt = _nearest_intersection(cross_line, left_bank, point)
        right_pt = _nearest_intersection(cross_line, right_bank, point)

        if left_pt is None or right_pt is None:
            failures += 1
            continue

        width = left_pt.distance(right_pt)
        if width <= 0.0:
            failures += 1
            continue

        results.append(
            {
                "chainage_m": float(chainage),
                "width_m": float(width),
                "geometry": LineString([left_pt, right_pt]),
                "sample_index": idx,
                "left_x": left_pt.x,
                "left_y": left_pt.y,
                "right_x": right_pt.x,
                "right_y": right_pt.y,
            }
        )

    return results, failures


def _estimate_tangent(line: LineString, chainage: float, step: float) -> Optional[Tuple[float, float]]:
    ahead_point = line.interpolate(min(chainage + step, line.length))
    behind_point = line.interpolate(max(chainage - step, 0.0))
    dx = ahead_point.x - behind_point.x
    dy = ahead_point.y - behind_point.y
    norm = math.hypot(dx, dy)
    if norm == 0.0:
        return None
    return dx / norm, dy / norm


def _normal_from_tangent(tx: float, ty: float) -> Tuple[float, float]:
    nx, ny = -ty, tx
    norm = math.hypot(nx, ny)
    if norm == 0.0:
        return 0.0, 0.0
    return nx / norm, ny / norm


def _nearest_intersection(cross_line: LineString, bank_geom: LineString, origin: Point) -> Optional[Point]:
    points = list(_iter_points(cross_line.intersection(bank_geom)))
    if not points:
        return None
    return min(points, key=lambda pt: pt.distance(origin))


def _iter_points(geom) -> Iterator[Point]:
    if geom is None or geom.is_empty:
        return
    if isinstance(geom, Point):
        yield geom
    elif isinstance(geom, LineString):
        coords = list(geom.coords)
        if coords:
            yield Point(coords[0])
            yield Point(coords[-1])
    elif hasattr(geom, "geoms"):
        for sub_geom in geom.geoms:
            yield from _iter_points(sub_geom)
