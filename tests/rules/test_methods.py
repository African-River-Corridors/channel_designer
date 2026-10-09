"""The width-method registry: a package outside this one can add a method and every front
door (size_channel, width_at_radius, make_width_fn, design.compute, simple.size_channel) uses it.
"""
from __future__ import annotations

import math

import numpy as np
import pytest

from channel_designer.rules import methods as M
from channel_designer.rules.design import ChannelDesignInputs, ChannelDesignParams, compute
from channel_designer.rules.rulesets import (
    RULESETS,
    ChannelRuleInputs,
    Ruleset,
    channel_shape,
    register_ruleset,
    rule_sets,
    size_channel,
    width_at_radius,
)
from channel_designer.rules.simple import PIANCInputs, size_channel as simple_size
from channel_designer.rules.width_profile import make_width_fn, width_params_dict

CALLS = []


def _straight(loa, beam, opts):
    return opts.get("k", 3.0) * beam


def _bend(loa, beam, opts, bend_r, design_r, defl):
    CALLS.append((bend_r, design_r, defl))
    return _straight(loa, beam, opts) + loa * loa / (4.0 * bend_r) * (defl / 45.0)


TOY = M.WidthMethod(name="toy", straight_width=_straight, bend_width=_bend,
                    min_radius=lambda loa, opts: 5.0 * loa,
                    breakdown=lambda loa, beam, opts: {"k": opts.get("k", 3.0)},
                    description="toy: k x beam, LOA^2/(4R) scaled by deflection")


@pytest.fixture
def toy():
    M.register_width_method(TOY, replace=True)
    CALLS.clear()
    yield TOY
    M.unregister_width_method("toy")


def test_registered_method_drives_size_channel(toy):
    r = size_channel(ChannelRuleInputs(beam_m=10.0, loa_m=80.0, draft_m=2.0,
                                       min_water_level_m=5.0, rule_set="toy",
                                       options={"k": 2.0}))
    assert r.straight_bottom_width_m == 20.0
    assert r.min_turn_radius_m == 400.0
    assert abs(r.bend_widening_m - 80.0 ** 2 / (4.0 * 240.0)) < 1e-12   # ref radius 3 x LOA
    assert r.width_breakdown["k"] == 2.0
    assert "toy" in rule_sets()


def test_registered_method_drives_width_at_radius_and_bridge(toy):
    inp = ChannelRuleInputs(beam_m=10.0, loa_m=80.0, draft_m=2.0, min_water_level_m=0.0,
                            rule_set="toy")
    fn = make_width_fn(80.0, 10.0, 2.0, rule_set="toy")
    for R in (100.0, 400.0, math.inf):
        assert fn(R) == width_at_radius(inp, R)
    assert fn(math.inf) == 30.0
    assert width_params_dict(80.0, 10.0, 2.0, rule_set="toy")["widening_coeff_note"] == TOY.description


def test_registered_method_drives_design_per_bend(toy):
    # A 200 m arc, 90 deg, below R2 = 3 x 80 = 240 m: one discrete bend, widened by the method.
    th = np.linspace(0.0, math.pi / 2, 40)
    arc = np.column_stack((200.0 * np.cos(th), 200.0 * np.sin(th)))
    p = ChannelDesignParams(width_method="toy", width_options={"k": 2.0}, edge_margin_m=0.0)
    r = compute(ChannelDesignInputs(total_loa=80.0, beam=10.0, centreline_points=arc, params=p))
    assert r.base_width_m == 20.0
    assert CALLS, "the method's bend_width was never asked"
    r1, r2, defl = CALLS[0]
    assert r2 == 240.0 and r1 < 240.0 and 60.0 < defl < 100.0
    assert float(np.max(r.bottom_width_m)) > 20.0


def test_registered_method_drives_simple_path(toy):
    c = simple_size(PIANCInputs(beam_m=10.0, loa_m=80.0, radius_multiple=4.0,
                                width_method="toy", width_options={"k": 2.5}))
    assert c.bottom_width_straight_m == 25.0
    assert abs(c.bend_widening_m - 80.0 ** 2 / (4.0 * 320.0)) < 1e-12


def test_duplicate_name_refused_unless_replace(toy):
    with pytest.raises(ValueError, match="already registered"):
        M.register_width_method(TOY)
    M.register_width_method(TOY, replace=True)


def test_builtin_cannot_be_unregistered():
    with pytest.raises(ValueError):
        M.unregister_width_method("pianc")


def test_unknown_method_is_refused_early():
    with pytest.raises(ValueError, match="rule_set must be one of"):
        ChannelRuleInputs(beam_m=10.0, loa_m=80.0, draft_m=2.0, min_water_level_m=0.0,
                          rule_set="nope")
    with pytest.raises(ValueError, match="unknown width method"):
        ChannelDesignParams(width_method="nope")


def test_entry_point_is_loaded_on_first_miss(monkeypatch):
    class EP:
        name = "toy"
        group = M.ENTRY_POINT_GROUP

        @staticmethod
        def load():
            return TOY

    import importlib.metadata as md
    monkeypatch.setattr(md, "entry_points", lambda group=None: [EP()] if group == M.ENTRY_POINT_GROUP else [])
    monkeypatch.setattr(M, "_ENTRY_POINTS_LOADED", False)
    try:
        assert M.get_width_method("toy") is TOY
    finally:
        M.unregister_width_method("toy")


def test_entry_point_name_must_match(monkeypatch):
    class EP:
        name = "other"

        @staticmethod
        def load():
            return TOY

    import importlib.metadata as md
    monkeypatch.setattr(md, "entry_points", lambda group=None: [EP()])
    monkeypatch.setattr(M, "_ENTRY_POINTS_LOADED", False)
    with pytest.raises(ValueError, match="must match"):
        M.get_width_method("other")


def test_every_registered_method_meets_the_contract():
    for name in M.width_methods():
        m = M.get_width_method(name)
        for loa, beam in ((60.0, 10.0), (110.0, 22.8)):
            straight = m.straight_width(loa, beam, {})
            assert straight > 0
            assert m.min_radius(loa, {}) > 0
            radii = (100.0, 200.0, 400.0, 1000.0, 5000.0)
            widths = [max(straight, m.bend_width(loa, beam, {}, R, R, 45.0)) for R in radii]
            assert all(a >= b - 1e-9 for a, b in zip(widths, widths[1:])), (name, widths)


def test_register_ruleset_for_channel_shape():
    rs = Ruleset("PIANC-B-wide", "pianc", "test: PIANC-B with a wider fairway", "test",
                 r_mult_convoy=3.0, r_mult_motor=3.0, width_mult=3.6, c_c=0.5, depth_mult=1.3)
    register_ruleset(rs)
    try:
        s = channel_shape("PIANC-B-wide", "CEMT_IV")
        b = channel_shape("PIANC-B", "CEMT_IV")
        assert abs(s.bottom_width_straight_m / b.bottom_width_straight_m - 3.6 / 3.2) < 1e-12
        with pytest.raises(ValueError, match="already registered"):
            register_ruleset(rs)
        with pytest.raises(ValueError, match="family"):
            register_ruleset(Ruleset("X", "other", "x", "x", 1.0, 1.0))
    finally:
        RULESETS.pop("PIANC-B-wide", None)
