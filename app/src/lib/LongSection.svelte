<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import type { LongSection } from "./api";

  let {
    data,
    station,
    colour,
    onPick,
  }: {
    data: LongSection | null;
    /** The station chainage, drawn as a vertical line. */
    station: number | null;
    colour: string;
    /** A click on the plot: the chainage under it. */
    onPick: (chainage: number) => void;
  } = $props();

  let host: HTMLDivElement;
  let plot: uPlot | undefined;
  let size = { width: 400, height: 200 };
  let marker: number | null = null;

  const series = (d: LongSection): uPlot.AlignedData => [d.chainage_m, d.bed_m, d.water_level_m, d.design_invert_m, d.cut_depth_m];

  function options(): uPlot.Options {
    return {
      ...size,
      legend: { show: true, live: false },
      cursor: { drag: { x: false, y: false } },
      scales: { x: { time: false }, m: {}, cut: { range: (_u, min, max) => [0, Math.max(max, 1) * 1.1] } },
      axes: [
        { label: "Chainage (m)", scale: "x" },
        { label: "Elevation (m)", scale: "m", size: 56 },
        { label: "Cut depth (m)", scale: "cut", side: 1, grid: { show: false }, size: 50 },
      ],
      series: [
        {},
        { label: "Bed", scale: "m", stroke: "#7a5a2a", width: 2 },
        { label: "Water level", scale: "m", stroke: "#1f6fbf", width: 2, paths: uPlot.paths.stepped!({ align: 1 }) },
        { label: "Design invert", scale: "m", stroke: colour, width: 2 },
        { label: "Cut depth", scale: "cut", stroke: "#b3261e", width: 1, dash: [4, 3] },
      ],
      hooks: {
        draw: [
          (u) => {
            if (marker == null) return;
            const x = u.valToPos(marker, "x", true);
            const { ctx } = u;
            ctx.save();
            ctx.strokeStyle = "#14325a";
            ctx.lineWidth = 2;
            ctx.beginPath();
            ctx.moveTo(x, u.bbox.top);
            ctx.lineTo(x, u.bbox.top + u.bbox.height);
            ctx.stroke();
            ctx.restore();
          },
        ],
      },
    };
  }

  function build() {
    plot?.destroy();
    plot = undefined;
    if (!data || data.chainage_m.length < 2) return;
    plot = new uPlot(options(), series(data), host);
    plot.over.addEventListener("click", (e) => {
      if (plot) onPick(plot.posToVal(e.offsetX, "x"));
    });
  }

  onMount(() => {
    const ro = new ResizeObserver(([entry]) => {
      size = { width: Math.max(200, Math.floor(entry.contentRect.width)), height: Math.max(120, Math.floor(entry.contentRect.height) - 4) };
      plot?.setSize(size);
    });
    ro.observe(host);
    return () => ro.disconnect();
  });
  onDestroy(() => plot?.destroy());

  $effect(() => {
    void data;
    void colour;
    build();
  });
  $effect(() => {
    marker = station;
    plot?.redraw(false);
  });
</script>

<div class="long" data-testid="long-section" data-stations={data?.chainage_m.length ?? 0}>
  <div class="plot" bind:this={host}></div>
</div>

<style>
  .long { height: 100%; min-height: 0; }
  .plot { width: 100%; height: 100%; }
  :global(.long .u-legend) { font-size: 11px; }
</style>
