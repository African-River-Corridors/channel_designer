"""The leftover-names check (tools/old_names.py) passes on the repo, and can fail.

A check that cannot fail proves nothing, so each case builds a small git tree with a planted old name.
Label: property
"""
import importlib.util
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("old_names", ROOT / "tools" / "old_names.py")
old_names = importlib.util.module_from_spec(_spec)
sys.modules["old_names"] = old_names
_spec.loader.exec_module(old_names)


def _tree(tmp_path: Path, files: dict[str, str], allowlist: str) -> tuple[Path, Path]:
    root = tmp_path / "repo"
    root.mkdir(parents=True)
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    for rel, text in files.items():
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        (root / rel).write_text(text)
    allow = tmp_path / "allow.txt"
    allow.write_text(allowlist)
    return root, allow


def test_the_repo_is_clean():
    assert old_names.check() == []
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "old_names.py")], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout


def test_each_old_name_in_an_unlisted_file_fails(tmp_path):
    for planted in ["import channel_designer", "run cdesign now", "the Channel Designer app",
                    "org.x.channeldesigner", "cargo test -p cd-core", "use cd_format::x"]:
        root, allow = _tree(tmp_path / planted.replace(" ", "_"), {"a.md": planted + "\n"}, "")
        problems = old_names.check(root, allow)
        assert len(problems) == 1 and problems[0].startswith("a.md:1:"), (planted, problems)


def test_a_new_file_that_git_has_not_seen_is_checked(tmp_path):
    root, allow = _tree(tmp_path, {"ok.md": "fine\n"}, "")
    assert old_names.check(root, allow) == []
    (root / "new.py").write_text("from channel_designer import x\n")
    assert [p.split(":")[0] for p in old_names.check(root, allow)] == ["new.py"]


def test_a_listed_file_may_hold_the_name(tmp_path):
    root, allow = _tree(tmp_path, {"shim/a.py": "import channel_designer\n"}, "shim/* | the shim\n")
    assert old_names.check(root, allow) == []


def test_similar_words_are_not_old_names(tmp_path):
    text = "ovcd-core and ovcd_core are another crate; owd-core is ours; cdesigns? no: cd-corex\n"
    root, allow = _tree(tmp_path, {"a.md": text}, "")
    assert old_names.check(root, allow) == []


def test_the_old_repo_url_is_its_own_class(tmp_path):
    url = "see https://github.com/African-River-Corridors/channel_designer/issues/1\n"
    root, allow = _tree(tmp_path, {"a.md": url}, "")
    problems = old_names.check(root, allow)
    assert len(problems) == 1 and "old repo URL" in problems[0]
    root, allow = _tree(tmp_path / "x", {"a.md": url}, "url: a.md | not updated yet\n")
    assert old_names.check(root, allow) == []
    # A name beside the URL still counts: the URL entry does not cover it.
    root, allow = _tree(tmp_path / "y", {"a.md": url + "import channel_designer\n"}, "url: a.md | x\n")
    assert [p.split(":")[1] for p in old_names.check(root, allow)] == ["2"]


def test_a_stale_entry_and_an_entry_without_a_reason_fail(tmp_path):
    root, allow = _tree(tmp_path, {"a.md": "fine\n"}, "gone/* | no such file\nb.md\n")
    problems = old_names.check(root, allow)
    assert any("matches no file" in p for p in problems)
    assert any("with a reason" in p for p in problems)


def test_the_old_import_name_is_gone():
    """The `channel_designer` import shim was removed (spec 0006, T10): the old import fails."""
    r = subprocess.run([sys.executable, "-c", "import channel_designer"], capture_output=True, text=True,
                       cwd=ROOT)
    assert r.returncode != 0
    assert "ModuleNotFoundError: No module named 'channel_designer'" in r.stderr, r.stderr
