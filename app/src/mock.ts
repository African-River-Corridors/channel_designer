// Dev only (`npm run dev`, then /mock.html?theme=light|dark): the real app on a fixed fake session, with
// no Rust behind it. It exists to take screenshots of every view. Production builds do not include it.
import type { ChannelShape, ConstraintStretch, CrossSection, Hillshade, LongSection, MapGeometry, OptionTable, Snapshot, StretchCollection } from "./lib/api";

const params = new URLSearchParams(location.search);
const theme = params.get("theme");
// ?state=empty: a new design with nothing loaded, to see the empty views.
const empty = params.get("state") === "empty";
// ?theme=auto leaves the choice to the OS (the app's default); light and dark pin it.
if (theme === "auto") delete document.documentElement.dataset.theme;
else if (theme) document.documentElement.dataset.theme = theme;

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

const RULES_NOTICE = "Every Rule is unverified until a second reader checks it against its source.";
const WG141 = "PIANC InCom WG 141, Design Guidelines for Inland Waterway Dimensions, PIANC Report 141, 2019, Table 5.7";
const shape: ChannelShape = {
  rule_sets: "pianc-wg141@0.3.0 (se_quality B)", vessel_standard_id: "cemt-iv", loa_m: 85, beam_m: 9.5, draught_m: 2.6, formation: "motor barge",
  bottom_width_straight_m: 88.5, min_bend_radius_m: 800, design_depth_m: 3.3, bend_widening_at_rmin_m: 6, bottom_width_at_rmin_m: 94.5,
  width_basis: "rule", source: "Rule Sets pianc-wg141@0.3.0, owd@0.3.0", warnings: ["Draught is the vessel default (2.6 m). Type a value to use your own."],
  rules_notice: RULES_NOTICE,
  design_basis: [
    ["bottom-width", null, 88.5, "pianc-wg141/Table5.7/bottom-width-b"],
    ["min-bend-radius", "free-flowing-river", 800, "pianc-wg141/Table5.7/min-bend-radius-b"],
    ["design-depth", null, 3.3, "pianc-wg141/5.3.4.3/design-depth-b"],
  ].map(([quantity, waterway, value, rule]) => ({
    quantity: quantity as string, waterway: waterway as string | null, value: value as number, unit: "m", rule: rule as string,
    rule_sets: ["pianc-wg141@0.3.0"], citation: WG141, origin: "guideline", status: "unverified", other_rules: [],
  })),
};
// The stretches of option 1, at chainages along its line.
const stretchList: ConstraintStretch[] = [
  { kind: "depth", start_chainage_m: 0, end_chainage_m: 3200, governing_rule: "pianc-wg141/5.3.4.3/design-depth-b", required: 3.3, river: 2.1, worst_chainage_m: 2400, deficit: 1.2, works: "dredge", dredge_m3: 182400, dry_cut_m3: 0 },
  { kind: "radius", start_chainage_m: 4800, end_chainage_m: 5600, governing_rule: "pianc-wg141/Table5.7/min-bend-radius-b", required: 800, river: 610, worst_chainage_m: 5200, deficit: 190, works: "realign", dredge_m3: null, dry_cut_m3: null },
  { kind: "width", start_chainage_m: 9000, end_chainage_m: 10400, governing_rule: "pianc-wg141/Table5.7/bottom-width-b", required: 88.5, river: 71.2, worst_chainage_m: 9800, deficit: 17.3, works: "bank-cut", dredge_m3: 41200, dry_cut_m3: 12400 },
];
const o1 = curve(1);
const along = (c: number) => o1[Math.min(60, Math.round((c / 14200) * 60))];
const stretches: StretchCollection = {
  type: "FeatureCollection", screening: table.screening, rules_notice: RULES_NOTICE, volumes_status: "stale",
  features: stretchList.map((c) => ({
    type: "Feature" as const,
    geometry: line(o1.slice(Math.round((c.start_chainage_m / 14200) * 60), Math.round((c.end_chainage_m / 14200) * 60) + 1).concat(c.start_chainage_m === c.end_chainage_m ? [along(c.end_chainage_m)] : [])),
    properties: { ...c, option_id: "o1" },
  })),
};
const screening = (verdict: "navigable-as-is" | "navigable-with-works" | "not-navigable", list: ConstraintStretch[]) => ({
  verdict, notice: table.screening, rules_notice: RULES_NOTICE, blocking_not_evaluated: { count: 0, rules: [] }, constraint_stretches: list,
});
const snapshot: Snapshot = {
  engine_version: "0.6.0", file_name: "reach-a.owd", path: null, dirty: false, title: "Reach A, screening", warnings: ["Draught is the vessel default (2.6 m). Type a value to use your own."], running: null,
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
      sizing: {
        settings: {
          rule_sets: [{ id: "pianc-wg141", version: "0.3.0", hash: "0a80", source: "typed" }, { id: "owd", version: "0.3.0", hash: "f6e4", source: "default" }],
          set_inputs: [{ name: "se_quality", value: "B", source: "typed" }],
          vessel_standard_id: "cemt-iv", draught_m: null,
        },
        results: shape,
      },
      alignment: { settings: null, results: null }, optimise: { settings: null, results: null },
      volumes: {
        settings: null,
        results: { options: [
          { id: "o1", name: opts[0].name, screening: screening("not-navigable", stretchList) },
          { id: "o2", name: opts[1].name, screening: screening("navigable-with-works", []) },
          { id: "o3", name: opts[2].name, screening: screening("navigable-as-is", []) },
        ] },
      },
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

const blank = (): Snapshot => ({
  ...snapshot, file_name: "Untitled", warnings: [], title: "Untitled",
  stages: snapshot.stages.map((x) => ({ ...x, status: "not_run" as const })),
  document: { ...snapshot.document, project: { ...snapshot.document.project, title: "Untitled", crs: null },
    inputs: { left_bank: null, right_bank: null, centreline: null, river_polygon: null, dem: null, water_level: null },
    stages: { ...snapshot.document.stages, sizing: { settings: null, results: null }, volumes: { settings: null, results: null } } },
});
const none = (why: string) => () => Promise.reject(why);

const answer: Record<string, () => unknown> = {
  snapshot: () => snapshot, set_rule_sets: () => snapshot, set_vessel: () => snapshot, stretches_geojson: () => stretches, sizing_choices: () => ({
    rule_sets: [
      { id: "pianc-wg141", version: "0.3.0", title: "PIANC WG 141", label: "PIANC WG 141 (pianc-wg141@0.3.0)", inputs: [{ name: "se_quality", label: "Safety and ease quality", default: "C", values: [{ value: "A", label: "A: the most room" }, { value: "B", label: "B: between A and C" }, { value: "C", label: "C: the least room" }] }] },
      { id: "gb50139", version: "0.3.0", title: "GB 50139", label: "GB 50139 (gb50139@0.3.0)", inputs: [{ name: "range_end", label: "End of each range the standard gives", default: null, values: [{ value: "permissive", label: "permissive: the less demanding end" }, { value: "strict", label: "strict: the more demanding end" }] }] },
    ],
    always: [{ id: "owd", version: "0.3.0", title: "OWD checks", label: "OWD checks (owd@0.3.0)" }],
    vessels: [{ id: "cemt-iv", label: "CEMT IV, motor barge", family: "cemt", formation: "motor barge", loa_m: [80, 85], beam_m: 9.5, draught_nominal_m: 2.6 }],
  }),
  map_geometry: () => geometry, option_table: () => table, long_section: () => longSection, cross_section: () => crossSection,
  station_near: () => 9000, hillshade: relief,
  export_sheets: () => "/tmp/owd-sheets",
  export_report: () => "/tmp/owd-reports",
  sheets: () => [{ number: "OWD-MOCK-PL-001", title: "PLAN OVERVIEW", warnings: [],
    svg: '<svg xmlns="http://www.w3.org/2000/svg" width="841mm" height="594mm" viewBox="0 0 841 594"><rect x="20" y="10" width="811" height="574" fill="none" stroke="#000"/></svg>' }],
  // The tile address fails at once, so the map shows the hillshade and its offline note.
  basemap_tiles: () => "http://127.0.0.1:9/{z}/{x}/{y}.jpg",
};

if (empty) {
  Object.assign(answer, {
    snapshot: blank, set_rule_sets: blank, set_vessel: blank, stretches_geojson: none("nothing to export"), map_geometry: none("nothing to draw"), hillshade: () => null,
    option_table: () => ({ screening: table.screening, volumes_status: "not run", rows: [], checks: [] }),
    long_section: none("Run volumes to see the long section."), cross_section: none("Run volumes to see the cross-section."),
  });
}

(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: (cmd: string) => Promise.resolve(answer[cmd] ? answer[cmd]() : null),
  transformCallback: () => 0,
  metadata: {},
};

await import("./main");
