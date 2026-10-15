"""A synthetic meandering river for examples and tests — generated in code, no data files.

``synthetic_river()`` returns a :class:`SyntheticRiver`: a sine-generated meandering
centreline in a local metric frame, its two bank lines, the wetted polygon, and an analytic
terrain sampler ``terrain(xs, ys) -> z`` with:

- a floodplain that rises gently upstream (the bed slope),
- a river channel incised below it (a parabolic bed inside the banks),
- an optional hill beside the river, so some bends cut into high ground.

The numbers are illustrative, not from any real river. Coordinates are metres in an
arbitrary local projected frame (x downstream→upstream).
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Callable

import numpy as np
import shapely
from shapely.geometry import LineString, Polygon


@dataclass
class SyntheticRiver:
    centreline: LineString
    left_bank: LineString
    right_bank: LineString
    polygon: Polygon
    terrain: Callable[[np.ndarray, np.ndarray], np.ndarray]
    width_m: float
    bed_slope: float
    floodplain_above_wl_m: float
    water_level_at_mouth_m: float

    def water_level(self, s: float) -> float:
        """Natural low water level at chainage ``s`` along the centreline (m)."""
        x = self.centreline.interpolate(s).x
        return self.water_level_at_mouth_m + self.bed_slope * x


def sine_generated_centreline(length_m: float = 6000.0, wavelength_m: float = 1500.0,
                              amplitude_m: float = 260.0, step_m: float = 10.0) -> LineString:
    """A meandering centreline y = A·sin(2πx/λ), from x = 0 to ``length_m``.

    Minimum radius of curvature is λ²/(4π²A) at the apexes (≈ 219 m for the defaults).
    """
    x = np.arange(0.0, length_m + step_m, step_m)
    y = amplitude_m * np.sin(2.0 * math.pi * x / wavelength_m)
    return LineString(np.column_stack([x, y]))


def synthetic_river(length_m: float = 6000.0, wavelength_m: float = 1500.0,
                    amplitude_m: float = 260.0, width_m: float = 40.0,
                    bed_depth_m: float = 2.0, floodplain_above_wl_m: float = 3.0,
                    bed_slope: float = 2.0e-4, water_level_at_mouth_m: float = 0.0,
                    hill_height_m: float = 15.0, hill_at=(1875.0, 150.0),
                    hill_radius_m: float = 150.0) -> SyntheticRiver:
    """Build the synthetic river. All defaults are illustrative."""
    line = sine_generated_centreline(length_m, wavelength_m, amplitude_m)
    half = width_m / 2.0
    left = line.offset_curve(half)
    right = line.offset_curve(-half)
    polygon = line.buffer(half, cap_style="flat")
    hx, hy = hill_at

    def terrain(xs, ys):
        xs = np.asarray(xs, dtype=float)
        ys = np.asarray(ys, dtype=float)
        pts = shapely.points(xs, ys)
        d = shapely.distance(pts, line)
        wl = water_level_at_mouth_m + bed_slope * xs
        flood = wl + floodplain_above_wl_m
        inside = d < half
        bed = wl - bed_depth_m * (1.0 - (d / half) ** 2)
        z = np.where(inside, bed, flood + 0.002 * np.maximum(0.0, d - half))
        r2 = (xs - hx) ** 2 + (ys - hy) ** 2
        z = z + hill_height_m * np.exp(-r2 / (2.0 * hill_radius_m ** 2)) * (~inside)
        return z

    return SyntheticRiver(line, left, right, polygon, terrain, width_m, bed_slope,
                          floodplain_above_wl_m, water_level_at_mouth_m)
