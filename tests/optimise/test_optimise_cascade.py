"""Cascade search: exact against brute force, guards that refuse instead of clamping."""
from __future__ import annotations

import itertools
import math

import numpy as np
import pytest
from shapely.geometry import LineString

from channel_designer.optimise.cascade import (
    CascadeConfig,
    LevelTable,
    build_level_table,
    cascade_levels,
    earthworks_reach_cost,
    score_cascade,
    search_cascade,
)


def _reach(a, b, level):
    # Deliberately nonlinear in chainage and level: cut falls as the pool rises, and an
    # expensive reach sits around 2-3 km.
    x = np.linspace(a, b, 9)
    dens = 100.0 + 400.0 * np.exp(-((x - 2500.0) / 500.0) ** 2)
    return float(np.trapezoid(dens, x)) * math.exp(-0.4 * level)


def _struct(lift):
    return 3.0e5 + 1.5e5 * lift ** 1.5


CFG = dict(candidate_sites_m=[700, 1200, 1900, 2600, 3300, 4100], lifts_m=(1.0, 1.5, 2.5),
           terminal_m=5000.0, terminal_min_level_m=3.0, min_spacing_m=800.0, level_cap_m=6.0)


def _brute(cfg):
    sites = sorted(cfg.candidate_sites_m)
    best = math.inf
    for n in range(1, len(sites) + 1):
        for combo in itertools.combinations(sites, n):
            pts = [cfg.start_m] + list(combo)
            if any(b - a < cfg.min_spacing_m for a, b in zip(pts, pts[1:])):
                continue
            for lifts in itertools.product(cfg.lifts_m, repeat=n):
                levels = np.cumsum((cfg.base_level_m,) + lifts)
                if levels[-1] > cfg.level_cap_m + 1e-9 or levels[-1] < cfg.terminal_min_level_m:
                    continue
                ends = list(combo) + [cfg.terminal_m]
                c = sum(_reach(a, b, l) for a, b, l in zip(pts, ends, levels))
                c += sum(_struct(L) for L in lifts)
                best = min(best, c)
    return best


def test_golden_dp_equals_brute_force():
    cfg = CascadeConfig(**CFG)
    out = search_cascade(cfg, _reach, _struct)
    assert abs(out.best.total_cost - _brute(cfg)) < 1e-6 * out.best.total_cost


def test_property_result_obeys_spacing_terminal_and_cap():
    cfg = CascadeConfig(**CFG)
    for c in search_cascade(cfg, _reach, _struct).cascades:
        ch = [cfg.start_m] + [w.chainage_m for w in c.weirs]
        assert all(b - a >= cfg.min_spacing_m for a, b in zip(ch, ch[1:]))
        assert cfg.terminal_min_level_m <= c.weirs[-1].pool_level_m <= cfg.level_cap_m
        assert c.reaches[0].level_m == cfg.base_level_m and c.reaches[-1].end_m == cfg.terminal_m
        assert abs(c.total_cost - (c.reach_cost + c.structure_cost)) < 1e-6 * c.total_cost


def test_property_ranked_cheapest_first():
    costs = [c.total_cost for c in search_cascade(CascadeConfig(**CFG), _reach, _struct).cascades]
    assert costs == sorted(costs) and len(costs) > 1


def test_golden_score_cascade_reproduces_the_search():
    cfg = CascadeConfig(**CFG)
    best = search_cascade(cfg, _reach, _struct).best
    fixed = score_cascade(cfg, [(w.chainage_m, w.lift_m) for w in best.weirs], _reach, _struct)
    assert abs(fixed.total_cost - best.total_cost) < 1e-6 * best.total_cost


def test_feasibility_terminal_above_cap_refuses():
    with pytest.raises(ValueError, match="level_cap_m"):
        CascadeConfig(**{**CFG, "terminal_min_level_m": 9.0})


def test_feasibility_sites_beyond_terminal_are_dropped_and_counted():
    cfg = CascadeConfig(**{**CFG, "candidate_sites_m": CFG["candidate_sites_m"] + [5500, 6000]})
    out = search_cascade(cfg, _reach, _struct)
    assert out.sites_dropped_beyond_terminal == 2
    assert all(w.chainage_m <= cfg.terminal_m for w in out.best.weirs)


def test_golden_levels_are_sums_of_lifts():
    cfg = CascadeConfig(candidate_sites_m=[1.0], lifts_m=(1.0, 1.5), terminal_m=2.0,
                        terminal_min_level_m=2.0, level_cap_m=3.0)
    assert cascade_levels(cfg) == [0.0, 1.0, 1.5, 2.0, 2.5, 3.0]


def test_level_table_reach_is_additive_and_refuses_unknown_levels():
    ch = np.arange(0.0, 1000.0, 25.0)
    rng = np.random.default_rng(1)
    t = LevelTable(ch, np.array([0.0, 1.0]), rng.random((ch.size, 2)), rng.random((ch.size, 2)), 25.0)
    a = t.reach_volumes(0.0, 400.0, 1.0); b = t.reach_volumes(400.0, 1000.0, 1.0)
    w = t.reach_volumes(0.0, 1000.0, 1.0)
    assert abs(a[0] + b[0] - w[0]) < 1e-9 and abs(a[1] + b[1] - w[1]) < 1e-9
    with pytest.raises(ValueError, match="not in the table"):
        t.reach_volumes(0.0, 100.0, 0.5)
    cost = earthworks_reach_cost(t, wet_weight=2.0, dry_weight=3.0)
    assert abs(cost(0.0, 1000.0, 1.0) - (2.0 * w[0] + 3.0 * w[1])) < 1e-9


def test_level_table_from_section_volumes_falls_as_level_rises():
    # Flat dry ground at 5 m; a higher pool means a shallower cut to grade = level - depth.
    line = LineString([(0, 0), (500, 0)])
    river = LineString([(0, 0), (500, 0)]).buffer(1.0)
    t = build_level_table(line, lambda x, y: np.full(np.shape(x), 5.0), river, [0.0, 1.0, 2.0],
                          depth_m=3.0, hb=10.0, maxw_m=60.0)
    tot = [sum(t.reach_volumes(0.0, 500.0, lv)) for lv in (0.0, 1.0, 2.0)]
    assert tot[0] > tot[1] > tot[2] > 0
