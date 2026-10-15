"""Navigation channel limits: left/right offset lines with curvature-adaptive widening.

Pure geometry. All operations use projected coordinates in metres (any UTM zone or other
metric CRS); the caller owns reprojection. No file I/O.
"""

from __future__ import annotations

import math
from typing import Iterable, List, Optional, Tuple

import numpy as np
from shapely.geometry import LineString, MultiLineString
from shapely.ops import linemerge


def extract_lines(geom) -> List[LineString]:
    """Extract LineString geometries from any geometry type."""
    if geom is None or geom.is_empty:
        return []
    if isinstance(geom, LineString):
        return [geom]
    if isinstance(geom, MultiLineString):
        return [line for line in geom.geoms if not line.is_empty]
    if hasattr(geom, "geoms"):
        lines: List[LineString] = []
        for part in geom.geoms:
            lines.extend(extract_lines(part))
        return lines
    merged = linemerge(geom)
    if isinstance(merged, LineString):
        return [merged]
    if isinstance(merged, MultiLineString):
        return [line for line in merged.geoms if not line.is_empty]
    return []


def merge_lines(lines: List[LineString]) -> LineString | MultiLineString:
    """Merge a list of LineStrings into one (or MultiLineString if not contiguous)."""
    if not lines:
        raise ValueError("Cannot merge an empty collection of line geometries.")
    if len(lines) == 1:
        return lines[0]
    merged = linemerge(MultiLineString(lines))
    if isinstance(merged, (LineString, MultiLineString)):
        return merged
    return MultiLineString(lines)


def ensure_multilinestring(geom: LineString | MultiLineString) -> MultiLineString:
    """Ensure geometry is a MultiLineString."""
    if isinstance(geom, MultiLineString):
        return geom
    if isinstance(geom, LineString):
        return MultiLineString([geom])
    merged = linemerge(geom)
    if isinstance(merged, MultiLineString):
        return merged
    if isinstance(merged, LineString):
        return MultiLineString([merged])
    raise ValueError("Unexpected geometry type while building navigation channel limits.")


def resample_line(line: LineString, step: float) -> np.ndarray:
    """Resample a LineString at uniform spacing along its length."""
    if step <= 0.0:
        raise ValueError("Sample step must be positive.")
    coords = np.asarray(line.coords, dtype=float)
    if coords.shape[0] < 2:
        raise ValueError("Line must contain at least two vertices.")

    seg_lengths = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cumulative = np.concatenate(([0.0], np.cumsum(seg_lengths)))
    total_length = cumulative[-1]
    if total_length <= 0.0:
        raise ValueError("Line has zero length, cannot resample.")

    n_samples = max(int(math.ceil(total_length / step)) + 1, 3)
    targets = np.linspace(0.0, total_length, n_samples, endpoint=True)
    x = np.interp(targets, cumulative, coords[:, 0])
    y = np.interp(targets, cumulative, coords[:, 1])
    return np.column_stack((x, y))


def circumcircle(
    p0: np.ndarray, p1: np.ndarray, p2: np.ndarray
) -> Optional[Tuple[np.ndarray, float]]:
    """Compute circumscribed circle centre and radius from three points."""
    ax, ay = p0
    bx, by = p1
    cx, cy = p2
    d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
    if abs(d) < 1e-9:
        return None
    ax2_ay2 = ax * ax + ay * ay
    bx2_by2 = bx * bx + by * by
    cx2_cy2 = cx * cx + cy * cy
    ux = (ax2_ay2 * (by - cy) + bx2_by2 * (cy - ay) + cx2_cy2 * (ay - by)) / d
    uy = (ax2_ay2 * (cx - bx) + bx2_by2 * (ax - cx) + cx2_cy2 * (bx - ax)) / d
    center = np.array([ux, uy], dtype=float)
    radius = float(np.hypot(ux - ax, uy - ay))
    return center, radius


