"""Changelog fragments: one file per pull request, assembled into CHANGELOG.md at release time.

    python3 tools/changelog.py check                     # every fragment is well formed
    python3 tools/changelog.py preview                   # print what a release would add
    python3 tools/changelog.py release 0.4.0 [--date D]  # assemble into CHANGELOG.md, delete the fragments

A pull request does not edit `CHANGELOG.md`. It adds a fragment, `changelog.d/<id>.<kind>.md`:

- `<id>` is the issue or pull request number (`294`), or a short slug when there is neither;
- `<kind>` is one of the keys of `KINDS` below (`added`, `changed`, `fixed`, `design-impact`, ...).

The fragment holds one entry in Markdown: a bullet (`- **Title.** text...`) or plain text, which becomes a
bullet. One pull request may add several fragments, one per kind. Two pull requests never write the same
file, so they never conflict.

`release` moves the entries under `## Unreleased` (if any are still there) and every fragment under a new
`## <version>` heading, or into that heading's section when a release pull request already made it. The
entries keep their order: first those already in `CHANGELOG.md`, then the fragments by id. Each kind goes
under its `### <heading>`. `## Unreleased` stays, empty. Then it deletes the fragments.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FRAGMENTS = ROOT / "changelog.d"
CHANGELOG = ROOT / "CHANGELOG.md"

#: Fragment kind -> the `###` heading it goes under, in the order new headings are added.
KINDS = {
    "added": "Added",
    "changed": "Changed",
    "deprecated": "Deprecated",
    "removed": "Removed",
    "fixed": "Fixed",
    "security": "Security",
    "design-impact": "Design impact",
}

#: Files in changelog.d that are not fragments.
NOT_FRAGMENTS = {"README.md", ".gitkeep"}

_NAME = re.compile(r"^(?P<id>[A-Za-z0-9][A-Za-z0-9_+-]*)\.(?P<kind>[a-z-]+)\.md$")


# --- fragments ------------------------------------------------------------------------------------

def as_entry(text: str) -> str:
    """A fragment's text as Markdown list item(s): plain text becomes one bullet."""
    text = text.strip("\n").rstrip()
    lines = [ln.rstrip() for ln in text.splitlines()]
    while lines and not lines[0].strip():
        lines.pop(0)
    if not lines:
        return ""
    if re.match(r"\s*[-*] \S", lines[0]):
        return "\n".join(lines)
    return "\n".join(["- " + lines[0].strip()] + [("  " + ln.strip()) if ln.strip() else "" for ln in lines[1:]])


def _sort_key(fid: str):
    return (0, int(fid), "") if fid.isdigit() else (1, 0, fid.lower())


def fragment_files(folder: Path) -> list:
    """Every file in the folder that should be a fragment, sorted by id then kind."""
    if not folder.is_dir():
        return []
    files = [p for p in folder.iterdir() if p.is_file() and p.name not in NOT_FRAGMENTS]
    def key(p):
        m = _NAME.match(p.name)
        return (_sort_key(m.group("id")), m.group("kind")) if m else ((2, 0, p.name), "")
    return sorted(files, key=key)


def problems(folder: Path) -> list:
    """What is wrong with the fragments, as messages (empty when all is well)."""
    out = []
    for p in fragment_files(folder):
        m = _NAME.match(p.name)
        if not m:
            out.append(f"{p.name}: name must be <id>.<kind>.md, such as 294.fixed.md")
            continue
        if m.group("kind") not in KINDS:
            out.append(f"{p.name}: unknown kind '{m.group('kind')}' (one of: {', '.join(KINDS)})")
            continue
        text = p.read_text(encoding="utf-8")
        if not as_entry(text):
            out.append(f"{p.name}: empty")
        elif re.search(r"^#", text, re.M):
            out.append(f"{p.name}: no headings in a fragment; the release step adds them")
    return out


def fragments(folder: Path) -> list:
    """`(path, kind, entry)` for every fragment, in release order. Raises on a bad fragment."""
    bad = problems(folder)
    if bad:
        raise ValueError("bad changelog fragment(s):\n  " + "\n  ".join(bad))
    out = []
    for p in fragment_files(folder):
        out.append((p, _NAME.match(p.name).group("kind"), as_entry(p.read_text(encoding="utf-8"))))
    return out


# --- CHANGELOG.md sections ------------------------------------------------------------------------

def _split_level2(text: str) -> list:
    """`[(heading_line or None, body)]`: the text before the first `## ` and each `## ` section."""
    parts = re.split(r"(?m)^(## .*)\n", text)
    out = [(None, parts[0])]
    for i in range(1, len(parts), 2):
        out.append((parts[i], parts[i + 1]))
    return out


