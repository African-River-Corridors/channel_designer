<script lang="ts">
  import type { OptionTable, Screening, Snapshot, StretchCollection, StretchKind, Verdict } from "./api";
  import OptionSwatch from "./OptionSwatch.svelte";

  let {
    table,
    snap,
    stretches,
    indexOf,
    selectedId,
    selectedStretch,
    onSelect,
    onSelectStretch,
  }: {
    table: OptionTable | null;
    snap: Snapshot | null;
    /** The constraint stretches as the session exports them, to know which are on the map. */
    stretches: StretchCollection | null;
    /** An option's place in the design's list: its colour and hatch, as on the map. */
    indexOf: (id: string) => number;
    selectedId: string | null;
    selectedStretch: { option: string; index: number } | null;
    onSelect: (id: string) => void;
    onSelectStretch: (s: { option: string; index: number } | null) => void;
  } = $props();

  let tab = $state<"options" | "stretches">("options");

  /** The screening outputs of an option, from the document's volumes results. */
  const screeningOf = (id: string | null): Screening | undefined =>
    id ? snap?.document.stages.volumes.results?.options?.find((o) => o.id === id)?.screening : undefined;
  let screening = $derived(screeningOf(selectedId));
  let selectedName = $derived(table?.rows.find((r) => r.id === selectedId)?.name ?? selectedId ?? "");
  let onMap = $derived(new Set((stretches?.features ?? []).map((f) => f.properties.option_id)));

  // Each tag is a word and a shape, never colour alone.
  const VERDICT: Record<Verdict, { shape: string; words: string; tone: string }> = {
    "navigable-as-is": { shape: "✓", words: "navigable as is", tone: "ok" },
    "navigable-with-works": { shape: "◐", words: "navigable with works", tone: "warn" },
    "not-navigable": { shape: "✕", words: "not navigable", tone: "fail" },
  };
  const KIND: Record<StretchKind, string> = { width: "↔", depth: "↕", radius: "↻", infeasible: "✕" };

  // One display rounding rule per quantity: volumes to whole cubic metres, metres to one decimal.
  const m3 = (x: number | null) => (x == null ? "—" : Math.round(x).toLocaleString("en"));
  const m = (x: number | null, digits = 1) => (x == null ? "—" : x.toFixed(digits));
  const volumesNote = $derived(
    table?.volumes_status === "stale"
      ? "Volumes are stale: they were not run on the current inputs."
      : table?.volumes_status === "not run"
        ? "Volumes have not been run: run the volumes stage to fill the dredge, dry cut and tallest face."
        : null,
  );
</script>

