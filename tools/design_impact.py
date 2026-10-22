"""Design-impact check: did a change move any design result?

    python tools/design_impact.py run --out results.json
    python tools/design_impact.py compare base.json head.json [--changelog CHANGELOG.md --base-changelog base/CHANGELOG.md]

`run` runs a fixed set of scenarios through the installed `owd` and writes every number
they produce to one JSON file. The scenarios use synthetic geometry built in code (the two worked
examples, a grid of Rule Set selections and vessel classes, a footprint and a Model A section run), so they need
no data files.

`compare` reads the results of the base branch and of the pull request and says what moved. A number
moved when it differs by more than the ADR 0005 tolerance (1e-9 relative and absolute, the one the
Rust port was held to before the Reference Results were deleted); anything that is not a float (a count, a name, a flag) must match exactly.
It prints a report and exits:

- 0 and "no design impact" when nothing moved;
- 0 and the list of what moved when something moved and the changelog has an entry for it;
- 1 when something moved and the changelog has no entry for it.

The entry is a new bullet under `## Unreleased` > `### Design impact` in `CHANGELOG.md`. Say what moved,
why, and which way. The check cannot judge whether the words are right; a reviewer does that.

The tolerance lives here in one place. Changing it changes what "moved" means: it needs a review.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import math
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

#: ADR 0005 (parity-two-classes-of-check): floats agree to 1e-9 relative, with 1e-9 absolute near zero.
REL_TOL = 1e-9
ABS_TOL = 1e-9


# --- the scenarios --------------------------------------------------------------------------

def _load_example(name: str):
    spec = importlib.util.spec_from_file_location(f"_example_{name}", ROOT / "examples" / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _volumes(v) -> dict:
    return {"dredge_m3": v.dredge_m3, "excav_m3": v.excav_m3, "total_m3": v.total_m3,
            "max_face_h_m": v.max_face_h_m, "n_stations": len(v.stations)}


def _line(line) -> dict:
    return {"length_m": line.length, "n_vertices": len(line.coords)}


def scenario_quickstart() -> dict:
    shape, rolled, route, results = _load_example("quickstart").main()
    return {
        "shape": {k: getattr(shape, k) for k in (
            "bottom_width_straight_m", "min_bend_radius_m", "design_depth_m",
            "bend_widening_at_rmin_m", "bottom_width_at_rmin_m")},
        "rolling_circle": {"n_arcs": rolled.n_arcs, "radius_ok": bool(rolled.radius_ok),
                           "measured_min_radius_m": rolled.measured_min_radius_m,
                           **_line(rolled.centreline)},
        "route": {"saving_pct": route.approx_saving_pct, "min_radius_m": route.min_radius_m,
                  "corridor_half_m": route.corridor_half_m, "radius_ok": bool(route.radius_ok),
                  **_line(route.line)},
        "volumes": {name: _volumes(v) for name, v in results.items()},
    }


def scenario_optimise_pools() -> dict:
    best, out, results = _load_example("optimise_pools").main()
    return {
        "weirs": [{"chainage_m": w.chainage_m, "pool_level_m": w.pool_level_m, "lift_m": w.lift_m}
                  for w in best.weirs],
        "total_cost": best.total_cost,
        "n_replaced": out.n_replaced,
        "pools": [{"start_m": p.start_m, "end_m": p.end_m, "converged": bool(p.result.converged),
                   "cost": p.result.cost, "baseline_cost": p.baseline_cost,
                   "n_primitives": len(p.result.prim_seq)} for p in out.pools],
        "volumes": {name: _volumes(v) for name, v in results.items()},
    }


# The six channel standards of OWD 0.2 by their old names, as Rule Set selections (spec 0007): the
# keys of this scenario stay the same, so a base branch from before the Rule Sets compares with it.
_OLD_NAMES = {
    "PIANC-A": dict(rule_sets=["pianc-wg141"], set_inputs={"se_quality": "A"}),
    "PIANC-B": dict(rule_sets=["pianc-wg141"], set_inputs={"se_quality": "B"}),
    "PIANC-C": dict(rule_sets=["pianc-wg141"], set_inputs={"se_quality": "C"}),
    "CN": dict(rule_sets=["gb50139"], set_inputs={"range_end": "permissive"}),
    "CN-conservative": dict(rule_sets=["gb50139"], set_inputs={"range_end": "strict"}),
    "CN-reduced": dict(rule_sets=["gb50139"], set_inputs={"range_end": "strict"},
                       reaches=[{"waterway": "canal", "reduced_radius_claim": "yes"}]),
}


def scenario_rulesets() -> dict:
    from owd.rules import channel_shape
    from vessel_designer.core.vessel_standards import VESSEL_STANDARDS
    out = {}
    for rid in sorted(_OLD_NAMES):
        for vid in sorted(VESSEL_STANDARDS):
            s = channel_shape(vid, **_OLD_NAMES[rid])
            out[f"{rid}/{vid}"] = {
                "bottom_width_straight_m": s.bottom_width_straight_m, "min_bend_radius_m": s.min_bend_radius_m,
                "design_depth_m": s.design_depth_m, "bend_widening_at_rmin_m": s.bend_widening_at_rmin_m,
                "width_basis": s.width_basis, "n_warnings": len(s.warnings)}
    return out


def scenario_size_and_design() -> dict:
    from owd.rules.design import ChannelDesignInputs, compute
    from owd.rules.rulesets import ChannelRuleInputs, size_channel
    from owd.synthetic import synthetic_river
    r = size_channel(ChannelRuleInputs(beam_m=9.5, loa_m=85.0, draft_m=2.5, min_water_level_m=10.0))
    d = compute(ChannelDesignInputs(total_loa=85.0, beam=9.5,
                                    centreline_points=list(synthetic_river().centreline.coords)))
    return {
        "size_channel": {k: getattr(r, k) for k in (
            "straight_bottom_width_m", "min_turn_radius_m", "bend_bottom_width_m", "required_depth_m",
            "top_width_straight_m")} | {"ok": bool(r.ok)},
        "compute": d.summary(),
    }


def scenario_footprint_and_model_a() -> dict:
    from owd.synthetic import synthetic_river
    from owd.volumes.footprint import channel_footprint
    from owd.volumes.sections import section_volumes
    river = synthetic_river()
    out = {}
    for beam in (8.0, 16.0):
        f = channel_footprint(river.centreline, river.left_bank, river.right_bank, loa=80.0, beam=beam, depth=3.0)
        out[f"footprint_beam{beam:g}"] = {"excavation_area_m2": f.total_excavation_area_m2,
                                          "n_polygons": len(f.excavation)}
    v = section_volumes(river.centreline, river.terrain, river.polygon, river.water_level, 3.25, 15.0,
                        model="a", maxw_m=150.0)
    out["model_a_centreline"] = _volumes(v)
    return out


SCENARIOS = {
    "quickstart": scenario_quickstart,
    "optimise_pools": scenario_optimise_pools,
    "rulesets": scenario_rulesets,
    "size_and_design": scenario_size_and_design,
    "footprint_and_model_a": scenario_footprint_and_model_a,
}


# --- flatten, compare ----------------------------------------------------------------------

def _scalar(x):
    if isinstance(x, float):
        if math.isnan(x):
            return "nan"
        if math.isinf(x):
            return "inf" if x > 0 else "-inf"
    return x


def flatten(value, prefix: str = "") -> dict:
    """Nested results as one flat `path -> scalar` mapping."""
    if isinstance(value, dict):
        out = {}
        for k, v in value.items():
            out.update(flatten(v, f"{prefix}/{k}" if prefix else str(k)))
        return out
    if isinstance(value, (list, tuple)):
        out = {}
        for i, v in enumerate(value):
            out.update(flatten(v, f"{prefix}[{i}]"))
        if not value:
            out[prefix + "[]"] = 0
        return out
    return {prefix: _scalar(value)}


def run() -> dict:
    return {name: flatten(fn()) for name, fn in SCENARIOS.items()}


def moved(a, b) -> bool:
    """True when two flat values differ by more than the tolerance (floats) or at all (the rest)."""
    if isinstance(a, bool) or isinstance(b, bool) or isinstance(a, str) or isinstance(b, str):
        return a != b
    if a is None or b is None:
        return a is not b
    fa, fb = float(a), float(b)
    if (isinstance(a, int) and isinstance(b, int)):
        return a != b
    return abs(fa - fb) > max(ABS_TOL, REL_TOL * max(abs(fa), abs(fb)))


def differences(base: dict, head: dict) -> list:
    """`(scenario, key, base, head)` for every value that moved, appeared or vanished."""
    out = []
    for scen in sorted(set(base) | set(head)):
        b, h = base.get(scen), head.get(scen)
        if b is None or h is None:
            out.append((scen, "(whole scenario)", "present" if b is not None else "absent",
                        "present" if h is not None else "absent"))
            continue
        for key in sorted(set(b) | set(h)):
            if key not in b or key not in h:
                out.append((scen, key, b.get(key, "(absent)"), h.get(key, "(absent)")))
            elif moved(b[key], h[key]):
                out.append((scen, key, b[key], h[key]))
    return out


# --- the changelog rule --------------------------------------------------------------------

def design_impact_entries(changelog: str) -> list:
    """The bullets under `## Unreleased` > `### Design impact`."""
    m = re.search(r"^## Unreleased[^\n]*\n(.*?)(?=^## |\Z)", changelog, re.S | re.M)
    if not m:
        return []
    sec = re.search(r"^### Design impact[^\n]*\n(.*?)(?=^### |\Z)", m.group(1), re.S | re.M)
    if not sec:
        return []
    return [ln.strip() for ln in sec.group(1).splitlines() if re.match(r"\s*[-*] \S", ln)]


def new_entries(head_changelog: str, base_changelog: str) -> list:
    base = set(design_impact_entries(base_changelog))
    return [e for e in design_impact_entries(head_changelog) if e not in base]


# --- report --------------------------------------------------------------------------------

def _fmt(x) -> str:
    return f"{x:.10g}" if isinstance(x, float) else str(x)


def report(diffs: list, entries: list) -> tuple:
    """`(text, exit_code)`."""
    if not diffs:
        return "## Design impact\n\nno design impact: every result is the same as on the base branch " \
               f"(to {REL_TOL:g} relative).\n", 0
    lines = ["## Design impact", "", f"{len(diffs)} result(s) moved against the base branch.", ""]
    by_scen: dict = {}
    for s, k, b, h in diffs:
        by_scen.setdefault(s, []).append((k, b, h))
    lines += ["| scenario | result | base | this PR | change |", "|---|---|---|---|---|"]
    shown = 0
    for s, rows in by_scen.items():
        for k, b, h in rows:
            if shown >= 60:
                break
            try:
                change = f"{(float(h) - float(b)) / float(b) * 100:+.4g} %" if float(b) else f"{float(h) - float(b):+.4g}"
            except (TypeError, ValueError):
                change = ""
            lines.append(f"| {s} | `{k}` | {_fmt(b)} | {_fmt(h)} | {change} |")
            shown += 1
    if len(diffs) > shown:
        lines.append(f"\n... and {len(diffs) - shown} more.")
    lines.append("")
    if entries:
        lines += ["The changelog has a design-impact entry for it:", ""] + [f"- {e.lstrip('-* ')}" for e in entries] + [""]
        return "\n".join(lines) + "\n", 0
    lines += ["**The changelog has no entry for this.** Add a bullet under `## Unreleased` > `### Design impact` "
              "in `CHANGELOG.md` that says what moved and why, or fix the change so nothing moves.", ""]
    return "\n".join(lines) + "\n", 1


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    c = sub.add_parser("compare")
    c.add_argument("base")
    c.add_argument("head")
    c.add_argument("--changelog", default=str(ROOT / "CHANGELOG.md"))
    c.add_argument("--base-changelog", default=None)
    a = ap.parse_args(argv)
    if a.cmd == "run":
        Path(a.out).write_text(json.dumps(run(), indent=1, sort_keys=True) + "\n", encoding="utf-8")
        return 0
    base = json.loads(Path(a.base).read_text(encoding="utf-8"))
    head = json.loads(Path(a.head).read_text(encoding="utf-8"))
    head_log = Path(a.changelog).read_text(encoding="utf-8")
    base_log = Path(a.base_changelog).read_text(encoding="utf-8") if a.base_changelog else ""
    text, code = report(differences(base, head), new_entries(head_log, base_log))
    sys.stdout.write(text)
    return code


if __name__ == "__main__":
    sys.exit(main())
