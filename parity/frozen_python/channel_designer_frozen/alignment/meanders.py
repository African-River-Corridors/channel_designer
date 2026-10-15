"""Meander analysis: find bends tighter than a radius threshold along a centreline.

1. Resample the centreline at ``sample_step`` metres.
2. Three-point circumcircle radius and signed curvature at every sample.
3. Runs of constant curvature sign, at least ``min_points`` long, whose minimum radius is at
   or below ``max_radius`` and whose deflection is at least ``min_deflection_deg``.
4. One record per meander: apex, radius, osculating circle centre, tangent, turn direction.

Pure geometry in a projected CRS (metres). The original pipeline's JSON/GeoJSON/KML writers
are not included — the records are plain dicts, ready to serialise.
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np
from shapely.geometry import LineString

Point2D = Tuple[float, float]


@dataclass
class Meander:
    apex_index: int
    apex_point: np.ndarray
    radius: float
    center: np.ndarray
    indices: np.ndarray
    orientation: int          # +1 left turn, -1 right turn
    deflection_deg: float


def resample_polyline(points: Sequence[Point2D], step: float) -> np.ndarray:
    coords = np.asarray(points, dtype=float)
    if coords.shape[0] < 2:
        raise ValueError("Centreline must contain at least two vertices.")
    if step <= 0:
        raise ValueError("Sample step must be positive.")

    seg_lengths = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cumulative = np.concatenate(([0.0], np.cumsum(seg_lengths)))
    total_length = cumulative[-1]
    if total_length <= 0.0:
        raise ValueError("Centreline has zero length, cannot resample.")

    n_samples = max(int(math.ceil(total_length / step)) + 1, 3)
    targets = np.linspace(0.0, total_length, n_samples, endpoint=True)
    x = np.interp(targets, cumulative, coords[:, 0])
    y = np.interp(targets, cumulative, coords[:, 1])
    return np.column_stack((x, y))


def circumcircle(p0: np.ndarray, p1: np.ndarray, p2: np.ndarray) -> Optional[Tuple[np.ndarray, float]]:
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


def curvature_radius(points: np.ndarray) -> Tuple[np.ndarray, np.ndarray, np.ndarray]:
    """(curvature, radius, signed curvature) at each point; signed > 0 for a left turn."""
    n = len(points)
    if n < 3:
        raise ValueError("Need at least three points to compute curvature.")

    curvature = np.zeros(n, dtype=float)
    radius = np.full(n, np.inf, dtype=float)
    signed_curvature = np.zeros(n, dtype=float)

    for idx in range(1, n - 1):
        p0 = points[idx - 1]
        p1 = points[idx]
        p2 = points[idx + 1]
        circle = circumcircle(p0, p1, p2)
        if circle is None:
            continue
        _centre, rad = circle
        if rad < 1e-9:
            radius[idx] = 0.0
            continue
        curvature[idx] = 1.0 / rad
        radius[idx] = rad
        cross = (p1[0] - p0[0]) * (p2[1] - p1[1]) - (p1[1] - p0[1]) * (p2[0] - p1[0])
        if cross > 1e-9:
            signed_curvature[idx] = curvature[idx]
        elif cross < -1e-9:
            signed_curvature[idx] = -curvature[idx]

    curvature[0], curvature[-1] = curvature[1], curvature[-2]
    radius[0], radius[-1] = radius[1], radius[-2]
    signed_curvature[0], signed_curvature[-1] = signed_curvature[1], signed_curvature[-2]
    return curvature, radius, signed_curvature


def compute_heading(points: np.ndarray) -> np.ndarray:
    diffs = np.diff(points, axis=0)
    headings = np.arctan2(diffs[:, 1], diffs[:, 0])
    return np.unwrap(headings)


def detect_curvature_runs(signed_curvature: np.ndarray, min_points: int) -> List[np.ndarray]:
    eps = 1e-6
    signs = np.sign(signed_curvature)
    signs[np.abs(signed_curvature) < eps] = 0

    runs: List[np.ndarray] = []
    start: Optional[int] = None
    current_sign = 0

    for idx, sign in enumerate(signs):
        if sign == 0:
            if start is not None and idx - start >= min_points:
                runs.append(np.arange(start, idx))
            start = None
            current_sign = 0
            continue
        if current_sign == 0:
            start = idx
            current_sign = sign
            continue
        if sign != current_sign:
            if start is not None and idx - start >= min_points:
                runs.append(np.arange(start, idx))
            start = idx
            current_sign = sign

    if start is not None and len(signed_curvature) - start >= min_points:
        runs.append(np.arange(start, len(signed_curvature)))
    return runs


def find_meander_runs(radius: np.ndarray, signed_curvature: np.ndarray,
                      threshold: float, min_points: int) -> List[np.ndarray]:
    runs = detect_curvature_runs(signed_curvature, min_points=min_points)
    meanders = []
    for run in runs:
        run_radii = radius[run]
        if not np.any(np.isfinite(run_radii)):
            continue
        if np.nanmin(run_radii) <= threshold:
            meanders.append(run)
    return meanders


def summarise_meanders(centerline: np.ndarray, radius: np.ndarray, signed_curvature: np.ndarray,
                       heading: np.ndarray, groups: List[np.ndarray],
                       min_deflection_deg: float) -> List[Meander]:
    meanders: List[Meander] = []
    for group in groups:
        local_radii = radius[group]
        if not np.any(np.isfinite(local_radii)):
            continue
        start_idx = group[0]
        end_idx = group[-1]
        start_heading_idx = max(start_idx - 1, 0)
        end_heading_idx = min(end_idx, len(heading) - 1)
        turn_deg = abs(math.degrees(heading[end_heading_idx] - heading[start_heading_idx]))
        if turn_deg < min_deflection_deg:
            continue

        local_min_idx = int(group[np.nanargmin(local_radii)])
        if local_min_idx <= 0 or local_min_idx >= len(centerline) - 1:
            continue
        circle = circumcircle(centerline[local_min_idx - 1], centerline[local_min_idx],
                              centerline[local_min_idx + 1])
        if circle is None:
            continue
        center, circle_radius = circle
        meanders.append(Meander(
            apex_index=local_min_idx, apex_point=centerline[local_min_idx],
            radius=circle_radius, center=center, indices=group,
            orientation=int(np.sign(signed_curvature[local_min_idx])),
            deflection_deg=turn_deg,
        ))
    return meanders


def tangent_angle(heading: np.ndarray, index: int) -> float:
    if heading.size == 0:
        return 0.0
    if index <= 0:
        return float(heading[0])
    if index >= heading.size:
        return float(heading[-1])
    # Average successive segment headings to approximate the tangent at the point.
    return 0.5 * (float(heading[index - 1]) + float(heading[index]))


def build_meander_records(meanders: Iterable[Meander], centerline: np.ndarray,
                          heading: np.ndarray) -> List[Dict[str, object]]:
    if centerline.shape[0] < 2:
        return []
    segment_lengths = np.linalg.norm(np.diff(centerline, axis=0), axis=1)
    chainage = np.concatenate(([0.0], np.cumsum(segment_lengths)))

    records = []
    for meander in meanders:
        apex_idx = meander.apex_index
        apex_point = centerline[apex_idx]
        angle_deg = (math.degrees(tangent_angle(heading, apex_idx)) + 360.0) % 360.0
        records.append({
            "apex_index": apex_idx,
            "apex_distance_m": float(chainage[apex_idx]),
            "apex_location": {"x": float(apex_point[0]), "y": float(apex_point[1])},
            "radius": float(meander.radius),
            "deflection_deg": float(meander.deflection_deg),
            "tangent_angle_deg": angle_deg,
            "tangent_direction": "left" if meander.orientation >= 0 else "right",
            "circle_center": {"x": float(meander.center[0]), "y": float(meander.center[1])},
        })
    records.sort(key=lambda item: item["apex_distance_m"])
    for idx, record in enumerate(records, start=1):
        record["id_number"] = idx
    return records


def find_meanders(centreline: LineString, max_radius: float, sample_step: float = 10.0,
                  min_points: int = 8, min_deflection_deg: float = 45.0) -> List[Dict[str, object]]:
    """Meanders tighter than ``max_radius`` (m) along ``centreline``.

    Defaults follow the original pipeline: 8-sample minimum run, 45° minimum deflection.
    The original set ``max_radius = 2 × R_min = 2 × radius_multiple × LOA`` — the study
    radius, wider than the design limit, so near-misses are reported too.
    """
    pts = resample_polyline(list(centreline.coords), sample_step)
    _curv, radius, signed = curvature_radius(pts)
    heading = compute_heading(pts)
    runs = find_meander_runs(radius, signed, threshold=max_radius, min_points=min_points)
    meanders = summarise_meanders(pts, radius, signed, heading, runs,
                                  min_deflection_deg=min_deflection_deg)
    kept = [m for m in meanders if np.isfinite(m.radius) and m.radius <= max_radius]
    return build_meander_records(kept, pts, heading)
