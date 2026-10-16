"""Excavation footprint in plan: design channel polygon minus the present river polygon.

Pure geometry in a projected CRS (metres). Areas only (m²); the volume of cut comes from
:mod:`.sections` or :mod:`.raster`.
"""

from __future__ import annotations

from .. import _rust

from dataclasses import dataclass, field
from typing import List

from shapely.geometry import LineString, Polygon


def orient_like(reference: LineString, candidate: LineString) -> LineString:
    """Flip candidate line to match the direction of the reference line."""
    return _rust.call("footprint", "orient_like", reference=reference, candidate=candidate)


def polygon_from_bank_pair(left: LineString, right: LineString) -> Polygon:
    """Construct a closed polygon from a pair of bank/channel lines."""
    return _rust.call("footprint", "polygon_from_bank_pair", left=left, right=right)


def iter_polygons(geom) -> List[Polygon]:
    """Extract Polygon geometries from any geometry type."""
    return _rust.call("footprint", "iter_polygons", geom=geom)


def subtract_polygons(channel_poly: Polygon, river_poly: Polygon,
                      min_area: float = 0.5) -> List[Polygon]:
    """Excavation areas = channel polygon minus river polygon, dropping slivers < min_area."""
    return _rust.call("footprint", "subtract_polygons", channel_poly=channel_poly, river_poly=river_poly,
                      min_area=min_area)


def classify_sides(polygons, left_line: LineString, right_line: LineString) -> List[str]:
    """Classify each polygon as 'left' or 'right' by its nearest channel boundary."""
    return _rust.call("footprint", "classify_sides", polygons=list(polygons), left_line=left_line,
                      right_line=right_line)


def compute_excavation(channel_left: LineString, channel_right: LineString,
                       bank_left: LineString, bank_right: LineString,
                       min_area: float = 0.5) -> List[dict]:
    """Bank excavation polygons from channel limits and bank lines.

    Returns a list of ``{"geometry", "bank_side", "area_m2"}`` dicts (empty if none).
    """
    return _rust.call("footprint", "compute_excavation", channel_left=channel_left, channel_right=channel_right,
                      bank_left=bank_left, bank_right=bank_right, min_area=min_area)


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
    r = _rust.call("footprint", "channel_footprint", centreline=centreline, bank_left=bank_left,
                   bank_right=bank_right, loa=loa, beam=beam, depth=depth, side_slope=side_slope,
                   width_beam_multiple=width_beam_multiple, min_radius_length_multiple=min_radius_length_multiple,
                   extra_width_coefficient=extra_width_coefficient, radius_sample_length=radius_sample_length,
                   max_extra_width_radius=max_extra_width_radius, min_area=min_area)
    return _rust.decode(r["footprint"], ChannelFootprint)
