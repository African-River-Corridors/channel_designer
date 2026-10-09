"""Drawings extra: a gallery sheet renders from synthetic sections; without the extra the
call fails with an install hint."""
from __future__ import annotations

import builtins
import importlib
import sys

import pytest
from shapely.geometry import LineString

from channel_designer.synthetic import synthetic_river
from channel_designer.volumes.sections import section_volumes


def _volumes():
    r = synthetic_river()
    reach = LineString(list(r.centreline.coords)[150:230])       # the reach past the hill
    return section_volumes(reach, r.terrain, r.polygon, r.water_level, 3.0, 15.0,
                           model="b", maxw_m=150.0)


def test_gallery_sheet_renders(tmp_path):
    pytest.importorskip("technical_drawings_for_agents")
    from channel_designer import drawings
    vols = _volumes()
    secs = drawings.pick_sections(vols, n=4, min_gap_m=100.0)
    assert 1 <= len(secs) <= 4
    info = drawings.sheet_info(number="EX-SEC-001", title="Synthetic river — cross-sections",
                               date="2026-10-09")
    svgs = drawings.section_gallery(secs, info, dxf_path=tmp_path / "sections.dxf")
    assert svgs and all(s.startswith("<svg") and s.rstrip().endswith("</svg>") for s in svgs)
    body = "\n".join(svgs)
    assert 'class="title-block"' in body and "SECTION 1" in body
    assert (tmp_path / "sections.dxf").stat().st_size > 0


def test_missing_extra_gives_an_install_hint(monkeypatch):
    real_import = builtins.__import__

    def fake_import(name, *args, **kwargs):
        if name.startswith("technical_drawings_for_agents"):
            raise ImportError(f"No module named {name!r}")
        return real_import(name, *args, **kwargs)

    for mod in [m for m in sys.modules if m.startswith("technical_drawings_for_agents")
                or m == "channel_designer.drawings.sections"]:
        monkeypatch.delitem(sys.modules, mod)
    drawings = importlib.import_module("channel_designer.drawings")
    monkeypatch.delattr(drawings, "sections", raising=False)
    monkeypatch.setattr(builtins, "__import__", fake_import)
    with pytest.raises(ImportError, match=r"channel_designer\[drawings\]"):
        drawings.section_gallery([], None)
