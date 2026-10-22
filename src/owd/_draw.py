"""Drawings: ``owd.draw`` gives the same sheets as ``owd draw`` (spec 0004).

The drawing is made by the Rust core through ``owd._core`` (which draws through ``otd``, the Drawing
Format Standard in Rust). This module only reads the files and writes the result.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Dict, Iterable, Optional, Union

PathLike = Union[str, "os.PathLike[str]"]


def draw(design: PathLike, *, dem: Optional[PathLike] = None, out: Optional[PathLike] = None,
         option: Optional[str] = None, sheets: Optional[Iterable[str]] = None,
         revision: Optional[str] = None, date: Optional[str] = None,
         style: Optional[PathLike] = None,
         overrides: Optional[Dict[str, object]] = None) -> Dict[str, bytes]:
    """Draw the sheets of a design line option and return the files, by name.

    The files are, for each sheet, the drawing document (``*.drawing.json``), the normalised content
    (``*.normalised.json``), ``*.svg``, ``*.pdf`` and ``*.dxf``: the bytes ``owd draw`` writes. If
    ``out`` is given they are also written into that folder.

    ``design`` is the ``.owd`` file, after the volumes stage. ``dem`` is the DEM file; the default is the
    one the design names, next to it. ``option`` is a design line option by id or name (default the
    first); ``sheets`` the kinds to draw (``"xs"`` cross sections, ``"ts"`` typical section, ``"so"`` setting-out table and CSV, ``"pl"`` plan sheets, ``"ls"`` long section; default all); ``revision`` and ``date`` are
    for the title block (the date defaults to the day the design was created, so the same design
    always gives the same bytes).

    ``style`` is a sheet style file (JSON); the members it holds are laid over the built-in default.
    ``overrides`` sets single values of the style by dotted path, for example
    ``{"sections.spacing_m": 250}``. The hash of the style after the overrides is printed on every
    sheet.

    Raises ``ValueError`` when the design is not ready, a drawn value differs from its result, or the
    standard refuses a drawing document.
    """
    from . import _core

    path = Path(design)
    text = path.read_text(encoding="utf-8")
    dem_path = Path(dem) if dem is not None else None
    if dem_path is None:
        recorded = (json.loads(text).get("inputs", {}).get("dem") or {}).get("path")
        if recorded:
            dem_path = path.parent / recorded
    dem_bytes = dem_path.read_bytes() if dem_path is not None and dem_path.exists() else None
    options = {k: v for k, v in (("option", option), ("revision", revision), ("date", date),
                                 ("sheets", None if sheets is None else list(sheets)),
                                 ("style", None if style is None else json.loads(Path(style).read_text(encoding="utf-8"))),
                                 ("overrides", overrides)) if v is not None}
    files = dict(_core.draw(text, dem_bytes, json.dumps(options)))
    if out is not None:
        folder = Path(out)
        folder.mkdir(parents=True, exist_ok=True)
        for name, data in files.items():
            (folder / name).write_bytes(data)
    return files
