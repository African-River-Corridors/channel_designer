"""One module per parity entry point. Each defines `ENTRY_POINT`, `adapter(fn, args)` (calls the
frozen Python) and `cases()` (the case list). The registry finds them by listing this folder."""
from __future__ import annotations

import importlib
import pkgutil


def registry() -> dict:
    out = {}
    for info in sorted(pkgutil.iter_modules(__path__), key=lambda i: i.name):
        mod = importlib.import_module(f"{__name__}.{info.name}")
        out[mod.ENTRY_POINT] = mod
    return out
