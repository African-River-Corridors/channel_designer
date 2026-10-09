"""Lattice alignment search: the edge-cost field, R_min by construction, and that the search
goes round a hill a straight line goes through. Synthetic geometry only."""
from __future__ import annotations

import ast
import math
from pathlib import Path

import numpy as np
import pytest
from shapely.geometry import LineString, Polygon

from channel_designer.alignment.limits import resample_line
from channel_designer.alignment.route import min_radius_of_line
from channel_designer.optimise.lattice import (
    FieldScorer,
    LatticeConfig,
    SectionSpec,
    build_terrain_grid,
    lattice_primitives,
    optimise_pool,
    solve_lattice,
)

FAR = Polygon([(1e6, 1e6), (1e6 + 1, 1e6), (1e6 + 1, 1e6 + 1)])     # no river anywhere


def _flat(z):
    return lambda x, y: np.full(np.shape(x), float(z))


def _straight_prim(L, step=1.0):
    n = int(L / step)
    return dict(pts=[(L * k / n, L * k / n, 0.0, 0.0) for k in range(1, n + 1)], arc_len=L)


def _grid(terrain, river=FAR, half=200.0, cell=2.0):
    corridor = LineString([(0, 0), (400, 0)]).buffer(half)
    return build_terrain_grid(terrain, river, corridor, cell_m=cell)


SPEC = SectionSpec(half_width=10.0, side_slope_h_per_v=2.0, rake_half_m=40.0, rake_step_m=0.5,
                   ds_step_m=2.0)


# --- edge cost ----------------------------------------------------------------------------

def test_golden_ground_at_grade_costs_nothing():
    sc = FieldScorer(_grid(_flat(0.0)), 0.0, 10.0, SPEC)
    c, ok = sc.score(_straight_prim(100.0), (100.0, 0.0, 0.0))
    assert ok and c == 0.0


def test_golden_dry_cut_is_fairway_plus_daylighted_slopes():
    # 1 m of dry ground over grade: fairway 2 x 10 m deep 1 m, plus a 1V:2H slope each side
    # daylighting 2 m out (a triangle of 1 m2 per side) -> 22 m2 per metre run.
    sc = FieldScorer(_grid(_flat(1.0)), 0.0, 10.0, SPEC)
    c, ok = sc.score(_straight_prim(100.0), (100.0, 0.0, 0.0))
    assert ok and abs(c / 100.0 - 22.0) / 22.0 < 0.03


def test_golden_wet_cut_has_no_side_slope_and_its_own_weight():
    river = LineString([(-100, 0), (600, 0)]).buffer(150.0)
    spec = SectionSpec(**{**SPEC.__dict__, "wet_weight": 2.0, "dry_weight": 7.0})
    sc = FieldScorer(_grid(_flat(1.0), river), 0.0, 10.0, spec)
    c, ok = sc.score(_straight_prim(100.0), (100.0, 0.0, 0.0))
    assert ok and abs(c / 100.0 - 2.0 * 20.5) / 41.0 < 0.03     # 20 m fairway + edge sample


def test_property_deep_dry_cut_carries_the_multiplier():
    base = FieldScorer(_grid(_flat(30.0)), 0.0, 10.0, SPEC).score(_straight_prim(50.0), (100, 0, 0))[0]
    spec = SectionSpec(**{**SPEC.__dict__, "deep_threshold_m": 20.0, "deep_multiplier": 2.0})
    deep = FieldScorer(_grid(_flat(30.0)), 0.0, 10.0, spec).score(_straight_prim(50.0), (100, 0, 0))[0]
    assert base < deep < 2.0 * base


def test_feasibility_off_the_terrain_is_not_ok():
    sc = FieldScorer(_grid(_flat(1.0)), 0.0, 10.0, SPEC)
    assert sc.score(_straight_prim(50.0), (5000.0, 0.0, 0.0)) == (math.inf, False)


# --- primitives ---------------------------------------------------------------------------

