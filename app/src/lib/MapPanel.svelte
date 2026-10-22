<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?url";
  import type * as GeoJSON from "geojson";
  import type { Hillshade, MapGeometry, StretchCollection } from "./api";
  import { ATTRIBUTION, BASEMAP_TILES } from "./basemap";
  import OptionSwatch from "./OptionSwatch.svelte";
  import { optionColour, token } from "./theme";

  let {
    geometry,
    selectedId = null,
    stretches = null,
    selectedStretch = null,
    hillshade = null,
    tiles = BASEMAP_TILES,
    stationPosition = null,
    onPick,
    onReady,
    offline = $bindable(false),
  }: {
    geometry: MapGeometry | null;
    selectedId?: string | null;
    /** The constraint stretches as the session exports them (WGS84 lines). */
    stretches?: StretchCollection | null;
    /** The stretch picked in the table: drawn heavier, and the map moves to it. */
    selectedStretch?: { option: string; index: number } | null;
    /** The DEM hillshade, shown in place of the imagery while the tiles cannot load. */
    hillshade?: Hillshade | null;
    /** The tile address. Only a test changes it. */
    tiles?: string;
    /** Where the section station is, as `[lon, lat]`: a marker follows it. */
    stationPosition?: [number, number] | null;
    /** A click on the map: the point under it. */
    onPick?: (lon: number, lat: number) => void;
    /** Called once with the map, when it has loaded: an editor adds its own layers and handlers. */
    onReady?: (map: maplibregl.Map) => void;
    /** True while the basemap tiles are failing. The parent reads it to fetch the hillshade. */
    offline?: boolean;
  } = $props();

  // The worker is served with the app (never from a CDN), so the map starts offline too.
  maplibregl.setWorkerUrl(workerUrl);

  /** A 1 x 1 transparent PNG. */
  const BLANK = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

  let host: HTMLDivElement;
  let map: maplibregl.Map | undefined;
  let ready = $state(false);
  let failure = $state<string | null>(null);
  let labels: maplibregl.Marker[] = [];
  let bendMarkers: maplibregl.Marker[] = [];
  let stationMarker: maplibregl.Marker | undefined;
  /** The option the map has drawn as selected: what a test reads to know the map followed the table. */
  let drawn = $state<string | null>(null);
  /** The stretch the map has drawn as picked, as `option:index`, for the same reason. */
  let drawnStretch = $state("");

  const empty = { type: "FeatureCollection", features: [] } as GeoJSON.FeatureCollection;
  const feature = (geometry: GeoJSON.Geometry, properties: GeoJSON.GeoJsonProperties = {}): GeoJSON.Feature => ({
    type: "Feature",
    geometry,
    properties,
  });

  function sources(g: MapGeometry | null, selected: string | null) {
    const bank = [...(g?.banks ?? [])].map((b) => feature(b.line as GeoJSON.Geometry, { side: b.side }));
    const centre = g?.centreline ? [feature(g.centreline as GeoJSON.Geometry)] : [];
    const polygon = g?.river_polygon ? [feature(g.river_polygon as GeoJSON.Geometry)] : [];
    const options = (g?.options ?? []).map((o, i) =>
      feature(o.line as GeoJSON.Geometry, {
        id: o.id,
        colour: optionColour(i, "map"),
        selected: o.id === selected,
        failed: !o.radius_ok,
        dim: selected != null && o.id !== selected,
      }),
    );
    const bends = (g?.options ?? []).flatMap((o) =>
      o.tight_bends.map((b) =>
        feature({ type: "Point", coordinates: b.position }, { option: o.id, radius: Math.round(b.radius_m) }),
      ),
    );
    const ticks = (g?.options ?? []).flatMap((o) =>
      o.ticks.map((t) => feature({ type: "Point", coordinates: t.position }, { option: o.id, label: t.label })),
    );
    return { bank, centre, polygon, options, bends, ticks };
  }

  /** The stretches of the option shown, each numbered in its option's list (the table's order). */
  function stretchFeatures(shown: string | null): GeoJSON.Feature[] {
    const seen = new Map<string, number>();
    const out: GeoJSON.Feature[] = [];
    for (const f of stretches?.features ?? []) {
      const option = f.properties.option_id;
      const index = seen.get(option) ?? 0;
      seen.set(option, index + 1);
      if (shown != null && option !== shown) continue;
      const picked = selectedStretch?.option === option && selectedStretch.index === index;
      out.push(feature(f.geometry as GeoJSON.Geometry, { option, index, kind: f.properties.kind, picked }));
    }
    return out;
  }

  const fc = (features: GeoJSON.Feature[]): GeoJSON.FeatureCollection => ({ type: "FeatureCollection", features });

  function draw() {
    if (!map || !ready) return;
    const s = sources(geometry, selectedId);
    (map.getSource("polygon") as maplibregl.GeoJSONSource).setData(fc(s.polygon));
    (map.getSource("banks") as maplibregl.GeoJSONSource).setData(fc(s.bank));
    (map.getSource("centre") as maplibregl.GeoJSONSource).setData(fc(s.centre));
    (map.getSource("options") as maplibregl.GeoJSONSource).setData(fc(s.options));
    (map.getSource("bends") as maplibregl.GeoJSONSource).setData(fc(s.bends));
    (map.getSource("ticks") as maplibregl.GeoJSONSource).setData(fc(s.ticks));
    const st = stretchFeatures(selectedId);
    (map.getSource("stretches") as maplibregl.GeoJSONSource).setData(fc(st));
    drawn = selectedId;
    const picked = st.find((f) => f.properties?.picked);
    drawnStretch = picked ? `${picked.properties!.option}:${picked.properties!.index}` : "";
    // Chainage labels are HTML markers: the map needs no glyph server, so it works offline too.
    labels.forEach((m) => m.remove());
    labels = [];
    const shown = geometry?.options.find((o) => o.id === selectedId) ?? geometry?.options[0];
    // A tight bend is a disc with a cross: a shape as well as a colour. Also HTML, so it needs no glyphs.
    bendMarkers.forEach((m) => m.remove());
    bendMarkers = [];
    for (const o of geometry?.options ?? []) {
      for (const b of o.tight_bends) {
        const el = document.createElement("span");
        el.className = "bend-marker";
        el.textContent = "✕";
        el.title = `Radius ${Math.round(b.radius_m)} m`;
        bendMarkers.push(new maplibregl.Marker({ element: el }).setLngLat(b.position).addTo(map));
      }
    }
    for (const t of shown?.ticks ?? []) {
      const el = document.createElement("span");
      el.className = "tick-label";
      el.textContent = t.label;
      labels.push(new maplibregl.Marker({ element: el, offset: [0, -10] }).setLngLat(t.position).addTo(map));
    }
  }

  /** Move the station marker, or take it off the map. */
  function placeStation() {
    if (!map || !ready) return;
    if (!stationPosition) {
      stationMarker?.remove();
      stationMarker = undefined;
      return;
    }
    if (!stationMarker) {
      const el = document.createElement("div");
      el.className = "station-marker";
      el.dataset.testid = "station-marker";
      stationMarker = new maplibregl.Marker({ element: el }).setLngLat(stationPosition).addTo(map);
    } else {
      stationMarker.setLngLat(stationPosition);
    }
    stationMarker.getElement().dataset.position = stationPosition.map((v) => v.toFixed(6)).join(",");
  }

  let shownShade: string | null = null;
  /** Lay the hillshade under the vector layers, and show it only while the imagery is offline. */
  function shade() {
    if (!map || !ready) return;
    const src = map.getSource("hillshade") as maplibregl.ImageSource;
    if (hillshade && hillshade.url !== shownShade) {
      shownShade = hillshade.url;
      src.updateImage({ url: hillshade.url, coordinates: hillshade.corners });
    }
    map.setLayoutProperty("hillshade", "visibility", offline && hillshade ? "visible" : "none");
    map.setLayoutProperty("basemap", "visibility", offline ? "none" : "visible");
  }

  /** Move the map to the picked stretch: the box around its line, with room around it. */
  let fittedStretch = "";
  function fitStretch() {
    if (!map || !ready) return;
    const key = selectedStretch ? `${selectedStretch.option}:${selectedStretch.index}` : "";
    if (key === fittedStretch) return;
    fittedStretch = key;
    const f = stretchFeatures(selectedStretch?.option ?? null).find((x) => x.properties?.picked);
    const pts = (f?.geometry as GeoJSON.LineString | undefined)?.coordinates ?? [];
    if (!pts.length) return;
    const lons = pts.map((p) => p[0]);
    const lats = pts.map((p) => p[1]);
    map.fitBounds([[Math.min(...lons), Math.min(...lats)], [Math.max(...lons), Math.max(...lats)]], { padding: 80, maxZoom: 16, animate: false });
  }

  let fitted: string | null = null;
  function fit() {
    if (!map || !geometry?.bbox) return;
    const key = geometry.bbox.join(",");
    if (key === fitted) return;
    fitted = key;
    const [w, s, e, n] = geometry.bbox;
    map.fitBounds([[w, s], [e, n]], { padding: 40, animate: false });
  }

  onMount(() => {
    try {
      map = new maplibregl.Map({
        container: host,
        attributionControl: false,
        style: {
          version: 8,
          sources: {
            basemap: { type: "raster", tiles: [tiles], tileSize: 256, maxzoom: 14, attribution: ATTRIBUTION },
          },
          layers: [{ id: "basemap", type: "raster", source: "basemap" }],
        },
        center: [0, 5],
        zoom: 2,
      });
    } catch (e) {
      failure = `The map could not start: ${e instanceof Error ? e.message : e}`;
      return;
    }
    map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
    map.addControl(new maplibregl.ScaleControl({ unit: "metric" }), "bottom-left");
    map.on("click", (e) => onPick?.(e.lngLat.lng, e.lngLat.lat));
    map.on("load", () => {
      const m = map!;
      // A placeholder picture until the hillshade arrives; `setCoordinates` and `updateImage` replace it.
      m.addSource("hillshade", { type: "image", url: BLANK, coordinates: [[0, 1], [1, 1], [1, 0], [0, 0]] });
      for (const id of ["polygon", "banks", "centre", "options", "bends", "ticks", "stretches"]) m.addSource(id, { type: "geojson", data: empty });
      m.addLayer({ id: "hillshade", type: "raster", source: "hillshade", paint: { "raster-fade-duration": 0 }, layout: { visibility: "none" } });
      // Colours are read from the tokens. On imagery the same cream and plate ink serve both themes.
      const cream = token("cream");
      const ink = token("plate-ink");
      m.addLayer({ id: "polygon", type: "fill", source: "polygon", paint: { "fill-color": token("river"), "fill-opacity": 0.18 } });
      m.addLayer({ id: "banks", type: "line", source: "banks", paint: { "line-color": cream, "line-width": 1.5 } });
      m.addLayer({ id: "centre", type: "line", source: "centre", paint: { "line-color": cream, "line-width": 1, "line-dasharray": [3, 3] } });
      // The deep line over a cream casing keeps each option readable on any imagery. The design line is the heaviest line.
      m.addLayer({ id: "options-casing", type: "line", source: "options", paint: { "line-color": token("opt-casing"), "line-width": ["case", ["get", "selected"], 8, 6] } });
      m.addLayer({ id: "options", type: "line", source: "options", paint: { "line-color": ["get", "colour"], "line-width": 4, "line-opacity": ["case", ["get", "dim"], 0.55, 1] } });
      // Constraint stretches: a dashed red band under the ticks, heavier and solid-cased when picked in the table.
      const fail = token("fail");
      m.addLayer({ id: "stretches-casing", type: "line", source: "stretches", filter: ["==", ["get", "picked"], true], paint: { "line-color": cream, "line-width": 12 } });
      m.addLayer({ id: "stretches", type: "line", source: "stretches", paint: { "line-color": fail, "line-width": ["case", ["get", "picked"], 7, 4], "line-dasharray": [2, 1] } });
      m.addLayer({ id: "ticks", type: "circle", source: "ticks", paint: { "circle-radius": 3, "circle-color": cream, "circle-stroke-color": ink, "circle-stroke-width": 1 } });
      ready = true;
      onReady?.(m);
    });
    // Offline: tiles keep failing and none has loaded. One success puts the imagery back.
    let failed = 0;
    let loaded = 0;
    map.on("error", (e) => {
      if ((e as { sourceId?: string }).sourceId !== "basemap") return;
      failed += 1;
      if (failed >= 2 && loaded === 0) offline = true;
    });
    map.on("sourcedata", (e) => {
      if (e.sourceId !== "basemap" || !("tile" in e) || !e.tile) return;
      loaded += 1;
      offline = false;
    });
  });

  onDestroy(() => map?.remove());

  $effect(() => {
    void geometry;
    void selectedId;
    void stretches;
    void selectedStretch;
    void ready;
    draw();
    fit();
    fitStretch();
  });

  $effect(() => {
    void stationPosition;
    void ready;
    placeStation();
  });

  $effect(() => {
    void hillshade;
    void offline;
    void ready;
    shade();
  });
