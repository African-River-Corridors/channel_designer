"""Smoothed river centreline from a pair of bank lines (paired cross-section method).

Each bank is resampled, mapped monotonically onto the opposite bank, and the midpoints are
blended, Chaikin-smoothed and moving-averaged. Pure geometry in a projected CRS (metres).
Reading bank files and reprojecting them is the caller's job.
"""
from __future__ import annotations

import math
from typing import Sequence, Tuple

import numpy as np
from shapely.geometry import LineString, Point


def ensure_orientation(left: np.ndarray, right: np.ndarray) -> Tuple[np.ndarray, np.ndarray]:
    """Flip ``right`` if it runs the opposite way to ``left``."""
    dist_same = np.linalg.norm(left[0] - right[0])
    dist_reverse = np.linalg.norm(left[0] - right[-1])
    if dist_reverse < dist_same:
        right = right[::-1]
    return left, right


def resample_polyline(points: Sequence[Sequence[float]], step: float) -> np.ndarray:
    """Resample a polyline at uniform spacing ``step`` (m). At least three samples."""
    coords = np.asarray(points, dtype=float)
    if coords.shape[0] < 2:
        raise ValueError("Polyline must contain at least two vertices.")
    if step <= 0:
        raise ValueError("Resample step must be positive.")

    seg_lengths = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cumulative = np.concatenate(([0.0], np.cumsum(seg_lengths)))
    total_length = cumulative[-1]
    if total_length == 0:
        raise ValueError("Polyline has zero length.")

    n_samples = max(int(math.ceil(total_length / step)) + 1, 3)
    targets = np.linspace(0.0, total_length, n_samples)
    x = np.interp(targets, cumulative, coords[:, 0])
    y = np.interp(targets, cumulative, coords[:, 1])
    return np.column_stack((x, y))


def reparameterize_fraction(points: np.ndarray) -> Tuple[np.ndarray, np.ndarray]:
    coords = np.asarray(points, dtype=float)
    seg_lengths = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cumulative = np.concatenate(([0.0], np.cumsum(seg_lengths)))
    total_length = cumulative[-1]
    if total_length == 0:
        raise ValueError("Polyline has zero length.")
    fractions = cumulative / total_length
    return coords, fractions


def moving_average(points: np.ndarray, window: int) -> np.ndarray:
    if window <= 1:
        return points
    if window % 2 == 0:
        window += 1  # enforce odd window
    kernel = np.ones(window) / window
    pad = window // 2
    padded = np.pad(points, ((pad, pad), (0, 0)), mode="edge")
    smoothed = np.vstack(
        [
            np.convolve(padded[:, 0], kernel, mode="valid"),
            np.convolve(padded[:, 1], kernel, mode="valid"),
        ]
    ).T
    return smoothed


def chaikin_smoothing(points: np.ndarray, iterations: int) -> np.ndarray:
    if iterations <= 0:
        return points
    pts = points.copy()
    for _ in range(iterations):
        new_points = [pts[0]]
        for i in range(len(pts) - 1):
            p0 = pts[i]
            p1 = pts[i + 1]
            q = 0.75 * p0 + 0.25 * p1
            r = 0.25 * p0 + 0.75 * p1
            new_points.extend([q, r])
        new_points.append(pts[-1])
        pts = np.asarray(new_points)
    return pts


def _interpolate_line(line: LineString, distance: float) -> np.ndarray:
    pt = line.interpolate(distance)
    return np.array([pt.x, pt.y], dtype=float)


def map_points_monotonic(source: np.ndarray, target_line: LineString, max_step: float) -> np.ndarray:
    """Project ``source`` points onto ``target_line``, never stepping backwards."""
    mapped = []
    prev_d = 0.0
    total_length = target_line.length
    for idx, pt in enumerate(source):
        d = target_line.project(Point(pt))
        if d < prev_d:
            d = prev_d
        elif d - prev_d > max_step:
            d = min(prev_d + max_step, total_length)
        if idx == len(source) - 1:
            d = total_length
        mapped.append(_interpolate_line(target_line, d))
        prev_d = d
    return np.asarray(mapped)


def compute_paired_centreline(
    left: np.ndarray,
    right: np.ndarray,
    resample_step: float,
    smooth_window: int,
    chaikin_iterations: int,
) -> np.ndarray:
    left_line = LineString(left.tolist())
    right_line = LineString(right.tolist())
    max_step = 5 * resample_step

    right_on_left = map_points_monotonic(left, right_line, max_step=max_step)
    left_mid = 0.5 * (left + right_on_left)

    left_on_right = map_points_monotonic(right, left_line, max_step=max_step)
    right_mid = 0.5 * (left_on_right + right)

    left_coords, left_frac = reparameterize_fraction(left_mid)
    right_coords, right_frac = reparameterize_fraction(right_mid)

    n_samples = max(len(left_mid), len(right_mid))
    target_frac = np.linspace(0.0, 1.0, n_samples)
    blended = 0.5 * (
        np.column_stack(
            (
                np.interp(target_frac, left_frac, left_coords[:, 0]),
                np.interp(target_frac, left_frac, left_coords[:, 1]),
            )
        )
        + np.column_stack(
            (
                np.interp(target_frac, right_frac, right_coords[:, 0]),
                np.interp(target_frac, right_frac, right_coords[:, 1]),
            )
        )
    )

    if chaikin_iterations > 0:
        blended = chaikin_smoothing(blended, chaikin_iterations)
    if smooth_window > 1:
        blended = moving_average(blended, smooth_window)
    return resample_polyline(blended.tolist(), resample_step)


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
    left = resample_polyline(left_bank.coords, resample_step)
    right = resample_polyline(right_bank.coords, resample_step)
    left, right = ensure_orientation(left, right)
    pts = compute_paired_centreline(left, right, resample_step, smooth_window, chaikin_iterations)
    return LineString(pts)


def trim_polyline(points: np.ndarray, cut_length: float) -> np.ndarray:
    """Remove ``cut_length`` metres from both ends of a polyline."""
    if cut_length <= 0:
        return points

    coords = np.asarray(points, dtype=float)
    if coords.shape[0] < 2:
        raise ValueError("Cannot trim a centreline with fewer than two vertices.")

    seg_lengths = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cumulative = np.concatenate(([0.0], np.cumsum(seg_lengths)))
    total_length = cumulative[-1]
    if total_length <= 2 * cut_length:
        raise ValueError(
            f"Centreline length {total_length:.2f} m is too short for the requested "
            f"end cut of {cut_length:.2f} m at both ends."
        )

    start_distance = cut_length
    end_distance = total_length - cut_length

    def interpolate_at(distance: float) -> np.ndarray:
        if distance <= 0:
            return coords[0]
        if distance >= total_length:
            return coords[-1]
        idx = int(np.searchsorted(cumulative, distance, side="right"))
        prev_dist = cumulative[idx - 1]
        next_dist = cumulative[idx]
        if math.isclose(distance, prev_dist):
            return coords[idx - 1]
        if math.isclose(distance, next_dist):
            return coords[idx]
        ratio = (distance - prev_dist) / (next_dist - prev_dist)
        return coords[idx - 1] + ratio * (coords[idx] - coords[idx - 1])

    new_points = [interpolate_at(start_distance)]
    interior_mask = (cumulative > start_distance) & (cumulative < end_distance)
    new_points.extend(coords[interior_mask])
    new_points.append(interpolate_at(end_distance))

    trimmed = np.vstack(new_points)
    if trimmed.shape[0] < 2:
        raise ValueError("Trimming removed too many points; resulting centreline is invalid.")
    return trimmed
