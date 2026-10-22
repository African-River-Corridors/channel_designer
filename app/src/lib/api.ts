// The only door to the Rust side. Every function forwards to a command (a Tauri command on the desktop,
// the WebAssembly session in a browser, see lib/platform); none computes.
import { platform } from "./platform";

const invoke = <T>(command: string, args?: Record<string, unknown>) => platform().call<T>(command, args);

export type StageName = "sizing" | "alignment" | "optimise" | "volumes";
export type StageState = "not_run" | "fresh" | "stale";

/** Who made a choice: `default` is the tool's, `typed` is the user's own. */
export type ChoiceSource = "default" | "typed";
/** One quantity of the design basis: its value and the Rule that governs it (spec 0007). */
export interface DesignBasisRow {
  quantity: string;
  waterway: string | null;
  value: number;
  unit: string;
  rule: string;
  rule_sets: string[];
  citation: string;
  origin: string;
  status: string;
  other_rules: { rule: string; limit: number }[];
}
/** The channel shape the sizing stage gives (all SI). */
export interface ChannelShape {
  /** The Rule Set selection as words, such as `pianc-wg141@0.3.0 (se_quality B)`. */
  rule_sets: string;
  vessel_standard_id: string;
  loa_m: number;
  beam_m: number;
  draught_m: number;
  formation: string;
  bottom_width_straight_m: number;
  min_bend_radius_m: number;
  design_depth_m: number;
  bend_widening_at_rmin_m: number;
  bottom_width_at_rmin_m: number;
  width_basis: string;
  source: string;
  warnings: string[];
  design_basis?: DesignBasisRow[];
  rules_notice?: string;
}
/** The Rule Set selection and the vessel, as the document stores them. */
export interface SizingSettings {
  rule_sets: { id: string; version: string; hash: string; source: ChoiceSource }[];
  set_inputs: { name: string; value: string; source: ChoiceSource }[];
  vessel_standard_id: string | null;
  draught_m: number | null;
}
export type StretchKind = "width" | "depth" | "radius" | "infeasible";
/** A stretch of a design line where the river falls short of a Rule, with the works it needs. */
export interface ConstraintStretch {
  kind: StretchKind;
  start_chainage_m: number;
  end_chainage_m: number;
  governing_rule: string | null;
  required: number | null;
  river: number | null;
  worst_chainage_m: number | null;
  deficit: number | null;
  works: string | null;
  dredge_m3: number | null;
  dry_cut_m3: number | null;
  reason?: string;
}
export type Verdict = "navigable-as-is" | "navigable-with-works" | "not-navigable";
/** The screening outputs of one option (spec 0007). */
export interface Screening {
  verdict: Verdict;
  notice: string;
  rules_notice: string;
  blocking_not_evaluated: { count: number; rules: string[] };
  constraint_stretches: ConstraintStretch[];
}
/** The parts of an option's volumes results the app reads. */
export interface VolumesOption {
  id: string;
  name: string;
  screening?: Screening;
}
interface StageRecord<S, R> {
  settings: S | null;
  results: R | null;
  depends_on?: string | null;
}
export interface Line {
  type: "LineString";
  coordinates: number[][];
}
export interface Polygon {
  type: "Polygon";
  coordinates: number[][][];
}
export type ProvenanceInput = "left_bank" | "right_bank" | "centreline" | "river_polygon" | "dem" | "water_level" | "route" | "bed";
/** `default` is made by OWD, `edited` is a default the user changed, `user` is the user's own. */
export type Mark = "default" | "edited" | "user";
export interface Provenance {
  source: string;
  date: string;
  notice: string;
  mark: Mark;
  provisional?: boolean;
}
export interface Snapshot {
  engine_version: string;
  document: {
    project: {
      title: string;
      created: string;
      crs: { epsg: number; name: string } | null;
      source_crs: Record<string, string> | null;
    };
    inputs: {
      left_bank: Line | null;
      right_bank: Line | null;
      centreline: Line | null;
      river_polygon: Polygon | null;
      dem: { path: string; sha256?: string } | null;
      water_level: { points: number[][] } | null;
    };
    stages: {
      sizing: StageRecord<SizingSettings, ChannelShape>;
      alignment: StageRecord<unknown, unknown>;
      optimise: StageRecord<unknown, unknown>;
      volumes: StageRecord<unknown, { options: VolumesOption[] }>;
    };
    /** Where each generated or loaded input came from (spec 0012). Absent for a design with none. */
    provenance?: Partial<Record<ProvenanceInput, Provenance>>;
  };
  /** True when a bank edit can be undone. */
  can_undo_bank_edit?: boolean;
  stages: { name: StageName; status: StageState }[];
  file_name: string;
  path: string | null;
  dirty: boolean;
  title: string;
  warnings: string[];
  /** The stage running on a worker now, if any. */
  running: StageName | null;
  /** Every licence notice the inputs in use require (spec 0012). */
  notices?: string[];
  /** Whether the "what fits" answer is current. */
  fits_status?: StageState;
  /** What "Start from a river" shows besides the document (spec 0012). */
  reach_view?: ReachView;
}
/** The planned route and the files it needs, as the session gives them. */
export interface ReachView {
  route: LonLat[] | null;
  length_m?: number;
  as_drawn?: number;
  jrc_tiles: { name: string; url: string; given: boolean }[];
  dem_tiles: { name: string; url: string }[];
  has_mask_banks: boolean;
  /** The reach's start and end as the session recorded them. */
  start?: LonLat;
  end?: LonLat;
  /** The whole river chosen from the search, when there is one: reaches are cut out of it. */
  river?: { osm_id: string; name: string | null; length_m: number; line: LonLat[] } | null;
  /** Constraint stretches of the answer, by Rule Set choice then vessel class, with their ends in WGS84. */
  fits_stretches: Record<string, Record<string, { kind: string; from: LonLat | null; to: LonLat | null }[]>>;
}
/** One vessel class in the "what fits" answer. */
export interface FitsClass {
  vessel_standard_id: string;
  class_name: string;
  family: string;
  loa_m: number;
  beam_m: number;
  fits: boolean;
  worst_width_deficit_m: number | null;
  worst_radius_deficit_m: number | null;
  stretch_count: number;
  warnings: string[];
  label: string;
}
/** The Rule Set and set input a "what fits" choice stands for. */
export interface FitsSelection {
  rule_sets: string[];
  set_inputs: { name: string; value: string }[];
}
export interface FitsResult {
  label: string;
  length_m: number;
  rule_sets: { rule_set: string; selection: FitsSelection | null; largest_fitting: string | null; classes: FitsClass[] }[];
}
/** A JSON part of the document the typed snapshot leaves open. */
export const docPart = <T>(snap: Snapshot | null, ...path: string[]): T | undefined =>
  path.reduce<unknown>((v, k) => (v && typeof v === "object" ? (v as Record<string, unknown>)[k] : undefined), snap?.document) as T | undefined;