</script>

<div class="map" data-testid="plan-map" data-hillshade={offline && hillshade ? "shown" : "hidden"} data-selected={drawn ?? ""} data-stretch={drawnStretch}>
  <div class="canvas" bind:this={host}></div>
  {#if failure}
    <p class="failure" role="alert" data-testid="map-failure">{failure}</p>
  {/if}
  <ul class="legend" data-testid="map-legend">
    {#each geometry?.options ?? [] as o, i}
      <li>
        <OptionSwatch index={i} />
        {o.name}{#if !o.radius_ok}<span class="fail">radius fails</span>{/if}
      </li>
    {/each}
    {#if geometry?.options.some((o) => o.tight_bends.length)}
      <li><span class="bend-key">✕</span>tight bend</li>
    {/if}
    {#if stretches?.features.some((f) => f.properties.option_id === selectedId)}
      <li data-testid="legend-stretch"><span class="stretch-key" aria-hidden="true"></span>constraint stretch</li>
    {/if}
  </ul>
  {#if offline}
    <p class="offline" role="status" data-testid="offline-note">
      Imagery offline{hillshade ? " — showing the DEM hillshade" : " — no DEM to show a hillshade from"}
    </p>
  {/if}
  <p class="attribution" data-testid="attribution">{ATTRIBUTION}</p>
</div>

<style>
  .map { position: relative; width: 100%; height: 100%; min-height: 160px; background: var(--paper); }
  .canvas { position: absolute; inset: 0; }
  .attribution { position: absolute; right: 0; bottom: 0; margin: 0; padding: 1px 6px; font-size: 10px; background: var(--surface); color: var(--ink-3); z-index: 2; max-width: 100%; }
  .legend { position: absolute; left: var(--sp-3); top: var(--sp-3); margin: 0; padding: var(--sp-2) var(--sp-3); list-style: none; font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); z-index: 2; }
  .legend li { display: flex; gap: var(--sp-2); align-items: center; margin: 3px 0; }
  .legend:not(:has(li)) { display: none; }
  .stretch-key { display: inline-block; width: 22px; height: 0; border-top: 4px dashed var(--fail); }
  .bend-key { display: inline-grid; place-items: center; width: 14px; height: 14px; border-radius: 50%; border: var(--lw-thin) solid var(--ink); font-size: var(--fs-11); font-weight: 700; color: var(--fail); }
  .offline { position: absolute; left: var(--sp-2); bottom: 28px; margin: 0; padding: 3px var(--sp-2); font-size: var(--fs-12); background: var(--warn-soft); color: var(--warn); border: var(--lw-thin) solid var(--rule-strong); border-radius: var(--radius); z-index: 2; }
  .fail { color: var(--fail); margin-left: var(--sp-2); font-weight: 600; }
  .fail::before { content: "✕ "; }
  .failure { position: absolute; left: 50%; top: 40%; transform: translateX(-50%); margin: 0; padding: var(--sp-2) var(--sp-3); background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); z-index: 2; max-width: 80%; text-align: center; }
  .failure { color: var(--fail); }
  :global(.station-marker) { width: 16px; height: 16px; border-radius: 50%; background: var(--plate-ink); border: 3px solid var(--cream); box-shadow: 0 0 0 1px var(--plate-ink); pointer-events: none; }
  :global(.tick-label) { font-size: var(--fs-11); color: var(--cream); text-shadow: 0 0 3px var(--plate-ink), 0 0 3px var(--plate-ink); white-space: nowrap; }
  :global(.bend-marker) { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; background: var(--surface); border: 1.5px solid var(--ink); color: var(--fail); font: 700 14px / 1 var(--font-ui); pointer-events: none; }
  :global(.maplibregl-ctrl-group) { border-radius: var(--radius); background: var(--surface); border: var(--lw-thin) solid var(--ink); box-shadow: none; }
  :global(.maplibregl-ctrl-group button) { border-color: var(--rule); }
  /* The control icons are dark drawings: show them in ink on the dark theme. */
  :global(:root[data-theme="dark"] .maplibregl-ctrl-icon) { filter: invert(1); }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"]) .maplibregl-ctrl-icon) { filter: invert(1); }
  }
  :global(.maplibregl-ctrl-scale) { background: var(--surface); color: var(--ink); border-color: var(--ink); }
</style>
