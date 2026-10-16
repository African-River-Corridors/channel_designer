"""The fairway fit: a design line inside navigable water, as a minimum-curvature corridor solve (spec 0005, ADR 0007).

A thin wrapper over the Rust core's ``alignment::fairway_fit``. The line is fitted between optional start and end
poses, holding any must-follow segments exactly, never tighter than ``r_min`` where the water allows, and reports
every stretch where no compliant line fits (``too_narrow``: no navigable width; ``r_min_not_possible``).

``navigable`` is the area the design line may use (build it with your own clearance rule, or from the water with
the core's ``navigable_water``). ``params`` takes ``r_min`` and any ``FitParams`` field by name (``chord_m``,
``spacing_m``, ``passes``, ``max_growth``, ...); the rest keep the core's defaults.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional, Sequence

from shapely.geometry import LineString

from .. import _rust


@dataclass
class Pose:
    at: tuple
    heading_rad: float


@dataclass
class Stretch:
    start_chainage_m: float
    end_chainage_m: float
    reason: str


@dataclass
class FitResult:
    line: LineString
    infeasible: list = field(default_factory=list)
    measured_min_radius_m: float = float("nan")
    radius_check_chord_m: float = float("nan")
    radius_ok: bool = False
    start_heading_error_rad: Optional[float] = None
    end_heading_error_rad: Optional[float] = None
    notes: list = field(default_factory=list)


def fairway_fit(line: LineString, navigable, *, r_min: float, start_pose: Optional[Pose] = None,
                end_pose: Optional[Pose] = None, must_follow: Sequence[LineString] = (), **params) -> FitResult:
    """Fit ``line`` inside ``navigable`` (a Polygon or MultiPolygon). See the module docstring."""
    r = _rust.call("fairway", "fairway_fit", line=line, navigable=navigable,
                   start_pose=None if start_pose is None else {"at": list(start_pose.at), "heading_rad": start_pose.heading_rad},
                   end_pose=None if end_pose is None else {"at": list(end_pose.at), "heading_rad": end_pose.heading_rad},
                   must_follow=list(must_follow), params={"r_min": r_min, **params})
    return FitResult(line=r["line"], infeasible=[Stretch(**s) for s in r["infeasible"]],
                     measured_min_radius_m=r["measured_min_radius_m"], radius_check_chord_m=r["radius_check_chord_m"],
                     radius_ok=r["radius_ok"], start_heading_error_rad=r["start_heading_error_rad"],
                     end_heading_error_rad=r["end_heading_error_rad"], notes=list(r["notes"]))