<div class="bottom" data-testid="bottom-panel">
  <section class="table" aria-label="Results">
    <div class="tabs" role="tablist" aria-label="Results">
      <button role="tab" id="tab-options" aria-selected={tab === "options"} aria-controls="panel-options" data-testid="tab-options" onclick={() => (tab = "options")}>Options</button>
      <button role="tab" id="tab-stretches" aria-selected={tab === "stretches"} aria-controls="panel-stretches" data-testid="tab-stretches" onclick={() => (tab = "stretches")}>
        Constraint stretches{#if screening}{" "}({screening.constraint_stretches.length}){/if}
      </button>
    </div>
    {#if tab === "options"}
    <div role="tabpanel" id="panel-options" aria-labelledby="tab-options">
    {#if table && table.rows.length}
      <table data-testid="option-table">
        <thead>
          <tr>
            <th></th><th>Option</th><th>Verdict</th><th>Dredge m³</th><th>Dry cut m³</th><th>Tallest face m</th><th>Min radius m</th><th>Radius ok</th>
          </tr>
        </thead>
        <tbody>
          {#each table.rows as r (r.id)}
            <tr
              class:failed={r.failed_checks.length > 0}
              class:selected={r.id === selectedId}
              data-testid="option-row"
              data-option={r.id}
              data-failed={r.failed_checks.length > 0}
              onclick={() => onSelect(r.id)}
              tabindex="0"
              onkeydown={(e) => (e.key === "Enter" || e.key === " ") && onSelect(r.id)}
            >
              <td><OptionSwatch index={indexOf(r.id)} /></td>
              <td class="name">{r.name}</td>
              <td class="name" data-col="verdict">
                {#if screeningOf(r.id)}
                  {@const v = VERDICT[screeningOf(r.id)!.verdict]}
                  <span class="mark {v.tone}" data-testid="verdict" data-verdict={screeningOf(r.id)!.verdict}><span aria-hidden="true">{v.shape}</span> {v.words}</span>
                {:else}<span class="dim">—</span>{/if}
              </td>
              <td data-col="dredge">{m3(r.dredge_m3)}</td>
              <td data-col="dry-cut">{m3(r.dry_cut_m3)}</td>
              <td data-col="tallest-face">{m(r.tallest_face_m)}</td>
              <td data-col="min-radius">{m(r.min_radius_m, 0)}</td>
              <td data-col="radius-ok">
                {#if r.failed_checks.length}<span class="mark fail" aria-label="fails {r.failed_checks.join(', ')}">✕ fails</span>{:else}<span class="mark ok">✓</span>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if volumesNote}<p class="note" data-testid="volumes-note">{volumesNote}</p>{/if}
    {:else}
      <p class="slot" data-testid="no-options">Run alignment to see the design line options here.</p>
    {/if}
    </div>
    {:else}
    <div role="tabpanel" id="panel-stretches" aria-labelledby="tab-stretches" data-testid="stretches-panel">
      {#if screening}
        <p class="notice" data-testid="screening-notice"><strong>{screening.notice}.</strong> {screening.rules_notice}</p>
        {#if screening.blocking_not_evaluated.count}
          <p class="note" data-testid="blocking-not-evaluated">! {screening.blocking_not_evaluated.count} blocking Rule(s) were not evaluated: {screening.blocking_not_evaluated.rules.join(", ")}.</p>
        {/if}
        {#if screening.constraint_stretches.length}
          <table data-testid="stretch-table" aria-label="Constraint stretches of {selectedName}">
            <thead>
              <tr>
                <th>Kind</th><th>From m</th><th>To m</th><th>Governing Rule</th><th>Required</th><th>River</th><th>Deficit</th><th>Works</th><th>Dredge m³</th><th>Dry cut m³</th>
              </tr>
            </thead>
            <tbody>
              {#each screening.constraint_stretches as c, i}
                {@const pick = { option: selectedId!, index: i }}
                <tr
                  class:selected={selectedStretch?.option === selectedId && selectedStretch?.index === i}
                  data-testid="stretch-row"
                  data-kind={c.kind}
                  data-index={i}
                  onclick={() => onSelectStretch(pick)}
                  tabindex="0"
                  aria-selected={selectedStretch?.option === selectedId && selectedStretch?.index === i}
                  onkeydown={(e) => (e.key === "Enter" || e.key === " ") && onSelectStretch(pick)}
                >
                  <td class="name"><span class="mark kind"><span aria-hidden="true">{KIND[c.kind]}</span> {c.kind}</span></td>
                  <td>{m(c.start_chainage_m, 0)}</td>
                  <td>{m(c.end_chainage_m, 0)}</td>
                  <td class="name"><code>{c.governing_rule ?? "—"}</code>{#if c.reason}<span class="sub">{c.reason}</span>{/if}</td>
                  <td>{m(c.required, 2)}</td>
                  <td>{m(c.river, 2)}</td>
                  <td>{m(c.deficit, 2)}</td>
                  <td class="name">{c.works ?? "—"}</td>
                  <td>{m3(c.dredge_m3)}</td>
                  <td>{m3(c.dry_cut_m3)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
          {#if !onMap.has(selectedId ?? "")}<p class="note">The map shows no stretches for this option.</p>{/if}
        {:else}
          <p class="slot" data-testid="no-stretches">No constraint stretches: the river meets every Rule along {selectedName}.</p>
        {/if}
        {#if volumesNote}<p class="note">{volumesNote}</p>{/if}
      {:else}
        <p class="slot" data-testid="no-screening">Run volumes to see the verdict and the constraint stretches of the selected option.</p>
      {/if}
    </div>
    {/if}
  </section>

  <section class="checks" aria-label="Checks">
    <h3>Checks</h3>
    {#if table && table.checks.length}
      <ul data-testid="checks-list">
        {#each table.checks as c}
          <li class:failed={!c.passed} data-testid="check" data-passed={c.passed}>
            <span class="mark {c.passed ? 'ok' : 'fail'}">{c.passed ? "✓" : "✕"}</span>
            <strong>{c.option}</strong> {c.check}: {c.detail}
          </li>
        {/each}
      </ul>
    {:else}
      <p class="slot">No checks yet.</p>
    {/if}
  </section>
</div>

<style>
  .bottom { display: grid; grid-template-columns: 1fr minmax(220px, 34%); height: 100%; min-height: 0; }
  .table, .checks { overflow: auto; padding: var(--sp-3); min-width: 0; }
  .checks { border-left: var(--lw-thin) solid var(--ink); }
  table { width: 100%; border-collapse: collapse; font-size: var(--fs-13); }
  th { text-align: right; font: 600 var(--fs-11) var(--font-ui); letter-spacing: 0.04em; color: var(--ink-2); padding: var(--sp-1) var(--sp-2); border-bottom: var(--lw-thin) solid var(--ink); position: sticky; top: 0; background: var(--surface-2); }
  th:nth-child(2), td.name { text-align: left; }
  .tabs { display: flex; gap: var(--sp-1); margin-bottom: var(--sp-2); border-bottom: var(--lw-thin) solid var(--ink); }
  .tabs button { font: 600 var(--fs-12) var(--font-ui); padding: 3px var(--sp-3); border: var(--lw-thin) solid var(--rule); border-bottom: none; border-radius: var(--radius) var(--radius) 0 0; background: var(--surface); color: var(--ink-2); }
  .tabs button[aria-selected="true"] { background: var(--ink); color: var(--paper); border-color: var(--ink); }
  .mark.warn { color: var(--warn); background: var(--warn-soft); }
  .mark.kind { color: var(--ink); background: var(--surface-2); }
  .mark { white-space: nowrap; }
  .dim { color: var(--ink-3); }
  code { font-size: var(--fs-11); }
  .sub { display: block; font-size: var(--fs-11); color: var(--ink-3); }
  .notice { margin: 0 0 var(--sp-2); padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); color: var(--warn); background: var(--warn-soft); }
  td { text-align: right; padding: 6px var(--sp-2); border-bottom: var(--lw-thin) solid var(--rule); font-variant-numeric: tabular-nums; }
  tbody tr { cursor: pointer; }
  tbody tr:hover td { background: var(--surface-2); }
  tr.selected td { background: var(--accent-soft); }
  tr.selected td:first-child { box-shadow: inset 3px 0 0 var(--ink); }
  /* Status: a word and a shape on a soft ground, never colour alone. */
  .mark { font: 600 var(--fs-11) var(--font-ui); padding: 1px 7px; border-radius: var(--radius); }
  .mark.fail { color: var(--fail); background: var(--fail-soft); }
  .mark.ok { color: var(--pass); background: var(--pass-soft); }
  .note { margin: var(--sp-2) 0 0; font-size: var(--fs-12); color: var(--warn); }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2) 0; }
  ul { list-style: none; margin: 0; padding: 0; font-size: var(--fs-12); }
  li { padding: 5px 0; border-bottom: var(--lw-thin) solid var(--rule); }
  li.failed { color: var(--fail); }
  li .mark { margin-right: var(--sp-1); }
</style>
