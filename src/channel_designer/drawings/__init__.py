"""Drawings: cross-section gallery sheets (SVG, DXF). Needs ``channel_designer[drawings]``.

Importing this package always works. Calling a drawing function without the extra raises an
``ImportError`` that says how to install it.
"""
from __future__ import annotations

_HINT = "pip install 'channel_designer[drawings]'"


def _sections():
    try:
        from . import sections
    except ImportError as exc:
        raise ImportError(
            f"channel_designer drawings need the optional extra: {_HINT}") from exc
    return sections


def section_gallery(*args, **kwargs):
    """See :func:`channel_designer.drawings.sections.section_gallery`."""
    return _sections().section_gallery(*args, **kwargs)


def gallery_sheet(*args, **kwargs):
    """See :func:`channel_designer.drawings.sections.gallery_sheet`."""
    return _sections().gallery_sheet(*args, **kwargs)


def pick_sections(*args, **kwargs):
    """See :func:`channel_designer.drawings.sections.pick_sections`."""
    return _sections().pick_sections(*args, **kwargs)


def sheet_info(**kwargs):
    """Build a :class:`channel_designer.drawings.sections.SheetInfo`."""
    return _sections().SheetInfo(**kwargs)
