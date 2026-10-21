<script lang="ts">
  // "Start from a river" (spec 0012, ticket #235). The user picks a reach; the session plans the
  // route, makes every input from open data one step at a time, takes a depth, and answers "what
  // fits". This view only sends what the user did and shows what comes back: every field is the
  // session's.
  import type * as maplibregl from "maplibre-gl";
  import type * as GeoJSON from "geojson";
  import * as api from "./api";
  import { docPart } from "./api";
  import type { FitsResult, FitsSelection, LonLat, Provenance, ProvenanceInput, ReachStepName, Snapshot } from "./api";
  import MarkBadge from "./MarkBadge.svelte";
  import { platform } from "./platform";
  import { token } from "./theme";

  let {
    map,
    snap,
    onSnap,
    onError,
    onUseVessel,
    picker = $bindable(),
  }: {
    map: maplibregl.Map | undefined;
    snap: Snapshot | null;
    onSnap: (next: Snapshot) => void;
    onError: (message: string | null) => void;
    /** "Use this vessel": the parent sets the design's sizing and leads into today's flow. */
    onUseVessel: (choice: string, selection: FitsSelection | null, vesselStandardId: string) => void;
    /** Set while a map click should pick a point of the reach. */
    picker?: ((lon: number, lat: number) => void) | undefined;
  } = $props();

  const isWeb = platform().kind === "web";
  const fmt = (p: LonLat | null) => (p ? `${p[0].toFixed(5)}, ${p[1].toFixed(5)}` : "");
  const parse = (t: string): LonLat | null => {
    const m = t.split(",").map((x) => Number(x.trim()));
    return m.length === 2 && m.every(Number.isFinite) ? [m[0], m[1]] : null;
  };

  let river = $state("");
  let startText = $state("");
  let endText = $state("");
  let viaText = $state("");
  let picking = $state<"start" | "end" | "via" | null>(null);
  let edgeFit = $state(true);
  let progress = $state<string | null>(null);
  let working = $state(false);
  let cancelled = false;
  let percentile = $state("");
  let typedLevel = $state("");
  let depth = $state("");
  let depthRows = $state("");
  let season = $state("dry");
  let shownClass = $state<string | null>(null);

  const view = $derived(snap?.reach_view);
  const provenance = $derived(snap?.document.provenance ?? {});
  const fits = $derived(docPart<FitsResult>(snap, "fits", "results"));
  const lowConfidence = $derived(docPart<{ stretches: { kind: string }[] }>(snap, "low_confidence_width")?.stretches ?? []);
  const INPUTS: { id: ProvenanceInput; label: string }[] = [
    { id: "route", label: "Route" },
    { id: "left_bank", label: "Left bank" },
    { id: "right_bank", label: "Right bank" },
    { id: "river_polygon", label: "River polygon" },
    { id: "centreline", label: "Centreline" },
    { id: "water_level", label: "Water level" },
    { id: "dem", label: "DEM" },
    { id: "bed", label: "Depth" },
  ];

  function pick(which: "start" | "end" | "via") {
    picking = picking === which ? null : which;
  }
  $effect(() => {
    picker = picking
      ? (lon: number, lat: number) => {
          const p = fmt([lon, lat]);
          if (picking === "start") startText = p;
          else if (picking === "end") endText = p;
          else viaText = viaText.trim() ? `${viaText.trim()}; ${p}` : p;
          picking = null;
        }
      : undefined;
  });

  async function act(work: () => Promise<Snapshot | null | void>): Promise<boolean> {
    onError(null);
    try {
      const next = await work();
      if (next) onSnap(next);
      return true;
    } catch (e) {
      onError(String(e));
      return false;
    }
  }

  async function route() {
    const start = parse(startText);
    const end = parse(endText);
    if (!start || !end) return onError("Give a start and an end as longitude, latitude (or pick them on the map).");
    const via: LonLat[] = [];
    for (const t of viaText.split(";").filter((x) => x.trim())) {
      const p = parse(t);
      if (!p) return onError(`\`${t.trim()}\` is not longitude, latitude.`);
      via.push(p);
    }
    working = true;
    await act(() => api.reachRoute(river, start, end, via));
    working = false;
  }

  /** The steps after the route, one call each, so Cancel can stop between them. */
  async function generate() {
    const steps: { step: ReachStepName; label: string }[] = [{ step: "banks", label: "banks from the JRC water mask" }];
    if (edgeFit) steps.push({ step: "edge_fit", label: "banks moved onto Sentinel-2" });
    steps.push({ step: "level", label: "water level from SWORD and SWOT" });
    working = true;
    cancelled = false;
    for (const [i, s] of steps.entries()) {
      if (cancelled) break;
      progress = `Step ${i + 1} of ${steps.length}: ${s.label}`;
      if (!(await act(() => api.reachStep(s.step))) && s.step !== "level") break;
    }
    progress = cancelled ? "Cancelled." : null;
    working = false;
  }

  async function cancel() {
    cancelled = true;
    await act(() => api.reachCancel());
  }

  async function overrideLevel() {
    const p = Number(percentile);
    if (!percentile.trim() || !Number.isFinite(p)) return onError("Give a percentile from 0 to 100.");
    working = true;
    await act(() => api.reachStep("level", p));
    working = false;
  }

  async function setDepth() {
    const points: [number, number][] = [];
    if (depth.trim()) points.push([0, Number(depth)]);
    for (const line of depthRows.split("\n").filter((l) => l.trim())) {
      const [c, d] = line.split(",").map((x) => Number(x.trim()));
      points.push([c, d]);
    }
    if (points.length === 0) return onError("Give a depth, or depths at chainages.");
    // One depth with rows: the rows hold beyond the first chainage, the one depth from the start.
    await act(() => api.setBed(points.sort((a, b) => a[0] - b[0]), season));
  }

  // --- the map: the route, the picked points and the stretches of the class shown --------------
  function addLayers(m: maplibregl.Map) {
    if (m.getSource("reach")) return;
    m.addSource("reach", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
    const ink = token("plate-ink");
    m.addLayer({ id: "reach-route", type: "line", source: "reach", filter: ["==", ["get", "kind"], "route"], paint: { "line-color": ink, "line-width": 2, "line-dasharray": [3, 2] } });
    m.addLayer({ id: "reach-stretch", type: "line", source: "reach", filter: ["==", ["get", "kind"], "stretch"], paint: { "line-color": token("fail"), "line-width": 6, "line-opacity": 0.8 } });
    m.addLayer({ id: "reach-points", type: "circle", source: "reach", filter: ["==", ["get", "kind"], "point"], paint: { "circle-radius": 6, "circle-color": token("cream"), "circle-stroke-color": ink, "circle-stroke-width": 2 } });
  }

  $effect(() => {
    const m = map;
    if (!m) return;
    const draw = () => {
      addLayers(m);
      const features: GeoJSON.Feature[] = [];
      if (view?.route) features.push({ type: "Feature", properties: { kind: "route" }, geometry: { type: "LineString", coordinates: view.route } });
      for (const t of [startText, endText, ...viaText.split(";")]) {
        const p = parse(t);
        if (p) features.push({ type: "Feature", properties: { kind: "point" }, geometry: { type: "Point", coordinates: p } });
      }
      const rs = fits?.rule_sets[0]?.rule_set;
      for (const s of (rs && shownClass ? view?.fits_stretches[rs]?.[shownClass] : undefined) ?? []) {
        if (s.from && s.to) features.push({ type: "Feature", properties: { kind: "stretch" }, geometry: { type: "LineString", coordinates: [s.from, s.to] } });
      }
      (m.getSource("reach") as maplibregl.GeoJSONSource | undefined)?.setData({ type: "FeatureCollection", features });
    };
    if (m.isStyleLoaded()) draw();
    else m.once("idle", draw);
  });

  const prov = (id: ProvenanceInput) => (provenance as Partial<Record<ProvenanceInput, Provenance>>)[id];
  const yes = (b: boolean) => (b ? "✓ fits" : "✕ does not fit");
  const m1 = (v: number | null | undefined) => (v == null ? "–" : `${v.toFixed(1)} m`);
</script>

<div class="reach" data-testid="reach-view">
  <h2>Start from a river</h2>

  <section aria-label="Pick the reach">
    <h3>1. Pick the reach</h3>
    <label>River <input data-testid="reach-river" bind:value={river} placeholder="for example Sassandra" /></label>
    <label>Start <input data-testid="reach-start" bind:value={startText} placeholder="lon, lat" /><button class:on={picking === "start"} data-testid="reach-pick-start" onclick={() => pick("start")}>Pick on map</button></label>
    <label>End <input data-testid="reach-end" bind:value={endText} placeholder="lon, lat" /><button class:on={picking === "end"} data-testid="reach-pick-end" onclick={() => pick("end")}>Pick on map</button></label>
    <label>Via <input data-testid="reach-via" bind:value={viaText} placeholder="lon, lat; lon, lat" /><button class:on={picking === "via"} data-testid="reach-pick-via" onclick={() => pick("via")}>Add on map</button></label>
    {#if picking}<p class="help" role="status">Click the map to place the {picking} point.</p>{/if}
    <button class="primary" data-testid="reach-route" disabled={working} onclick={route}>Find the route</button>
    {#if view?.route}
      <p data-testid="reach-length">Route: {((view.length_m ?? 0) / 1000).toFixed(1)} km{view.as_drawn ? `, ${view.as_drawn} stretch(es) kept as drawn (check them)` : ""}</p>
    {/if}
  </section>

  {#if view?.route}
    <section aria-label="Make the inputs">
      <h3>2. Make the inputs</h3>
      <p class="help">The JRC water mask tiles this reach needs{isWeb ? " (download them, then give them here: the host does not let a web page read them)" : " (given here, or read from the network)"}:</p>
      <ul class="files">
        {#each view.jrc_tiles as t}
          <li data-testid="reach-jrc-{t.name}" data-given={t.given}><a href={t.url} target="_blank" rel="noreferrer">{t.name}</a> {t.given ? "■ given" : "◇ not given"}</li>
        {/each}
      </ul>
      <button data-testid="reach-add-tile" disabled={working} onclick={() => act(() => api.reachAddTile())}>Give a JRC tile…</button>
      <label class="check"><input type="checkbox" data-testid="reach-edge-fit" bind:checked={edgeFit} /> Move the banks onto Sentinel-2 imagery</label>
      <div class="row">
        <button class="primary" data-testid="reach-generate" disabled={working} onclick={generate}>Make the inputs</button>
        {#if working && progress}<button data-testid="reach-cancel" onclick={cancel}>Cancel</button>{/if}
      </div>
      {#if progress}<p role="status" data-testid="reach-progress">{progress}</p>{/if}

      <table class="marks" data-testid="reach-inputs">
        <thead><tr><th>Input</th><th>Made by</th><th>Source, date</th></tr></thead>
        <tbody>
          {#each INPUTS as i}
            {@const p = prov(i.id)}
            <tr data-testid="reach-input-{i.id}">
              <th>{i.label}</th>
              <td><MarkBadge mark={p?.mark} input={i.id} /></td>
              <td class="src">{p ? `${p.source}, ${p.date}${p.provisional ? ", provisional" : ""}` : ""}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if lowConfidence.length}
        <p class="note" data-testid="reach-low-confidence">{lowConfidence.length} stretch(es) of low-confidence width to check: {[...new Set(lowConfidence.map((s) => s.kind))].join(", ")}.</p>
      {/if}
    </section>

    <section aria-label="DEM">
      <h3>3. DEM</h3>
      <ul class="files">
        {#each view.dem_tiles as t}<li><a href={t.url} target="_blank" rel="noreferrer">{t.name}</a></li>{/each}
      </ul>
      {#if isWeb}
        <p class="help">Download the tiles, then load one as the DEM (drop it on the map, or Load).</p>
      {:else}
        <p class="help">Fetch them (save the study first: the tiles are written next to it), or load a DEM of your own.</p>
        <button data-testid="reach-fetch-dem" onclick={() => act(api.fetchDem)}>Fetch DEM</button>
      {/if}
      <button data-testid="reach-load-dem" onclick={() => act(() => api.loadInput("dem"))}>Load DEM…</button>
    </section>

    <section aria-label="Water level">
      <h3>4. Water level</h3>
      {#if prov("water_level")}
        <p data-testid="reach-level-source">From {prov("water_level")!.source}, {prov("water_level")!.date}.</p>
      {:else}
        <p class="help">Not made yet.</p>
      {/if}
      <label>Another percentile <input data-testid="reach-percentile" bind:value={percentile} placeholder="10" /><button data-testid="reach-level" disabled={working} onclick={overrideLevel}>Use it</button></label>
      <label>Or a level of your own (m) <input data-testid="reach-typed-level" bind:value={typedLevel} /><button data-testid="reach-set-level" onclick={() => act(() => api.setWater(typedLevel, ""))}>Set it</button></label>
    </section>

    <section aria-label="Depth">
      <h3>5. Depth</h3>
      <p class="help">With no survey, give the depth people who use the river know. It is assumed, not measured.</p>
      <label>Depth (m) <input data-testid="bed-depth" bind:value={depth} placeholder="3" /></label>
      <label class="col">Depths at chainages (one <code>chainage_m, depth_m</code> per line) <textarea data-testid="bed-table" rows="3" bind:value={depthRows}></textarea></label>
      <label>Season <input data-testid="bed-season" bind:value={season} /></label>
      <button data-testid="bed-set" onclick={setDepth}>Set the depth</button>
    </section>

    <section aria-label="What fits">
      <h3>6. What fits</h3>
      <button class="primary" data-testid="fits-run" disabled={working} onclick={() => act(api.runFits)}>Answer what fits</button>
      {#if fits}
        <p class="help" data-testid="fits-label">{fits.label}{snap?.fits_status === "stale" ? " — stale: the river changed since; answer again." : ""}</p>
        {#each fits.rule_sets as rs}
          <h4>{rs.rule_set}: largest that fits as it is, <span data-testid="fits-largest-{rs.rule_set}">{rs.largest_fitting ?? "none"}</span></h4>
          <table class="fits" data-testid="fits-table">
            <thead><tr><th>Class</th><th>As it is</th><th>Width short</th><th>Radius short</th><th>Stretches</th><th></th></tr></thead>
            <tbody>
              {#each rs.classes as c}
                <tr data-testid="fits-row-{c.vessel_standard_id}" data-fits={c.fits} class:shown={shownClass === c.vessel_standard_id} onclick={() => (shownClass = c.vessel_standard_id)}>
                  <th>{c.class_name}</th>
                  <td class:pass={c.fits} class:fail={!c.fits}>{yes(c.fits)}</td>
                  <td>{m1(c.worst_width_deficit_m)}</td>
                  <td>{m1(c.worst_radius_deficit_m)}</td>
                  <td>{c.stretch_count}</td>
                  <td><button data-testid="use-vessel-{c.vessel_standard_id}" onclick={() => onUseVessel(rs.rule_set, rs.selection, c.vessel_standard_id)}>Use this vessel</button></td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/each}
      {/if}
    </section>
  {/if}
</div>

<style>
  .reach { padding: var(--sp-3) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); font-size: var(--fs-13); }
  h2 { margin: 0; font-size: var(--fs-16); }
  h3 { margin: 0 0 var(--sp-1); font-size: var(--fs-13); }
  h4 { margin: var(--sp-2) 0 var(--sp-1); font-size: var(--fs-12); font-weight: 600; }
  section { display: flex; flex-direction: column; gap: var(--sp-1); border-top: var(--lw-thin) solid var(--rule); padding-top: var(--sp-2); }
  label { display: flex; gap: var(--sp-1); align-items: center; flex-wrap: wrap; }
  label.col { flex-direction: column; align-items: stretch; }
  label input:not([type]) { flex: 1; min-width: 6em; }
  .check { gap: var(--sp-2); }
  .row { display: flex; gap: var(--sp-2); }
  button.on { background: var(--accent); color: var(--accent-ink); }
  .help { margin: 0; color: var(--ink-3); }
  .note { margin: 0; padding: 2px var(--sp-2); background: var(--warn-soft); color: var(--warn); }
  .note::before { content: "! "; font-weight: 700; }
  .files { margin: 0; padding-left: var(--sp-4); }
  .marks, .fits { border-collapse: collapse; width: 100%; font-size: var(--fs-12); }
  .marks th, .marks td, .fits th, .fits td { text-align: left; padding: 1px var(--sp-1); border-bottom: var(--lw-thin) solid var(--rule); }
  .src { color: var(--ink-3); }
  .pass { color: var(--pass); }
  .fail { color: var(--fail); }
  tr.shown { background: var(--surface-2); }
  .fits tbody tr { cursor: pointer; }
</style>
