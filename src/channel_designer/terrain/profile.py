"""Long profile of a line over a terrain sampler.

Pure numpy and shapely: the terrain comes in as a callable ``terrain(xs, ys) -> z`` (the same contract as
``volumes.sections.section_volumes``), in the line's CRS. Nothing here reads a file.

``ascent_m`` and ``descent_m`` sum the rises and falls between consecutive valid samples. They depend on the
sample step and on DEM noise, so compare them only between profiles sampled at the same step.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from shapely.geometry import LineString


@dataclass(frozen=True)
class LineProfile:
    chainage_m: np.ndarray
    x: np.ndarray
    y: np.ndarray
    z: np.ndarray            # NaN where the terrain has no data

    @property
    def length_m(self) -> float:
        return float(self.chainage_m[-1]) if len(self.chainage_m) else 0.0

    @property
    def n_nodata(self) -> int:
        return int(np.isnan(self.z).sum())

    def _valid(self) -> np.ndarray:
        return self.z[~np.isnan(self.z)]

    @property
    def min_z(self) -> float:
        v = self._valid()
        return float(v.min()) if v.size else float("nan")

    @property
    def max_z(self) -> float:
        v = self._valid()
        return float(v.max()) if v.size else float("nan")

    @property
    def ascent_m(self) -> float:
        d = np.diff(self._valid())
        return float(d[d > 0].sum())

    @property
    def descent_m(self) -> float:
        d = np.diff(self._valid())
        return float(-d[d < 0].sum())

    def at(self, chainage_m: float) -> float:
        """Terrain level at a chainage, linear between samples (NaN outside the line)."""
        if chainage_m < 0 or chainage_m > self.length_m:
            return float("nan")
        return float(np.interp(chainage_m, self.chainage_m, self.z))


def line_profile(line: LineString, terrain, step_m: float = 30.0) -> LineProfile:
    """Sample ``terrain`` every ``step_m`` along ``line``, always including both ends.

    ``step_m`` should be no finer than the DEM's pixel: finer sampling only interpolates.
    """
    if step_m <= 0:
        raise ValueError("step_m must be positive")
    if line.length == 0:
        raise ValueError("line has zero length")
    s = np.arange(0.0, line.length, step_m)
    if s[-1] < line.length:
        s = np.append(s, line.length)
    pts = [line.interpolate(float(c)) for c in s]
    x = np.array([p.x for p in pts])
    y = np.array([p.y for p in pts])
    z = np.asarray(terrain(x, y), dtype=float)
    if z.shape != x.shape:
        raise ValueError(f"terrain returned shape {z.shape} for {x.shape} points")
    return LineProfile(chainage_m=s, x=x, y=y, z=z)
