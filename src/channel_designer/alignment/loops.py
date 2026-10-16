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

from .. import _rust

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
    return _rust.call("loops", "cumulative_turn_deg", float, coords=coords)


def is_self_intersecting(coords) -> bool:
    """True if the polyline crosses itself."""
    return _rust.call("loops", "is_self_intersecting", bool, coords=coords)


def loop_verdict(coords, window_len_m: float):
    """Returns (reject: bool, reason: str|None, metrics: dict).

    ``reason`` is None when the candidate is acceptable. It is a short human string when it
    is not, so a caller can print WHY a window was dropped rather than a bare count.
    """
    reject, reason, metrics = _rust.call("loops", "loop_verdict", coords=coords, window_len_m=window_len_m)
    return bool(reject), reason, metrics


def is_loop_artefact(coords, window_len_m: float) -> bool:
    """Boolean convenience wrapper over :func:`loop_verdict`."""
    return _rust.call("loops", "is_loop_artefact", bool, coords=coords, window_len_m=window_len_m)


_rust.core_constants(__name__, "REJECT_SELF_INTERSECTING", "TURN_LIMIT_DEG", "LEN_RATIO_BACKSTOP")
