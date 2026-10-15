"""The committed parity cases: complete, deterministic, and honest about search ties.

Spec 0001, ticket 23. Risk stopped: an entry point with no cases (so a port could ship untested), a
generator whose output moves between runs (so the oracle guard cannot tell a change from noise), and a
search case whose near tie is judged after a failure instead of recorded up front.
"""
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
PARITY = ROOT / "parity"


def _load(modname, relpath):
    spec = importlib.util.spec_from_file_location(modname, PARITY / relpath)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture(scope="module")
def cov():
    sys.path.insert(0, str(ROOT))
    import parity.coverage as coverage
    return coverage


@pytest.fixture(scope="module")
def docs(cov):
    return cov.load()


def test_every_python_module_with_public_functions_has_an_entry_point(docs):
    """One entry point per module of the frozen package (drawings are spec 0004's)."""
    frozen = PARITY / "frozen_python" / "channel_designer_frozen"
    modules = {p.stem for sub in ("rules", "alignment", "volumes", "optimise", "terrain")
               for p in (frozen / sub).glob("*.py") if p.stem != "__init__"}
    modules |= {"synthetic"}
    named = {"profile": "terrain_profile"}
    wanted = {named.get(m, m) for m in modules}
    assert wanted <= set(docs) | {"cut_model_extras"}, sorted(wanted - set(docs))


def test_every_entry_point_has_tiers_one_and_two_and_tier_three_or_a_reason(docs, cov):
    for ep, doc in docs.items():
        tiers = {c["tier"] for c in doc["cases"]}
        assert {1, 2} <= tiers, f"{ep}: tiers {sorted(tiers)}"
        if 3 not in tiers:
            assert cov.NO_ERROR_CASES.get(ep), f"{ep} has no error cases and no stated reason"
        else:
            assert ep not in cov.NO_ERROR_CASES, f"{ep} has error cases, so it needs no reason"


def test_cases_have_unique_ids_reasons_and_known_tiers(docs):
    for ep, doc in docs.items():
        ids = [c["id"] for c in doc["cases"]]
        assert len(ids) == len(set(ids)), ep
        for c in doc["cases"]:
            assert c["tier"] in (1, 2, 3) and c["reason"].strip(), (ep, c["id"])
            assert isinstance(c["input"]["fn"], str) and "args" in c["input"], (ep, c["id"])


def test_error_cases_all_error_and_the_others_do_not():
    """Tier 3 must really be refused by the frozen Python; tiers 1 and 2 must not be."""
    ref = {p.stem: json.loads(p.read_text()) for p in (PARITY / "reference").glob("*.json")}
    docs_ = {p.stem: json.loads(p.read_text()) for p in (PARITY / "cases").glob("*.json")}
    bad = []
    for ep, doc in docs_.items():
        out = {r["id"]: r["output"] for r in ref[ep]["results"]}
        for c in doc["cases"]:
            is_error = "error" in out[c["id"]]
            if is_error != (c["tier"] == 3):
                bad.append(f"{ep} {c['id']}: tier {c['tier']} but error={is_error}")
    assert not bad, bad


def test_search_cases_store_the_closest_call_and_the_written_down_threshold(docs):
    common = _load("parity_common", "common.py")
    n = 0
    sys.path.insert(0, str(ROOT))
    from parity.modules import registry
    for ep, mod in registry().items():
        searches = getattr(mod, "SEARCH_FNS", ())
        for c in docs[ep]["cases"]:
            if c["input"]["fn"] in searches and c["tier"] != 3:
                s = c["search"]
                n += 1
                assert s["near_tie_rel"] == common.NEAR_TIE_REL == 1e-6
                assert isinstance(s["near_tie"], bool) and s["decisions"] >= 0
                if s["runner_up"] is None:
                    assert s["near_tie"] is False and s["gap_rel"] is None
                else:
                    assert s["near_tie"] == (s["gap_rel"] <= common.NEAR_TIE_REL)
            else:
                assert "search" not in c, (ep, c["id"])
    assert n > 50, n


def test_the_search_entry_points_are_the_four_named_in_the_adr_and_the_dp_and_the_cascade(docs):
    sys.path.insert(0, str(ROOT))
    from parity.modules import registry
    have = {ep for ep, m in registry().items() if getattr(m, "SEARCH_FNS", None)}
    assert have == {"rolling_circle", "route", "cascade", "lattice"}


def test_near_tie_helper_flags_at_the_threshold_and_not_above():
    common = _load("parity_common2", "common.py")
    assert common.search_meta(100.0, 100.0 * (1 + 0.9e-6), 3, "x")["near_tie"] is True
    assert common.search_meta(100.0, 100.0 * (1 + 1.1e-6), 3, "x")["near_tie"] is False
    assert common.search_meta(0.0, 1e-7, 3, "x")["near_tie"] is True        # absolute near zero
    assert common.search_meta(5.0, None, 0, "x")["near_tie"] is False


def test_regenerating_twice_gives_byte_identical_reference_results_and_matches_the_committed_ones(tmp_path):
    outs = []
    for name in ("a", "b"):
        out = tmp_path / name
        subprocess.run([sys.executable, str(PARITY / "driver.py"), str(out)], check=True, cwd=ROOT)
        outs.append(out)
    names = sorted(p.name for p in outs[0].glob("*.json"))
    assert names == sorted(p.name for p in (PARITY / "reference").glob("*.json"))
    for n in names:
        a, b = (outs[0] / n).read_bytes(), (outs[1] / n).read_bytes()
        assert a == b, f"{n}: two runs differ"
        assert a == (PARITY / "reference" / n).read_bytes(), f"{n}: differs from the committed file"


def test_the_committed_cases_are_what_the_generator_writes(tmp_path):
    code = (
        "import sys, json; sys.path.insert(0, sys.argv[1]);"
        "from parity.gen_cases import build; from parity.modules import registry; from parity.common import dumps;"
        "import pathlib;"
        "out = pathlib.Path(sys.argv[2]);"
        "[(out / f'{ep}.json').write_text(dumps(build(ep, m)), encoding='utf-8', newline='\\n') "
        " for ep, m in registry().items()]"
    )
    subprocess.run([sys.executable, "-c", code, str(ROOT), str(tmp_path)], check=True, cwd=ROOT)
    for p in sorted(tmp_path.glob("*.json")):
        assert p.read_bytes() == (PARITY / "cases" / p.name).read_bytes(), f"{p.name} is not what gen_cases writes"
