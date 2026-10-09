"""Is an optimised candidate a LOOP ARTEFACT, or a legitimately winding channel?

A length ratio is a poor proxy for winding: a channel that goes further to go around a hill
is the optimiser doing its job. A tight length cap can reject a much cheaper window that is
simple (does not cross itself) and turns well under two full turns in total.

A loop artefact is a path that returns on itself — a topological property, not a length. So
test for it directly:

  * SELF-INTERSECTION is the primary discriminator. A lattice path that loops crosses itself;
    a legitimately sinuous channel does not. ``shapely``'s ``is_simple`` answers this exactly.
  * CUMULATIVE TURN catches the pathological spiral that never quite crosses. Generous by
    design — a legitimate winding window must pass.
  * LENGTH RATIO survives only as a loose BACKSTOP, at 3.0. It catches something neither test
    above anticipated; it does not express the rule.

THE THRESHOLDS ARE USER CHOICES, not from any standard. Say so when quoting a rejection.
"""
from __future__ import annotations

import math

import numpy as np

#: A path that crosses itself is a loop. This is the real test; the rest are guards.
REJECT_SELF_INTERSECTING = True

#: Cumulative absolute heading change, degrees. 1080 = three full turns. A user choice, not a
#: standard.
TURN_LIMIT_DEG = 1080.0

#: Loose backstop on path length / window length. At 3.0 it catches only the wildly
#: implausible; a tight cap rejects legitimate winding windows. A user choice.
LEN_RATIO_BACKSTOP = 3.0

def cumulative_turn_deg(coords) -> float:
    """Total absolute heading change along a polyline, in degrees."""
    c = np.asarray(coords, float)
    if len(c) < 3:
        return 0.0
    d = np.diff(c[:, :2], axis=0)
    seg = np.hypot(d[:, 0], d[:, 1])
    keep = seg > 1e-9
    d, seg = d[keep], seg[keep]
    if len(d) < 2:
        return 0.0
    head = np.arctan2(d[:, 1], d[:, 0])
    dh = np.diff(head)
    dh = (dh + math.pi) % (2.0 * math.pi) - math.pi      # wrap to (-pi, pi]
    return float(np.abs(dh).sum() * 180.0 / math.pi)


def is_self_intersecting(coords) -> bool:
    """True if the polyline crosses itself. Falls back to False if shapely is unavailable,
    because a missing dependency must not silently start rejecting everything."""
    try:
        from shapely.geometry import LineString
    except Exception:
        return False
    c = [(float(x), float(y)) for x, y, *_ in np.asarray(coords, float)]
    if len(c) < 3:
        return False
    return not LineString(c).is_simple


def loop_verdict(coords, window_len_m: float):
    """Returns (reject: bool, reason: str|None, metrics: dict).

    ``reason`` is None when the candidate is acceptable. It is a short human string when it
    is not, so a caller can print WHY a window was dropped rather than a bare count.
    """
    c = np.asarray(coords, float)
    plen = float(np.hypot(np.diff(c[:, 0]), np.diff(c[:, 1])).sum()) if len(c) > 1 else 0.0
    ratio = (plen / window_len_m) if window_len_m else float("inf")
    turn = cumulative_turn_deg(c)
    selfx = is_self_intersecting(c) if REJECT_SELF_INTERSECTING else False
    m = {"path_len_m": plen, "window_len_m": float(window_len_m),
         "len_ratio": ratio, "turn_deg": turn, "self_intersecting": bool(selfx)}
    if selfx:
        return True, "self-intersecting", m
    if turn > TURN_LIMIT_DEG:
        return True, f"cumulative turn {turn:.0f} deg > {TURN_LIMIT_DEG:.0f}", m
    if ratio > LEN_RATIO_BACKSTOP:
        return True, f"length ratio {ratio:.2f} > {LEN_RATIO_BACKSTOP:.1f} backstop", m
    return False, None, m


def is_loop_artefact(coords, window_len_m: float) -> bool:
    """Boolean convenience wrapper over :func:`loop_verdict`."""
    return loop_verdict(coords, window_len_m)[0]
