<script lang="ts">
  import type { CrossSection } from "./api";

  let { section, colour }: { section: CrossSection | null; colour: string } = $props();

  const W = 360;
  const H = 220;
  const PAD = { l: 44, r: 10, t: 10, b: 28 };

  const finite = (v: number | null): v is number => v != null && Number.isFinite(v);

  // Display geometry only: the numbers all come from the session.
  let view = $derived.by(() => {
    if (!section) return null;
    const off = section.offsets_m;
    const z = section.terrain_m.filter(finite);
    const face = section.cut_face_m.filter(finite);
    const all = [...z, ...face, section.water_level_m, section.design_invert_m];
    const lo = Math.min(...all);
    const hi = Math.max(...all);
    const pad = Math.max((hi - lo) * 0.08, 0.5);
    const [xmin, xmax] = [off[0], off[off.length - 1]];
    const x = (o: number) => PAD.l + ((o - xmin) / (xmax - xmin)) * (W - PAD.l - PAD.r);
    const y = (e: number) => H - PAD.b - ((e - (lo - pad)) / (hi + pad - (lo - pad))) * (H - PAD.t - PAD.b);
    // Break a line wherever a value is missing.
    const path = (vals: (number | null)[]) => {
      let d = "";
      let pen = false;
      vals.forEach((v, i) => {
        if (!finite(v)) {
          pen = false;
          return;
        }
        d += `${pen ? "L" : "M"}${x(off[i]).toFixed(1)},${y(v).toFixed(1)}`;
        pen = true;
      });
      return d;
    };
    const ticks = (a: number, b: number, n: number) => {
      const step = (b - a) / n;
      return Array.from({ length: n + 1 }, (_, i) => a + i * step);
    };
    return { x, y, terrain: path(section.terrain_m), face: path(section.cut_face_m), xticks: ticks(xmin, xmax, 4), yticks: ticks(lo - pad, hi + pad, 4) };
  });

  const km = (m: number) => `${Math.floor(m / 1000)}+${String(Math.round(m % 1000)).padStart(3, "0")}`;
</script>

<div class="cross" data-testid="cross-section" data-chainage={section?.chainage_m ?? ""}>
  {#if section && view}
    <svg viewBox="0 0 {W} {H}" role="img" aria-label="Cross-section at chainage {km(section.chainage_m)}" data-testid="cross-section-svg">
      {#each view.yticks as t}
        <line x1={PAD.l} x2={W - PAD.r} y1={view.y(t)} y2={view.y(t)} class="grid" />
        <text x={PAD.l - 4} y={view.y(t) + 3} class="tick end">{t.toFixed(1)}</text>
      {/each}
      {#each view.xticks as t}
        <text x={view.x(t)} y={H - PAD.b + 12} class="tick mid">{Math.round(t)}</text>
      {/each}
      <line x1={PAD.l} x2={W - PAD.r} y1={view.y(section.water_level_m)} y2={view.y(section.water_level_m)} class="water" />
      <path d={view.terrain} class="terrain" data-testid="cs-terrain" />
      <path d={view.face} class="face" style:stroke={colour} data-testid="cs-face" />
      <line
        x1={view.x(-section.half_width_m)}
        x2={view.x(section.half_width_m)}
        y1={view.y(section.design_invert_m)}
        y2={view.y(section.design_invert_m)}
        class="invert"
        style:stroke={colour}
      />
      <text x={W / 2} y={H - 2} class="tick mid">Offset from the design line (m): left − · right +</text>
    </svg>
    <dl class="facts" data-testid="cross-facts">
      <dt>Chainage</dt><dd data-testid="cs-chainage">{km(section.chainage_m)}</dd>
      <dt>Water level</dt><dd>{section.water_level_m.toFixed(2)} m</dd>
      <dt>Invert</dt><dd>{section.design_invert_m.toFixed(2)} m</dd>
      <dt>Dredge</dt><dd>{section.dredge_area_m2.toFixed(0)} m²</dd>
      <dt>Dry cut</dt><dd>{section.excav_area_m2.toFixed(0)} m²</dd>
      <dt>Face</dt><dd>{section.max_face_h_m.toFixed(1)} m</dd>
    </dl>
  {:else}
    <p class="slot" data-testid="no-cross-section">Run volumes to see the cross-section.</p>
  {/if}
</div>

<style>
  .cross { display: flex; gap: 8px; align-items: flex-start; height: 100%; padding: 4px; box-sizing: border-box; overflow: auto; }
  svg { flex: 1 1 auto; min-width: 200px; max-height: 100%; }
  .grid { stroke: #e3dfd0; stroke-width: 1; }
  .tick { font-size: 9px; fill: var(--dim); }
  .tick.end { text-anchor: end; }
  .tick.mid { text-anchor: middle; }
  .terrain { fill: none; stroke: #7a5a2a; stroke-width: 1.6; }
  .face { fill: none; stroke-width: 2.4; }
  .invert { stroke-width: 2.4; }
  .water { stroke: #1f6fbf; stroke-width: 1.4; stroke-dasharray: 5 3; }
  .facts { margin: 0; display: grid; grid-template-columns: auto auto; gap: 1px 8px; font-size: 11px; align-content: start; }
  dt { color: var(--dim); }
  dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
  .slot { color: var(--dim); font-style: italic; margin: 8px; }
</style>