def _split_level3(body: str) -> tuple:
    """`(preamble lines, [(name, lines)])` of one `## ` section; blank edges trimmed."""
    parts = re.split(r"(?m)^### (.*)\n", body)
    def trim(s):
        lines = s.split("\n")
        while lines and not lines[0].strip():
            lines.pop(0)
        while lines and not lines[-1].strip():
            lines.pop()
        return lines
    return trim(parts[0]), [(parts[i].strip(), trim(parts[i + 1])) for i in range(1, len(parts), 2)]


def _merge(into: tuple, add: tuple) -> tuple:
    pre, subs = list(into[0]), [(n, list(ls)) for n, ls in into[1]]
    if add[0]:
        pre += ([""] if pre and _is_para_end(pre, add[0]) else []) + add[0]
    for name, lines in add[1]:
        if not lines:
            continue
        for n, ls in subs:
            if n.lower() == name.lower():
                ls += ([""] if ls and _is_para_end(ls, lines) else []) + lines
                break
        else:
            subs.append((name, list(lines)))
    return pre, subs


def _is_para_end(have: list, more: list) -> bool:
    """A blank line between two blocks unless both are plain bullet lists (keep the list tight)."""
    return not (re.match(r"[-*] ", more[0]) and re.match(r"\s*[-*] |\s+\S", have[-1]))


def _render(section: tuple) -> str:
    pre, subs = section
    blocks = []
    if pre:
        blocks.append("\n".join(pre))
    for name, lines in subs:
        if lines:
            blocks.append(f"### {name}\n\n" + "\n".join(lines))
    return "\n\n".join(blocks)


def assemble(changelog: str, frags: list, version: str, date: str | None = None) -> str:
    """The new CHANGELOG.md text: `## Unreleased` and the fragments under `## <version>`."""
    sections = _split_level2(changelog)
    idx_unrel = next((i for i, (h, _) in enumerate(sections) if h and re.match(r"## Unreleased\b", h)), None)
    if idx_unrel is None:
        raise ValueError("CHANGELOG.md has no '## Unreleased' heading")
    ver_re = re.compile(rf"## \[?{re.escape(version)}\]?(\s|$)")
    idx_ver = next((i for i, (h, _) in enumerate(sections) if h and ver_re.match(h)), None)

    unreleased = _split_level3(sections[idx_unrel][1])
    by_kind: dict = {}
    for _, kind, entry in frags:
        by_kind.setdefault(kind, []).append(entry)
    from_frags = ([], [(KINDS[k], "\n".join(by_kind[k]).split("\n")) for k in KINDS if k in by_kind])

    if idx_ver is not None:
        body = _merge(_merge(_split_level3(sections[idx_ver][1]), unreleased), from_frags)
        heading = sections[idx_ver][0]
    else:
        body = _merge(unreleased, from_frags)
        heading = f"## {version}" + (f" - {date}" if date else "")
    if not body[0] and not any(ls for _, ls in body[1]):
        raise ValueError("nothing to release: '## Unreleased' is empty and there are no fragments")

    out = []
    for i, (h, b) in enumerate(sections):
        if i == idx_ver:
            continue
        if h is None:
            out.append(b)
        elif i == idx_unrel:
            out.append(f"{h}\n\n{heading}\n\n{_render(body)}\n\n")
        else:
            out.append(f"{h}\n{b}")
    return "".join(out).rstrip("\n") + "\n"


# --- command line ---------------------------------------------------------------------------------

def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--fragments", default=str(FRAGMENTS), help="the fragment folder (default changelog.d)")
    ap.add_argument("--changelog", default=str(CHANGELOG))
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check", help="fail if any fragment is malformed")
    pv = sub.add_parser("preview", help="print the release section the fragments would make")
    pv.add_argument("--version", default="X.Y.Z")
    r = sub.add_parser("release", help="assemble the fragments into CHANGELOG.md and delete them")
    r.add_argument("version")
    r.add_argument("--date", default=None, help="written after the version, as '## 0.4.0 - 2026-11-01'")
    a = ap.parse_args(argv)
    folder, log = Path(a.fragments), Path(a.changelog)

    if a.cmd == "check":
        bad = problems(folder)
        for b in bad:
            print(f"changelog: {b}", file=sys.stderr)
        n = len(fragment_files(folder))
        print(f"changelog: {n} fragment(s), {len(bad)} problem(s)")
        return 1 if bad else 0
    try:
        frags = fragments(folder)
        new = assemble(log.read_text(encoding="utf-8"), frags, a.version, getattr(a, "date", None))
    except ValueError as e:
        print(f"changelog: {e}", file=sys.stderr)
        return 1
    if a.cmd == "preview":
        sec = _split_level2(new)
        ver = next(i for i, (h, _) in enumerate(sec) if h and h.startswith(f"## {a.version}"))
        sys.stdout.write(f"{sec[ver][0]}\n{sec[ver][1]}")
        return 0
    log.write_text(new, encoding="utf-8")
    for p, _, _ in frags:
        p.unlink()
    print(f"changelog: {len(frags)} fragment(s) moved into {log.name} under {a.version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
