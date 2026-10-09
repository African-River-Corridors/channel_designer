"""Release guard: fail if a tracked file holds an embargoed term or a machine-local path.

    python3 tools/release_guard.py

Scans every file `git ls-files` lists, case-insensitively, for the terms in TERMS. Prints one
`file:line:term` per hit and exits 1 if any hit is not allowlisted. Standard library only.

Allowlist: `.release-guard-allow` at the repo root, one entry per line:

    <path-glob>  <term>  # reason

`<path-glob>` is matched with fnmatch against the repo-relative path. `<term>` is one of TERMS
(case-insensitive), or `*` for every term. Blank lines and lines starting with `#` are ignored.
The guard does not scan itself or the allowlist file.
"""
from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
from pathlib import Path

# term -> pattern. Plain substrings, except short words (irr and client names), which must be whole
# words so ordinary words ("losses") never need an allowlist entry.
TERMS = {
    "capex": re.compile(r"capex", re.IGNORECASE),
    "tariff": re.compile(r"tariff", re.IGNORECASE),
    "irr": re.compile(r"\birr\b", re.IGNORECASE),
    "genser": re.compile(r"genser", re.IGNORECASE),
    "offtake": re.compile(r"offtake", re.IGNORECASE),
    "pipeline crossing": re.compile(r"pipeline crossing", re.IGNORECASE),
    "osse": re.compile(r"\bosse\b", re.IGNORECASE),
    "dubri": re.compile(r"\bdubri\b", re.IGNORECASE),
    "hyde": re.compile(r"\bhyde\b", re.IGNORECASE),
    "ghana gas": re.compile(r"ghana gas", re.IGNORECASE),
    "/Users/": re.compile(r"/users/", re.IGNORECASE),
}
ALLOW_FILE = ".release-guard-allow"
SELF = "tools/release_guard.py"


def load_allow(root: Path) -> list[tuple[str, str]]:
    path = root / ALLOW_FILE
    if not path.exists():
        return []
    entries = []
    for n, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) < 2:
            sys.exit(f"{ALLOW_FILE}:{n}: need '<path-glob>  <term>  # reason'")
        glob, term = parts[0], " ".join(parts[1:]).lower()
        if term != "*" and term not in {t.lower() for t in TERMS}:
            sys.exit(f"{ALLOW_FILE}:{n}: unknown term {term!r}")
        entries.append((glob, term))
    return entries


def allowed(path: str, term: str, allow: list[tuple[str, str]]) -> bool:
    return any(fnmatch.fnmatch(path, g) and t in ("*", term.lower()) for g, t in allow)


def scan(root: Path) -> tuple[list[str], list[str]]:
    """(blocking hits, allowlisted hits), each as 'file:line:term'."""
    files = subprocess.run(["git", "ls-files", "-z"], cwd=root, check=True,
                           capture_output=True).stdout.decode().split("\0")
    allow = load_allow(root)
    hits, ok = [], []
    for rel in filter(None, files):
        if rel in (SELF, ALLOW_FILE):
            continue
        p = root / rel
        if not p.is_file():
            continue
        data = p.read_bytes()
        if b"\0" in data[:8192]:
            continue                                   # binary
        text = data.decode("utf-8", errors="replace")
        for n, line in enumerate(text.splitlines(), 1):
            for term, pat in TERMS.items():
                if pat.search(line):
                    (ok if allowed(rel, term, allow) else hits).append(f"{rel}:{n}:{term}")
    return hits, ok


def main() -> int:
    root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True,
                               capture_output=True, text=True).stdout.strip())
    hits, ok = scan(root)
    for h in hits:
        print(h)
    print(f"release guard: {len(hits)} hit(s), {len(ok)} allowlisted", file=sys.stderr)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
