<script lang="ts">
  import type { RunMethod, Snapshot, StageName, StageState } from "./api";

  let {
    snap,
    busy,
    samples,
    onRun,
    onCancel,
  }: {
    snap: Snapshot | null;
    /** A stage is running (the name), or nothing is. */
    busy: StageName | null;
    /** How many terrain points the running stage has read: the only progress the engine gives. */
    samples: number;
    onRun: (stage: StageName, method?: RunMethod) => void;
    onCancel: () => void;
  } = $props();

  // Display labels only: the stage names and their order come from the session.
  const LABEL: Record<StageName, string> = {
    sizing: "Sizing",
    alignment: "Alignment",
    optimise: "Optimise",
    volumes: "Volumes",
  };
  const CHIP: Record<StageState, string> = { not_run: "not run", fresh: "fresh", stale: "stale" };
  // Sizing runs by itself when the vessel changes; the other three have a Run button.
  const RUNNABLE: StageName[] = ["alignment", "optimise", "volumes"];
  let method = $state<RunMethod>("rolling_circle");
</script>

<section aria-label="Stages">
  <h3>Stages</h3>
  <ol class="stages">
    {#each snap?.stages ?? [] as s}
      <li data-testid="stage-{s.name}">
        <span class="name">{LABEL[s.name]}</span>
        <span class="chip {s.status}" class:running={busy === s.name} data-testid="status-{s.name}">{busy === s.name ? "running" : CHIP[s.status]}</span>
        {#if RUNNABLE.includes(s.name)}
          {#if s.name === "alignment"}
            <select aria-label="Alignment method" bind:value={method} disabled={busy !== null}>
              <option value="rolling_circle">Rolling circle</option>
              <option value="reroute">Re-route</option>
            </select>
          {/if}
          {#if busy === s.name}
            <button data-testid="cancel-{s.name}" onclick={onCancel}>Cancel</button>
          {:else}
            <button
              data-testid="run-{s.name}"
              disabled={busy !== null}
              onclick={() => onRun(s.name, s.name === "alignment" ? method : undefined)}>Run</button
            >
          {/if}
        {/if}
      </li>
    {/each}
  </ol>
  {#if busy}
    <p class="progress" role="status" data-testid="progress">
      Running {LABEL[busy].toLowerCase()}{samples > 0 ? `: ${samples.toLocaleString()} DEM points read` : "…"}
    </p>
  {/if}
</section>

<style>
  .stages { list-style: none; margin: 0; padding: 0; }
  li { display: flex; gap: var(--sp-2); align-items: center; padding: 5px 0; border-bottom: var(--lw-thin) solid var(--rule); }
  .name { flex: 1; }
  select { font-size: var(--fs-12); padding: 3px 6px; max-width: 120px; }
  /* A word and a shape on every chip; the shape comes from CSS so the word stays the only text. */
  .chip { font-size: var(--fs-11); font-weight: 600; padding: 1px 7px; border-radius: var(--radius); color: var(--ink-3); background: var(--surface-2); }
  .chip.fresh { color: var(--pass); background: var(--pass-soft); }
  .chip.fresh::before { content: "✓ "; }
  .chip.stale { color: var(--warn); background: var(--warn-soft); }
  .chip.stale::before { content: "! "; }
  .chip.running { color: var(--ink); background: var(--accent-soft); }
  .chip.running::before { content: "… "; }
  .progress { margin: var(--sp-2) 0 0; font-size: var(--fs-12); color: var(--ink-3); }
</style>
