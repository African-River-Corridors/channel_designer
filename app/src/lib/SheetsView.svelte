<script lang="ts">
  import * as api from "./api";
  import type { ReportAsk, SheetPage } from "./api";

  let {
    option,
    onNotice,
    onError,
  }: {
    /** The active design line option. */
    option: string | null;
    onNotice: (message: string) => void;
    onError: (message: string) => void;
  } = $props();

  let pages = $state<SheetPage[] | null>(null);
  let note = $state<string | null>(null);
  let chosen = $state(0);
  let zoom = $state(1);
  let exporting = $state(false);
  let asked = 0;

  // The Report button (spec 0014): the same reports as `owd report`, from the same builder.
  let reportOpen = $state(false);
  let reporting = $state(false);
  let ask = $state<Omit<ReportAsk, "date">>({ kind: "screening", format: "pdf", revision: "A", status: "draft", prepared: "" });

  async function writeReport() {
    reporting = true;
    try {
      const date = new Date().toISOString().slice(0, 10);
      const where = await api.exportReport(option, { ...ask, date });
      if (where) onNotice(`Report written: ${where}`);
    } catch (e) {
      onError(String(e));
    } finally {
      reporting = false;
    }
  }

  // The sheets come from the same builder as `owd draw`; this view only places and zooms them.
  $effect(() => {
    const id = option;
    const ask = ++asked;
    pages = null;
    note = id ? "Drawing the sheets…" : "Choose a design line option to see its sheets.";
    if (!id) return;
    api.sheets(id).then(
      (p) => {
        if (ask !== asked) return;
        pages = p;
        chosen = 0;
        note = p.length ? null : "The option has no sheets to draw.";
      },
      (e) => {
        if (ask !== asked) return;
        note = String(e);
      },
    );
  });

  const page = $derived(pages?.[chosen] ?? null);

  async function exportPdfs() {
    if (!option) return;
    exporting = true;
    try {
      const where = await api.exportSheets(option);
      if (where) onNotice(`Sheets exported as PDF: ${where}`);
    } catch (e) {
      onError(String(e));
    } finally {
      exporting = false;
    }
  }
</script>

<div class="sheets" data-testid="sheets-view">
  <nav class="rail" aria-label="Sheets">
    {#if pages?.length}
      <ul>
        {#each pages as p, i (p.number)}
          <li>
            <button class:on={i === chosen} aria-pressed={i === chosen} data-testid="sheet-tab" onclick={() => ((chosen = i), (zoom = 1))}>
              <span class="number">{p.number}</span>
              <span class="title">{p.title}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <button class="primary" disabled={!pages?.length || exporting} onclick={exportPdfs} data-testid="export-sheets" title="Each sheet as PDF, the same files as owd draw writes">
      {exporting ? "Exporting…" : "Export PDFs"}
    </button>
    <button aria-expanded={reportOpen} onclick={() => (reportOpen = !reportOpen)} data-testid="report-toggle" title="A report of the design as PDF, HTML or Word, the same files as owd report writes">Report…</button>
    {#if reportOpen}
      <form class="report" data-testid="report-form" onsubmit={(e) => (e.preventDefault(), writeReport())}>
        <label>Kind
          <select bind:value={ask.kind} data-testid="report-kind">
            <option value="screening">Screening report</option>
            <option value="quantities">Quantity schedule</option>
            <option value="sections">Section pack</option>
            <option value="run">Run report</option>
            <option value="all">All four</option>
          </select>
        </label>
        <label>Format
          <select bind:value={ask.format} data-testid="report-format">
            <option value="pdf">PDF</option>
            <option value="html">HTML</option>
            <option value="docx">Word</option>
            <option value="all">All three</option>
          </select>
        </label>
        <label>Revision <input bind:value={ask.revision} data-testid="report-revision" /></label>
        <label>Status
          <select bind:value={ask.status} data-testid="report-status">
            <option value="draft">Draft</option>
            <option value="for-checking">For checking</option>
          </select>
        </label>
        <label>Prepared by <input bind:value={ask.prepared} data-testid="report-prepared" placeholder="a name or a role" /></label>
        <button class="primary" type="submit" disabled={reporting} data-testid="export-report">{reporting ? "Writing…" : "Write report"}</button>
      </form>
    {/if}
  </nav>
  <div class="stage">
    {#if page}
      <div class="tools">
        <button onclick={() => (zoom = Math.max(0.25, zoom / 1.5))} aria-label="Zoom out">−</button>
        <button onclick={() => (zoom = 1)} aria-label="Fit the sheet">Fit</button>
        <button onclick={() => (zoom = Math.min(8, zoom * 1.5))} aria-label="Zoom in">+</button>
        {#each page.warnings as w}<span class="warn" data-testid="sheet-warning">{w}</span>{/each}
      </div>
      <div class="scroll">
        <!-- The SVG is the sheet as the builder wrote it: placed, never tinted or inverted. -->
        <div class="paper" style:width="{zoom * 100}%" data-testid="sheet" data-number={page.number}>
          {@html page.svg}
        </div>
      </div>
    {:else}
      <p class="slot" data-testid="sheets-note">{note}</p>
    {/if}
  </div>
</div>

<style>
  .sheets { position: absolute; inset: 0; display: grid; grid-template-columns: minmax(180px, 22%) 1fr; background: var(--surface); z-index: 5; }
  .rail { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-2); background: var(--surface-2); border-right: var(--lw-thin) solid var(--ink); min-height: 0; overflow: auto; }
  .rail ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .rail li button { width: 100%; text-align: left; display: flex; flex-direction: column; gap: 2px; }
  .rail .number { font-size: var(--fs-12); font-variant-numeric: tabular-nums; }
  .rail .title { font-size: var(--fs-11); color: var(--ink-3); }
  .rail > .primary { margin-top: auto; }
  .report { display: flex; flex-direction: column; gap: var(--sp-1); font-size: var(--fs-12); }
  .report label { display: flex; flex-direction: column; gap: 2px; }
  .stage { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .tools { display: flex; gap: var(--sp-2); align-items: center; padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); }
  .tools .warn { color: var(--ink-3); }
  .scroll { flex: 1; min-height: 0; overflow: auto; padding: var(--sp-3); }
  .paper { background: var(--sheet); color: var(--sheet-ink); box-shadow: 0 0 0 var(--lw-thin) var(--ink); }
  .paper :global(svg) { display: block; width: 100%; height: auto; }
  .slot { margin: var(--sp-4); }
</style>
