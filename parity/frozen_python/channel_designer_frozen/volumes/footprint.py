"""Excavation footprint in plan: design channel polygon minus the present river polygon.

Pure geometry in a projected CRS (metres). Areas only (m²); the volume of cut comes from
:mod:`.sections` or :mod:`.raster`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List

from shapely.geometry import (
    GeometryCollection,
    LineString,
    MultiPolygon,
    Point,
    Polygon,
)

from ..alignment.limits import build_channel_limits, extract_lines


def orient_like(reference: LineString, candidate: LineString) -> LineString:
    """Flip candidate line to match the direction of the reference line."""
    ref_start = Point(reference.coords[0])
    ref_end = Point(reference.coords[-1])
    cand_start = Point(candidate.coords[0])
    cand_end = Point(candidate.coords[-1])

    same_direction = ref_start.distance(cand_start) + ref_end.distance(cand_end)
    opposite_direction = ref_start.distance(cand_end) + ref_end.distance(cand_start)
    if opposite_direction < same_direction:
        return LineString(list(candidate.coords)[::-1])
    return candidate


def polygon_from_bank_pair(left: LineString, right: LineString) -> Polygon:
    """Construct a closed polygon from a pair of bank/channel lines."""
    coords_left = list(left.coords)
    coords_right = list(right.coords)[::-1]
    ring = coords_left + coords_right
    if ring[0] != ring[-1]:
        ring.append(ring[0])
    polygon = Polygon(ring)
    if not polygon.is_valid:
        polygon = polygon.buffer(0)
    if polygon.is_empty:
        raise ValueError("Constructed polygon is empty; check the input bank geometries.")
    return polygon


def iter_polygons(geom) -> List[Polygon]:
    """Extract Polygon geometries from any geometry type."""
    if geom.is_empty:
        return []
    if isinstance(geom, Polygon):
        return [geom]
    if isinstance(geom, MultiPolygon):
        return [poly for poly in geom.geoms if not poly.is_empty]
    if isinstance(geom, GeometryCollection):
        collected = []
        for part in geom.geoms:
            collected.extend(iter_polygons(part))
        return collected
    return []


def subtract_polygons(channel_poly: Polygon, river_poly: Polygon,
                      min_area: float = 0.5) -> List[Polygon]:
    """Excavation areas = channel polygon minus river polygon, dropping slivers < min_area."""
    difference = channel_poly.difference(river_poly.buffer(0))
    return [g for g in iter_polygons(difference) if g.area >= max(min_area, 0.0)]


def classify_sides(polygons, left_line: LineString, right_line: LineString) -> List[str]:
    """Classify each polygon as 'left' or 'right' by its nearest channel boundary."""
    sides = []
    for poly in polygons:
        rep = poly.representative_point()
        sides.append("left" if rep.distance(left_line) <= rep.distance(right_line) else "right")
    return sides


def compute_excavation(channel_left: LineString, channel_right: LineString,
                       bank_left: LineString, bank_right: LineString,
                       min_area: float = 0.5) -> List[dict]:
    """Bank excavation polygons from channel limits and bank lines.

    Returns a list of ``{"geometry", "bank_side", "area_m2"}`` dicts (empty if none).
    """
    channel_right = orient_like(channel_left, channel_right)
    bank_right = orient_like(bank_left, bank_right)
    channel_polygon = polygon_from_bank_pair(channel_left, channel_right)
    river_polygon = polygon_from_bank_pair(bank_left, bank_right)
    polys = subtract_polygons(channel_polygon, river_polygon, min_area)
    sides = classify_sides(polys, channel_left, channel_right)
    return [{"geometry": g, "bank_side": s, "area_m2": float(g.area)} for g, s in zip(polys, sides)]


@dataclass
class ChannelFootprint:
    centreline: List[LineString]
    channel_left: List[LineString]
    channel_right: List[LineString]
    excavation: List[dict]
    diagnostics: dict = field(default_factory=dict)

    @property
    def total_excavation_area_m2(self) -> float:
        return float(sum(e["area_m2"] for e in self.excavation))


def channel_footprint(centreline, bank_left: LineString, bank_right: LineString, *,
                      loa: float, beam: float, depth: float, side_slope: float = 3.0,
                      width_beam_multiple: float = 2.8, min_radius_length_multiple: float = 3.0,
                      extra_width_coefficient: float = 0.6, radius_sample_length: float = 10.0,
                      max_extra_width_radius: float = 2000.0,
                      min_area: float = 0.5) -> ChannelFootprint:
    """Channel limits at the waterline plus the plan excavation outside the present banks.

    Base half-width at WL = width_beam_multiple × beam / 2 + side_slope × depth; bends widen
    by ``extra_width_coefficient × LOA² / R`` (banded) below ``max_extra_width_radius``.
    """
    lines = extract_lines(centreline)
    if not lines:
        raise ValueError("centreline has no line geometry")
    base_offset = (width_beam_multiple * beam) / 2.0 + side_slope * depth
    left_geom, right_geom, diagnostics = build_channel_limits(
        centreline_geoms=lines, base_offset=base_offset, loa=loa,
        extra_width_coefficient=extra_width_coefficient,
        min_radius_length_multiple=min_radius_length_multiple,
        max_extra_width_radius=max_extra_width_radius,
        radius_threshold=max_extra_width_radius,
        radius_sample_length=radius_sample_length,
    )
    left_lines = extract_lines(left_geom)
    right_lines = extract_lines(right_geom)
    excavation = compute_excavation(
        channel_left=max(left_lines, key=lambda line: line.length),
        channel_right=max(right_lines, key=lambda line: line.length),
        bank_left=bank_left, bank_right=bank_right, min_area=min_area,
    )
    return ChannelFootprint(lines, left_lines, right_lines, excavation, diagnostics)
