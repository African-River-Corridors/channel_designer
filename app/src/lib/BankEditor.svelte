<script lang="ts">
  // The bank-line editor (spec 0012, ticket #236): a vertex editor over the plan map. It only sends
  // what the user did to the session (`editBank`) and draws what comes back. The session splices,
  // checks for crossings, marks the line `edited`, rebuilds the polygon and centreline, and makes
  // every later stage stale.
  import * as maplibregl from "maplibre-gl";
  import type * as GeoJSON from "geojson";
  import * as api from "./api";
  import type { BankEdit, LonLat, MapGeometry, ProvenanceInput, Snapshot } from "./api";
  import MarkBadge from "./MarkBadge.svelte";
  import { token } from "./theme";

  let {
    map,
    geometry,
    snap,
    onSnap,
    active = $bindable(false),
  }: {
    map: maplibregl.Map | undefined;
    geometry: MapGeometry | null;
    snap: Snapshot | null;
    /** The snapshot a successful edit or undo returned. */
    onSnap: (next: Snapshot) => void;
    /** True while editing: the parent stops using map clicks for anything else. */
    active?: boolean;
  } = $props();

  type Tool = "move" | "add" | "delete" | "redraw";
  const TOOLS: { id: Tool; label: string; help: string }[] = [
    { id: "move", label: "Move", help: "Drag a vertex" },
    { id: "add", label: "Add", help: "Click on the bank to add a vertex" },
    { id: "delete", label: "Delete", help: "Click a vertex to delete it" },
    { id: "redraw", label: "Redraw", help: "Click points along a new stretch, then Finish" },
  ];
  /** Past this many vertices in view the editor asks the user to zoom in. */
  const MAX_VERTICES = 400;
  const INPUTS: { id: ProvenanceInput; label: string }[] = [
    { id: "left_bank", label: "Left bank" },
    { id: "right_bank", label: "Right bank" },
    { id: "river_polygon", label: "River polygon" },
    { id: "centreline", label: "Centreline" },
  ];

  let side = $state<"left" | "right">("left");
  let tool = $state<Tool>("move");
  let message = $state<string | null>(null);
  let draft = $state<LonLat[]>([]);
  let crowded = $state(false);
  let working = $state(false);
  /** Bumped when the map moves, or when markers must be rebuilt. */
  let tick = $state(0);
  let markers: maplibregl.Marker[] = [];

  const role = $derived(side === "left" ? ("left_bank" as const) : ("right_bank" as const));
  const bank = $derived((geometry?.banks.find((b) => b.side === side)?.line.coordinates ?? null) as LonLat[] | null);
  const hasBanks = $derived((geometry?.banks.length ?? 0) > 0);

  async function send(edit: BankEdit) {
    if (working) return;
    working = true;
    message = null;
    try {
      onSnap(await api.editBank(role, edit));
    } catch (e) {
      message = String(e);
      tick++; // put a dragged vertex back where the session still has it
    } finally {
      working = false;
    }
  }

  async function undo() {
    if (working || !snap?.can_undo_bank_edit) return;
    working = true;
    message = null;
    try {
      onSnap(await api.undoBankEdit());
    } catch (e) {
      message = String(e);
    } finally {
      working = false;
    }
  }

  async function finish() {
    if (draft.length < 2) {
      message = "Click at least two points along the new stretch.";
      return;
    }
    const points = draft;
    draft = [];
    await send({ kind: "splice", points });
  }

  // --- vertex markers ---------------------------------------------------------------------
  function clearMarkers() {
    markers.forEach((m) => m.remove());
    markers = [];
  }

  function drawMarkers() {
    clearMarkers();
    crowded = false;
    if (!map || !active || !bank) return;
    const view = map.getBounds().toArray() as [LonLat, LonLat];
    const [[w, s], [e, n]] = view;
    const shown: number[] = [];
    bank.forEach((p, i) => {
      if (p[0] >= w && p[0] <= e && p[1] >= s && p[1] <= n) shown.push(i);
    });
    if (shown.length > MAX_VERTICES) {
      crowded = true;
      return;
    }
    for (const i of shown) {
      const el = document.createElement("button");
      el.className = "bank-vertex";
      el.type = "button";
      el.dataset.testid = "bank-vertex";
      el.dataset.index = String(i);
      el.title = `Vertex ${i}`;
      el.setAttribute("aria-label", `Bank vertex ${i}`);
      el.style.pointerEvents = tool === "move" || tool === "delete" ? "auto" : "none";
      const marker = new maplibregl.Marker({ element: el, draggable: tool === "move" }).setLngLat(bank[i]);
      el.addEventListener("click", (ev) => {
        ev.stopPropagation();
        if (tool === "delete") send({ kind: "delete", index: i });
      });
      marker.on("dragend", () => {
        const at = marker.getLngLat();
        send({ kind: "move", index: i, to: [at.lng, at.lat] });
      });
      markers.push(marker.addTo(map));
    }
  }

  // --- map clicks: add a vertex, or a point of a redrawn stretch --------------------------
  /** The index of the segment of the bank nearest the click, if it is close on the screen. */
  function segmentAt(pt: maplibregl.Point): number | null {
    if (!map || !bank) return null;
    let best: { i: number; d: number } | null = null;
    for (let i = 0; i + 1 < bank.length; i++) {
      const a = map.project(bank[i]);
      const b = map.project(bank[i + 1]);
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const len2 = dx * dx + dy * dy;
      const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((pt.x - a.x) * dx + (pt.y - a.y) * dy) / len2));
      const d = Math.hypot(pt.x - (a.x + t * dx), pt.y - (a.y + t * dy));
      if (!best || d < best.d) best = { i, d };
    }
    return best && best.d <= 14 ? best.i : null;
  }

  function onClick(e: maplibregl.MapMouseEvent) {
    if (!active) return;
    const at: LonLat = [e.lngLat.lng, e.lngLat.lat];
    if (tool === "add") {
      const i = segmentAt(e.point);
      if (i === null) message = "Click on the bank line to add a vertex there.";
      else send({ kind: "add", after: i, at });
    } else if (tool === "redraw") {
      message = null;
      draft = [...draft, at];
    }
  }

  // --- the draft of a redrawn stretch -----------------------------------------------------
  function drawDraft() {
    if (!map) return;
    const src = map.getSource("bank-draft") as maplibregl.GeoJSONSource | undefined;
    const features: GeoJSON.Feature[] = [];
    if (draft.length >= 2) features.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: draft } });
    for (const p of draft) features.push({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: p } });
    src?.setData({ type: "FeatureCollection", features });
  }

  function addLayers(m: maplibregl.Map) {
    if (m.getSource("bank-draft")) return;
    m.addSource("bank-draft", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
    const ink = token("plate-ink");
    m.addLayer({ id: "bank-draft-line", type: "line", source: "bank-draft", filter: ["==", "$type", "LineString"], paint: { "line-color": ink, "line-width": 3, "line-dasharray": [2, 2] } });
    m.addLayer({ id: "bank-draft-points", type: "circle", source: "bank-draft", filter: ["==", "$type", "Point"], paint: { "circle-radius": 5, "circle-color": token("cream"), "circle-stroke-color": ink, "circle-stroke-width": 2 } });
  }

  $effect(() => {
    const m = map;
    if (!m) return;
    addLayers(m);
    const moved = () => tick++;
    m.on("click", onClick);
    m.on("moveend", moved);
    return () => {
      m.off("click", onClick);
      m.off("moveend", moved);
    };
  });

  $effect(() => {
    void tick;
    void bank;
    void tool;
    void active;
    drawMarkers();
    return clearMarkers;
  });

  $effect(() => {
    void draft;
    drawDraft();
  });

  $effect(() => {
    if (tool !== "redraw" || !active) draft = [];
  });

  $effect(() => {
    // The bank changed (the other side, an undo): a half-drawn stretch no longer fits it.
    void side;
    draft = [];
  });

  function onKey(e: KeyboardEvent) {
    if (!active) return;
    const typing = (e.target as HTMLElement | null)?.closest("input, textarea, select");
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !typing) {
      e.preventDefault();
      undo();
    } else if (e.key === "Enter" && tool === "redraw" && !typing) {
      finish();
    } else if (e.key === "Escape") {
      draft = [];
    }
  }

  const mark = (id: ProvenanceInput) => snap?.document.provenance?.[id];
