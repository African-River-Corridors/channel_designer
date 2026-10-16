<script lang="ts">
  import type { OptionTable } from "./api";

  let {
    table,
    colourOf,
    selectedId,
    onSelect,
  }: {
    table: OptionTable | null;
    /** The colour an option has on the map, by its place in the design's list. */
    colourOf: (id: string) => string;
    selectedId: string | null;
    onSelect: (id: string) => void;
  } = $props();

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
  <section class="table" aria-label="Option comparison">
    {#if table && table.rows.length}
      <table data-testid="option-table">
        <thead>
          <tr>
            <th></th><th>Option</th><th>Dredge m³</th><th>Dry cut m³</th><th>Tallest face m</th><th>Min radius m</th><th>Radius ok</th>
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
              <td><span class="swatch" style:background={colourOf(r.id)}></span></td>
              <td class="name">{r.name}</td>
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
  .table, .checks { overflow: auto; padding: 6px 10px; min-width: 0; }
  .checks { border-left: 1px solid var(--line); }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: right; font-weight: normal; color: var(--dim); font-size: 12px; padding: 3px 8px; border-bottom: 1px solid var(--line); position: sticky; top: 0; background: #fff; }
  th:nth-child(2), td.name { text-align: left; }
  td { text-align: right; padding: 4px 8px; border-bottom: 1px solid var(--line); font-variant-numeric: tabular-nums; }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: #f4f1e8; }
  tr.selected { background: #fdf1d3; outline: 2px solid var(--gold); outline-offset: -2px; }
  tr.failed td { color: #8f1d14; background: #fbe4e1; }
  tr.failed.selected td { background: #f8d4cf; }
  .swatch { display: inline-block; width: 16px; height: 4px; vertical-align: middle; }
  .mark.fail { color: #b3261e; font-weight: bold; }
  .mark.ok { color: #1d5c1d; }
  .note { margin: 6px 0 0; font-size: 12px; color: #6b4a00; }
  .slot { color: var(--dim); font-style: italic; margin: 8px 0; }
  .checks h3 { font-size: 14px; }
  ul { list-style: none; margin: 0; padding: 0; font-size: 12px; }
  li { padding: 4px 0; border-bottom: 1px solid var(--line); }
  li.failed { color: #8f1d14; }
</style>
