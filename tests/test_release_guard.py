"""The release guard can fail: it finds a planted term, honours the allowlist, and treats irr as a word."""
import importlib.util
import subprocess
from pathlib import Path

GUARD = Path(__file__).resolve().parents[1] / "tools" / "release_guard.py"
spec = importlib.util.spec_from_file_location("release_guard", GUARD)
rg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rg)


def _repo(tmp_path, files, allow=""):
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True)
    for name, text in files.items():
        (tmp_path / name).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / name).write_text(text)
    if allow:
        (tmp_path / rg.ALLOW_FILE).write_text(allow)
    subprocess.run(["git", "-C", str(tmp_path), "add", "."], check=True)
    return tmp_path


def test_finds_planted_terms(tmp_path):
    root = _repo(tmp_path, {"a.py": "x = 1\n# CAPEX here\npath = '/Users/me/x'\n"})
    hits, _ = rg.scan(root)
    assert hits == ["a.py:2:capex", "a.py:3:/Users/"]


def test_irr_is_a_whole_word(tmp_path):
    root = _repo(tmp_path, {"a.md": "irregular mirror\nthe IRR is 12%\n"})
    hits, _ = rg.scan(root)
    assert hits == ["a.md:2:irr"]


def test_allowlist_by_glob_and_term(tmp_path):
    root = _repo(tmp_path, {"docs/a.md": "tariff\n", "b.md": "tariff\n"},
                 allow="docs/*  tariff  # reason\n")
    hits, ok = rg.scan(root)
    assert hits == ["b.md:1:tariff"] and ok == ["docs/a.md:1:tariff"]


def test_untracked_files_are_not_scanned(tmp_path):
    root = _repo(tmp_path, {"a.md": "clean\n"})
    (root / "new.md").write_text("genser\n")
    assert rg.scan(root)[0] == []
