"""Smoothed river centreline from a pair of bank lines (paired cross-section method).

Each bank is resampled, mapped monotonically onto the opposite bank, and the midpoints are
blended, Chaikin-smoothed and moving-averaged. Pure geometry in a projected CRS (metres).
Reading bank files and reprojecting them is the caller's job.
"""
from __future__ import annotations

from .. import _rust

from typing import Sequence, Tuple

import numpy as np
from shapely.geometry import LineString


def ensure_orientation(left: np.ndarray, right: np.ndarray) -> Tuple[np.ndarray, np.ndarray]:
    """Flip ``right`` if it runs the opposite way to ``left``."""
    a, b = _rust.call("centreline", "ensure_orientation", left=left, right=right)
    return np.asarray(a, dtype=float), np.asarray(b, dtype=float)


def resample_polyline(points: Sequence[Sequence[float]], step: float) -> np.ndarray:
    """Resample a polyline at uniform spacing ``step`` (m). At least three samples."""
    return _rust.call("centreline", "resample_polyline", np.ndarray, points=points, step=step)


def reparameterize_fraction(points: np.ndarray) -> Tuple[np.ndarray, np.ndarray]:
    a, b = _rust.call("centreline", "reparameterize_fraction", points=points)
    return np.asarray(a, dtype=float), np.asarray(b, dtype=float)


def moving_average(points: np.ndarray, window: int) -> np.ndarray:
    return _rust.call("centreline", "moving_average", np.ndarray, points=points, window=int(window))


def chaikin_smoothing(points: np.ndarray, iterations: int) -> np.ndarray:
    return _rust.call("centreline", "chaikin_smoothing", np.ndarray, points=points, iterations=int(iterations))


def map_points_monotonic(source: np.ndarray, target_line: LineString, max_step: float) -> np.ndarray:
    """Project ``source`` points onto ``target_line``, never stepping backwards."""
    return _rust.call("centreline", "map_points_monotonic", np.ndarray, source=source, target_line=target_line,
                      max_step=max_step)


def compute_paired_centreline(
    left: np.ndarray,
    right: np.ndarray,
    resample_step: float,
    smooth_window: int,
    chaikin_iterations: int,
) -> np.ndarray:
    return _rust.call("centreline", "compute_paired_centreline", np.ndarray, left=left, right=right,
                      resample_step=resample_step, smooth_window=int(smooth_window),
                      chaikin_iterations=int(chaikin_iterations))


def build_centreline(
    left_bank: LineString,
    right_bank: LineString,
    resample_step: float = 20.0,
    smooth_window: int = 5,
    chaikin_iterations: int = 2,
) -> LineString:
    """Smoothed centreline between two bank lines (projected CRS, metres).

    Defaults match the original pipeline: 20 m resampling, 5-point moving average,
    2 Chaikin passes.
    """
    return _rust.call("centreline", "build_centreline", LineString, left_bank=left_bank, right_bank=right_bank,
                      resample_step=resample_step, smooth_window=int(smooth_window),
                      chaikin_iterations=int(chaikin_iterations))


def trim_polyline(points: np.ndarray, cut_length: float) -> np.ndarray:
    """Remove ``cut_length`` metres from both ends of a polyline."""
    return _rust.call("centreline", "trim_polyline", np.ndarray, points=points, cut_length=cut_length)
