"""Test triad for loop rejection: golden, property, feasibility."""
import math

import numpy as np

from channel_designer.alignment.loops import (
    LEN_RATIO_BACKSTOP,
    TURN_LIMIT_DEG,
    cumulative_turn_deg,
    is_self_intersecting,
    loop_verdict,
)


def _arc(radius, sweep_deg, n=400, cx=0.0, cy=0.0, t0=0.0):
    t = np.linspace(t0, t0 + math.radians(sweep_deg), n)
    return np.column_stack([cx + radius * np.cos(t), cy + radius * np.sin(t)])


# --- golden ---------------------------------------------------------------------------

def test_golden_straight_line_has_no_turn_and_is_accepted():
    c = np.column_stack([np.linspace(0, 1000, 50), np.zeros(50)])
    assert cumulative_turn_deg(c) < 1e-6
    reject, why, m = loop_verdict(c, 1000.0)
    assert not reject and why is None
    assert abs(m["len_ratio"] - 1.0) < 1e-6


def test_golden_cumulative_turn_of_a_half_circle_is_180_deg():
    assert abs(cumulative_turn_deg(_arc(500.0, 180.0)) - 180.0) < 1.0


def test_golden_a_full_circle_turns_360_and_self_intersects():
    c = _arc(300.0, 361.0)          # just past closure, so it crosses itself
    assert abs(cumulative_turn_deg(c) - 361.0) < 2.0
    assert is_self_intersecting(c)


# --- property -------------------------------------------------------------------------

def test_property_turn_is_invariant_to_direction_and_translation():
    c = _arc(400.0, 250.0)
    base = cumulative_turn_deg(c)
    assert abs(cumulative_turn_deg(c[::-1]) - base) < 1e-6      # reversed
    assert abs(cumulative_turn_deg(c + 12345.0) - base) < 1e-6  # translated
    rot = np.column_stack([c[:, 0] * math.cos(0.7) - c[:, 1] * math.sin(0.7),
                           c[:, 0] * math.sin(0.7) + c[:, 1] * math.cos(0.7)])
    assert abs(cumulative_turn_deg(rot) - base) < 1e-6          # rotated


def test_property_duplicate_points_do_not_invent_turn():
    """A lattice path often repeats a vertex. That must not read as a heading change."""
    c = np.column_stack([np.linspace(0, 100, 20), np.zeros(20)])
    dup = np.repeat(c, 3, axis=0)
    assert cumulative_turn_deg(dup) < 1e-6


def test_property_rejection_is_monotone_in_length_ratio():
    """Past the backstop, longer is never MORE acceptable."""
    c = np.column_stack([np.linspace(0, 1000, 200), np.zeros(200)])
    assert not loop_verdict(c, 1000.0)[0]
    assert not loop_verdict(c, 1000.0 / (LEN_RATIO_BACKSTOP * 0.9))[0]
    assert loop_verdict(c, 1000.0 / (LEN_RATIO_BACKSTOP * 1.1))[0]


# --- feasibility: the rejections must fire when they should, and only then -------------

def test_feasibility_a_long_simple_window_is_NO_LONGER_rejected():
    """THE REGRESSION THIS MODULE EXISTS FOR.

    A cheaper window can be much longer than the design line when it goes around high ground:
    here a length ratio of 1.84 and a cumulative turn of 419 deg, and still simple (no
    self-crossing). The old MAX_LEN_RATIO = 1.30 threw such windows away. A synthetic stand-in
    with those properties must now pass.
    """
    c = _arc(600.0, 419.0 * 0.5)              # simple arc, no crossing
    c = np.vstack([c, c[-1] + (c[-1] - c[-2]) * np.arange(1, 60)[:, None]])   # straight tail
    turn = cumulative_turn_deg(c)
    assert not is_self_intersecting(c), "the stand-in must be simple, like the real window"
    plen = float(np.hypot(np.diff(c[:, 0]), np.diff(c[:, 1])).sum())
    window = plen / 1.84                       # force the real window's length ratio
    reject, why, m = loop_verdict(c, window)
    assert abs(m["len_ratio"] - 1.84) < 0.01
    assert not reject, f"rejected a legitimate winding window: {why} (turn {turn:.0f} deg)"
    # and it would have been rejected by the retired rule
    assert m["len_ratio"] >= 1.30


def test_feasibility_a_real_loop_is_rejected():
    c = _arc(300.0, 400.0)                     # overlaps itself
    reject, why, _ = loop_verdict(c, 1000.0)
    assert reject and "self-intersecting" in why


def test_feasibility_a_spiral_that_never_crosses_is_still_rejected():
    """Growing-radius spiral: simple, so the crossing test misses it. Turn must catch it."""
    t = np.linspace(0.0, 2.0 * math.pi * 3.5, 3000)   # 3.5 turns = 1260 deg
    r = 200.0 + 60.0 * t
    c = np.column_stack([r * np.cos(t), r * np.sin(t)])
    assert not is_self_intersecting(c), "an expanding spiral does not cross itself"
    reject, why, m = loop_verdict(c, 1e9)      # huge window so length ratio cannot fire
    assert m["turn_deg"] > TURN_LIMIT_DEG
    assert reject and "cumulative turn" in why


def test_feasibility_no_length_ratio_cap_as_the_rule():
    """The retired 1.30 length cap is not a live name; only the 3.0 backstop exists."""
    from channel_designer.alignment import loops
    assert not hasattr(loops, "MAX_LEN_RATIO")
    assert loops.LEN_RATIO_BACKSTOP == 3.0
