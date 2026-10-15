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


def _commit(root, message="work", author="Dev <dev@example.org>"):
    env = {"GIT_AUTHOR_NAME": author.split(" <")[0], "GIT_AUTHOR_EMAIL": author.split("<")[1][:-1],
           "GIT_COMMITTER_NAME": "Dev", "GIT_COMMITTER_EMAIL": "dev@example.org",
           "PATH": __import__("os").environ["PATH"], "HOME": str(root)}
    subprocess.run(["git", "-C", str(root), "commit", "-q", "-m", message], check=True, env=env)


def test_history_finds_a_term_that_a_later_commit_removed(tmp_path):
    root = _repo(tmp_path, {"a.md": "the capex is 3\n"})
    _commit(root)
    (root / "a.md").write_text("clean\n")
    subprocess.run(["git", "-C", str(root), "add", "."], check=True)
    _commit(root, "scrub")
    assert rg.scan(root)[0] == []                        # the tree is clean
    hits, _ = rg.scan_history(root)
    assert len(hits) == 1 and hits[0].endswith(":a.md:1:capex")


def test_history_finds_a_term_in_a_commit_message(tmp_path):
    root = _repo(tmp_path, {"a.md": "clean\n"})
    _commit(root, "note about the genser deal")
    hits, _ = rg.scan_history(root)
    assert len(hits) == 1 and hits[0].endswith(":(commit):3:genser")


def test_history_allowlist_can_name_one_commit(tmp_path):
    root = _repo(tmp_path, {"a.md": "capex\n"})
    _commit(root)
    sha = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], check=True,
                         capture_output=True, text=True).stdout.strip()[:12]
    (root / rg.ALLOW_FILE).write_text(f"{sha}:a.md  capex  # already published\n")
    hits, ok = rg.scan_history(root)
    assert hits == [] and len(ok) == 1
