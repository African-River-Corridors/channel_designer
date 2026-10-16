// The only door to the Rust side. Every function forwards to a Tauri command; none computes.
import { invoke } from "@tauri-apps/api/core";

export type StageName = "sizing" | "alignment" | "optimise" | "volumes";
export type StageState = "not_run" | "fresh" | "stale";

/** The channel shape the sizing stage gives (all SI). */
export interface ChannelShape {
  ruleset_id: string;
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
}
export interface SizingSettings {
  ruleset_id: string;
  vessel_standard_id: string;
  draught_m: number | null;
}
export interface Line {
  type: "LineString";
  coordinates: number[][];
}
export interface Polygon {
  type: "Polygon";
  coordinates: number[][][];
}
export interface Snapshot {
  engine_version: string;
  document: {
    project: { title: string; created: string; crs: { epsg: number; name: string } | null };
    inputs: {
      left_bank: Line | null;
      right_bank: Line | null;
      centreline: Line | null;
      river_polygon: Polygon | null;
      dem: { path: string; sha256?: string } | null;
    };
    stages: Record<StageName, { settings: SizingSettings | null; results: ChannelShape | null }>;
  };
  stages: { name: StageName; status: StageState }[];
  file_name: string;
  path: string | null;
  dirty: boolean;
  title: string;
  warnings: string[];
}
export interface Choices {
  rulesets: { id: string; label: string; family: string }[];
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
/** Choose ruleset, vessel class and draught (as typed; empty is the vessel class's own); the channel is sized at once. Rejects with the reason. */
export const setSizing = (rulesetId: string, vesselStandardId: string, draught: string) =>
  invoke<Snapshot>("set_sizing", { rulesetId, vesselStandardId, draught });
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