def test_property_no_primitive_is_tighter_than_r_min():
    cfg = LatticeConfig(r_min_m=150.0, radius_multiples=(1.0, 2.0), n_heading=24)
    prims = lattice_primitives(cfg)
    assert min(p["R"] for p in prims) == 150.0
    for p in prims:
        if p["sign"]:
            assert abs(abs(p["end"][2]) - 2 * math.pi / 24) < 1e-12   # exactly one bin
    with pytest.raises(ValueError):
        LatticeConfig(r_min_m=150.0, radius_multiples=(0.8, 1.0))


# --- the search ---------------------------------------------------------------------------

def _hill(x, y):
    x = np.asarray(x, float); y = np.asarray(y, float)
    return 1.0 + 12.0 * np.exp(-((x - 300.0) ** 2 + y ** 2) / (2 * 50.0 ** 2))


def test_golden_search_goes_round_a_hill_and_holds_r_min():
    base = LineString([(0, 0), (600, 0)])
    spec = SectionSpec(half_width=8.0, side_slope_h_per_v=2.0, rake_half_m=60.0,
                       rake_step_m=4.0, ds_step_m=8.0)
    cfg = LatticeConfig(r_min_m=150.0, radius_multiples=(1.0, 2.0, 4.0), pitch_m=10.0,
                        n_heading=24)
    pa = optimise_pool(base, _hill, FAR, 0.0, 600.0, water_level_m=3.0, depth_m=3.0,
                       section=spec, cfg=cfg, corridor_half_m=150.0, cell_m=4.0)
    r = pa.result
    assert r.converged and r.end_err_m <= cfg.goal_pos_tol_m
    assert r.cost < 0.8 * pa.baseline_cost
    ys = np.asarray(r.line.coords)[:, 1]
    assert np.max(np.abs(ys)) > 40.0                         # it left the straight line
    rmin, _ = min_radius_of_line(resample_line(r.line, 25.0))
    assert rmin >= 0.99 * cfg.r_min_m


def test_property_admissible_heuristics_agree_with_dijkstra():
    base = LineString([(0, 0), (400, 0)])
    grid = build_terrain_grid(_hill, FAR, base.buffer(70.0), cell_m=5.0)
    spec = SectionSpec(half_width=8.0, rake_half_m=50.0, rake_step_m=4.0, ds_step_m=8.0)
    costs = {}
    for h in ("none", "c_min"):
        cfg = LatticeConfig(r_min_m=120.0, radius_multiples=(1.0, 2.0), pitch_m=15.0,
                            n_heading=16, heuristic=h)
        costs[h] = solve_lattice(grid, (0.0, 0.0, 0.0), (400.0, 0.0, 0.0), 0.0, spec, cfg).cost
    assert abs(costs["none"] - costs["c_min"]) < 1e-6 * costs["none"]
    cfg = LatticeConfig(r_min_m=120.0, radius_multiples=(1.0, 2.0), pitch_m=15.0,
                        n_heading=16, heuristic="reverse_dijkstra")
    r = solve_lattice(grid, (0.0, 0.0, 0.0), (400.0, 0.0, 0.0), 0.0, spec, cfg)
    assert r.converged and r.cost < 1.1 * costs["none"]


def test_feasibility_expansion_cap_reports_instead_of_hanging():
    base = LineString([(0, 0), (400, 0)])
    grid = build_terrain_grid(_hill, FAR, base.buffer(120.0), cell_m=4.0)
    cfg = LatticeConfig(r_min_m=120.0, pitch_m=10.0, n_heading=16, max_expand=5)
    r = solve_lattice(grid, (0.0, 0.0, 0.0), (400.0, 0.0, 0.0), 0.0, SPEC, cfg)
    assert not r.converged and r.hit_cap and r.line is None


def test_optimise_layer_reads_no_environment_and_no_paths():
    src = Path(__file__).resolve().parents[2] / "src" / "channel_designer" / "optimise"
    for py in src.glob("*.py"):
        text = py.read_text()
        assert "os.environ" not in text and "getenv(" not in text, py.name
        for node in ast.walk(ast.parse(text)):
            if isinstance(node, (ast.Import, ast.ImportFrom)):
                names = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""]
                assert not any(n.split(".")[0] in ("os", "pathlib") for n in names), py.name