</script>

<svelte:window onkeydown={onKey} />

<div class="editor" class:open={active} data-testid="bank-editor">
  {#if !active}
    <button class="toggle" data-testid="bank-edit-toggle" disabled={!hasBanks} title={hasBanks ? "Edit the bank lines on the map" : "Load or generate bank lines first"} onclick={() => (active = true)}>Edit banks</button>
  {:else}
    <div class="panel" role="group" aria-label="Bank line editor">
      <div class="row">
        <strong>Edit banks</strong>
        <button data-testid="bank-edit-done" onclick={() => (active = false)}>Done</button>
      </div>
      <div class="row" role="radiogroup" aria-label="Bank">
        {#each ["left", "right"] as s}
          <button role="radio" aria-checked={side === s} class:on={side === s} data-testid="bank-side-{s}" onclick={() => (side = s as "left" | "right")}>{s === "left" ? "Left" : "Right"} bank</button>
        {/each}
      </div>
      <div class="row" role="radiogroup" aria-label="Tool">
        {#each TOOLS as t}
          <button role="radio" aria-checked={tool === t.id} class:on={tool === t.id} title={t.help} data-testid="bank-tool-{t.id}" onclick={() => (tool = t.id)}>{t.label}</button>
        {/each}
        <button data-testid="bank-undo" disabled={!snap?.can_undo_bank_edit || working} title="Undo the last edit (Ctrl or Cmd + Z)" onclick={undo}>Undo</button>
      </div>
      <p class="help">{TOOLS.find((t) => t.id === tool)?.help}.</p>
      {#if tool === "redraw"}
        <div class="row">
          <button class="primary" data-testid="bank-finish" disabled={draft.length < 2 || working} onclick={finish}>Finish ({draft.length} points)</button>
          <button disabled={draft.length === 0} onclick={() => (draft = [])}>Clear</button>
        </div>
      {/if}
      {#if crowded}
        <p class="note" role="status" data-testid="bank-crowded">Too many vertices in view. Zoom in to edit them.</p>
      {/if}
      {#if message}
        <p class="alert" role="alert" data-testid="bank-message">{message}</p>
      {/if}
      <table class="marks">
        <tbody>
          {#each INPUTS as i}
            <tr><th scope="row">{i.label}</th><td><MarkBadge mark={mark(i.id)?.mark} input={i.id} /></td></tr>
          {/each}
        </tbody>
      </table>
      <p class="help">An edit rebuilds the river polygon and centreline. Every stage after them goes out of date. Nothing reruns by itself.</p>
    </div>
  {/if}
</div>

<style>
  .editor { position: absolute; top: var(--sp-3); right: 52px; z-index: 3; max-width: calc(100% - 2 * var(--sp-3) - 40px); }
  .panel { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-2) var(--sp-3); background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); font-size: var(--fs-12); width: 300px; max-width: 100%; box-sizing: border-box; }
  .row { display: flex; flex-wrap: wrap; gap: var(--sp-1); align-items: center; justify-content: space-between; }
  .row button { padding: 2px var(--sp-2); }
  .row button.on { background: var(--accent); color: var(--accent-ink); }
  .toggle { box-shadow: 0 0 0 1px var(--ink); }
  .help { margin: 0; color: var(--ink-3); }
  .note { margin: 0; padding: 2px var(--sp-2); background: var(--warn-soft); color: var(--warn); }
  .note::before { content: "! "; font-weight: 700; }
  .alert { margin: 0; padding: 2px var(--sp-2); background: var(--fail-soft); color: var(--fail); }
  .alert::before { content: "✕ "; font-weight: 700; }
  .marks { border-collapse: collapse; width: 100%; }
  .marks th { text-align: left; font-weight: 400; padding: 1px 0; color: var(--ink-2); }
  .marks td { text-align: right; padding: 1px 0; }
  :global(.bank-vertex) { width: 14px; height: 14px; padding: 0; border-radius: 50%; background: var(--cream); border: 2px solid var(--plate-ink); box-shadow: 0 0 0 1px var(--cream); cursor: grab; }
  :global(.bank-vertex:focus-visible) { outline: 2px solid var(--plate-ink); outline-offset: 2px; }
</style>
