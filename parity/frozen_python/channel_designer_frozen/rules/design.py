"""Channel design — centreline geometry + barge size -> navigable channel section.

One-lane inland-waterway channel sizing along a surveyed centreline. Adds
curvature-driven bend widening to the straight/single-bend rules in :mod:`.simple`.

Pure Python + numpy only, so it runs client-side in Pyodide (NO shapely / geopandas).

Two-stage precompute — LOA and beam touch *different* parts of the geometry, so they
are swept separately to avoid recomputing the expensive curvature profile:

    Stage 0  _compute_curvature(centreline)   resample -> circumcircle radius ->
                                              rolling-average smoothing.  ONCE.
    Stage 1  precompute_loa(curv, total_loa)  R_min = radius_multiple x total_loa,
                                              tight-bend detection, banded extra width.
    Stage 2  apply_beam(curv, loa, beam)      W_base = width_multiple x beam,
                                              bottom/top widths, half-offsets.

Combined sweep API: ``compute_scenario_grid(centreline, loas, beams, params)``.
Single-scenario contract: ``compute(ChannelDesignInputs(...)) -> ChannelDesignResult``.

Design basis:
- Straight bottom width = ``width_multiple x beam``.
- Design bend radius R2 = ``radius_multiple x total_loa`` (the target the centreline is
  straightened to).
- Bend widening = the banded ``extra_width_coefficient x loa^2 / R`` allowance on the smoothed
  radius, zero at and above ``max_extra_width_radius``.
- Depth = draft + under-keel clearance, UKC = max(ukc_min, ukc_fraction x draft).
- Side slope: top width = bottom + 2 x side_slope x design_depth.

A registered width method (:mod:`channel_designer.rules.methods`) can replace the first and
third items: set ``ChannelDesignParams.width_method``. The straight width then comes from the
method, and each discrete bend (a run of one turn direction) whose radius R1 is below R2 is
widened to ``method.bend_width(loa, beam, options, R1, R2, deflection)``. Where a bend width
exceeds the LOA, a cut-through is indicated.

"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Optional

import numpy as np

from vessel_designer.core.checks import Check
from .methods import get_width_method


# =============================================================================
# Under-keel clearance / depth <-> draft  (carried over verbatim from pianc.py)
# =============================================================================

def under_keel_clearance(draft_m: float, ukc_fraction: float = 0.15, ukc_min_m: float = 0.5) -> float:
    return max(ukc_min_m, ukc_fraction * draft_m)


def required_depth(draft_m: float, ukc_fraction: float = 0.15, ukc_min_m: float = 0.5) -> float:
    """Channel depth (below min water level) needed to float a given laden draft."""
    return draft_m + under_keel_clearance(draft_m, ukc_fraction, ukc_min_m)


def max_draft_for_depth(depth_m: float, ukc_fraction: float = 0.15, ukc_min_m: float = 0.5) -> float:
    """Inverse: the deepest laden draft a given channel depth allows."""
    if depth_m <= 0:
        return 0.0
    # fractional branch: depth = draft x (1 + ukc_fraction)
    d_frac = depth_m / (1.0 + ukc_fraction)
    if ukc_fraction * d_frac >= ukc_min_m:
        return d_frac
    # absolute-UKC branch: depth = draft + ukc_min
    return max(0.0, depth_m - ukc_min_m)




# =============================================================================
# Parameters (shared design assumptions, swept across the grid)
# =============================================================================

@dataclass
class ChannelDesignParams:
    """Design assumptions shared across a LOA x beam sweep.

    Defaults are examples, not recommendations — set your own for your waterway. Note the
    default ``width_multiple`` 2.8 is below WG 141's 3.0 x beam single-lane minimum;
    ``rulesets.channel_shape`` with a named ruleset is the standard-backed path.
    """

    width_multiple: float = 2.8          # one-lane straight bottom width = k_w x beam
    radius_multiple: float = 3.0         # design R_min = k_r x total_loa (the single radius knob;
                                         # set 4.0 / 5.0 for the future comparison scenarios)
    side_slope: float = 4.0              # bank slope (horizontal per vertical)

    # --- optional registered width method (see channel_designer.rules.methods) ---
    # None: straight = width_multiple x beam, banded extra width below. A name: the method
    # sizes the straight width and each discrete bend; R2 stays radius_multiple x LOA.
    width_method: Optional[str] = None
    width_options: dict = field(default_factory=dict)   # passed to the method untouched

    draft_m: float = 1.75                # laden draft -> design depth (if not overridden)
    design_depth_m: Optional[float] = None   # explicit channel depth; else from draft + UKC
    ukc_fraction: float = 0.15
    ukc_min_m: float = 0.5

    extra_width_coefficient: float = 0.6     # bend-widening coefficient (x loa^2 / R)
    max_extra_width_radius: float = 2000.0   # no widening for radii >= this
    n_bands: int = 4                         # banded extra-width resolution

    radius_sample_length: float = 10.0   # centreline resample spacing (m)
    rolling_window_m: float = 50.0       # half-window for radius smoothing (m)
    edge_margin_m: float = 100.0         # ignore widening/tight bends within this of the ends

    reference_level_m: float = 0.0       # water-surface datum for bed_level reporting

    passing_place_spacing_m: float = 2000.0      # one passing place every ~2 km
    passing_place_length_multiple: float = 5.0   # passing-place length = k x total_loa
    passing_place_width_multiple: float = 2.0    # two-lane width = k x base width

    extra_width_reasonable_ratio: float = 1.0    # warn if max extra > ratio x base width

    def __post_init__(self) -> None:
        if self.width_multiple <= 0:
            raise ValueError("width_multiple must be > 0")
        if self.radius_multiple <= 0:
            raise ValueError("radius_multiple must be > 0")
        if self.side_slope < 0:
            raise ValueError("side_slope must be >= 0")
        if self.radius_sample_length <= 0:
            raise ValueError("radius_sample_length must be > 0")
        if self.max_extra_width_radius <= 0:
            raise ValueError("max_extra_width_radius must be > 0")
        if self.n_bands < 1:
            raise ValueError("n_bands must be >= 1")
        if self.design_depth_m is not None and self.design_depth_m <= 0:
            raise ValueError("design_depth_m must be > 0 when given")
        if self.width_method is not None:
            get_width_method(self.width_method)          # unknown name -> ValueError now


def resolve_design_depth(params: ChannelDesignParams) -> float:
    """Design depth = explicit override, else draft + under-keel clearance."""
    if params.design_depth_m is not None:
        return params.design_depth_m
    return required_depth(params.draft_m, params.ukc_fraction, params.ukc_min_m)


# =============================================================================
# Inputs / Result (the standard module contract)
# =============================================================================

@dataclass
class ChannelDesignInputs:
    """Inputs for a single channel-design scenario.

    ``centreline_points`` is an (N, 2) or (N, 3) array-like of plan coordinates in a
    projected CRS (metres). A 3rd column is treated as the water-surface / terrain level
    used for ``bed_level`` reporting. ``None`` (or < 3 points) yields a straight channel.
    """

    total_loa: float                     # convoy length (barge + pusher) — drives bends
    beam: float                          # moulded beam — drives base width
    centreline_points: Optional[object] = None
    params: ChannelDesignParams = field(default_factory=ChannelDesignParams)

    def __post_init__(self) -> None:
        if self.total_loa <= 0:
            raise ValueError("total_loa must be > 0")
        if self.beam <= 0:
            raise ValueError("beam must be > 0")


@dataclass
class ChannelDesignResult:
    """One-lane channel geometry along the centreline + summary, checks, warnings."""

    total_loa: float
    beam: float
    design_depth_m: float
    base_width_m: float                  # straight one-lane bottom width = k_w x beam
    required_min_radius_m: float         # R_min threshold = k_r x total_loa

    # Per-chainage arrays (parallel) — numpy arrays.
    chainage_m: object
    radius_m: object                     # raw circumcircle radius
    smoothed_radius_m: object            # rolling-averaged radius (drives widening)
    extra_width_m: object
    bottom_width_m: object
    top_width_m: object
    bed_level_m: object
    half_offset_m: object
    radius_ok: object                    # bool: not a sub-R_min bend

    # Summary
    total_length_m: float
    min_bend_radius_m: float             # tightest actual smoothed radius on the route
    tight_bend_count: int
    mean_width_m: float                  # mean bottom width
    max_extra_width_m: float
    max_top_width_m: float

    # Passing places
    passing_place_spacing_m: float
    passing_place_length_m: float
    passing_place_width_m: float

    checks: list = field(default_factory=list)
    warnings: list = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return all(c.ok for c in self.checks)

    def sections(self):
        """Per-chainage sections as a list of plain dicts."""
        ch = np.asarray(self.chainage_m)
        out = []
        for i in range(len(ch)):
            out.append({
                "chainage": float(self.chainage_m[i]),
                "radius": _jnum(self.radius_m[i]),
                "smoothed_radius": _jnum(self.smoothed_radius_m[i]),
                "extra_width": float(self.extra_width_m[i]),
                "bottom_width": float(self.bottom_width_m[i]),
                "top_width": float(self.top_width_m[i]),
                "design_depth": self.design_depth_m,
                "bed_level": float(self.bed_level_m[i]),
                "half_offset": float(self.half_offset_m[i]),
                "radius_ok": bool(self.radius_ok[i]),
            })
        return out

    def summary(self) -> dict:
        return {
            "total_loa": self.total_loa,
            "beam": self.beam,
            "design_depth_m": self.design_depth_m,
            "base_width_m": self.base_width_m,
            "required_min_radius_m": self.required_min_radius_m,
            "total_length_m": self.total_length_m,
            "min_bend_radius_m": _jnum(self.min_bend_radius_m),
            "tight_bend_count": self.tight_bend_count,
            "mean_width_m": self.mean_width_m,
            "max_extra_width_m": self.max_extra_width_m,
            "max_top_width_m": self.max_top_width_m,
            "passing_place_spacing_m": self.passing_place_spacing_m,
            "passing_place_length_m": self.passing_place_length_m,
            "passing_place_width_m": self.passing_place_width_m,
            "ok": self.ok,
        }

    def to_dict(self, n_samples: Optional[int] = 160) -> dict:
        """JSON-friendly summary + (optionally downsampled) sections + checks."""
        ch = np.asarray(self.chainage_m)
        idx = _downsample_indices(len(ch), n_samples)
        sec = {
            "chainage_m": [float(self.chainage_m[i]) for i in idx],
            "smoothed_radius_m": [_jnum(self.smoothed_radius_m[i]) for i in idx],
            "extra_width_m": [round(float(self.extra_width_m[i]), 3) for i in idx],
            "bottom_width_m": [round(float(self.bottom_width_m[i]), 3) for i in idx],
            "top_width_m": [round(float(self.top_width_m[i]), 3) for i in idx],
            "bed_level_m": [round(float(self.bed_level_m[i]), 3) for i in idx],
            "half_offset_m": [round(float(self.half_offset_m[i]), 3) for i in idx],
            "radius_ok": [bool(self.radius_ok[i]) for i in idx],
        }
        return {
            **self.summary(),
            "sections": sec,
            "checks": [{"name": c.name, "ok": c.ok, "message": c.message} for c in self.checks],
            "warnings": list(self.warnings),
        }


# =============================================================================
# Core math (extracted + optimised from RiverMeanderAnalysis)
# =============================================================================

def _resample_points(coords: np.ndarray, step: float) -> np.ndarray:
    """Resample a polyline at fixed arc-length spacing (RMA ``_resample_line``)."""
    coords = np.asarray(coords, dtype=float)
    if coords.shape[0] < 2:
        return coords
    seg = np.linalg.norm(np.diff(coords, axis=0), axis=1)
    cum = np.concatenate(([0.0], np.cumsum(seg)))
    total = float(cum[-1])
    if total <= 0.0:
        return coords
    n = max(int(math.ceil(total / step)) + 1, 3)
    targets = np.linspace(0.0, total, n, endpoint=True)
    x = np.interp(targets, cum, coords[:, 0])
    y = np.interp(targets, cum, coords[:, 1])
    return np.column_stack((x, y))


def _circumcircle(p0, p1, p2):
    """Circumcircle (centre, radius) of 3 points, or None if collinear (RMA)."""
    ax, ay = p0
    bx, by = p1
    cx, cy = p2
    d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
    if abs(d) < 1e-9:
        return None
    a2 = ax * ax + ay * ay
    b2 = bx * bx + by * by
    c2 = cx * cx + cy * cy
    ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d
    uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d
    return np.array([ux, uy]), float(np.hypot(ux - ax, uy - ay))


def _compute_curvature_radius(points: np.ndarray) -> np.ndarray:
    """Circumcircle radius at every interior point — vectorised (RMA equivalent)."""
    n = len(points)
    radii = np.full(n, np.inf, dtype=float)
    if n < 3:
        return radii
    a, b, c = points[:-2], points[1:-1], points[2:]
    d = 2.0 * (a[:, 0] * (b[:, 1] - c[:, 1])
               + b[:, 0] * (c[:, 1] - a[:, 1])
               + c[:, 0] * (a[:, 1] - b[:, 1]))
    a2 = (a * a).sum(axis=1)
    b2 = (b * b).sum(axis=1)
    c2 = (c * c).sum(axis=1)
    with np.errstate(divide="ignore", invalid="ignore"):
        ux = (a2 * (b[:, 1] - c[:, 1]) + b2 * (c[:, 1] - a[:, 1]) + c2 * (a[:, 1] - b[:, 1])) / d
        uy = (a2 * (c[:, 0] - b[:, 0]) + b2 * (a[:, 0] - c[:, 0]) + c2 * (b[:, 0] - a[:, 0])) / d
        r = np.hypot(ux - a[:, 0], uy - a[:, 1])
    r = np.where(np.abs(d) < 1e-9, np.inf, r)
    r = np.where(r > 0.0, r, np.inf)
    radii[1:-1] = r
    radii[0] = radii[1]
    radii[-1] = radii[-2]
    return radii


def _compute_chainages(points: np.ndarray) -> np.ndarray:
    """Cumulative chainage along a polyline (RMA ``_compute_chainages``)."""
    if len(points) == 0:
        return np.zeros(0)
    seg = np.linalg.norm(np.diff(points, axis=0), axis=1)
    return np.concatenate(([0.0], np.cumsum(seg)))


def _compute_heading(points: np.ndarray) -> np.ndarray:
    """Unwrapped tangent bearing (rad) at each point, from forward differences."""
    n = len(points)
    if n < 2:
        return np.zeros(n)
    d = np.diff(points, axis=0)
    ang = np.arctan2(d[:, 1], d[:, 0])
    ang = np.concatenate((ang, ang[-1:]))     # pad to length n
    return np.unwrap(ang)


def _segment_bends(heading: np.ndarray, smoothed: np.ndarray, turn_eps: float = 1e-3):
    """Segment the centreline into discrete bends by turn direction (inflection points).

    A bend is a maximal run of consistently-signed heading change (left- or right-turning).
    For every point in a bend, returns its total deflection φ (deg, |heading_end − start|)
    and the bend's characteristic radius R1 (the tightest smoothed radius in the run).
    Straight points get φ = 0 and R1 = ∞. This is the discrete-bend basis a width method's
    ``bend_width(R1, R2, φ)`` needs — a whole bend, not only its "below R_min" run.
    """
    n = len(heading) if heading is not None else 0
    phi = np.zeros(n, dtype=float)
    r1 = np.full(n, np.inf, dtype=float)
    if n < 3:
        return phi, r1
    turn = np.diff(heading)                       # signed heading increment, length n-1
    sign = np.where(turn > turn_eps, 1, np.where(turn < -turn_eps, -1, 0))
    i = 0
    while i < n - 1:
        if sign[i] == 0:
            i += 1
            continue
        s = sign[i]
        j = i
        while j + 1 < n - 1 and sign[j + 1] == s:
            j += 1
        a, b = i, j + 1                            # bend spans points a..b
        deflection = abs(float(heading[b] - heading[a]))
        seg = smoothed[a:b + 1]
        finite = np.isfinite(seg)
        rr = float(seg[finite].min()) if np.any(finite) else float("inf")
        phi[a:b + 1] = math.degrees(deflection)
        r1[a:b + 1] = rr
        i = j + 1
    return phi, r1


def _rolling_average_radius(radii: np.ndarray, chainages: np.ndarray, window: float) -> np.ndarray:
    """Distance-windowed mean of finite radii.

    Optimised O(n) cumsum sliding window (the RMA original was O(n^2) with a
    per-point ``searchsorted``). Resampling is uniform, so a distance window maps to
    a fixed sample half-width ``k``.
    """
    n = len(radii)
    avg = np.full(n, np.inf, dtype=float)
    if n == 0:
        return avg
    spacing = (chainages[-1] / (n - 1)) if n > 1 and chainages[-1] > 0 else 1.0
    k = max(int(round(window / spacing)), 0)

    valid = np.isfinite(radii) & (radii > 0.0)
    vals = np.where(valid, radii, 0.0)
    csum = np.concatenate(([0.0], np.cumsum(vals)))
    ccnt = np.concatenate(([0.0], np.cumsum(valid.astype(float))))

    idx = np.arange(n)
    lo = np.maximum(0, idx - k)
    hi = np.minimum(n, idx + k + 1)          # exclusive upper
    s = csum[hi] - csum[lo]
    c = ccnt[hi] - ccnt[lo]
    nz = c > 0
    avg[nz] = s[nz] / c[nz]
    return avg


def _band_edges(total_loa: float, params: ChannelDesignParams) -> np.ndarray:
    """Radius band edges [R_min, ..., upper] — ``n_bands + 1`` values."""
    r_min = params.radius_multiple * total_loa
    upper = max(params.max_extra_width_radius, r_min + 1.0)
    return np.linspace(r_min, upper, params.n_bands + 1)


def _band_widths(total_loa: float, params: ChannelDesignParams) -> np.ndarray:
    """Extra width per band: coefficient x loa^2 / lower_edge; outermost band = 0."""
    edges = _band_edges(total_loa, params)
    coeff = params.extra_width_coefficient
    widths = [coeff * total_loa ** 2 / e for e in edges[:-1]]
    widths.append(0.0)
    return np.asarray(widths, dtype=float)


def extra_width(radius: float, total_loa: float, params: ChannelDesignParams) -> float:
    """Banded bend-widening for a single radius (m). 0 if straight / above threshold."""
    if not np.isfinite(radius) or radius <= 0.0:
        return 0.0
    if params.extra_width_coefficient <= 0.0 or total_loa <= 0.0:
        return 0.0
    if radius >= params.max_extra_width_radius:
        return 0.0
    edges = _band_edges(total_loa, params)
    widths = _band_widths(total_loa, params)
    limits = edges[1:]
    i = int(np.searchsorted(limits, radius, side="right"))
    i = min(max(i, 0), len(widths) - 1)
    return float(widths[i])


def min_bend_radius(total_loa: float, params: ChannelDesignParams) -> float:
    """Required (design) bend radius R2 = radius_multiple x total_loa — the target the
    centreline is straightened to (cut-throughs). Bends below it are widened; where the
    width exceeds the LOA, a cut-through is indicated."""
    return params.radius_multiple * total_loa


def _extra_width_array(smoothed: np.ndarray, chainages: np.ndarray, total_length: float,
                       total_loa: float, params: ChannelDesignParams) -> np.ndarray:
    """Banded extra width along the route (RMA ``_compute_extra_width`` equivalent)."""
    extra = np.zeros_like(smoothed, dtype=float)
    if params.extra_width_coefficient <= 0.0 or total_loa <= 0.0:
        return extra
    margin = params.edge_margin_m
    inner = (chainages >= margin) & (chainages <= max(total_length - margin, 0.0))
    threshold = params.max_extra_width_radius
    mask = inner & np.isfinite(smoothed) & (smoothed > 0.0) & (smoothed < threshold)
    if not np.any(mask):
        return extra
    edges = _band_edges(total_loa, params)
    widths = _band_widths(total_loa, params)
    limits = edges[1:]
    i = np.searchsorted(limits, smoothed[mask], side="right")
    i = np.clip(i, 0, len(widths) - 1)
    extra[mask] = widths[i]
    return extra


# =============================================================================
# Stage 0 / 1 / 2
# =============================================================================

@dataclass
class CurvatureProfile:
    """LOA/beam-independent centreline geometry — computed once."""

    points: object
    chainages: object
    radius: object
    smoothed_radius: object
    total_length: float
    z: object = None
    heading: object = None               # tangent bearing (rad) at each resampled point


@dataclass
class LoaProfile:
    """LOA-dependent stage: tight-bend detection + banded extra width."""

    total_loa: float
    required_min_radius: float
    radius_ok: object
    tight_mask: object
    bend_phi_deg: object                 # per-point discrete-bend deflection φ (deg)
    bend_r1_m: object                    # per-point bend characteristic radius R1 (m)


def _compute_curvature(centreline_points, params: ChannelDesignParams) -> CurvatureProfile:
    """Stage 0 — resample, curvature radius, rolling-average smoothing (ONCE)."""
    pts = None if centreline_points is None else np.asarray(centreline_points, dtype=float)
    if pts is None or pts.ndim != 2 or pts.shape[0] < 3:
        # Straight channel: a single representative section, no curvature.
        return CurvatureProfile(
            points=np.zeros((1, 2)),
            chainages=np.zeros(1),
            radius=np.array([np.inf]),
            smoothed_radius=np.array([np.inf]),
            total_length=0.0,
            z=None,
            heading=np.zeros(1),
        )
    xy = pts[:, :2]
    z_in = pts[:, 2] if pts.shape[1] >= 3 else None
    rp = _resample_points(xy, params.radius_sample_length)
    chain = _compute_chainages(rp)
    radius = _compute_curvature_radius(rp)
    smoothed = _rolling_average_radius(radius, chain, params.rolling_window_m)
    heading = _compute_heading(rp)
    z = None
    if z_in is not None:
        orig_chain = _compute_chainages(xy)
        z = np.interp(chain, orig_chain, z_in)
    total_length = float(chain[-1]) if len(chain) else 0.0
    return CurvatureProfile(rp, chain, radius, smoothed, total_length, z, heading)


def precompute_loa(curv: CurvatureProfile, total_loa: float,
                   params: ChannelDesignParams) -> LoaProfile:
    """Stage 1 — given the curvature profile, derive the LOA-dependent geometry.

    R_min = radius_multiple x total_loa; tight bends are those below it, and
    each discrete bend gets a deflection angle φ for a width method's bend rule.
    """
    r_min = min_bend_radius(total_loa, params)
    finite = np.isfinite(curv.smoothed_radius)
    is_tight = finite & (curv.smoothed_radius < r_min)
    radius_ok = ~is_tight                                   # straight (inf) is ok
    margin = params.edge_margin_m
    inner = (curv.chainages >= margin) & (curv.chainages <= max(curv.total_length - margin, 0.0))
    tight_mask = is_tight & inner
    bend_phi, bend_r1 = _segment_bends(curv.heading, curv.smoothed_radius)
    return LoaProfile(total_loa, r_min, radius_ok, tight_mask, bend_phi, bend_r1)


def apply_beam(curv: CurvatureProfile, loa: LoaProfile, beam: float,
               params: ChannelDesignParams) -> ChannelDesignResult:
    """Stage 2 — add the beam-dependent widths to a LOA profile -> full result.

    Default: straight bottom width = ``width_multiple x beam``, plus the banded extra width
    on the smoothed radius. With ``params.width_method`` set: straight = the method's
    straight width, and at each discrete bend whose radius R1 is below R_min the bottom
    widens to ``method.bend_width(loa, beam, options, R1, R_min, φ)``.
    """
    depth = resolve_design_depth(params)
    r_min = loa.required_min_radius
    smoothed = np.asarray(curv.smoothed_radius, dtype=float)
    n = len(smoothed)

    if params.width_method is None:
        base = params.width_multiple * beam
        extra_band = _extra_width_array(smoothed, np.asarray(curv.chainages, dtype=float),
                                        curv.total_length, loa.total_loa, params)
        bottom = base + extra_band
    else:
        method = get_width_method(params.width_method)
        opts = params.width_options
        base = method.straight_width(loa.total_loa, beam, opts)
        phi = np.asarray(loa.bend_phi_deg, dtype=float)
        bend_r1 = np.asarray(loa.bend_r1_m, dtype=float)
        bottom = np.full(n, base, dtype=float)
        # Cache by (rounded R1, rounded φ) so each distinct bend is evaluated once.
        cache: dict = {}
        for i in np.nonzero((bend_r1 < r_min) & np.isfinite(bend_r1) & (bend_r1 > 0.0))[0]:
            key = (round(float(bend_r1[i]), 1), round(float(phi[i]), 2))
            bw = cache.get(key)
            if bw is None:
                bw = method.bend_width(loa.total_loa, beam, opts, key[0], r_min, key[1])
                cache[key] = bw
            if bw > bottom[i]:
                bottom[i] = bw
    extra = bottom - base
    top = bottom + 2.0 * params.side_slope * depth
    half = top / 2.0

    if curv.z is not None:
        ref = np.asarray(curv.z, dtype=float)
    else:
        ref = np.full_like(np.asarray(curv.chainages, dtype=float), params.reference_level_m)
    bed = ref - depth

    finite = np.isfinite(smoothed)
    min_radius = float(smoothed[finite].min()) if np.any(finite) else float("inf")
    tight_count = int(np.asarray(loa.tight_mask).sum())
    max_extra = float(extra.max()) if len(extra) else 0.0

    # ---- checks (data, not buried ifs) ----
    req_depth = required_depth(params.draft_m, params.ukc_fraction, params.ukc_min_m)
    max_bottom = float(bottom.max()) if len(bottom) else base
    checks = [
        Check("bends_navigable", tight_count == 0,
              f"{tight_count} bend(s) tighter than the design radius R2={loa.required_min_radius:.0f} m "
              f"(widened; cut-through if width > LOA)"),
        Check("width_positive", float(bottom.min()) > 0.0,
              f"min bottom width {float(bottom.min()):.1f} m > 0"),
        Check("depth_sufficient", depth + 1e-9 >= req_depth,
              f"design depth {depth:.2f} m >= required {req_depth:.2f} m for draft {params.draft_m:.2f} m"),
        Check("bend_width_within_loa", max_bottom <= loa.total_loa + 1e-6,
              f"max bend width {max_bottom:.1f} m vs LOA {loa.total_loa:.0f} m — above LOA, "
              f"a cut-through is indicated", severity="warning"),
    ]

    return ChannelDesignResult(
        total_loa=loa.total_loa,
        beam=beam,
        design_depth_m=depth,
        base_width_m=base,
        required_min_radius_m=loa.required_min_radius,
        chainage_m=np.asarray(curv.chainages, dtype=float),
        radius_m=np.asarray(curv.radius, dtype=float),
        smoothed_radius_m=smoothed,
        extra_width_m=np.asarray(extra, dtype=float),
        bottom_width_m=bottom,
        top_width_m=top,
        bed_level_m=bed,
        half_offset_m=half,
        radius_ok=np.asarray(loa.radius_ok, dtype=bool),
        total_length_m=curv.total_length,
        min_bend_radius_m=min_radius,
        tight_bend_count=tight_count,
        mean_width_m=float(bottom.mean()) if len(bottom) else base,
        max_extra_width_m=max_extra,
        max_top_width_m=float(top.max()) if len(top) else float(base + 2.0 * params.side_slope * depth),
        passing_place_spacing_m=params.passing_place_spacing_m,
        passing_place_length_m=params.passing_place_length_multiple * loa.total_loa,
        passing_place_width_m=params.passing_place_width_multiple * base,
        checks=checks,
        warnings=[],
    )


# =============================================================================
# Public API
# =============================================================================

def compute(inputs: ChannelDesignInputs) -> ChannelDesignResult:
    """Single-scenario channel design (Stage 0 -> 1 -> 2)."""
    curv = _compute_curvature(inputs.centreline_points, inputs.params)
    loa = precompute_loa(curv, inputs.total_loa, inputs.params)
    return apply_beam(curv, loa, inputs.beam, inputs.params)


def compute_scenario_grid(centreline_points, loa_values, beam_values,
                          params: Optional[ChannelDesignParams] = None,
                          n_samples: Optional[int] = 160) -> dict:
    """Sweep LOA x beam over one centreline.

    Curvature is computed once (Stage 0), then for each LOA the tight-bend / extra-width
    stage runs once (Stage 1), then each beam is applied (Stage 2). Returns a JSON-friendly
    dict keyed ``loa{L}_beam{B}``.
    """
    params = params or ChannelDesignParams()
    curv = _compute_curvature(centreline_points, params)
    depth = resolve_design_depth(params)
    scenarios = {}
    for loa_val in loa_values:
        loa = precompute_loa(curv, float(loa_val), params)
        for beam_val in beam_values:
            res = apply_beam(curv, loa, float(beam_val), params)
            scenarios[f"loa{_tag(loa_val)}_beam{_tag(beam_val)}"] = res.to_dict(n_samples=n_samples)
    return {
        "design_depth_m": depth,
        "total_length_m": curv.total_length,
        "loa_values": [float(v) for v in loa_values],
        "beam_values": [float(v) for v in beam_values],
        "scenarios": scenarios,
    }


# =============================================================================
# helpers
# =============================================================================

def _jnum(x) -> Optional[float]:
    """JSON-safe float: inf/nan -> None."""
    xf = float(x)
    if math.isinf(xf) or math.isnan(xf):
        return None
    return xf


def _tag(v) -> str:
    f = float(v)
    return str(int(f)) if f == int(f) else str(f)


def _downsample_indices(n: int, n_samples: Optional[int]):
    if n_samples is None or n <= n_samples or n == 0:
        return range(n)
    step = n / n_samples
    return [int(i * step) for i in range(n_samples)]
