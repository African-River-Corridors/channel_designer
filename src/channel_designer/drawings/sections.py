"""Cross-section gallery sheets (SVG, optional DXF) for a channel's cut.

Every geometric quantity comes from :mod:`channel_designer.volumes` — this module only
lays out and draws. Furniture (paper-space sheet, frame, scale bars, ISO 7200 title block,
status watermark, hatches, DXF) comes from ``technical_drawings_for_agents``.

Two generic pieces from the original section pack:

- :class:`PanelSheet` — a paper-space sheet with SEVERAL viewports, each at its own derived
  scale and scale bar, under one frame, watermark and title block.
- :func:`pack_gallery` — greedy packing of variable-height section panels at ONE common scale
  so sections compare.

Needs ``channel_designer[drawings]``.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import List, Optional, Sequence

import numpy as np

try:
    from technical_drawings_for_agents.dxf import DxfBuilder
    from technical_drawings_for_agents.sheet import (
        PlotScale,
        SheetFrame,
        Viewport,
        resolve_sheet,
        sheet_frame,
        sheet_scale_bar,
    )
    from technical_drawings_for_agents.svg import svg_pattern_defs, svg_status_watermark
    from technical_drawings_for_agents.titleblock import TitleBlockFields, iso7200_title_block
except ImportError as exc:  # pragma: no cover - exercised by test_drawings_extra
    raise ImportError(
        "channel_designer drawings need the optional extra: "
        "pip install 'channel_designer[drawings]'") from exc

from ..volumes.sections import CrossSection, SectionVolumes

# ---- palette (print-safe, light) ----------------------------------------------------------
INK = "#111111"; DIM = "#555555"; GRID = "#c9ced6"
C_DREDGE = "#1f6fb2"; C_DRY = "#8a6a3a"; C_FACE = "#c0392b"; C_WL = "#1f6fb2"; C_WATER = "#d5e8f7"
C_GROUND = "#3b3024"

A3_FRAME_H = 209.0 - 8.0


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def text(x, y, s, size=2.5, fill=INK, anchor="start", weight="normal", rotate=0.0,
         family="Helvetica, Arial, sans-serif", italic=False):
    tr = f' transform="rotate({rotate:.1f} {x:.3f} {y:.3f})"' if rotate else ""
    st = ' font-style="italic"' if italic else ""
    return (f'<text x="{x:.3f}" y="{y:.3f}" font-size="{size:.2f}" fill="{fill}" '
            f'text-anchor="{anchor}" font-weight="{weight}" font-family="{family}"{st}{tr}>'
            f'{esc(s)}</text>')


def pline(pts_mm, stroke=INK, w=0.25, dash=None, fill="none", close=False, opacity=None):
    tag = "polygon" if close else "polyline"
    d = f' stroke-dasharray="{dash}"' if dash else ""
    op = f' opacity="{opacity}"' if opacity is not None else ""
    return (f'<{tag} points="' + " ".join(f"{x:.3f},{y:.3f}" for x, y in pts_mm)
            + f'" fill="{fill}" stroke="{stroke}" stroke-width="{w:.3f}" '
            f'stroke-linejoin="round" stroke-linecap="round"{d}{op}/>')


def _runs(idx):
    out, cur = [], []
    for j in idx:
        if cur and j != cur[-1] + 1:
            out.append(cur); cur = []
        cur.append(int(j))
    if cur:
        out.append(cur)
    return out


def pick_sections(volumes: SectionVolumes, n: int = 9, min_gap_m: float = 100.0,
                  s_range: Optional[Sequence[float]] = None) -> List[CrossSection]:
    """Adaptive section choice: equal steps of cumulative cut volume (denser where the cut
    is), a minimum gap so two sections never show the same ground, and the tallest face
    always included."""
    rr = [s for s in volumes.stations
          if s_range is None or s_range[0] <= s.chainage_m <= s_range[1]]
    if not rr:
        return []
    v = np.array([(s.dredge_area_m2 + s.excav_area_m2) * volumes.station_m for s in rr])
    cv = np.cumsum(v)
    tallest = max(rr, key=lambda s: s.max_face_h_m)
    if cv[-1] <= 0:
        step = max(1, len(rr) // n)
        return rr[::step][:n]
    out: List[CrossSection] = []
    for m in range(n, 4 * n):
        targets = np.linspace(cv[-1] / (2 * m), cv[-1] - cv[-1] / (2 * m), m)
        picks = [rr[min(int(np.searchsorted(cv, t)), len(rr) - 1)] for t in targets]
        picks[int(np.argmin([abs(p.chainage_m - tallest.chainage_m) for p in picks]))] = tallest
        out = []
        for p in sorted(picks, key=lambda s: s.chainage_m):
            if out and p.chainage_m - out[-1].chainage_m < min_gap_m:
                if p is tallest:
                    out[-1] = p
                continue
            out.append(p)
        if len(out) >= n:
            return out[:n] if tallest in out[:n] else out
    return out


class PanelSheet:
    """A paper-space sheet with SEVERAL viewports, each at its own derived scale.

    The toolkit's ``SheetDrawing`` is one viewport per sheet. A cross-section gallery needs N
    viewports, each with its own PlotScale and scale bar, under ONE frame, watermark and
    title block. This assembler reuses the toolkit's chrome and deliberately emits NO
    sheet-level scale record, because the sheet has no single scale.
    """

    def __init__(self, sheet, status="DRAFT"):
        self.sheet = sheet; self.status = status; self.defs = []; self.el = []

    def add(self, *e):
        self.el.extend(x for x in e if x)

    def add_defs(self, *e):
        self.defs.extend(x for x in e if x)

    def render(self):
        w, h = self.sheet.paper.width_mm, self.sheet.paper.height_mm
        parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w:g}mm" height="{h:g}mm" '
                 f'viewBox="0 0 {w:g} {h:g}">',
                 f'<rect width="{w:g}" height="{h:g}" fill="#ffffff"/>',
                 "<defs>" + "\n".join(self.defs) + "</defs>", sheet_frame(self.sheet)]
        parts += self.el
        if self.status:
            parts.append(svg_status_watermark(w, h, self.status))
        parts.append("</svg>")
        return "\n".join(parts)


def design_surface(sec: CrossSection) -> np.ndarray:
    """Ground after construction: the cut face where it is below ground, else the ground."""
    return np.where(np.isnan(sec.cutface), sec.terrain, np.minimum(sec.cutface, sec.terrain))


def panel_extent(sec: CrossSection, margin_m: float = 15.0, min_half_m: float = 60.0):
    """Model extent a panel must show: full wet channel + cut + margin (offset, elevation)."""
    offs, terr, cut = sec.offsets, sec.terrain, sec.cutface
    design = design_surface(sec)
    with np.errstate(invalid="ignore"):
        idx = np.where((design < sec.wl) | ((terr - cut) > 1e-6))[0]
    if idx.size:
        lo, hi = offs[idx].min() - margin_m, offs[idx].max() + margin_m
    else:
        lo, hi = -min_half_m, min_half_m
    half = max(abs(lo), abs(hi), min_half_m)
    lo, hi = -half, half
    sel = (offs >= lo) & (offs <= hi)
    zmax = float(np.nanmax(terr[sel])) + 3 if np.isfinite(terr[sel]).any() else sec.wl + 3
    zmin = sec.grade - 4
    return lo, hi, zmin, zmax


def pack_gallery(secs: Sequence[CrossSection], scale=2000, cap_h=6.5, bar_h=9.0,
                 frame_h=A3_FRAME_H, gap=4.0):
    """Greedy-pack variable-height panels (all at ONE scale, so sections compare) into sheets.

    Returns a list of sheets, each a list of ``(section, panel_height_mm)``.
    """
    sheets, cur, used = [], [], gap
    for s in secs:
        _lo, _hi, zmin, zmax = panel_extent(s)
        ph = (zmax - zmin) * 1000 / scale + cap_h + bar_h
        if cur and used + ph + gap > frame_h:
            sheets.append(cur); cur, used = [], gap
        cur.append((s, ph)); used += ph + gap
    if cur:
        sheets.append(cur)
    return sheets


@dataclass
class SheetInfo:
    """Title-block text for a gallery sheet. Nothing here is defaulted to a real party."""

    number: str
    title: str
    date: str
    legal_owner: str = ""
    responsible_dept: str = ""
    technical_reference: str = ""
    created_by: str = ""
    checked_by: str = ""
    approved_by: str = ""
    revision: str = "A"
    document_type: str = "Section pack"
    document_status: str = "DRAFT"
    status_watermark: str = "DRAFT"
    extra_notes: List[str] = field(default_factory=list)


def gallery_sheet(panels, info: SheetInfo, *, sheet_no: int = 1, n_sheets: int = 1,
                  part_label: str = "", first_index: int = 1, dxf=None, scale=2000,
                  cap_h=6.5, bar_h=9.0, gap=4.0) -> str:
    """One A3 landscape sheet of cross-section panels. Returns the SVG text."""
    sheet = resolve_sheet({"size": "A3", "orientation": "landscape", "margins_mm": 10,
                           "reserve": [{"name": "title-block", "edge": "bottom", "size_mm": 68}],
                           "viewport": {"extent_m": [0, 0, 1, 1], "scale": 1000}})  # placeholder
    d = PanelSheet(sheet, status=info.status_watermark)
    d.add_defs(svg_pattern_defs(None, patterns=["soil", "water"], tile=2.0, stroke_width=0.16))
    fr = sheet.viewport.frame
    pw = fr.width_mm - 2 * gap
    d.add(text(fr.x_mm + 2, fr.y_mm + 5,
               f"CROSS-SECTIONS {part_label} — panels 1:{scale}, V = H · looking upstream, "
               f"left bank (−) / right bank (+) · ground, full wet channel at WL, dredge and "
               f"dry cut", size=3.0, weight="bold"))
    py = fr.y_mm + 8 + gap
    for k, (sec, ph) in enumerate(panels):
        n_sec = first_index + k
        px = fr.x_mm + gap
        offs, terr, wet, cut = sec.offsets, sec.terrain, sec.wet, sec.cutface
        wl, grade = sec.wl, sec.grade
        design = design_surface(sec)
        with np.errstate(invalid="ignore"):
            wetmask = design < wl
            cutmask = (terr - cut) > 1e-6
        lo, hi, zmin, zmax = panel_extent(sec)
        sel = (offs >= lo) & (offs <= hi)
        inner = SheetFrame(px + 12, py + cap_h, pw - 14, ph - cap_h - bar_h)
        denom = scale if (hi - lo) * 1000 / scale <= inner.width_mm else 5000
        vp = Viewport((lo, zmin, hi, zmax), PlotScale(denom), inner)
        P = vp.point
        d.add(f'<rect x="{px:.3f}" y="{py:.3f}" width="{pw:.3f}" height="{ph:.3f}" '
              f'fill="#ffffff" stroke="{INK}" stroke-width="0.25"/>')
        d.add(text(px + 2, py + 4.4,
                   f"SECTION {n_sec}  ·  ch {sec.chainage_m / 1000:.3f} km  ·  1:{denom}",
                   size=2.8, weight="bold"))
        d.add(text(px + pw - 2, py + 4.4,
                   f"WL {wl:+.1f} m · invert {grade:+.2f} m · bottom {2 * sec.hb:.1f} m  ·  "
                   f"dredge {sec.dredge_area_m2:,.0f} m²  ·  dry cut {sec.excav_area_m2:,.0f} m²"
                   f"  ·  max face {sec.max_face_h_m:.1f} m above WL  ·  bench L/R: "
                   f"{'yes' if sec.bench_left else 'no'}/{'yes' if sec.bench_right else 'no'}",
                   size=2.2, anchor="end", fill="#222"))
        g = []
        for z in np.arange(math.ceil(zmin / 10) * 10, zmax, 10):
            x1, yy = P(lo, z); x2, _ = P(hi, z)
            g.append(f'<line x1="{x1:.3f}" y1="{yy:.3f}" x2="{x2:.3f}" y2="{yy:.3f}" '
                     f'stroke="{GRID}" stroke-width="0.1"/>')
            d.add(text(inner.x_mm - 0.8, yy + 0.6, f"{z:.0f}", size=1.7, fill=DIM, anchor="end"))
        for o in np.arange(math.ceil(lo / 50) * 50, hi, 50):
            xx, y1 = P(o, zmin); _, y2 = P(o, zmax)
            g.append(f'<line x1="{xx:.3f}" y1="{y1:.3f}" x2="{xx:.3f}" y2="{y2:.3f}" '
                     f'stroke="{GRID}" stroke-width="0.1"/>')
            d.add(text(xx, inner.y_mm - 0.6, f"{o:+.0f}", size=1.6, fill=DIM, anchor="middle"))
        for run in _runs(np.where(wetmask & sel)[0]):
            pts = ([P(offs[run[0]], wl)] + [P(offs[j], design[j]) for j in run]
                   + [P(offs[run[-1]], wl)])
            g.append(pline(pts, stroke="none", w=0, fill=C_WATER, close=True))
        for run in _runs(np.where(cutmask & sel)[0]):
            sub, cur = [], [run[0]]
            for a_, b_ in zip(run, run[1:]):
                if wet[b_] != wet[a_]:
                    sub.append(cur); cur = []
                cur.append(b_)
            sub.append(cur)
            for s_ in sub:
                if len(s_) < 2:
                    continue
                pts = [P(offs[j], terr[j]) for j in s_] + [P(offs[j], cut[j]) for j in s_[::-1]]
                g.append(pline(pts, stroke="none", w=0, close=True,
                               fill="url(#hatch-water)" if wet[s_[0]] else "url(#hatch-soil)"))
        gj = np.where(sel & ~np.isnan(terr))[0]
        if len(gj):
            g.append(pline([P(offs[j], terr[j]) for j in gj], stroke=C_GROUND, w=0.4))
        with np.errstate(invalid="ignore"):
            segs_cut = np.where(sel & ~np.isnan(cut) & (cut <= terr + 1e-6))[0]
        for run in _runs(segs_cut):
            g.append(pline([P(offs[j], cut[j]) for j in run], stroke=C_FACE, w=0.35))
        x1, yw = P(lo, wl); x2, _ = P(hi, wl)
        g.append(f'<line x1="{x1:.3f}" y1="{yw:.3f}" x2="{x2:.3f}" y2="{yw:.3f}" '
                 f'stroke="{C_WL}" stroke-width="0.25" stroke-dasharray="2,1"/>')
        cid = f"clip{esc(info.number)}{k}".replace(" ", "_")
        d.add_defs(f'<clipPath id="{cid}"><rect x="{inner.x_mm:.3f}" y="{inner.y_mm - 4:.3f}" '
                   f'width="{inner.width_mm:.3f}" height="{inner.height_mm + 4:.3f}"/></clipPath>')
        d.add(f'<g clip-path="url(#{cid})">' + "\n".join(g) + "</g>")
        d.add(sheet_scale_bar(vp, SheetFrame(px, py, pw, ph),
                              length_m={2000: 100, 5000: 200}.get(denom, 100), divisions=5,
                              unit="m"))
        if dxf is not None:
            x0 = (n_sec - 1) * 600.0
            dxf.polyline([(x0 + offs[j], terr[j]) for j in gj], layer="OUTLINE")
            for run in _runs(segs_cut):
                dxf.polyline([(x0 + offs[j], cut[j]) for j in run], layer="OBJECT")
            dxf.line((x0 + lo, wl), (x0 + hi, wl), layer="WATER")
            dxf.text((x0 + lo, zmax + 2),
                     f"S{n_sec} ch {sec.chainage_m / 1000:.3f} km  dredge "
                     f"{sec.dredge_area_m2:.0f} m2  dry {sec.excav_area_m2:.0f} m2",
                     height=3.0, layer="TEXT")
        py += ph + gap

    tb = sheet.regions["title-block"]; x = tb.x_mm + 2; y = tb.y_mm + 4
    d.add(text(x, y, "LEGEND AND NOTES", size=2.4, weight="bold")); y += 4

    def sw(kind, label):
        nonlocal y
        if kind == "water":
            d.add(f'<rect x="{x}" y="{y - 2.2}" width="7" height="3" fill="{C_WATER}"/>')
        elif kind == "dredge":
            d.add(f'<rect x="{x}" y="{y - 2.2}" width="7" height="3" fill="url(#hatch-water)" '
                  f'stroke="{C_DREDGE}" stroke-width="0.2"/>')
        elif kind == "dry":
            d.add(f'<rect x="{x}" y="{y - 2.2}" width="7" height="3" fill="url(#hatch-soil)" '
                  f'stroke="{C_DRY}" stroke-width="0.2"/>')
        else:
            col, w, dash = kind.split(":")
            dd = f' stroke-dasharray="{dash}"' if dash else ""
            d.add(f'<line x1="{x}" y1="{y - 0.8}" x2="{x + 7}" y2="{y - 0.8}" stroke="{col}" '
                  f'stroke-width="{w}"{dd}/>')
        d.add(text(x + 9, y, label, size=2.2)); y += 3.5

    sw("water", "Water at WL — the full wet channel after construction")
    sw("dredge", "Dredge: cut below WL inside the present river")
    sw("dry", "Dry excavation: bank, berm and face")
    sw(f"{C_GROUND}:0.4:", "Existing ground and river bed")
    sw(f"{C_FACE}:0.35:", "Design cut face — channel_designer.volumes.cut_model")
    sw(f"{C_WL}:0.25:2,1", "Water level (WL)")
    y += 0.5
    for note in (["Areas per panel from volumes.cut_model.section_area — the same call, on the "
                  "same profile, as the volume model. One scale for every panel; V = H."]
                 + list(info.extra_notes)):
        d.add(text(x, y, note, size=2.0, fill="#222")); y += 3
    fields = TitleBlockFields(
        identification_number=info.number, title=info.title, revision=info.revision,
        date_of_issue=info.date, document_type=info.document_type,
        document_status=info.document_status, legal_owner=info.legal_owner,
        responsible_dept=info.responsible_dept, technical_reference=info.technical_reference,
        created_by=info.created_by, checked_by=info.checked_by, approved_by=info.approved_by,
        sheet=f"{sheet_no} of {n_sheets}", provenance="channel_designer",
        scale=f"1:{scale}")
    d.add(iso7200_title_block(sheet, fields))
    return d.render()


def section_gallery(sections: Sequence[CrossSection], info: SheetInfo, *, scale=2000,
                    dxf_path=None) -> List[str]:
    """SVG text for every gallery sheet needed to show ``sections`` at one scale.

    Sheet numbers are ``info.number`` + ``A``, ``B``, ... when more than one sheet is needed.
    If ``dxf_path`` is given, all sections are also written to one DXF in real metres.
    """
    packed = pack_gallery(sections, scale=scale)
    dx = DxfBuilder() if dxf_path is not None else None
    out, first = [], 1
    for gi, panels in enumerate(packed):
        suffix = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"[gi] if len(packed) > 1 else ""
        sheet_info = SheetInfo(**{**info.__dict__, "number": f"{info.number}{suffix}"})
        out.append(gallery_sheet(panels, sheet_info, sheet_no=gi + 1, n_sheets=len(packed),
                                 part_label=f"({suffix.lower()})" if suffix else "",
                                 first_index=first, dxf=dx, scale=scale))
        first += len(panels)
    if dx is not None:
        dx.save(dxf_path)
    return out
