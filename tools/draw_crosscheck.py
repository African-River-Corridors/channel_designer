"""Cross-implementation check of the drawing crate (spec 0004, ticket 1).

For each fixture design, `owd draw --document` prints the drawing documents. Each document is run
through the Rust implementation of the Drawing Format Standard (`owd draw run`, which is `otd`) and
through the Python implementation (`technical_drawings_for_agents run`). Both must read the document,
and their normalised content must be equal (clause 16.9: numbers are compared as numbers, so 2 and
2.0 are the same).

    python3 tools/draw_crosscheck.py [--owd target/debug/owd] [--tool technical_drawings_for_agents]

The Python tool is installed at the commit that the pinned `otd` crate's standard/PIN.json records
(`python3 tools/otd.py pin`; see the `drawing-crosscheck` job in .github/workflows/ci.yml), so a
suite update moves both together. Exit 1 on any difference, and on a run that compared too few
documents to mean anything. Standard library only.

The fixtures are the web example (every stage fresh), the same design sized for a larger vessel so it
has tight bends, and the least-cost re-route design. All three need the DEM beside them, which is in
`examples/web/`.
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / "examples" / "web"
MIN_DOCUMENTS = 3  # one sheet for each fixture design, at least


def same(a, b) -> bool:
    """Clause 16.9: parsed values are equal; 2 and 2.0 are the same number."""
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    return a == b


def first_difference(a, b, path="") -> str:
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f"{path}/{k} is missing on one side"
            if not same(a[k], b[k]):
                return first_difference(a[k], b[k], f"{path}/{k}")
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            return f"{path} has {len(a)} items, then {len(b)}"
        for i, (x, y) in enumerate(zip(a, b)):
            if not same(x, y):
                return first_difference(x, y, f"{path}[{i}]")
    return f"{path}: {json.dumps(a)[:200]} != {json.dumps(b)[:200]}"


def run_json(command: list[str]) -> dict:
    proc = subprocess.run(command, capture_output=True, text=True)
    if proc.returncode != 0 or not proc.stdout.strip():
        raise SystemExit(f"FAIL: {' '.join(command)} gave exit {proc.returncode}: {proc.stderr.strip()[:500]}")
    return json.loads(proc.stdout)


def owd(owd_bin: str, *args: str, ok: tuple[int, ...] = (0,)) -> None:
    """Run an `owd` command that sets a fixture up. Exit 1 (a failed check) is fine for a design that
    is meant to have a tight bend: the design is still written."""
    proc = subprocess.run([owd_bin, *args], capture_output=True, text=True)
    if proc.returncode not in ok:
        raise SystemExit(f"FAIL: owd {' '.join(args)} gave exit {proc.returncode}: {proc.stderr.strip()[:500]}")


def fixtures(owd_bin: str, tmp: Path) -> list[Path]:
    """The designs to draw, each in a folder of its own with the DEM next to it."""
    out = []
    for name, source in (("example", "river-example.owd"), ("tight-bends", "river-example.owd"),
                         ("reroute", "river-reroute.owd")):
        folder = tmp / name
        folder.mkdir()
        shutil.copy(WEB / "river-dem.tif", folder / "river-dem.tif")
        doc = folder / f"{name}.owd"
        shutil.copy(WEB / source, doc)
        d = str(doc)
        if name == "tight-bends":
            # A larger vessel class needs a larger radius, so the line has bends that are too tight.
            owd(owd_bin, "size", d, "--rule-set", "pianc-wg141", "--set-input", "se_quality=B",
                "--vessel", "CEMT-Va")
            owd(owd_bin, "align", d, "--method", "rolling-circle", ok=(0, 1))
            owd(owd_bin, "volumes", d, ok=(0, 1))
        elif name == "reroute":
            # The stored design has stale stages: re-run the ones the sections need.
            owd(owd_bin, "align", d, "--method", "reroute", "--dem", str(folder / "river-dem.tif"),
                "--water-level", "0.0", "--water-slope", "0.0", ok=(0, 1))
            owd(owd_bin, "volumes", d, ok=(0, 1))
        out.append(doc)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--owd", default=str(ROOT / "target" / "debug" / "owd"))
    ap.add_argument("--tool", default="technical_drawings_for_agents")
    args = ap.parse_args()

    compared = 0
    problems: list[str] = []
    with tempfile.TemporaryDirectory() as tmp_name:
        tmp = Path(tmp_name)
        for fixture in fixtures(args.owd, tmp):
            # every file of every sheet, so that a document's images (the plan relief) are next to it
            sheets = tmp / f"{fixture.stem}-sheets"
            proc = subprocess.run([args.owd, "draw", str(fixture), "--out", str(sheets)], capture_output=True, text=True)
            if proc.returncode != 0:
                problems.append(f"{fixture.name}: owd draw failed: {proc.stderr.strip()[:300]}")
                continue
            for path in sorted(sheets.glob("*.drawing.json")):
                number = path.name[: -len(".drawing.json")].upper()
                rust = run_json([args.owd, "draw", "run", str(path)])
                python = run_json([args.tool, "run", str(path)])
                label = f"{fixture.name} / {number}"
                if rust["result"] != "ok" or python["result"] != "ok":
                    problems.append(f"{label}: rust {rust['result']} {rust.get('errors')}, python {python['result']} {python.get('errors')}")
                elif not same(rust["normalised"], python["normalised"]):
                    problems.append(f"{label}: normalised content differs at {first_difference(rust['normalised'], python['normalised'])}")
                else:
                    compared += 1
    for p in problems:
        print("FAIL", p)
    print(f"{compared} documents have equal normalised content in both implementations")
    if compared < MIN_DOCUMENTS and not problems:
        print(f"FAIL: only {compared} documents were compared, expected at least {MIN_DOCUMENTS}")
        return 1
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
