// Dev only (`npm run dev`, then /mock.html?theme=light|dark): the real app on a fixed fake session, with
// no Rust behind it. It exists to take screenshots of every view. Production builds do not include it.
import type { CrossSection, Hillshade, LongSection, MapGeometry, OptionTable, Snapshot } from "./lib/api";

const theme = new URLSearchParams(location.search).get("theme");
if (theme) document.documentElement.dataset.theme = theme;

const LON0 = 0.5, LAT0 = 6.0, SPAN = 0.06;
type LL = [number, number];
const curve = (amp: number, dy = 0, n = 60): LL[] =>
  Array.from({ length: n + 1 }, (_, i) => {
    const t = i / n;
    return [LON0 + t * SPAN, LAT0 + dy + amp * Math.sin(t * Math.PI * 2) * 0.006] as LL;
  });
const line = (coordinates: LL[]) => ({ type: "LineString" as const, coordinates });
const banks = [line(curve(1, 0.0035)), line(curve(1, -0.0035))];
const ring = [...banks[0].coordinates, ...banks[1].coordinates.slice().reverse(), banks[0].coordinates[0]];

const tick = (l: LL[]) => [0, 15, 30, 45, 60].map((i, k) => ({ chainage_m: k * 3500, label: `${k * 3.5}+000`.replace(".5", ""), position: l[i] }));
const opts = [
  { id: "o1", name: "1 · follows the river", amp: 1, r: 610, ok: false },
  { id: "o2", name: "2 · eases the bends", amp: 0.55, r: 840, ok: true },
  { id: "o3", name: "3 · straightest", amp: 0.12, r: 2950, ok: true },
];
const geometry: MapGeometry = {
  crs: "EPSG:4326",
  bbox: [LON0, LAT0 - 0.012, LON0 + SPAN, LAT0 + 0.012],
  banks: [{ side: "left", line: banks[0] }, { side: "right", line: banks[1] }],
  centreline: line(curve(1)),
  river_polygon: { type: "Polygon", coordinates: [ring] },
  options: opts.map((o) => {
    const l = curve(o.amp);
    return {
      id: o.id, name: o.name, method: "rolling_circle", line: line(l), length_m: 14200, radius_ok: o.ok,
      r_min_m: 800, measured_min_radius_m: o.r,
      tight_bends: o.ok ? [] : [{ chainage_m: 5200, radius_m: o.r, position: l[17] }],
      ticks: tick(l),
    };
  }),
};

const table: OptionTable = {
  screening: "Screening — not for construction",
  volumes_status: "stale",
  rows: [
    [412300, 0, 0.0, 610, false], [538900, 12400, 2.1, 840, true], [301700, 486200, 7.8, 2950, true],
  ].map(([d, c, f, r, ok], i) => ({
    id: opts[i].id, name: opts[i].name, method: "rolling_circle", dredge_m3: d as number, dry_cut_m3: c as number,
    total_m3: (d as number) + (c as number), tallest_face_m: f as number, min_radius_m: r as number, r_min_m: 800,
    radius_ok: ok as boolean, failed_checks: ok ? [] : ["bend radius"],
  })),
  checks: [
    { option_id: "o1", option: "1 · follows the river", check: "Bend radius", passed: false, detail: "610 m is less than R min 800 m." },
    { option_id: "o1", option: "1 · follows the river", check: "Width", passed: true, detail: "88.5 m meets the rule set width." },
    { option_id: "o2", option: "2 · eases the bends", check: "Bend radius", passed: true, detail: "840 m meets R min 800 m." },
  ],
};

const ch = Array.from({ length: 72 }, (_, i) => i * 200);
const wob = (i: number, a: number) => a * Math.sin(i * 0.31) + a * 0.5 * Math.sin(i * 0.11);
const longSection: LongSection = {
  screening: table.screening, option_id: "o1", station_m: 9000, chainage_m: ch,
  bed_m: ch.map((_, i) => 2.4 + wob(i, 0.5)), water_level_m: ch.map((_, i) => 4.2 - Math.floor(i / 12) * 0.12),
  design_invert_m: ch.map(() => 0.9), cut_depth_m: ch.map((_, i) => Math.max(0, 1 + wob(i, 0.7))),
  dredge_area_m2: ch.map(() => 120), excav_area_m2: ch.map(() => 0), max_face_h_m: ch.map(() => 0),
  positions: ch.map((_, i) => curve(1, 0, 71)[i] ?? curve(1)[60]),
};
const offs = Array.from({ length: 41 }, (_, i) => -100 + i * 5);
const crossSection: CrossSection = {
  screening: table.screening, chainage_m: 9000, position: curve(1)[38], offsets_m: offs,
  terrain_m: offs.map((o) => 1.2 + Math.min(4.5, Math.abs(o) / 22) + (Math.abs(o) > 70 ? 1.5 : 0)),
  wet: offs.map((o) => Math.abs(o) < 60), cut_face_m: offs.map((o) => (Math.abs(o) < 45 ? 0.9 + Math.abs(o) / 40 : null)),
  water_level_m: 4.2, design_invert_m: 0.9, half_width_m: 44, depth_m: 3.3, model: "trapezoid",
  dredge_area_m2: 212, excav_area_m2: 0, max_face_h_m: 0, bench_left: false, bench_right: false,
};