/** One set input of a Rule Set: its values, with words, and the default (null: the user must choose). */
export interface SetInputChoice {
  name: string;
  label: string;
  values: { value: string; label: string }[];
  default: string | null;
}
export interface RuleSetChoice {
  id: string;
  version: string;
  title: string;
  label: string;
  inputs: SetInputChoice[];
}
export interface Choices {
  rule_sets: RuleSetChoice[];
  /** Sets that are always selected (the `owd` checks). */
  always: { id: string; version: string; title: string; label: string }[];
  vessels: {
    id: string;
    label: string;
    family: string;
    formation: string;
    loa_m: [number, number];
    beam_m: number;
    draught_nominal_m: number;
  }[];
}

export const snapshot = () => invoke<Snapshot>("snapshot");
export const sizingChoices = () => invoke<Choices>("sizing_choices");
/** Each of these returns null when the user cancels a dialog. */
export const newFile = () => invoke<Snapshot | null>("new_file");
export const openFile = () => invoke<Snapshot | null>("open_file");
export const save = () => invoke<Snapshot | null>("save");
export const saveAs = () => invoke<Snapshot | null>("save_as");
/** Choose the Rule Sets (empty: keep them) and the set inputs; with a vessel class chosen the channel is sized at once. Rejects with the reason. */
export const setRuleSets = (ids: string[], inputs: Record<string, string>) => invoke<Snapshot>("set_rule_sets", { ids, inputs });
/** Choose the vessel class and draught (as typed; empty is the vessel class's own); the channel is sized at once. Rejects with the reason. */
export const setVessel = (vesselStandardId: string, draught: string) => invoke<Snapshot>("set_vessel", { vesselStandardId, draught });
export const runStale = () => invoke<Snapshot>("run_stale");

