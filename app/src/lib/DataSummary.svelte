<script lang="ts">
  import type { InputRole, Snapshot } from "./api";

  let {
    snap,
    busy,
    dropTarget,
    onLoad,
    onFetch,
    onWater,
  }: {
    snap: Snapshot | null;
    busy: boolean;
    /** The row a dragged file is over, if any. */
    dropTarget: InputRole | null;
    onLoad: (role: InputRole) => void;
    onFetch: () => void;
    onWater: (level: string, fall: string) => void;
  } = $props();

  const count = (g: { coordinates: unknown[] } | null | undefined) => (g ? `${g.coordinates.length} positions` : "not loaded");
  let inputs = $derived(snap?.document.inputs);
  let crs = $derived(snap?.document.project.crs);
  // The CRSs the data came in, when they are not the project's own: the zone it was moved to.
  let moved = $derived(
    [...new Set(Object.values(snap?.document.project.source_crs ?? {}))].filter((c) => crs && c !== `EPSG:${crs.epsg}`),
  );
  let level = $state("");
  let fall = $state("");
  // Show the document's water level, as a level at the start and the fall to the end.
  $effect(() => {
    const p = inputs?.water_level?.points;
    if (!p?.length) {
      level = "";
      fall = "";
      return;
    }
    level = String(p[0][1]);
    const last = p[p.length - 1];
    fall = p.length > 1 && last[0] > p[0][0] ? String((p[0][1] - last[1]) / (last[0] - p[0][0])) : "";
  });

  const ROWS: { role: InputRole; label: string }[] = [
    { role: "left_bank", label: "Left bank" },
    { role: "right_bank", label: "Right bank" },
    { role: "river_polygon", label: "River polygon" },
    { role: "centreline", label: "Centreline" },
    { role: "dem", label: "DEM" },
  ];
  const value = (role: InputRole) => {
    if (role === "dem") return inputs?.dem ? inputs.dem.path : "not loaded";
    if (role === "river_polygon") return inputs?.river_polygon ? `${inputs.river_polygon.coordinates[0].length} positions` : "not loaded";
    return count(inputs?.[role]);
  };
</script>

<section aria-label="Data">
  <h3>Data</h3>
  <table>
    <tbody>
      <tr><th>Project CRS</th><td data-testid="project-crs" colspan="2">{crs ? crs.name : "none yet"}</td></tr>
      {#if moved.length}
        <tr><td colspan="3" class="moved" data-testid="moved-note">Data in {moved.join(", ")} was moved to {crs?.name}.</td></tr>
      {/if}
      {#each ROWS as r}
        <tr class:drop={dropTarget === r.role} data-drop-role={r.role}>
          <th>{r.label}</th>
          <td data-testid="data-{r.role}">{value(r.role)}</td>
          <td class="act"><button disabled={busy} data-testid="load-{r.role}" onclick={() => onLoad(r.role)}>Load…</button></td>
        </tr>
      {/each}
    </tbody>
  </table>
  <p class="row">
    <button disabled={busy} data-testid="fetch-dem" onclick={onFetch} title="Download a Copernicus GLO-30 DEM for the reach, beside the saved design">Fetch DEM</button>
  </p>
  <p class="hint">Drop a file on its row to load it. GeoJSON and GeoPackage in WGS84 or UTM; DEM as GeoTIFF.</p>

  <div class="water">
    <label>Water level at the river start (m)
      <input aria-label="Water level" inputmode="decimal" bind:value={level} disabled={busy} data-testid="water-level" />
    </label>
    <label>Fall per metre of river (optional)
      <input aria-label="Water fall" inputmode="decimal" bind:value={fall} disabled={busy} data-testid="water-fall" />
    </label>
    <button disabled={busy || level.trim() === ""} data-testid="set-water" onclick={() => onWater(level, fall)}>Set water level</button>
  </div>
</section>

<style>
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: normal; color: var(--dim); padding: 3px 0; font-size: 12px; }
  td { text-align: right; padding: 3px 0; font-size: 13px; }
  td.act { width: 1%; padding-left: 6px; }
  button { font-size: 12px; padding: 1px 8px; }
  tr.drop { background: #fdf1d3; outline: 2px dashed var(--gold); }
  .moved { text-align: left; font-size: 12px; color: #6b4a00; background: #fdf1d3; padding: 3px 6px; }
  .row { margin: 6px 0 0; }
  .hint { font-size: 11px; color: var(--dim); margin: 4px 0 0; }
  .water { margin-top: 10px; display: flex; flex-direction: column; gap: 4px; }
  .water label { display: flex; flex-direction: column; gap: 2px; font-size: 12px; color: var(--dim); }
  .water input { font: inherit; padding: 3px 4px; color: var(--ink); }
  .water button { align-self: flex-start; margin-top: 2px; }
</style>