const shape = {
  ruleset_id: "pianc-b", vessel_standard_id: "cemt-iv", loa_m: 85, beam_m: 9.5, draught_m: 2.6, formation: "motor barge",
  bottom_width_straight_m: 88.5, min_bend_radius_m: 800, design_depth_m: 3.3, bend_widening_at_rmin_m: 6, bottom_width_at_rmin_m: 94.5,
  width_basis: "PIANC WG 141, quality B", source: "PIANC WG 141", warnings: ["Draught is the vessel default (2.6 m). Type a value to use your own."],
};
const snapshot: Snapshot = {
  engine_version: "0.6.0", file_name: "reach-a.owd", path: null, dirty: false, title: "Reach A, screening", warnings: [], running: null,
  stages: [
    { name: "sizing", status: "fresh" }, { name: "alignment", status: "fresh" },
    { name: "optimise", status: "not_run" }, { name: "volumes", status: "stale" },
  ],
  document: {
    project: { title: "Reach A, screening", created: "2026-10-21", crs: { epsg: 32631, name: "WGS 84 / UTM zone 31N" }, source_crs: null },
    inputs: {
      left_bank: line(banks[0].coordinates), right_bank: line(banks[1].coordinates), centreline: null,
      river_polygon: { type: "Polygon", coordinates: [ring] }, dem: { path: "glo30-reach-a.tif" }, water_level: { points: [[0, 4], [20000, 3]] },
    },
    stages: {
      sizing: { settings: { ruleset_id: "pianc-b", vessel_standard_id: "cemt-iv", draught_m: null }, results: shape },
      alignment: { settings: null, results: null }, optimise: { settings: null, results: null }, volumes: { settings: null, results: null },
    },
  },
};

/** A grey relief drawn on a canvas, standing in for the DEM hillshade. */
function relief(): Hillshade {
  const c = document.createElement("canvas");
  c.width = 480; c.height = 240;
  const g = c.getContext("2d")!;
  const img = g.createImageData(c.width, c.height);
  for (let y = 0; y < c.height; y++)
    for (let x = 0; x < c.width; x++) {
      const v = 150 + 50 * Math.sin(x / 31 + Math.sin(y / 23)) * Math.cos(y / 41) + 20 * Math.sin((x + y) / 9);
      img.data.set([v, v, v * 0.96, 255], (y * c.width + x) * 4);
    }
  g.putImageData(img, 0, 0);
  const [w, s, e, n] = geometry.bbox!;
  return { url: c.toDataURL("image/png"), corners: [[w, n], [e, n], [e, s], [w, s]], width: c.width, height: c.height };
}

const answer: Record<string, () => unknown> = {
  snapshot: () => snapshot, set_sizing: () => snapshot, sizing_choices: () => ({
    rulesets: [{ id: "pianc-b", label: "PIANC WG 141, quality B", family: "pianc" }, { id: "pianc-a", label: "PIANC WG 141, quality A", family: "pianc" }],
    vessels: [{ id: "cemt-iv", label: "CEMT IV, motor barge", family: "cemt", formation: "motor barge", loa_m: [80, 85], beam_m: 9.5, draught_nominal_m: 2.6 }],
  }),
  map_geometry: () => new Promise((ok) => setTimeout(() => ok(geometry), 400)), option_table: () => table, long_section: () => longSection, cross_section: () => crossSection,
  station_near: () => 9000, hillshade: relief,
  // The tile address fails at once, so the map shows the hillshade and its offline note.
  basemap_tiles: () => "http://127.0.0.1:9/{z}/{x}/{y}.jpg",
};

(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: (cmd: string) => Promise.resolve(answer[cmd] ? answer[cmd]() : null),
  transformCallback: () => 0,
  metadata: {},
};

await import("./main");
