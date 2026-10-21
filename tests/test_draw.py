"""``owd.draw`` gives the same sheets as ``owd draw`` (spec 0004, ticket 1): the same files, byte for byte."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

import owd

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "examples" / "web" / "river-example.owd"


def _owd_binary() -> Path:
    """The command-line tool, built from this checkout (the same Rust code the extension holds)."""
    subprocess.run(["cargo", "build", "-q", "-p", "owd-cli"], cwd=ROOT, check=True)
    meta = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT,
                          capture_output=True, text=True, check=True).stdout
    return Path(json.loads(meta)["target_directory"]) / "debug" / "owd"


def test_draw_returns_five_files_per_sheet_and_writes_them(tmp_path):
    files = owd.draw(EXAMPLE, out=tmp_path)
    assert files, "at least one sheet"
    for name, data in files.items():
        assert (tmp_path / name).read_bytes() == data
    # a sheet's stem is that of its drawing document; a plan sheet also has its relief image
    stems = {n[: -len(".drawing.json")] for n in files if n.endswith(".drawing.json")}
    assert any(n.endswith("-relief.jpg") for n in files), "the plan sheets' relief"
    for stem in stems:
        for ext in ("drawing.json", "normalised.json", "svg", "pdf", "dxf"):
            assert f"{stem}.{ext}" in files
        assert files[f"{stem}.pdf"].startswith(b"%PDF")
        doc = json.loads(files[f"{stem}.drawing.json"])
        # plan sheets hold an image, which needs standard 1.1 (clause 18.2.1)
        version = "1.1" if "-pl-" in stem else "1.0"
        assert doc["status"] == "concept" and doc["standard_version"] == version


def test_draw_and_owd_draw_give_byte_identical_files(tmp_path):
    cli_out = tmp_path / "cli"
    subprocess.run([str(_owd_binary()), "draw", str(EXAMPLE), "--out", str(cli_out)],
                   check=True, capture_output=True)
    cli = {p.name: p.read_bytes() for p in cli_out.iterdir()}
    py = owd.draw(EXAMPLE)
    assert py == cli


def test_draw_refuses_what_it_cannot_draw(tmp_path):
    with pytest.raises(ValueError, match="no design line option `nope`"):
        owd.draw(EXAMPLE, option="nope")
    with pytest.raises(ValueError, match="unknown sheet kind `zz`"):
        owd.draw(EXAMPLE, sheets=["zz"])
    bare = tmp_path / "bare.owd"
    bare.write_text(EXAMPLE.read_text(encoding="utf-8"), encoding="utf-8")
    with pytest.raises(ValueError, match="is missing"):
        owd.draw(bare)  # the DEM is not beside this copy


def test_style_overrides_and_files_change_the_sheets_and_their_hash(tmp_path):
    def hash_note(files):
        doc = json.loads(next(v for k, v in files.items() if k.endswith(".drawing.json")))
        return next(n for n in doc["notes"] if n.startswith("SHEET STYLE"))

    base = owd.draw(EXAMPLE)
    finer = owd.draw(EXAMPLE, overrides={"sections.spacing_m": 250})
    assert hash_note(base) != hash_note(finer)
    style = tmp_path / "fine.style.json"
    style.write_text(json.dumps({"sections": {"spacing_m": 250}}), encoding="utf-8")
    assert owd.draw(EXAMPLE, style=style) == finer
    with pytest.raises(ValueError, match="/gallery/min_columns"):
        owd.draw(EXAMPLE, overrides={"gallery.min_columns": 0})