export type LonLat = [number, number];
export interface MapOption {
  id: string;
  name: string;
  method: string;
  line: Line;
  length_m: number;
  radius_ok: boolean;
  r_min_m: number;
  measured_min_radius_m: number;
  tight_bends: { chainage_m: number; radius_m: number; position: LonLat }[];
  ticks: { chainage_m: number; label: string; position: LonLat }[];
}
/** The plan map's geometry, all in WGS84 (the session converts it). */
export interface MapGeometry {
  crs: "EPSG:4326";
  bbox: [number, number, number, number] | null;
  banks: { side: "left" | "right"; line: Line }[];
  centreline: Line | null;
  river_polygon: Polygon | null;
  options: MapOption[];
}
/** Rejects with the reason when the design has nothing to draw yet. */
export const mapGeometry = () => invoke<MapGeometry>("map_geometry");
/** One constraint stretch on the map: the part of its option's design line between its ends, in WGS84. */
export interface StretchFeature {
  type: "Feature";
  geometry: Line;
  properties: ConstraintStretch & { option_id: string };
}
/** The constraint stretches of every option, as the session exports them (WGS84). */
export interface StretchCollection {
  type: "FeatureCollection";
  screening: string;
  rules_notice: string;
  volumes_status: "not run" | "fresh" | "stale";
  features: StretchFeature[];
}
export const stretchesGeojson = () => invoke<StretchCollection>("stretches_geojson");

/** The DEM hillshade for the offline map: a PNG data URL and its corners (top-left, top-right, bottom-right, bottom-left), or null with no DEM. */
export interface Hillshade {
  url: string;
  corners: [LonLat, LonLat, LonLat, LonLat];
  width: number;
  height: number;
}
export const hillshade = () => invoke<Hillshade | null>("hillshade");
/** A debug-build test hook: a replacement tile address. Null in release builds. */
export const basemapTiles = () => invoke<string | null>("basemap_tiles");

export type InputRole = "left_bank" | "right_bank" | "centreline" | "river_polygon" | "dem";
/** Load one input. With no path the user picks a file (a dropped file is a path on the desktop, a File in a browser); null comes back on cancel. Rejects with the session's reason. */
export const loadInput = (role: InputRole, path?: string | File) => invoke<Snapshot | null>("load_input", { role, path: path ?? null });
/** The water level at the river's start, and optionally a fall per metre (both as typed). */
export const setWater = (level: string, fall: string) => invoke<Snapshot>("set_water", { level, fall });
export type RunMethod = "rolling_circle" | "reroute";
/** The snapshot after a run; `run.outcome` is `done` or `cancelled` (a cancelled run leaves the design as it was). Rejects with the reason. */
export type RunResult = Snapshot & { run: { stage: StageName; outcome: "done" | "cancelled" } };
export const runStage = (stage: StageName, method?: RunMethod) => invoke<RunResult>("run_stage", { stage, method: method ?? null });
export const cancelRun = () => invoke<boolean>("cancel_run");
/** Fetch a Copernicus DEM beside the saved design. */
export const fetchDem = () => invoke<Snapshot>("fetch_dem");

/** One row of the option comparison table. A number the session does not have is null. */
export interface OptionRow {
  id: string;
  name: string;
  method: string;
  dredge_m3: number | null;
  dry_cut_m3: number | null;
  total_m3: number | null;
  tallest_face_m: number | null;
  min_radius_m: number | null;
  r_min_m: number | null;
  radius_ok: boolean | null;
  failed_checks: string[];
}
export interface CheckRow {
  option_id: string;
  option: string;
  check: string;
  passed: boolean;
  detail: string;
}
/** The table and checks list; rows and checks arrive with failures first. */
export interface OptionTable {
  screening: string;
  volumes_status: "not run" | "fresh" | "stale";
  rows: OptionRow[];
  checks: CheckRow[];
}
export const optionTable = () => invoke<OptionTable>("option_table");

/** The long section of one option as series along its chainage. A value the session lacks is null. */
export interface LongSection {
  screening: string;
  option_id: string;
  station_m: number;
  chainage_m: number[];
  bed_m: (number | null)[];
  water_level_m: (number | null)[];
  design_invert_m: (number | null)[];
  cut_depth_m: (number | null)[];
  dredge_area_m2: number[];
  excav_area_m2: number[];
  max_face_h_m: number[];
  positions: LonLat[];
}
/** The cross-section at one station: offsets are negative left, positive right of the line. */
export interface CrossSection {
  screening: string;
  chainage_m: number;
  position: LonLat;
  offsets_m: number[];
  terrain_m: (number | null)[];
  wet: boolean[];
  cut_face_m: (number | null)[];
  water_level_m: number;
  design_invert_m: number;
  half_width_m: number;
  depth_m: number;
  model: string;
  dredge_area_m2: number;
  excav_area_m2: number;
  max_face_h_m: number;
  bench_left: boolean;
  bench_right: boolean;
}
/** These three reject with the session's reason (for example: volumes not run, or stale). */
export const longSection = (option: string) => invoke<LongSection>("long_section", { option });
export const crossSection = (option: string, chainage: number) => invoke<CrossSection>("cross_section", { option, chainage });
export const stationNear = (option: string, lon: number, lat: number) => invoke<number>("station_near", { option, lon, lat });

