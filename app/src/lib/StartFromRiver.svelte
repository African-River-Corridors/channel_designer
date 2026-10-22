<script lang="ts">
  // "Start from a river" (spec 0012, ticket #235). The user picks a reach; the session plans the
  // route, makes every input from open data one step at a time, takes a depth, and answers "what
  // fits". This view only sends what the user did and shows what comes back: every field is the
  // session's.
  import type * as maplibregl from "maplibre-gl";
  import type * as GeoJSON from "geojson";
  import * as api from "./api";
  import { docPart } from "./api";
  import type { FitsResult, FitsSelection, LonLat, Provenance, ProvenanceInput, ReachStepName, RiverHit, Snapshot } from "./api";
  import type { ReachProgress } from "./platform/types";
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
  let country = $state("");
  let hits = $state<RiverHit[]>([]);
  let listOpen = $state(false);
  let active = $state(-1);
  let searching = $state(false);
  /** Why the search as you type failed; then Enter asks Nominatim once. */
  let failed = $state<string | null>(null);
  /** The source of the hits shown: Photon as you type, or Nominatim once. */
  let hitSource = $state("osm-photon");
  /** The hit the user chose; cleared when they type again. */
  let chosen = $state<RiverHit | null>(null);
  /** Set when the user types in the search box: they let go of the river the session holds. */
  let letGo = $state(false);
  /** Answers by country and text, and river lengths by OSM id once known: a repeat asks nothing. */
  const answers = new Map<string, RiverHit[]>();
  const lengths = new Map<string, number>();
  let asked = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Wait this long after the last key before asking (owd_reach::search::DEBOUNCE_MS). */
  const DEBOUNCE_MS = 350;
  const MIN_CHARS = 3;
  let startText = $state("");
  let endText = $state("");
  let viaText = $state("");
  let picking = $state<"start" | "end" | "via" | null>(null);
  let edgeFit = $state(true);
  let progress = $state<string | null>(null);
  // --- what a long network step is doing (#320) ---------------------------------------------
  /** The latest report of the step that is running: step `done` of `total`, in words. */
  let detail = $state<ReachProgress | null>(null);
  let started = $state(0);
  let now = $state(0);
  $effect(() => {
    let off: (() => void) | undefined;
    platform()
      .onReachProgress((p) => (detail = p))
      .then((u) => (off = u))
      .catch(() => {});
    return () => off?.();
  });
  $effect(() => {
    if (!working) return;
    started = Date.now();
    now = started;
    detail = null;
    const tick = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(tick);
  });
  const elapsed = $derived.by(() => {
    const t = Math.max(0, Math.round((now - started) / 1000));
    return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, "0")}`;
  });
  /** One long step: what it is, for the status line, and the work. */
  async function long(label: string, work: () => Promise<Snapshot | null | void>) {
    progress = label;
    working = true;
    const ok = await act(work);
    working = false;
    progress = null;
    return ok;
  }
  let working = $state(false);
  // The QA overlay (spec 0013): the stretch to draw, in km along the centreline, and where it went.
  let qaFrom = $state<number | null>(null);
  let qaTo = $state<number | null>(null);
  let qaSaved = $state<string | null>(null);
  let cancelled = false;
  let percentile = $state("");
  let typedLevel = $state("");
  let depth = $state("");
  let depthRows = $state("");
  let season = $state("dry");
  let shownClass = $state<string | null>(null);

  const view = $derived(snap?.reach_view);
  /** The reach the document records: an opened study has one before its route is drawn again. */
  const docReach = $derived(docPart<{ start: LonLat; end: LonLat; length_m?: number }>(snap, "reach"));
  /** Reaches are cut out of the river the session holds, until the user types a new search. */
  const holding = $derived(!!view?.river && !letGo);
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

  // --- the river search: a list as the user types ------------------------------------------------
  function typed() {
    chosen = null;
    letGo = true;
    clearTimeout(timer);
    const text = river.trim();
    if (text.length < MIN_CHARS) {
      hits = [];
      listOpen = false;
      return;
    }
    timer = setTimeout(() => search(text), DEBOUNCE_MS);
  }
  async function search(text: string) {
    const key = `${country.trim().toUpperCase()}|${text.toLowerCase()}`;
    const mine = ++asked;
    let found = answers.get(key);
    let why: string | null = null;
    if (!found) {
      searching = true;
      try {
        found = await api.reachSearch(text, country);
        answers.set(key, found);
      } catch (e) {
        why = String(e);
        found = [];
      } finally {
        if (mine === asked) searching = false;
      }
    }
    if (mine !== asked) return; // a later key won
    failed = why;
    hitSource = "osm-photon";
    hits = found;
    active = found.length ? 0 : -1;
    listOpen = true;
  }
  /** Nominatim, one request, when the user presses Enter after the search as you type failed. */
  async function searchOnce() {
    const text = river.trim();
    if (text.length < MIN_CHARS) return;
    const mine = ++asked;
    searching = true;
    try {
      const found = await api.reachSearch(text, country, true);
      if (mine !== asked) return;
      failed = null;
      hitSource = "osm-nominatim";
      hits = found;
      active = found.length ? 0 : -1;
      listOpen = true;
    } catch (e) {
      if (mine === asked) failed = `The river search (Nominatim) did not answer either: ${String(e)}`;
    } finally {
      if (mine === asked) searching = false;
    }
  }
  async function choose(h: RiverHit) {
    chosen = h;
    letGo = false;
    river = h.label;
    listOpen = false;
    const ok = await long(`Reading the ${h.name} from OpenStreetMap`, () => api.reachRiver(h.osm_id, h.also, hitSource));
    if (ok && map) map.fitBounds([[h.bbox[0], h.bbox[1]], [h.bbox[2], h.bbox[3]]], { padding: 40, duration: 0 });
  }
  function keys(e: KeyboardEvent) {
    if (e.key === "Enter" && failed && !chosen) {
      e.preventDefault();
      searchOnce();
      return;
    }
    if (!listOpen || hits.length === 0) return;
    if (e.key === "ArrowDown") active = (active + 1) % hits.length;
    else if (e.key === "ArrowUp") active = (active - 1 + hits.length) % hits.length;
    else if (e.key === "Enter" && active >= 0) choose(hits[active]);
    else if (e.key === "Escape") listOpen = false;
    else return;
    e.preventDefault();
  }
  const km = (m: number) => `${(m / 1000).toFixed(m < 10000 ? 1 : 0)} km`;
  // The session's start and end fill the fields when they change (a river chosen, a reach cut, a
  // study opened).
  let shownEnds = "";
  $effect(() => {
    const start = view?.start ?? docReach?.start;
    const end = view?.end ?? docReach?.end;
    const k = JSON.stringify([start, end]);
    if (k === shownEnds) return;
    shownEnds = k;
    if (start) startText = fmt(start);
    if (end) endText = fmt(end);
  });
  // A river the session holds anew (chosen, or an opened study drawn again) is held until the user
  // types a new search.
  let heldRiver = "";
  $effect(() => {
    const id = view?.river?.osm_id ?? "";
    if (id === heldRiver) return;
    heldRiver = id;
    if (id) {
      letGo = false;
      if (!chosen && view?.river?.name) river = view.river.name;
    }
  });
  // An opened study keeps its points and way ids, not the route's line: draw it again from OSM, once
  // per reach (#318).
  let restoring = $state(false);
  let restoredFor = "";
  async function restore() {
    restoring = true;
    await long("Drawing the route again from OpenStreetMap", () => api.reachRestore());
    restoring = false;
  }
  $effect(() => {
    if (!view?.restore || working) return;
    const k = JSON.stringify(docReach);
    if (k === restoredFor) return;
    restoredFor = k;
    restore();
  });
  $effect(() => {
    if (view?.river) lengths.set(view.river.osm_id, view.river.length_m);
  });

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
    // With a river chosen, the reach is cut out of it; otherwise the route is planned as before.
    await long("Finding the route", () => (holding ? api.reachCut(start, end, via) : api.reachRoute(river, start, end, via)));
  }

  /** The steps after the route, one call each, so Cancel can stop between them. */
  async function generate() {
    const steps: { step: ReachStepName; label: string }[] = [{ step: "banks", label: "banks from Sentinel-2 water" }];
    if (edgeFit) steps.push({ step: "edge_fit", label: "banks moved onto Sentinel-2" });
    steps.push({ step: "level", label: "water level from SWORD and SWOT" });
    cancelled = false;
    for (const [i, s] of steps.entries()) {
      if (cancelled) break;
      if (!(await long(`Step ${i + 1} of ${steps.length}: ${s.label}`, () => api.reachStep(s.step))) && s.step !== "level") break;
    }
    if (cancelled) onError("Cancelled: the study is as it was before the step.");
  }

  /** Draw the banks over the Sentinel-2 image of their scenes and save the PNG and the KMZ. */
  async function qaOverlay() {
    working = true;
    qaSaved = null;
    progress = "Drawing the banks on the Sentinel-2 image";
    await act(async () => {
      qaSaved = await api.exportQaOverlay(qaFrom ?? undefined, qaTo ?? undefined);
    });
    progress = null;
    working = false;
  }

  async function cancel() {
    cancelled = true;
    await act(() => api.reachCancel());
  }

  async function overrideLevel() {
    const p = Number(percentile);
    if (!percentile.trim() || !Number.isFinite(p)) return onError("Give a percentile from 0 to 100.");
    await long("Water level from SWORD and SWOT", () => api.reachStep("level", p));
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
    // A light casing under the route and the river, so the dark line reads on satellite imagery.
    m.addLayer({ id: "reach-casing", type: "line", source: "reach", filter: ["match", ["get", "kind"], ["route", "river"], true, false], paint: { "line-color": token("cream"), "line-width": 5, "line-opacity": 0.7 } });
    m.addLayer({ id: "reach-river", type: "line", source: "reach", filter: ["==", ["get", "kind"], "river"], paint: { "line-color": ink, "line-width": 1.5, "line-opacity": 0.45 } });
    m.addLayer({ id: "reach-route", type: "line", source: "reach", filter: ["==", ["get", "kind"], "route"], paint: { "line-color": ink, "line-width": 2, "line-dasharray": [3, 2] } });
    m.addLayer({ id: "reach-stretch", type: "line", source: "reach", filter: ["==", ["get", "kind"], "stretch"], paint: { "line-color": token("fail"), "line-width": 6, "line-opacity": 0.8 } });
    m.addLayer({ id: "reach-points", type: "circle", source: "reach", filter: ["==", ["get", "kind"], "point"], paint: { "circle-radius": 6, "circle-color": token("cream"), "circle-stroke-color": ink, "circle-stroke-width": 2 } });
  }

  $effect(() => {
    const m = map;
    if (!m) return;
    // The features are made here, so the effect follows every value they read; the map takes them
    // now, or once its style is ready (tiles still loading do not hold them back).
    const features: GeoJSON.Feature[] = [];
    if (view?.river) features.push({ type: "Feature", properties: { kind: "river" }, geometry: { type: "LineString", coordinates: view.river.line } });
    if (view?.route) features.push({ type: "Feature", properties: { kind: "route" }, geometry: { type: "LineString", coordinates: view.route } });
    const ends: [string, string][] = [["start", startText], ["end", endText], ...viaText.split(";").map((t): [string, string] => ["via", t])];
    for (const [role, t] of ends) {
      const p = parse(t);
      if (p) features.push({ type: "Feature", properties: { kind: "point", role }, geometry: { type: "Point", coordinates: p } });
    }
    const rs = fits?.rule_sets[0]?.rule_set;
    for (const s of (rs && shownClass ? view?.fits_stretches[rs]?.[shownClass] : undefined) ?? []) {
      if (s.from && s.to) features.push({ type: "Feature", properties: { kind: "stretch" }, geometry: { type: "LineString", coordinates: [s.from, s.to] } });
    }
    const draw = () => {
      addLayers(m);
      (m.getSource("reach") as maplibregl.GeoJSONSource | undefined)?.setData({ type: "FeatureCollection", features });
    };
    try {
      draw();
    } catch {
      m.once("styledata", draw); // the style itself is not loaded yet
    }
  });

  // Drag Start or End along the river: the reach is cut again where it is let go.
  $effect(() => {
    const m = map;
    if (!m) return;
    let dragging: "start" | "end" | null = null;
    const down = (e: maplibregl.MapLayerMouseEvent) => {
      const role = e.features?.[0]?.properties?.role;
      if (!holding || (role !== "start" && role !== "end")) return;
      e.preventDefault();
      dragging = role;
      m.getCanvas().style.cursor = "grabbing";
    };
    // The dragged point glides along the river: the nearest point of the line the session gave.
    const onRiver = (lon: number, lat: number): LonLat => {
      const k = Math.cos((lat * Math.PI) / 180);
      let best: LonLat = [lon, lat];
      let d = Infinity;
      for (const q of view?.river?.line ?? []) {
        const dd = ((q[0] - lon) * k) ** 2 + (q[1] - lat) ** 2;
        if (dd < d) [d, best] = [dd, q];
      }
      return best;
    };
    const move = (e: maplibregl.MapMouseEvent) => {
      if (!dragging) return;
      const p = fmt(onRiver(e.lngLat.lng, e.lngLat.lat));
      if (dragging === "start") startText = p;
      else endText = p;
    };
    const up = () => {
      if (!dragging) return;
      dragging = null;
      m.getCanvas().style.cursor = "";
      route();
    };
    const over = () => holding && (m.getCanvas().style.cursor = "grab");
    const out = () => !dragging && (m.getCanvas().style.cursor = "");
    m.on("mousedown", "reach-points", down);
    m.on("mouseenter", "reach-points", over);
    m.on("mouseleave", "reach-points", out);
    m.on("mousemove", move);
    m.on("mouseup", up);
    return () => {
      m.off("mousedown", "reach-points", down);
      m.off("mouseenter", "reach-points", over);
      m.off("mouseleave", "reach-points", out);
      m.off("mousemove", move);
      m.off("mouseup", up);
    };
  });

  const prov = (id: ProvenanceInput) => (provenance as Partial<Record<ProvenanceInput, Provenance>>)[id];
  const yes = (b: boolean) => (b ? "✓ fits" : "✕ does not fit");
  const m1 = (v: number | null | undefined) => (v == null ? "–" : `${v.toFixed(1)} m`);
</script>

<div class="reach" data-testid="reach-view">
  <h2>Start from a river</h2>
  {#if working && progress}
    <div class="status" role="status" aria-live="polite" data-testid="reach-status">
      <p class="phase" data-testid="reach-progress">{progress}</p>
      <progress data-testid="reach-bar" max={detail?.total || undefined} value={detail?.total ? detail.done : undefined}></progress>
      <p class="now"><span data-testid="reach-now">{detail?.what ?? "starting…"}</span> <span class="time" data-testid="reach-elapsed">{elapsed}</span></p>
      <button data-testid="reach-cancel" onclick={cancel}>Cancel</button>
    </div>
  {/if}

  <section aria-label="Pick the reach">
    <h3>1. Pick the reach</h3>
    <div class="search">
      <label>River
        <input
          data-testid="reach-river"
          role="combobox"
          aria-expanded={listOpen}
          aria-controls="reach-river-list"
          aria-autocomplete="list"
          aria-activedescendant={listOpen && active >= 0 ? `reach-hit-${active}` : undefined}
          autocomplete="off"
          bind:value={river}
          oninput={typed}
          onkeydown={keys}
          onfocus={() => hits.length && !chosen && (listOpen = true)}
          placeholder="type 3 letters, for example Ank"
        />
        <input class="cc" data-testid="reach-country" bind:value={country} oninput={typed} maxlength="2" placeholder="Any" title="Country code, optional (for example GH)" aria-label="Country code (optional), for example GH" />
      </label>
      {#if listOpen}
        <div class="dropdown">
        {#if failed}<p class="failed" role="alert" data-testid="reach-search-failed">{failed}</p>{/if}
        <ul class="hits" id="reach-river-list" role="listbox" data-testid="reach-hits">
          {#each hits as h, i}
            {@const len = lengths.get(h.osm_id)}
            <li id="reach-hit-{i}" role="option" aria-selected={i === active} class:active={i === active} data-testid="reach-hit" data-osm={h.osm_id}>
              <button type="button" onmousedown={(e) => e.preventDefault()} onclick={() => choose(h)}>
                <span class="name">{h.label}</span>
                <span class="meta">{h.kind}{len ? ` · ${km(len)}` : ""}</span>
              </button>
            </li>
          {:else}
            {#if !failed}<li class="none" data-testid="reach-no-hits">No river or canal found.</li>{/if}
          {/each}
        </ul>
        <p class="credit" data-testid="reach-search-credit">© OpenStreetMap contributors (ODbL) · search by {hitSource === "osm-nominatim" ? "Nominatim" : "Photon"}</p>
        </div>
      {/if}
      {#if searching}<p class="help" role="status">Searching…</p>{/if}
    </div>
    <label>Start <input data-testid="reach-start" bind:value={startText} placeholder="lon, lat" /><button class:on={picking === "start"} data-testid="reach-pick-start" onclick={() => pick("start")}>Pick on map</button></label>
    <label>End <input data-testid="reach-end" bind:value={endText} placeholder="lon, lat" /><button class:on={picking === "end"} data-testid="reach-pick-end" onclick={() => pick("end")}>Pick on map</button></label>
    <label>Via <input data-testid="reach-via" bind:value={viaText} placeholder="lon, lat; lon, lat" /><button class:on={picking === "via"} data-testid="reach-pick-via" onclick={() => pick("via")}>Add on map</button></label>
    {#if picking}<p class="help" role="status">Click the map to place the {picking} point.</p>{/if}
    {#if holding && view?.river}<p class="help" data-testid="reach-river-help">Start is the mouth and End the upstream end of the {view.river.name ?? "river"} ({km(view.river.length_m)}). Drag either on the map, or pick one, to cut a reach.</p>{/if}
    <button class="primary" data-testid="reach-route" disabled={working} onclick={route}>Find the route</button>
    {#if restoring}<p class="help" role="status" data-testid="reach-restoring">Drawing the route again from OpenStreetMap…</p>{/if}
    {#if view?.restore && !restoring}<button data-testid="reach-restore" onclick={restore}>Draw the route again</button>{/if}
    {#if view?.route}
      <p data-testid="reach-length">Route: {((view.length_m ?? 0) / 1000).toFixed(1)} km{view.as_drawn ? `, ${view.as_drawn} stretch(es) kept as drawn (check them)` : ""}</p>
      {#if view.route_notes?.length}<p class="help" data-testid="reach-route-notes">Overpass: {view.route_notes.join("; ")}.</p>{/if}
    {/if}
  </section>

  {#if view && (view.route || docReach)}
    <section aria-label="Make the inputs">
      <h3>2. Make the inputs</h3>
      <p class="help">The JRC water mask tiles this reach needs{isWeb ? " (download them, then give them here: the host does not let a web page read them)" : " (downloaded into the study's folder when you make the inputs, with the licence notice; or give your own here)"}:</p>
      <ul class="files">
        {#each view.jrc_tiles as t}
          <li data-testid="reach-jrc-{t.name}" data-given={t.given}><a href={t.url} target="_blank" rel="noreferrer">{t.name}</a> {t.local ? "■ in the study's folder" : t.given ? "■ given" : "◇ not given"}</li>
        {/each}
      </ul>
      <button data-testid="reach-add-tile" disabled={working} onclick={() => act(() => api.reachAddTile())}>Give a JRC tile…</button>
      <label class="check"><input type="checkbox" data-testid="reach-edge-fit" bind:checked={edgeFit} /> Move the banks onto Sentinel-2 imagery</label>
      <div class="row">
        <button class="primary" data-testid="reach-generate" disabled={working} onclick={generate}>Make the inputs</button>
      </div>

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
      {#if prov("left_bank")}
        <p class="help">Check the banks by eye: they are drawn over the Sentinel-2 image, with the stretches to check, as a picture and a KMZ for Google Earth. At most 30 km at a time.</p>
        <div class="row">
          <label>From km <input type="number" min="0" step="0.1" data-testid="reach-qa-from" bind:value={qaFrom} /></label>
          <label>to km <input type="number" min="0" step="0.1" data-testid="reach-qa-to" bind:value={qaTo} /></label>
          <button data-testid="reach-qa-overlay" disabled={working} onclick={qaOverlay}>Check the banks on the image…</button>
        </div>
        {#if qaSaved}<p role="status" data-testid="reach-qa-saved">Saved: {qaSaved}</p>{/if}
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
        <button data-testid="reach-fetch-dem" disabled={working} onclick={() => long("Fetching the Copernicus DEM", api.fetchDem)}>Fetch DEM</button>
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
  .status { position: sticky; top: 0; z-index: 4; display: grid; grid-template-columns: 1fr auto; gap: 2px var(--sp-2); align-items: center; padding: var(--sp-2); background: var(--surface); border: var(--lw-thin) solid var(--rule); }
  .status .phase { margin: 0; font-weight: 600; grid-column: 1; }
  .status progress { grid-column: 1; width: 100%; }
  .status .now { margin: 0; grid-column: 1; color: var(--ink-3); }
  .status .time { font-variant-numeric: tabular-nums; margin-left: var(--sp-2); }
  .status button { grid-column: 2; grid-row: 1 / span 3; }
  .search { position: relative; display: flex; flex-direction: column; gap: var(--sp-1); }
  .search input.cc { flex: 0 0 3.5em; min-width: 0; text-transform: uppercase; }
  .search input.cc::placeholder { text-transform: none; }
  .dropdown { position: absolute; top: 100%; left: 0; right: 0; z-index: 5; margin-top: 2px; background: var(--surface); border: var(--lw-thin) solid var(--rule); box-shadow: 0 4px 12px rgb(0 0 0 / 0.25); }
  .hits { margin: 0; padding: 0; list-style: none; max-height: 16em; overflow-y: auto; }
  .dropdown .failed { margin: 0; padding: 4px var(--sp-2); background: var(--warn-soft); color: var(--warn); }
  .hits li button { display: flex; justify-content: space-between; gap: var(--sp-2); width: 100%; text-align: left; border: 0; border-radius: 0; background: none; padding: 4px var(--sp-2); }
  .hits li.active button, .hits li button:hover { background: var(--accent); color: var(--accent-ink); }
  .hits .meta { color: inherit; opacity: 0.7; white-space: nowrap; }
  .hits .none, .dropdown .credit { padding: 4px var(--sp-2); color: var(--ink-3); }
  .dropdown .credit { margin: 0; font-size: var(--fs-11, 11px); border-top: var(--lw-thin) solid var(--rule); }
</style>
