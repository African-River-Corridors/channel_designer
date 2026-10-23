"""Check that the package can go to PyPI: no direct (URL or git) dependencies once PyPI is on.

PyPI refuses a package whose metadata names a dependency by URL (`name @ git+https://...`); 0.3.0's
upload failed on exactly that. PyPI publishing is off before v0.4 (release.yml), so before 0.4 a
direct dependency is only reported. From 0.4 it fails, so the release check goes red before the tag,
not after the upload.

Usage: python tools/pypi_ready.py [pyproject.toml]. Exit 1 when the package is 0.4 or later and has a
direct dependency.
"""
import re
import sys
import tomllib
from pathlib import Path

PYPI_FROM = (0, 4)


def direct_dependencies(project: dict) -> list[str]:
    deps = list(project.get("dependencies", []))
    for extra in project.get("optional-dependencies", {}).values():
        deps.extend(extra)
    return [d for d in deps if " @ " in d or re.search(r"\b(git\+|https?://|file:)", d)]


def version_tuple(version: str) -> tuple[int, ...]:
    return tuple(int(p) for p in re.findall(r"\d+", version)[:3])


def check(pyproject: Path) -> tuple[bool, list[str]]:
    project = tomllib.loads(pyproject.read_text(encoding="utf-8"))["project"]
    direct = direct_dependencies(project)
    pypi_on = version_tuple(project["version"])[:2] >= PYPI_FROM
    return (not (pypi_on and direct)), direct


def main(argv: list[str]) -> int:
    path = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parents[1] / "pyproject.toml"
    ok, direct = check(path)
    for d in direct:
        print(f"{'error' if not ok else 'note'}: direct dependency PyPI refuses: {d}")
    if not ok:
        print("PyPI publishing is on from 0.4: take these dependencies from PyPI first.")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
