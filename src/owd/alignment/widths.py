"""River width measured perpendicular to the centreline, bank to bank.

At each station a cross-line is cast along the local normal and intersected with the left and
right bank lines. Pure geometry in a projected CRS (metres). Narrow-span polygons and KMZ
export from the original pipeline are not included.
"""
from __future__ import annotations

from .. import _rust

from typing import List, Tuple

import numpy as np
from shapely.geometry import LineString


def resample_distances(total_length: float, spacing: float) -> np.ndarray:
    return _rust.call("widths", "resample_distances", np.ndarray, total_length=total_length, spacing=spacing)


def measure_widths(
    centreline: LineString,
    left_bank: LineString,
    right_bank: LineString,
    spacing: float,
    cross_span: float,
) -> Tuple[List[dict], int]:
    """Bank-to-bank width every ``spacing`` metres.

    ``cross_span`` is the half-length of each cross-line (m) — make it wider than half the
    widest river reach. Returns ``(records, failures)``; each record has ``chainage_m``,
    ``width_m``, ``geometry`` (the bank-to-bank segment) and the two bank points.
    """
    r = _rust.call("widths", "measure_widths", centreline=centreline, left_bank=left_bank,
                   right_bank=right_bank, spacing=spacing, cross_span=cross_span)
    return r["rows"], int(r["failures"])
