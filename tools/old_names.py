"""Leftover-names check (spec 0006): did an old name of the tool come back?

    python tools/old_names.py

The tool was called `channel_designer` until 2026-10-21 (command `cdesign`, file extension `.cdesign`,
crates `cd-*`, app "Channel Designer"). Its names are now `owd` / Open Waterway Designer. For one release
the old names still work, so a few files must keep them: the shims and their tests, the legacy fixture,
history. This script fails when an old name shows up anywhere else.

It reads every file git knows (tracked, or new and not ignored) and looks for the old names. A file
may hold them only if `tools/old_names_allowlist.txt` says so, with a reason. The old repo URL
(`github.com/African-River-Corridors/channel_designer`) is its own class, listed with a `url:` prefix, because
those entries go when the URLs are updated after the repo rename. The check also fails on an allowlist entry that
matches no file, or has no reason, so the list cannot rot.

Exit 0 when clean, 1 otherwise.
"""
from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOWLIST = Path(__file__).with_name("old_names_allowlist.txt")

#: The old repo URL. Taken out of a line first, so the name inside it is not counted as a name.
OLD_URL = re.compile(r"github\.com/African-River-Corridors/channel_designer(?![\w-])")

#: Each old name. `cd-core` and its kin must not sit inside another word (`ovcd-core` is a different crate).
NAMES = {
    "channel_designer": re.compile(r"(?<![A-Za-z0-9])channel_designer(?![A-Za-z0-9])"),
    "cdesign": re.compile(r"(?<![A-Za-z0-9])cdesign(?![A-Za-z0-9])"),
    "Channel Designer": re.compile(r"Channel Designer"),
    "channeldesigner": re.compile(r"channeldesigner"),
    "cd-* crate": re.compile(r"(?<![A-Za-z0-9])cd[-_](?:core|format|cli|py|app)(?![A-Za-z0-9])"),
}


def read_allowlist(path: Path = ALLOWLIST) -> tuple[list[tuple[str, str]], list[tuple[str, str]], list[str]]:
    """(name entries, url entries, problems). An entry is `glob | reason`; `url:` marks a URL entry."""
    names, urls, problems = [], [], []
    for n, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        is_url = line.startswith("url:")
        if is_url:
            line = line[4:].strip()
        glob, sep, reason = (part.strip() for part in line.partition("|"))
        if not glob or not sep or not reason:
            problems.append(f"{path.name}:{n}: an entry is `glob | reason`, with a reason: {raw!r}")
            continue
        (urls if is_url else names).append((glob, reason))
    return names, urls, problems


def tracked_files(root: Path) -> list[str]:
    out = subprocess.run(["git", "ls-files", "-co", "--exclude-standard", "-z"], cwd=root,
                         capture_output=True, check=True).stdout
    return sorted(p for p in out.decode().split("\0") if p and (root / p).is_file())


def allowed(path: str, entries: list[tuple[str, str]]) -> bool:
    return any(fnmatch.fnmatchcase(path, glob) for glob, _ in entries)


def check(root: Path = ROOT, allowlist: Path = ALLOWLIST) -> list[str]:
    """Every problem, as a line of text. Empty when clean."""
    names, urls, problems = read_allowlist(allowlist)
    files = tracked_files(root)
    for kind, entries in (("name", names), ("url", urls)):
        for glob, _ in entries:
            if not any(fnmatch.fnmatchcase(f, glob) for f in files):
                problems.append(f"{allowlist.name}: the {kind} entry `{glob}` matches no file; remove it")
    for rel in files:
        try:
            text = (root / rel).read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue  # a binary file
        for lineno, line in enumerate(text.splitlines(), 1):
            bare = line
            if OLD_URL.search(line):
                if not allowed(rel, urls):
                    problems.append(f"{rel}:{lineno}: the old repo URL; use github.com/African-River-Corridors/open-waterway-designer")
                bare = OLD_URL.sub("", line)
            if allowed(rel, names):
                continue
            for label, pattern in NAMES.items():
                if pattern.search(bare):
                    problems.append(f"{rel}:{lineno}: old name `{label}`: {line.strip()[:100]}")
    return problems


def main() -> int:
    problems = check()
    for p in problems:
        print(p)
    print(f"old names: {len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
