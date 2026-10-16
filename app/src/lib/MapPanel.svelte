<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?url";
  import type * as GeoJSON from "geojson";
  import type { MapGeometry } from "./api";
  import { ATTRIBUTION, BASEMAP_TILES, OPTION_COLOURS } from "./basemap";

  let { geometry, selectedId = null }: { geometry: MapGeometry | null; selectedId?: string | null } = $props();

  // The worker is served with the app (never from a CDN), so the map starts offline too.
  maplibregl.setWorkerUrl(workerUrl);

  let host: HTMLDivElement;
  let map: maplibregl.Map | undefined;
  let ready = $state(false);
  let failure = $state<string | null>(null);
  let labels: maplibregl.Marker[] = [];

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
        colour: OPTION_COLOURS[i % OPTION_COLOURS.length],
        selected: o.id === selected,
        failed: !o.radius_ok,
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
    // Chainage labels are HTML markers: the map needs no glyph server, so it works offline too.
    labels.forEach((m) => m.remove());
    labels = [];
    const shown = geometry?.options.find((o) => o.id === selectedId) ?? geometry?.options[0];
    for (const t of shown?.ticks ?? []) {
      const el = document.createElement("span");
      el.className = "tick-label";
      el.textContent = t.label;
      labels.push(new maplibregl.Marker({ element: el, offset: [0, -10] }).setLngLat(t.position).addTo(map));
    }
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
            basemap: { type: "raster", tiles: [BASEMAP_TILES], tileSize: 256, maxzoom: 14, attribution: ATTRIBUTION },
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
    map.on("load", () => {
      const m = map!;
      for (const id of ["polygon", "banks", "centre", "options", "bends", "ticks"]) m.addSource(id, { type: "geojson", data: empty });
      m.addLayer({ id: "polygon", type: "fill", source: "polygon", paint: { "fill-color": "#2a6ebb", "fill-opacity": 0.18 } });
      m.addLayer({ id: "banks", type: "line", source: "banks", paint: { "line-color": "#ffffff", "line-width": 1.5 } });
      m.addLayer({ id: "centre", type: "line", source: "centre", paint: { "line-color": "#ffffff", "line-width": 1, "line-dasharray": [3, 3] } });
      // A casing under each option keeps its colour readable on any imagery.
      m.addLayer({ id: "options-casing", type: "line", source: "options", paint: { "line-color": "#000000", "line-opacity": 0.6, "line-width": ["case", ["get", "selected"], 8, 5] } });
      m.addLayer({ id: "options", type: "line", source: "options", paint: { "line-color": ["get", "colour"], "line-width": ["case", ["get", "selected"], 5, 3] } });
      m.addLayer({ id: "ticks", type: "circle", source: "ticks", paint: { "circle-radius": 3, "circle-color": "#ffffff", "circle-stroke-color": "#000000", "circle-stroke-width": 1 } });
      m.addLayer({ id: "bends", type: "circle", source: "bends", paint: { "circle-radius": 9, "circle-color": "rgba(255,0,0,0.25)", "circle-stroke-color": "#ff2a2a", "circle-stroke-width": 3 } });
      ready = true;
    });
    map.on("error", (e: unknown) => {
      // A failed tile is expected offline; the offline fallback (a later ticket) answers it.
      void e;
    });
  });

  onDestroy(() => map?.remove());

  $effect(() => {
    void geometry;
    void selectedId;
    void ready;
    draw();
    fit();
  });
</script>

<div class="map" data-testid="plan-map">
  <div class="canvas" bind:this={host}></div>
  {#if failure}
    <p class="failure" role="alert" data-testid="map-failure">{failure}</p>
  {/if}
  {#if ready && !geometry}
    <p class="hint" data-testid="map-empty">Load bank lines and a river polygon, and run alignment, to see the design lines here.</p>
  {/if}
  <ul class="legend" data-testid="map-legend">
    {#each geometry?.options ?? [] as o, i}
      <li>
        <span class="swatch" style:background={OPTION_COLOURS[i % OPTION_COLOURS.length]}></span>
        {o.name}{#if !o.radius_ok}<span class="fail">radius fails</span>{/if}
      </li>
    {/each}
    {#if geometry?.options.some((o) => o.tight_bends.length)}
      <li><span class="swatch ring"></span>tight bend</li>
    {/if}
  </ul>
  <p class="attribution" data-testid="attribution">{ATTRIBUTION}</p>
</div>

<style>
  .map { position: relative; width: 100%; height: 100%; min-height: 160px; background: #1b2430; }
  .canvas { position: absolute; inset: 0; }
  .attribution { position: absolute; right: 0; bottom: 0; margin: 0; padding: 1px 6px; font-size: 10px; background: rgba(255, 255, 255, 0.85); color: #222; z-index: 2; max-width: 100%; }
  .legend { position: absolute; left: 8px; top: 8px; margin: 0; padding: 4px 8px; list-style: none; font-size: 12px; background: rgba(255, 255, 255, 0.9); border-radius: 3px; z-index: 2; }
  .legend:empty { display: none; }
  .swatch { display: inline-block; width: 18px; height: 4px; margin-right: 6px; vertical-align: middle; }
  .swatch.ring { height: 10px; width: 10px; border: 2px solid #ff2a2a; border-radius: 50%; background: none; }
  .fail { color: #b3261e; margin-left: 6px; }
  .hint, .failure { position: absolute; left: 50%; top: 40%; transform: translateX(-50%); margin: 0; padding: 8px 12px; background: rgba(255, 255, 255, 0.92); border-radius: 4px; z-index: 2; max-width: 80%; text-align: center; }
  .failure { color: #b3261e; }
  :global(.tick-label) { font-size: 10px; color: #fff; text-shadow: 0 0 3px #000, 0 0 3px #000; white-space: nowrap; }
</style>
