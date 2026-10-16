<script lang="ts">
  import { untrack } from "svelte";
  import type { Choices, Snapshot } from "./api";

  let {
    snap,
    choices,
    onChange,
  }: {
    snap: Snapshot | null;
    choices: Choices | null;
    /** Send the chosen ruleset, vessel class and the draught as typed (empty: the vessel class's own). */
    onChange: (ruleset: string, vessel: string, draught: string) => void;
  } = $props();

  let settings = $derived(snap?.document.stages.sizing.settings ?? null);
  let shape = $derived(snap?.document.stages.sizing.results ?? null);
  let ruleset = $state("");
  let vessel = $state("");
  let draught = $state("");

  // Take the choices from the document whenever it changes (open, new). A draught being typed
  // stays as typed while it means the same number as the document's.
  $effect(() => {
    ruleset = settings?.ruleset_id ?? choices?.rulesets[0]?.id ?? "";
    vessel = settings?.vessel_standard_id ?? choices?.vessels[0]?.id ?? "";
    const doc = settings?.draught_m;
    // Not tracked: typing alone must not rerun this, only a new document.
    const typed = untrack(() => draught).trim();
    if (doc == null ? typed !== "" : Number(typed) !== doc) draught = doc == null ? "" : String(doc);
  });

  function send() {
    onChange(ruleset, vessel, draught);
  }

  const fmt = (x: number, digits = 1) => x.toFixed(digits);
</script>

<section aria-label="Vessel and ruleset">
  <h3>Vessel and ruleset</h3>
  <label>
    Ruleset
    <select aria-label="Ruleset" bind:value={ruleset} onchange={send}>
      {#each choices?.rulesets ?? [] as r}<option value={r.id}>{r.label}</option>{/each}
    </select>
  </label>
  <label>
    Vessel class
    <select aria-label="Vessel class" bind:value={vessel} onchange={send}>
      {#each choices?.vessels ?? [] as v}<option value={v.id}>{v.label}</option>{/each}
    </select>
  </label>
  <label>
    Draught (m)
    <input aria-label="Draught" inputmode="decimal" placeholder="vessel default" bind:value={draught} oninput={send} />
  </label>

  {#if shape}
    <table class="shape" data-testid="channel-shape">
      <caption>Channel shape</caption>
      <tbody>
        <tr><th>Bottom width</th><td data-testid="bottom-width">{fmt(shape.bottom_width_straight_m)} m</td></tr>
        <tr><th>Minimum bend radius</th><td data-testid="min-radius">{fmt(shape.min_bend_radius_m, 0)} m</td></tr>
        <tr><th>Design depth</th><td data-testid="design-depth">{fmt(shape.design_depth_m, 2)} m</td></tr>
        <tr><th>Vessel</th><td>{fmt(shape.loa_m)} x {fmt(shape.beam_m)} m, draught {fmt(shape.draught_m, 2)} m</td></tr>
      </tbody>
    </table>
    {#each shape.warnings as w}<p class="warn">{w}</p>{/each}
  {:else}
    <p class="dim" data-testid="no-shape">Choose a ruleset and a vessel class to see the channel shape.</p>
  {/if}
</section>

<style>
  label { display: flex; flex-direction: column; gap: 2px; margin: 6px 0; font-size: 12px; color: var(--dim); }
  select, input { font: inherit; padding: 3px 4px; color: var(--ink); }
  .shape { width: 100%; border-collapse: collapse; margin-top: 10px; }
  caption { text-align: left; font-family: var(--serif); color: var(--navy); font-weight: bold; padding-bottom: 4px; }
  th { text-align: left; font-weight: normal; color: var(--dim); padding: 3px 0; font-size: 12px; }
  td { text-align: right; padding: 3px 0; }
  .warn { background: #fdf1d3; color: #6b4a00; padding: 4px 6px; font-size: 12px; }
  .dim { color: var(--dim); }
</style>
