"""The design-impact check: what counts as moved, what counts as a changelog entry, and the exit code.

Label: property. No pinned numbers: each test builds its own inputs.
"""
import copy
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import design_impact as di  # noqa: E402

BASE = {"s": {"vol": 1000.0, "n": 3, "name": "a", "r": 255.0}}
LOG_NONE = "# Changelog\n\n## Unreleased\n\n### Design impact\n\nNothing yet.\n\n## 0.2.0\n\n- x\n"
LOG_ENTRY = LOG_NONE.replace("Nothing yet.", "- Cut Model B volume rises 0.5 % because the berm is 2 m.")


def test_last_bit_noise_is_not_a_move_but_a_real_change_is():
    head = copy.deepcopy(BASE)
    head["s"]["vol"] = math.nextafter(1000.0, math.inf)
    assert di.differences(BASE, head) == []
    head["s"]["vol"] = 1000.0 * (1 + 1e-6)
    assert [d[1] for d in di.differences(BASE, head)] == ["vol"]


def test_counts_and_names_must_match_exactly():
    for key, bad in (("n", 4), ("name", "b")):
        head = copy.deepcopy(BASE)
        head["s"][key] = bad
        assert [d[1] for d in di.differences(BASE, head)] == [key]


def test_a_vanished_or_new_result_is_a_move():
    head = copy.deepcopy(BASE)
    del head["s"]["r"]
    head["s"]["extra"] = 1
    assert sorted(d[1] for d in di.differences(BASE, head)) == ["extra", "r"]
    assert di.differences(BASE, {})[0][1] == "(whole scenario)"


def test_a_pr_that_moves_nothing_says_so_and_passes():
    text, code = di.report(di.differences(BASE, copy.deepcopy(BASE)), [])
    assert code == 0 and "no design impact" in text


def test_a_move_with_no_changelog_entry_fails():
    head = copy.deepcopy(BASE)
    head["s"]["vol"] *= 1.01
    text, code = di.report(di.differences(BASE, head), di.new_entries(LOG_NONE, LOG_NONE))
    assert code == 1 and "no entry" in text and "vol" in text


def test_the_same_move_with_an_entry_passes_and_reports_what_moved():
    head = copy.deepcopy(BASE)
    head["s"]["vol"] *= 1.01
    text, code = di.report(di.differences(BASE, head), di.new_entries(LOG_ENTRY, LOG_NONE))
    assert code == 0 and "`vol`" in text and "1000" in text and "+1 %" in text and "berm" in text


def test_an_entry_already_on_the_base_does_not_cover_a_new_move():
    assert di.new_entries(LOG_ENTRY, LOG_ENTRY) == []


def test_a_bullet_elsewhere_in_the_changelog_is_not_an_entry():
    log = LOG_NONE.replace("- x", "- a design change described in the wrong place")
    assert di.new_entries(log, LOG_NONE) == []


def test_the_scenarios_are_deterministic():
    one, two = di.run(), di.run()
    assert di.differences(one, two) == []
    assert sum(len(v) for v in one.values()) > 100


# --- the entry as a changelog fragment (changelog.d/<id>.design-impact.md) --------------------------

def _frags(tmp_path, name, files):
    folder = tmp_path / name
    folder.mkdir()
    for fname, text in files.items():
        (folder / fname).write_text(text, encoding="utf-8")
    return folder


def test_a_design_impact_fragment_is_an_entry(tmp_path):
    head = copy.deepcopy(BASE)
    head["s"]["vol"] *= 1.01
    base_f = _frags(tmp_path, "base", {})
    head_f = _frags(tmp_path, "head", {"301.design-impact.md": "Cut Model B volume rises 1 % (berm).\n"})
    text, code = di.report(di.differences(BASE, head), di.new_entries(LOG_NONE, LOG_NONE, head_f, base_f))
    assert code == 0 and "berm" in text


def test_a_fragment_of_another_kind_is_not_a_design_impact_entry(tmp_path):
    head = copy.deepcopy(BASE)
    head["s"]["vol"] *= 1.01
    head_f = _frags(tmp_path, "head", {"301.changed.md": "- Cut Model B volume rises 1 %.\n"})
    text, code = di.report(di.differences(BASE, head), di.new_entries(LOG_NONE, LOG_NONE, head_f, None))
    assert code == 1 and "changelog.d/<pr>.design-impact.md" in text


def test_a_fragment_already_on_the_base_does_not_cover_a_new_move(tmp_path):
    files = {"290.design-impact.md": "- An older move.\n"}
    base_f, head_f = _frags(tmp_path, "base", files), _frags(tmp_path, "head", files)
    assert di.new_entries(LOG_NONE, LOG_NONE, head_f, base_f) == []


def test_the_compare_command_reads_the_fragments_next_to_each_changelog(tmp_path):
    import json
    head = copy.deepcopy(BASE)
    head["s"]["vol"] *= 1.01
    for side in ("base", "head"):
        (tmp_path / side).mkdir()
        (tmp_path / side / "CHANGELOG.md").write_text(LOG_NONE, encoding="utf-8")
    (tmp_path / "base.json").write_text(json.dumps(BASE))
    (tmp_path / "head.json").write_text(json.dumps(head))
    args = [str(tmp_path / "base.json"), str(tmp_path / "head.json"),
            "--changelog", str(tmp_path / "head" / "CHANGELOG.md"),
            "--base-changelog", str(tmp_path / "base" / "CHANGELOG.md")]
    assert di.main(["compare"] + args) == 1
    (tmp_path / "head" / "changelog.d").mkdir()
    (tmp_path / "head" / "changelog.d" / "301.design-impact.md").write_text("- vol rises 1 %.\n")
    assert di.main(["compare"] + args) == 0