def compute_curvature_radius(points: np.ndarray) -> np.ndarray:
    """Compute radius of curvature at each point via circumscribed circles."""
    n = len(points)
    radii = np.full(n, np.inf, dtype=float)
    if n < 3:
        return radii

    for idx in range(1, n - 1):
        circle = circumcircle(points[idx - 1], points[idx], points[idx + 1])
        if circle is None:
            continue
        _, radius = circle
        if radius <= 0.0:
            continue
        radii[idx] = radius

    if n > 1:
        radii[0] = radii[1]
        radii[-1] = radii[-2]
    return radii


def compute_normals(points: np.ndarray) -> np.ndarray:
    """Compute unit normal vectors (perpendicular to tangent) at each point."""
    n = len(points)
    tangents = np.zeros_like(points)
    if n < 2:
        return tangents

    tangents[1:-1] = points[2:] - points[:-2]
    tangents[0] = points[1] - points[0]
    tangents[-1] = points[-1] - points[-2]

    norms = np.linalg.norm(tangents, axis=1, keepdims=True)
    mask = norms[:, 0] > 0.0
    tangents[mask] /= norms[mask]

    normals = np.zeros_like(points)
    normals[:, 0] = -tangents[:, 1]
    normals[:, 1] = tangents[:, 0]
    return normals


def compute_chainages(points: np.ndarray) -> np.ndarray:
    """Compute cumulative chainage (distance) along a point sequence."""
    diffs = np.diff(points, axis=0)
    seg_lengths = np.linalg.norm(diffs, axis=1)
    return np.concatenate(([0.0], np.cumsum(seg_lengths)))


def rolling_average_radius(
    radii: np.ndarray, chainages: np.ndarray, window: float
) -> np.ndarray:
    """Smooth curvature radii with a rolling window along the chainage."""
    averages = np.full_like(radii, np.inf)
    if len(radii) == 0:
        return averages
    for idx in range(len(radii)):
        start_distance = chainages[idx] - window
        end_distance = chainages[idx] + window
        start_idx = int(np.searchsorted(chainages, start_distance, side="left"))
        end_idx = int(np.searchsorted(chainages, end_distance, side="right"))
        window_radii = radii[start_idx:end_idx]
        valid = np.isfinite(window_radii) & (window_radii > 0.0)
        if np.any(valid):
            averages[idx] = float(window_radii[valid].mean())
    return averages


def compute_extra_width(
    radii: np.ndarray,
    chainages: np.ndarray,
    total_length: float,
    loa: float,
    extra_width_coefficient: float,
    min_radius_length_multiple: float,
    max_extra_width_radius: Optional[float],
    radius_threshold: float,
) -> np.ndarray:
    """Compute additional channel width at tight bends based on curvature."""
    extra = np.zeros_like(radii, dtype=float)
    if loa <= 0.0 or extra_width_coefficient <= 0.0:
        return extra

    inner_mask = (chainages >= 100.0) & (chainages <= max(total_length - 100.0, 0.0))
    mask = inner_mask & np.isfinite(radii) & (radii > 0.0) & (radii < radius_threshold)
    if not np.any(mask):
        return extra

    if (
        max_extra_width_radius
        and min_radius_length_multiple > 0.0
        and loa > 0.0
        and max_extra_width_radius > 0.0
    ):
        min_radius = loa * min_radius_length_multiple
        if min_radius <= 0.0:
            min_radius = 1.0
        upper_radius = max(max_extra_width_radius, min_radius + 1.0)
        band_edges = np.linspace(min_radius, upper_radius, num=5)
        band_limits = band_edges[1:]
        band_widths = [extra_width_coefficient * (loa ** 2) / edge for edge in band_edges[:-1]]
        band_widths.append(0.0)
        widths_array = np.asarray(band_widths, dtype=float)
        band_indices = np.searchsorted(band_limits, radii[mask], side="right")
        band_indices = np.clip(band_indices, 0, len(widths_array) - 1)
        extra[mask] = widths_array[band_indices]
    else:
        extra[mask] = extra_width_coefficient * (loa ** 2) / radii[mask]

    return extra