/** Export through the session to a file the user picks. Returns the path, or null when they cancel. Rejects with the reason (for example: no options yet). */
export const exportFile = (kind: "geojson" | "csv") => invoke<string | null>("export_file", { kind });

/** A drawing sheet as `owd draw` makes it (spec 0004): its number, title and SVG. */
export type SheetPage = { number: string; title: string; svg: string; warnings: string[] };
/** The drawing sheets of an option, from the same builder as `owd draw`. */
export const sheets = (option: string) => invoke<SheetPage[]>("sheets", { option });
/** Write each sheet's PDF of an option; the folder (desktop) or what was saved (web), or null on cancel. */
export const exportSheets = (option: string) => invoke<string | null>("export_sheets", { option });

/** Web only: open the bundled example (a short synthetic river with its DEM). null when the user cancels. */
export const openExample = () => invoke<Snapshot | null>("open_example");

/** One edit to a bank, with its points as `[lon, lat]`: redraw a stretch, or move, add and delete a vertex. */
export type BankEdit =
  | { kind: "splice"; points: LonLat[] }
  | { kind: "move"; index: number; to: LonLat }
  | { kind: "add"; after: number; at: LonLat }
  | { kind: "delete"; index: number };
/** Edit a bank. The session marks it `edited`, rebuilds the polygon and centreline, and makes later stages stale. Rejects with the reason (for example: it would cross the other bank). */
export const editBank = (side: "left_bank" | "right_bank", edit: BankEdit) => invoke<Snapshot>("edit_bank", { side, edit });
/** Undo the last bank edit. */
export const undoBankEdit = () => invoke<Snapshot>("undo_bank_edit");

/** Plan a reach (start, end and via points as [lon, lat]) and start a new study from it. null when the user keeps an unsaved design. */
export const reachRoute = (river: string, start: LonLat, end: LonLat, via: LonLat[]) => invoke<Snapshot | null>("reach_route", { river, start, end, via });
/** Give a JRC occurrence tile the user downloaded (a File in a browser). null on cancel. */
/** One river from the search: "name — region, country", and the OSM object to route. */
export interface RiverHit {
  osm_id: string;
  label: string;
  name: string;
  kind: string;
  region: string | null;
  country: string | null;
  country_code: string | null;
  also: string[];
  bbox: [number, number, number, number];
}
/** The rivers and canals whose name starts with or matches a text (at least 3 letters): Photon as the
 * user types, or Nominatim once (`once`) when Photon fails. */
export const reachSearch = (text: string, country: string, once = false) => invoke<RiverHit[]>("reach_search", { text, country: country.trim() || null, once });
/** Start a new study from a whole river chosen from the search. null when the user keeps an unsaved design. */
export const reachRiver = (osmId: string, also: string[], search: string) => invoke<Snapshot | null>("reach_river", { osmId, also, search });
/** Cut a reach out of the chosen river. null when the user keeps an unsaved design. */
export const reachCut = (start: LonLat, end: LonLat, via: LonLat[]) => invoke<Snapshot | null>("reach_cut", { start, end, via });
export const reachAddTile = (file?: File) => invoke<Snapshot | null>("reach_add_tile", { file: file ?? null });
export type ReachStepName = "banks" | "edge_fit" | "level";
/** Run one step of the reach on the network. Rejects with the reason, or with `cancelled`. */
export const reachStep = (step: ReachStepName, percentile?: number) => invoke<Snapshot>("reach_step", { step, percentile: percentile ?? null });
/** Cancel the reach step that is running: its result is dropped. */
export const reachCancel = () => invoke<void>("reach_cancel");
/** Give the depth of a river with no survey: `[chainage, depth]` points, and a season label. */
export const setBed = (points: [number, number][], season: string) => invoke<Snapshot>("set_bed", { points, season });
/** Answer "what fits" over every vessel class. */
export const runFits = () => invoke<Snapshot>("run_fits");
