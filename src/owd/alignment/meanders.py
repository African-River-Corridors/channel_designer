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

from .. import _rust

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
    return _rust.call("meanders", "resample_polyline", np.ndarray, points=points, step=step)


def circumcircle(p0: np.ndarray, p1: np.ndarray, p2: np.ndarray) -> Optional[Tuple[np.ndarray, float]]:
    r = _rust.call("meanders", "circumcircle", p0=p0, p1=p1, p2=p2)
    return None if r is None else (np.asarray(r["center"], dtype=float), float(r["radius"]))


def curvature_radius(points: np.ndarray) -> Tuple[np.ndarray, np.ndarray, np.ndarray]:
    """(curvature, radius, signed curvature) at each point; signed > 0 for a left turn."""
    r = _rust.call("meanders", "curvature_radius", points=points)
    return tuple(np.asarray(r[k], dtype=float) for k in ("curvature", "radius", "signed_curvature"))


def compute_heading(points: np.ndarray) -> np.ndarray:
    return _rust.call("meanders", "compute_heading", np.ndarray, points=points)


def detect_curvature_runs(signed_curvature: np.ndarray, min_points: int) -> List[np.ndarray]:
    runs = _rust.call("meanders", "detect_curvature_runs", signed_curvature=signed_curvature,
                      min_points=int(min_points))
    return [np.asarray(r, dtype=int) for r in runs]


def find_meander_runs(radius: np.ndarray, signed_curvature: np.ndarray,
                      threshold: float, min_points: int) -> List[np.ndarray]:
    runs = _rust.call("meanders", "find_meander_runs", radius=radius, signed_curvature=signed_curvature,
                      threshold=threshold, min_points=int(min_points))
    return [np.asarray(r, dtype=int) for r in runs]


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
    return _rust.call("meanders", "tangent_angle", heading=heading, indices=[int(index)])[0]


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
    return _rust.call("meanders", "find_meanders", centreline=centreline, max_radius=max_radius,
                      sample_step=sample_step, min_points=int(min_points),
                      min_deflection_deg=min_deflection_deg)
