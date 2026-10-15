"""Release guard: fail if a tracked file holds an embargoed term or a machine-local path.

    python3 tools/release_guard.py              # every file `git ls-files` lists
    python3 tools/release_guard.py --history    # every line added in `git log -p --all`,
                                                # plus commit authors and messages

Scans case-insensitively for the terms in TERMS. Prints one `where:line:term` per hit and exits 1
if any hit is not allowlisted. In history mode `where` is `<commit12>:<path>`, with
`<commit12>:(commit)` for the author, committer and message. Standard library only.

Allowlist: `.release-guard-allow` at the repo root, one entry per line:

    <path-glob>  <term>  # reason

`<path-glob>` is matched with fnmatch against the repo-relative path. `<term>` is one of TERMS
(case-insensitive), or `*` for every term. Blank lines and lines starting with `#` are ignored.
The guard does not scan itself or the allowlist file. In history mode a glob may also name one
commit, as `<commit12>:<path>`, to excuse a hit already in published history without excusing the
same text in the tree.
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


def _git(root: Path, *args: str) -> str:
    return subprocess.run(["git", "-c", "core.quotepath=off", *args], cwd=root, check=True,
                          capture_output=True).stdout.decode("utf-8", errors="replace")


def scan_history(root: Path) -> tuple[list[str], list[str]]:
    """Every added line in every commit reachable from any ref, plus each commit's author,
    committer and message. Hits as '<commit12>:<path>:<line>:<term>'."""
    allow = load_allow(root)
    hits, ok = [], []

    def check(where: str, path: str, n: int, line: str) -> None:
        for term, pat in TERMS.items():
            if pat.search(line):
                excused = allowed(path, term, allow) or allowed(where, term, allow)
                (ok if excused else hits).append(f"{where}:{n}:{term}")

    meta = _git(root, "log", "--all", "--format=%H%x00%an <%ae>%n%cn <%ce>%n%B%x01")
    for rec in filter(str.strip, meta.split("\x01")):
        sha, _, body = rec.strip("\n").partition("\x00")
        for n, line in enumerate(body.splitlines(), 1):
            check(f"{sha[:12]}:(commit)", "(commit)", n, line)

    patch = _git(root, "log", "--all", "-p", "--no-color", "--no-ext-diff", "-U0",
                 "--format=commit %H")
    sha, path, n, header = "", None, 0, False
    for raw in patch.splitlines():
        if raw.startswith("commit ") and len(raw) == 47:
            sha, path, header = raw[7:], None, False
        elif raw.startswith("diff --git "):
            path, header = None, True
        elif header and raw.startswith("+++ "):
            target = raw[4:]
            path = None if target == "/dev/null" else target[2:] if target.startswith("b/") else target
        elif raw.startswith("@@"):
            header = False
            n = int(raw.split("+", 1)[1].split()[0].split(",")[0])
        elif raw.startswith("+") and not header and path and path not in (SELF, ALLOW_FILE):
            check(f"{sha[:12]}:{path}", path, n, raw[1:])
            n += 1
    return hits, ok


def main() -> int:
    root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True,
                               capture_output=True, text=True).stdout.strip())
    history = "--history" in sys.argv[1:]
    hits, ok = scan_history(root) if history else scan(root)
    for h in hits:
        print(h)
    mode = "history" if history else "tree"
    print(f"release guard ({mode}): {len(hits)} hit(s), {len(ok)} allowlisted", file=sys.stderr)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
