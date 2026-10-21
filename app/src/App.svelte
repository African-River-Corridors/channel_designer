<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import type { Choices, CrossSection as CrossSectionData, Hillshade, InputRole, LongSection as LongSectionData, MapGeometry, OptionTable, RunMethod, Snapshot, StageName } from "./lib/api";
  import CrossSection from "./lib/CrossSection.svelte";
  import LongSection from "./lib/LongSection.svelte";
  import BottomPanel from "./lib/BottomPanel.svelte";
  import { optionVar } from "./lib/theme";
  import { BASEMAP_TILES } from "./lib/basemap";
  import MapPanel from "./lib/MapPanel.svelte";
  import LeftPanel from "./lib/LeftPanel.svelte";
  import markLight from "./assets/owd-mark.svg";
  import markDark from "./assets/owd-mark-reversed-clear.svg";

  // Display state only: the channel design, its stage status and its results all come from Rust.
  let snap = $state<Snapshot | null>(null);
  let choices = $state<Choices | null>(null);
  let error = $state<string | null>(null);
  let geometry = $state<MapGeometry | null>(null);
  let selectedId = $state<string | null>(null);
  let table = $state<OptionTable | null>(null);
  let longSection = $state<LongSectionData | null>(null);
  let crossSection = $state<CrossSectionData | null>(null);
  let sectionNote = $state<string | null>(null);
  /** The station: a chainage on the selected option's line, set by the slider, a plot click or a map click. */
  let station = $state<number | null>(null);
  let offline = $state(false);
  let hillshade = $state<Hillshade | null>(null);
  let tiles = $state<string | null>(null);
  let shadeFor: string | null = null;

  // The map redraws from the session's geometry whenever the design changes. A design with
  // nothing to draw yet (no CRS, no inputs) just shows the empty map.
  $effect(() => {
    void snap?.document;
    api.mapGeometry().then(
      (g) => (geometry = g),
      () => (geometry = null),
    );
  });

  // A stage running on a worker: which, and how many DEM points it has read so far.
  let busy = $state<StageName | null>(null);
  let samples = $state(0);
  let notice = $state<string | null>(null);
  let dropTarget = $state<InputRole | null>(null);

  async function run(stage: StageName, method?: RunMethod): Promise<boolean> {
    await flush();
    busy = stage;
    samples = 0;
    notice = null;
    const next = await guard(() => api.runStage(stage, method));
    busy = null;
    if (!next) return false;
    snap = next;
    if (next.run.outcome === "cancelled") {
      notice = `The ${stage} run was cancelled. The design is as it was.`;
      return false;
    }
    return true;
  }

  async function cancel() {
    await guard(() => api.cancelRun());
  }

  /** Rerun the stale stages in order, each on the worker; stop at the first that does not finish. */
  async function runStale() {
    for (const s of snap?.stages ?? []) {
      if (s.status !== "stale") continue;
      if (!(await run(s.name))) return;
    }
  }

  async function load(role: InputRole, path?: string) {
    await fileAction(() => api.loadInput(role, path));
  }

  async function water(level: string, fall: string) {
    await fileAction(() => api.setWater(level, fall));
  }

  async function exportAs(kind: "geojson" | "csv") {
    await flush();
    notice = null;
    const path = await guard(() => api.exportFile(kind));
    if (path) notice = `Exported to ${path}. The file says "Screening — not for construction".`;
  }

  async function fetchDem() {
    await fileAction(api.fetchDem);
  }

  /** The row under a window position given in physical pixels, for drag and drop. */
  function rowAt(x: number, y: number): InputRole | null {
    const el = document.elementFromPoint(x / window.devicePixelRatio, y / window.devicePixelRatio);
    return (el?.closest("[data-drop-role]")?.getAttribute("data-drop-role") as InputRole | null) ?? null;
  }

  let leftWidth = $state(340);
  let bottomHeight = $state(200);

  // Sizing recomputes live, after a pause in typing (about 150 ms); nothing else reruns by itself.
  const PAUSE_MS = 150;
  let pending: ReturnType<typeof setTimeout> | undefined;
  let queued: (() => Promise<Snapshot>) | undefined;

  async function guard<T>(work: () => Promise<T>): Promise<T | undefined> {
    try {
      error = null;
      return await work();
    } catch (e) {
      error = String(e);
      return undefined;
    }
  }

  /** Send the queued sizing edit now. A refusal keeps the last good result and shows the reason. */
  async function flush() {
    clearTimeout(pending);
    pending = undefined;
    const send = queued;
    queued = undefined;
    if (!send) return;
    // A refused choice keeps the last good channel shape, shows the reason, and leaves the form as typed.
    const next = await guard(send);
    if (next) snap = next;
  }

  function onSizing(ruleset: string, vessel: string, draught: string) {
    queued = () => api.setSizing(ruleset, vessel, draught);
    clearTimeout(pending);
    pending = setTimeout(() => flush(), PAUSE_MS);
  }

  async function fileAction(action: () => Promise<Snapshot | null>) {
    await flush();
    const next = await guard(action);
    if (next) snap = next;
  }

  /** New and Open start a different study: its first option (a failing one, if any) is selected, not the old selection. */
  async function openDoc(action: () => Promise<Snapshot | null>) {
    await flush();
    const next = await guard(action);
    if (!next) return;
    selectedId = null;
    station = null;
    snap = next;
  }

  function onKey(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey)) return;
    const k = e.key.toLowerCase();
    if (k === "s") {
      e.preventDefault();
      fileAction(e.shiftKey ? api.saveAs : api.save);
    } else if (k === "o") {
      e.preventDefault();
      openDoc(api.openFile);
    } else if (k === "n") {
      e.preventDefault();
      openDoc(api.newFile);
    }
  }

  // The option table and checks list come from the session too, and refresh with every change.
  $effect(() => {
    void snap?.document;
    api.optionTable().then(
      (t) => {
        table = t;
        // Keep the selection while its option exists; otherwise select the first row (a failure, if any).
        if (!t.rows.some((r) => r.id === selectedId)) selectedId = t.rows[0]?.id ?? null;
      },
      () => (table = null),
    );
  });

  // The long section of the selected option, drawn from the station walk the volumes summed. It is
  // asked for again whenever the design or the selection changes; a refusal says why instead.
  let sectionRequest = 0;
  $effect(() => {
    const id = selectedId;
    void snap?.document;
    const mine = ++sectionRequest;
    if (!id) {
      longSection = null;
      crossSection = null;
      return;
    }
    api.longSection(id).then(
      (ls) => {
        if (mine !== sectionRequest) return;
        longSection = ls;
        sectionNote = null;
        if (station == null || station > ls.chainage_m[ls.chainage_m.length - 1]) station = ls.chainage_m[0];
      },
      (e) => {
        if (mine !== sectionRequest) return;
        longSection = null;
        crossSection = null;
        sectionNote = String(e);
      },
    );
  });

  // The cross-section at the station, latest request wins.
  let crossRequest = 0;
  $effect(() => {
    const id = selectedId;
    const at = station;
    void longSection;
    if (!id || at == null || !longSection) return;
    const mine = ++crossRequest;
    api.crossSection(id, at).then(
      (cs) => {
        if (mine === crossRequest) crossSection = cs;
      },
      () => {},
    );
  });

  /** The slider moves between the stations the session drew. */
  let stationIndex = $derived(longSection && station != null ? longSection.chainage_m.findIndex((c) => c >= station! - 1e-9) : 0);
  function slide(i: number) {
    if (longSection) station = longSection.chainage_m[i];
  }
  function pickChainage(c: number) {
    if (longSection) station = longSection.chainage_m.reduce((best, x) => (Math.abs(x - c) < Math.abs(best - c) ? x : best));
  }
  async function pickPoint(lon: number, lat: number) {
    if (!selectedId || !longSection) return;
    const c = await guard(() => api.stationNear(selectedId!, lon, lat));
    if (c != null) station = c;
  }

  /** An option's colour (a CSS reference to its token): its place in the design's list, which is the order the map draws. */
  const colourOf = (id: string) => {
    const i = geometry?.options.findIndex((o) => o.id === id) ?? -1;
    return optionVar(i < 0 ? 0 : i);
  };

  // The hillshade is drawn once by the Rust side, and only when the imagery is offline. A new DEM
  // reference clears it.
  $effect(() => {
    const key = snap?.document.inputs.dem ? `${snap.document.inputs.dem.path}:${snap.document.inputs.dem.sha256}` : null;
    if (key !== shadeFor) {
      shadeFor = null;
      hillshade = null;
    }
    if (offline && shadeFor === null) {
      shadeFor = key ?? "";
      api.hillshade().then(
        (h) => (hillshade = h),
        () => (hillshade = null),
      );
    }
  });

  onMount(() => {
    const undo: (() => void)[] = [];
    // Outside the Tauri window (a browser, a test) these have nothing to listen to.
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen<number>("stage-progress", (e) => (samples = e.payload)))
      .then((un) => undo.push(un))
      .catch(() => {});
    import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((e) => {
          const p = e.payload;
          if (p.type === "leave") dropTarget = null;
          else if (p.type === "over") dropTarget = rowAt(p.position.x, p.position.y);
          else if (p.type === "drop") {
            const role = rowAt(p.position.x, p.position.y);
            dropTarget = null;
            const file = p.paths[0];
            if (!file) return;
            // A GeoTIFF is the DEM wherever it lands; vector data must land on its own row.
            if (/\.tiff?$/i.test(file)) load("dem", file);
            else if (role) load(role, file);
            else error = "Drop the file on the row it belongs to: left bank, right bank, river polygon or centreline.";
          }
        }),
      )
      .then((un) => undo.push(un))
      .catch(() => {});
    guard(async () => {
      tiles = (await api.basemapTiles()) ?? BASEMAP_TILES;
      choices = await api.sizingChoices();
      snap = await api.snapshot();
    });
    return () => undo.forEach((u) => u());
  });

  function drag(axis: "x" | "y") {
    return (down: PointerEvent) => {
      const target = down.currentTarget as HTMLElement;
      target.setPointerCapture(down.pointerId);
      const move = (e: PointerEvent) => {
        if (axis === "x") leftWidth = Math.min(Math.max(e.clientX, 220), window.innerWidth - 300);
        else bottomHeight = Math.min(Math.max(window.innerHeight - e.clientY, 80), window.innerHeight - 200);
      };
      const up = () => {
        target.removeEventListener("pointermove", move);
        target.removeEventListener("pointerup", up);
      };
      target.addEventListener("pointermove", move);
      target.addEventListener("pointerup", up);
    };
  }

  let anyStale = $derived(snap?.stages.some((s) => s.status === "stale") ?? false);
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <header>
    <span class="brand"><img class="mark-light" src={markLight} alt="OWD" /><img class="mark-dark" src={markDark} alt="OWD" /></span>
    <button onclick={() => openDoc(api.newFile)}>New</button>
    <button onclick={() => openDoc(api.openFile)}>Open</button>
    <button onclick={() => fileAction(api.save)}>Save</button>
    <button onclick={() => fileAction(api.saveAs)}>Save as</button>
    <button disabled={!(table?.rows.length)} onclick={() => exportAs("geojson")} title="Design line options as GeoJSON (WGS84), with their checks">Export GeoJSON</button>
    <button disabled={!(table?.rows.length)} onclick={() => exportAs("csv")} title="The quantity table as CSV">Export CSV</button>
    <button class="primary" disabled={!anyStale || busy !== null} onclick={runStale} title="Rerun every stale stage, in order">Run stale</button>
    <span class="grow"></span>
    {#if snap}
      <span class="meta" data-testid="file-label">{snap.file_name}{snap.dirty ? " (unsaved)" : ""}</span>
      <span class="meta" data-testid="study-title">{snap.document.project.title}</span>
      <span class="meta" data-testid="engine-version">Engine {snap.engine_version}</span>
    {/if}
  </header>

  {#if error}
    <div class="banner error" role="alert" data-testid="error">
      {error}
      <button onclick={() => (error = null)}>Dismiss</button>
    </div>
  {/if}
  {#if notice}
    <div class="banner warn" role="status" data-testid="notice">
      {notice}
      <button onclick={() => (notice = null)}>Dismiss</button>
    </div>
  {/if}
  {#each snap?.warnings ?? [] as w}
    <div class="banner warn" role="status" data-testid="warning">{w}</div>
  {/each}

  <div class="body" style:grid-template-columns="{leftWidth}px 6px 1fr" style:grid-template-rows="1fr 6px {bottomHeight}px">
    <section class="left"><LeftPanel {snap} {choices} {onSizing} {busy} {samples} {dropTarget} onRun={run} onCancel={cancel} onLoad={(r) => load(r)} onFetch={fetchDem} onWater={water} /></section>
    <div class="handle v" role="separator" aria-orientation="vertical" onpointerdown={drag("x")}></div>
    <section class="main">
      <div class="mapbox">
        {#if tiles}<MapPanel {geometry} {selectedId} {hillshade} {tiles} stationPosition={crossSection?.position ?? null} onPick={pickPoint} bind:offline />{/if}
      </div>
      <div class="sections" data-testid="sections">
        <div class="longbox">
          {#if longSection}
            <LongSection data={longSection} {station} colour={selectedId ? colourOf(selectedId) : "#14325a"} onPick={pickChainage} />
          {:else}
            <p class="slot" data-testid="section-note">{sectionNote ?? "Run volumes to see the long section."}</p>
          {/if}
        </div>
        <div class="crossbox">
          <label class="slider">
            Station
            <input
              type="range"
              data-testid="station-slider"
              aria-label="Station"
              min="0"
              max={Math.max((longSection?.chainage_m.length ?? 1) - 1, 0)}
              step="1"
              disabled={!longSection}
              value={stationIndex < 0 ? 0 : stationIndex}
              oninput={(e) => slide(Number(e.currentTarget.value))}
            />
          </label>
          <div class="csbody"><CrossSection section={crossSection} colour={selectedId ? colourOf(selectedId) : "#14325a"} /></div>
        </div>
      </div>
    </section>
    <div class="handle h" role="separator" aria-orientation="horizontal" onpointerdown={drag("y")}></div>
    <section class="bottom">
      <BottomPanel {table} {colourOf} {selectedId} onSelect={(id) => (selectedId = id)} />
    </section>
  </div>

  <footer data-testid="status-line">Screening — not for construction</footer>
</div>

<style>
  .shell { display: flex; flex-direction: column; height: 100vh; }
  header { display: flex; gap: var(--sp-2); align-items: center; padding: var(--sp-2) var(--sp-3); border-bottom: var(--lw-thin) solid var(--ink); background: var(--paper); }
  .brand { display: flex; align-items: center; margin-right: var(--sp-3); }
  .brand img { height: 26px; }
  .mark-dark { display: none; }
  .grow { flex: 1; }
  .meta { color: var(--ink-3); font-size: var(--fs-12); margin-left: var(--sp-3); font-variant-numeric: tabular-nums; }
  .banner { display: flex; gap: var(--sp-2); align-items: center; padding: 6px var(--sp-3); font-size: var(--fs-13); border-bottom: var(--lw-thin) solid var(--rule); }
  .banner.error { background: var(--fail-soft); color: var(--fail); }
  .banner.error::before { content: "✕"; font-weight: 700; }
  .banner.warn { background: var(--warn-soft); color: var(--warn); }
  .banner.warn::before { content: "!"; font-weight: 700; }
  .body { flex: 1; display: grid; min-height: 0; }
  .body > section { overflow: auto; min-width: 0; min-height: 0; }
  .left { grid-column: 1; grid-row: 1 / 4; background: var(--surface); }
  .handle.v { grid-column: 2; grid-row: 1 / 4; cursor: col-resize; background: var(--ink); }
  .main { grid-column: 3; grid-row: 1; overflow: hidden; display: grid; grid-template-rows: minmax(120px, 1fr) minmax(150px, 40%); }
  .mapbox { min-height: 0; position: relative; }
  .sections { display: grid; grid-template-columns: 1fr minmax(260px, 42%); min-height: 0; border-top: var(--lw-thin) solid var(--ink); background: var(--surface); }
  .longbox, .crossbox { min-width: 0; min-height: 0; overflow: hidden; }
  .crossbox { border-left: var(--lw-thin) solid var(--ink); display: flex; flex-direction: column; }
  .csbody { flex: 1; min-height: 0; }
  .slider { display: flex; gap: var(--sp-2); align-items: center; padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); color: var(--ink-3); }
  .slider input { flex: 1; accent-color: var(--ink); }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2) 10px; font-size: var(--fs-13); }
  .handle.h { grid-column: 3; grid-row: 2; cursor: row-resize; background: var(--ink); }
  .bottom { grid-column: 3; grid-row: 3; background: var(--surface); }
  .handle:hover { background: var(--ink-2); }
  footer { padding: var(--sp-1) var(--sp-3); background: var(--accent); color: var(--accent-ink); font: 600 var(--fs-12) var(--font-ui); letter-spacing: 0.04em; }

  /* The reversed mark shows on the dark theme: chosen by the OS, or by data-theme. */
  :global(:root[data-theme="dark"]) .mark-light { display: none; }
  :global(:root[data-theme="dark"]) .mark-dark { display: block; }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .mark-light { display: none; }
    :global(:root:not([data-theme="light"])) .mark-dark { display: block; }
  }
</style>
