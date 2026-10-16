<script lang="ts">
  import type { Snapshot, StageName, StageState } from "./api";

  let { snap }: { snap: Snapshot | null } = $props();

  // Display labels only: the stage names and their order come from the session.
  const LABEL: Record<StageName, string> = {
    sizing: "Sizing",
    alignment: "Alignment",
    optimise: "Optimise",
    volumes: "Volumes",
  };
  const CHIP: Record<StageState, string> = { not_run: "not run", fresh: "fresh", stale: "stale" };
</script>

<section aria-label="Stages">
  <h3>Stages</h3>
  <ol class="stages">
    {#each snap?.stages ?? [] as s}
      <li data-testid="stage-{s.name}">
        <span class="name">{LABEL[s.name]}</span>
        <span class="chip {s.status}" data-testid="status-{s.name}">{CHIP[s.status]}</span>
      </li>
    {/each}
  </ol>
</section>

<style>
  .stages { list-style: none; margin: 0; padding: 0; }
  li { display: flex; justify-content: space-between; align-items: center; padding: 4px 0; border-bottom: 1px solid var(--line); }
  .chip { font-size: 11px; padding: 1px 8px; border-radius: 10px; border: 1px solid var(--line); color: var(--dim); }
  .chip.fresh { background: #e3f1e3; border-color: #9cc79c; color: #1d5c1d; }
  .chip.stale { background: #fdf1d3; border-color: var(--gold); color: #6b4a00; }
</style>
