<script lang="ts">
  import type { Snapshot } from "./api";

  let { snap }: { snap: Snapshot | null } = $props();

  const count = (g: { coordinates: unknown[] } | null | undefined) => (g ? `${g.coordinates.length} positions` : "not loaded");
  let inputs = $derived(snap?.document.inputs);
  let crs = $derived(snap?.document.project.crs);
</script>

<section aria-label="Data">
  <h3>Data</h3>
  <table>
    <tbody>
      <tr><th>Project CRS</th><td data-testid="project-crs">{crs ? crs.name : "none yet"}</td></tr>
      <tr><th>Left bank</th><td>{count(inputs?.left_bank)}</td></tr>
      <tr><th>Right bank</th><td>{count(inputs?.right_bank)}</td></tr>
      <tr><th>River polygon</th><td>{inputs?.river_polygon ? `${inputs.river_polygon.coordinates[0].length} positions` : "not loaded"}</td></tr>
      <tr><th>DEM</th><td>{inputs?.dem ? inputs.dem.path : "not loaded"}</td></tr>
    </tbody>
  </table>
</section>

<style>
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: normal; color: var(--dim); padding: 3px 0; font-size: 12px; }
  td { text-align: right; padding: 3px 0; font-size: 13px; }
</style>
