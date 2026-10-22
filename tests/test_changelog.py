"""Changelog fragments (tools/changelog.py): the check, and the release step that assembles them.

Label: property. No pinned numbers: each test builds its own changelog and fragments.
"""
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import changelog as cl  # noqa: E402

HEAD = "# Changelog\n\nIntro.\n\n"
OLD = "## 0.2.0\n\n### Added\n\n- old thing\n"


def _write(folder, files):
    folder.mkdir(exist_ok=True)
    for name, text in files.items():
        (folder / name).write_text(text, encoding="utf-8")
    return folder


def test_the_repo_fragments_pass_the_check():
    assert cl.problems(cl.FRAGMENTS) == []
    assert cl.main(["check"]) == 0


@pytest.mark.parametrize("name,text,msg", [
    ("301.md", "- x\n", "name must be"),
    ("301.typo.md", "- x\n", "unknown kind"),
    ("301.fixed.md", "\n\n", "empty"),
    ("301.fixed.md", "### Fixed\n\n- x\n", "no headings"),
])
def test_the_check_fails_on_a_bad_fragment(tmp_path, name, text, msg):
    folder = _write(tmp_path / "d", {name: text})
    assert any(msg in p for p in cl.problems(folder))
    assert cl.main(["--fragments", str(folder), "check"]) == 1


def test_plain_text_becomes_one_bullet_and_a_bullet_stays_as_it_is():
    assert cl.as_entry("Line one\nline two\n") == "- Line one\n  line two"
    assert cl.as_entry("- **A.** text\n  more\n") == "- **A.** text\n  more"


def test_release_with_no_fragments_only_renames_unreleased():
    log = HEAD + "## Unreleased\n\n- a\n- b\n\n### Fixed\n\n- c\n\n" + OLD
    assert cl.assemble(log, [], "0.3.0") == log.replace("## Unreleased\n\n", "## Unreleased\n\n## 0.3.0\n\n")


def test_release_merges_unreleased_entries_and_fragments_under_the_version(tmp_path):
    folder = _write(tmp_path / "d", {
        "12.fixed.md": "- fix twelve\n",
        "3.fixed.md": "Fix three.\n",
        "3.design-impact.md": "- volumes rise\n",
        "slug.added.md": "- new feature\n",
        "README.md": "how to write a fragment\n",
    })
    log = HEAD + "## Unreleased\n\n- top\n\n### Fixed\n\n- old fix\n\n" + OLD
    out = cl.assemble(log, cl.fragments(folder), "0.3.0", "2026-11-01")
    assert out == (HEAD + "## Unreleased\n\n## 0.3.0 - 2026-11-01\n\n- top\n\n"
                   "### Fixed\n\n- old fix\n- Fix three.\n- fix twelve\n\n"
                   "### Added\n\n- new feature\n\n### Design impact\n\n- volumes rise\n\n" + OLD)


def test_release_into_a_version_heading_that_a_release_pr_already_made():
    # A release pull request moved `## Unreleased` into `## 0.3.0`; entries landed under
    # Unreleased again afterwards, and fragments too. All three end up under 0.3.0.
    log = (HEAD + "## Unreleased\n\n- late entry\n\n### Design impact\n\n- late move\n\n"
           "## 0.3.0\n\n- first\n\n### Design impact\n\n- early move\n\n" + OLD)
    frags = [(None, "fixed", "- fragment fix"), (None, "design-impact", "- fragment move")]
    out = cl.assemble(log, frags, "0.3.0")
    assert out == (HEAD + "## Unreleased\n\n## 0.3.0\n\n- first\n- late entry\n\n"
                   "### Design impact\n\n- early move\n- late move\n- fragment move\n\n"
                   "### Fixed\n\n- fragment fix\n\n" + OLD)


def test_release_command_writes_the_changelog_and_deletes_only_the_fragments(tmp_path):
    folder = _write(tmp_path / "d", {"7.changed.md": "- seven\n", "README.md": "keep me\n"})
    log = tmp_path / "CHANGELOG.md"
    log.write_text(HEAD + "## Unreleased\n\n" + OLD, encoding="utf-8")
    assert cl.main(["--fragments", str(folder), "--changelog", str(log), "release", "0.3.0"]) == 0
    assert "## 0.3.0\n\n### Changed\n\n- seven\n" in log.read_text(encoding="utf-8")
    assert [p.name for p in folder.iterdir()] == ["README.md"]


def test_release_with_nothing_to_release_fails(tmp_path):
    log = tmp_path / "CHANGELOG.md"
    log.write_text(HEAD + "## Unreleased\n\n" + OLD, encoding="utf-8")
    assert cl.main(["--fragments", str(tmp_path / "none"), "--changelog", str(log), "release", "0.3.0"]) == 1
    assert "## 0.3.0" not in log.read_text(encoding="utf-8")


def test_two_pull_requests_write_different_files():
    # The point of fragments: the file name holds the PR id, so two PRs never touch the same path.
    names = {f"{pr}.{kind}.md" for pr in (300, 301) for kind in ("fixed",)}
    assert len(names) == 2 and all(cl._NAME.match(n) for n in names)
