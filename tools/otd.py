"""Where the pinned `otd` crate is: the Drawing Format Standard in Rust, from open-technical-drawer.

    python3 tools/otd.py        # the crate's folder (in cargo's git checkout)
    python3 tools/otd.py pin    # "<repository>@<commit>" of the standard that `otd` vendors

OWD pins `otd` by git tag in the root Cargo.toml (the workspace dependency `otd`). The vendored
standard, its PIN.json and its conformance suite live in that crate's `standard/` folder. Cargo
fetches the crate on first use. Standard library only.
"""
from __future__ import annotations

import functools
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


@functools.cache
def root() -> Path:
    """The folder of the `otd` package that Cargo.lock pins."""
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    found = [Path(p["manifest_path"]).parent for p in json.loads(out)["packages"] if p["name"] == "otd"]
    if len(found) != 1:
        raise SystemExit(f"expected one otd package in cargo metadata, found {len(found)}")
    return found[0]


def standard() -> Path:
    return root() / "standard"


def pin() -> str:
    p = json.loads((standard() / "PIN.json").read_text(encoding="utf-8"))
    return f"{p['repository']}@{p['commit']}"


if __name__ == "__main__":
    print(pin() if sys.argv[1:] == ["pin"] else root())
