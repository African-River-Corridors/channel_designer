"""Navigation channel limits: left/right offset lines with curvature-adaptive widening.

Pure geometry. All operations use projected coordinates in metres (any UTM zone or other
metric CRS); the caller owns reprojection. No file I/O.
"""

from __future__ import annotations

from .. import _rust

from typing import Iterable, List, Optional, Tuple

import numpy as np
from shapely.geometry import LineString, MultiLineString


def extract_lines(geom) -> List[LineString]:
    """Extract LineString geometries from any geometry type."""
    return _rust.call("limits", "extract_lines", geom=geom)


def merge_lines(lines: List[LineString]) -> LineString | MultiLineString:
    """Merge a list of LineStrings into one (or MultiLineString if not contiguous)."""
    return _rust.call("limits", "merge_lines", lines=list(lines))


def ensure_multilinestring(geom: LineString | MultiLineString) -> MultiLineString:
    """Ensure geometry is a MultiLineString."""
    return _rust.call("limits", "ensure_multilinestring", geom=geom)


def resample_line(line: LineString, step: float) -> np.ndarray:
    """Resample a LineString at uniform spacing along its length."""
    return _rust.call("limits", "resample_line", np.ndarray, line=line, step=step)


def circumcircle(
    p0: np.ndarray, p1: np.ndarray, p2: np.ndarray
) -> Optional[Tuple[np.ndarray, float]]:
    """Compute circumscribed circle centre and radius from three points."""
    r = _rust.call("limits", "circumcircle", p0=p0, p1=p1, p2=p2)
    return None if r is None else (np.asarray(r["center"], dtype=float), float(r["radius"]))


def compute_curvature_radius(points: np.ndarray) -> np.ndarray:
    """Compute radius of curvature at each point via circumscribed circles."""
    return _rust.call("limits", "compute_curvature_radius", np.ndarray, points=points)


def compute_normals(points: np.ndarray) -> np.ndarray:
    """Compute unit normal vectors (perpendicular to tangent) at each point."""
    return _rust.call("limits", "compute_normals", np.ndarray, points=points)


def compute_chainages(points: np.ndarray) -> np.ndarray:
    """Compute cumulative chainage (distance) along a point sequence."""
    return _rust.call("limits", "compute_chainages", np.ndarray, points=points)


def rolling_average_radius(
    radii: np.ndarray, chainages: np.ndarray, window: float
) -> np.ndarray:
    """Smooth curvature radii with a rolling window along the chainage."""
    return _rust.call("limits", "rolling_average_radius", np.ndarray, radii=radii, chainages=chainages,
                      window=window)


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
    return _rust.call("limits", "compute_extra_width", np.ndarray, radii=radii, chainages=chainages,
                      total_length=total_length, loa=loa, extra_width_coefficient=extra_width_coefficient,
                      min_radius_length_multiple=min_radius_length_multiple,
                      max_extra_width_radius=max_extra_width_radius, radius_threshold=radius_threshold)


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
    r = _rust.call("limits", "build_variable_offset_line", line=line, base_offset=base_offset, loa=loa,
                   extra_width_coefficient=extra_width_coefficient,
                   min_radius_length_multiple=min_radius_length_multiple,
                   max_extra_width_radius=max_extra_width_radius, radius_threshold=radius_threshold,
                   radius_sample_length=radius_sample_length)
    return (r["left"], r["right"], np.asarray(r["smoothed_radii"], dtype=float),
            np.asarray(r["extra"], dtype=float))


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
    r = _rust.call("limits", "build_channel_limits", centreline_geoms=list(centreline_geoms),
                   base_offset=base_offset, loa=loa, extra_width_coefficient=extra_width_coefficient,
                   min_radius_length_multiple=min_radius_length_multiple,
                   max_extra_width_radius=max_extra_width_radius, radius_threshold=radius_threshold,
                   radius_sample_length=radius_sample_length)
    return r["left"], r["right"], r["diagnostics"]