def build_variable_offset_line(
    line: LineString,
    base_offset: float,
    loa: float,
    extra_width_coefficient: float = 0.6,
    min_radius_length_multiple: float = 3.0,
    max_extra_width_radius: Optional[float] = 2000.0,
    radius_threshold: float = 2000.0,
    radius_sample_length: float = 10.0,
) -> Tuple[LineString, LineString, np.ndarray, np.ndarray]:
    """Build left/right offset lines with curvature-adaptive widening.

    Returns (left_line, right_line, smoothed_radii, extra_widths).
    """
    step = max(radius_sample_length, 0.5)
    points = resample_line(line, step)
    normals = compute_normals(points)
    radii = compute_curvature_radius(points)
    chainages = compute_chainages(points)
    smoothed_radii = rolling_average_radius(radii, chainages, window=50.0)
    extra = compute_extra_width(
        smoothed_radii,
        chainages,
        chainages[-1] if len(chainages) else 0.0,
        loa,
        extra_width_coefficient,
        min_radius_length_multiple,
        max_extra_width_radius,
        radius_threshold,
    )
    offsets = base_offset + (extra / 2.0)

    left_points = points + normals * offsets[:, None]
    right_points = points - normals * offsets[:, None]

    left_line = LineString(left_points)
    right_line = LineString(right_points)
    return left_line, right_line, smoothed_radii, extra


def build_channel_limits(
    centreline_geoms: Iterable[LineString],
    base_offset: float,
    loa: float,
    extra_width_coefficient: float = 0.6,
    min_radius_length_multiple: float = 3.0,
    max_extra_width_radius: Optional[float] = 2000.0,
    radius_threshold: float = 2000.0,
    radius_sample_length: float = 10.0,
) -> Tuple[MultiLineString, MultiLineString, dict]:
    """Compute navigation channel boundary lines from centreline geometries.

    Args:
        centreline_geoms: Centreline LineString(s) in projected coordinates.
        base_offset: Half-channel width at water level (m).
        loa: Vessel length overall (m), used for curvature-based widening.
        extra_width_coefficient: Multiplier for curvature-adaptive extra width.
        min_radius_length_multiple: LOA multiplier for minimum bend radius.
        max_extra_width_radius: Don't apply extra width above this radius.
        radius_threshold: Curvature radius below which extra width applies.
        radius_sample_length: Resampling step for curvature computation (m).

    Returns:
        (left_boundary, right_boundary, diagnostics_dict)
    """
    left_lines: List[LineString] = []
    right_lines: List[LineString] = []
    all_radii: List[float] = []
    all_extra: List[float] = []

    for geom in centreline_geoms:
        if geom is None or geom.is_empty or geom.length <= 0.0:
            continue
        for line in extract_lines(geom):
            if line.length <= 0.0:
                continue
            left_line, right_line, radii, extras = build_variable_offset_line(
                line,
                base_offset,
                loa,
                extra_width_coefficient,
                min_radius_length_multiple,
                max_extra_width_radius,
                radius_threshold,
                radius_sample_length,
            )
            if left_line.length <= 0.0 or right_line.length <= 0.0:
                continue
            left_lines.append(left_line)
            right_lines.append(right_line)
            if radii.size:
                valid = radii[np.isfinite(radii) & (radii > 0)]
                all_radii.extend(valid.tolist())
            if extras.size:
                all_extra.extend(extras.tolist())

    if not left_lines or not right_lines:
        raise ValueError("Failed to generate channel limits from the centreline geometries.")

    left_geom = ensure_multilinestring(merge_lines(left_lines))
    right_geom = ensure_multilinestring(merge_lines(right_lines))
    diagnostics = {
        "min_radius": min(all_radii) if all_radii else None,
        "max_extra_width": max(all_extra) if all_extra else 0.0,
        "mean_extra_width": float(np.mean(all_extra)) if all_extra else 0.0,
    }
    return left_geom, right_geom, diagnostics
