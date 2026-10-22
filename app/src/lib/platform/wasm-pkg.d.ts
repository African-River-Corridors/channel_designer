// Stand-in types for the wasm-bindgen output (app/wasm-pkg/, built by tools/build_web.sh, not in git),
// so `npm run check` works before the WebAssembly build has run.
declare module "*/wasm-pkg/owd_web.js" {
  export class Session {
    constructor();
    snapshot(): string;
    sizing_choices(): string;
    new_design(): string;
    open_text(name: string, text: string, dem?: Uint8Array): string;
    save_text(name: string): string;
    saved(name: string): string;
    is_dirty(): boolean;
    set_rule_sets(ids: string, inputs: string): string;
    set_vessel(vessel_standard_id: string, draught: string): string;
    stretches_geojson(): string;
    run_stale(): string;
    load_input(role: string, name: string, bytes: Uint8Array, layer?: string): string;
    load_dem(name: string, bytes: Uint8Array): string;
    set_water(level: string, fall: string): string;
    run_stage(stage: string, method?: string, progress?: (samples: number) => void): string;
    option_table(): string;
    map_geometry(): string;
    hillshade(): string;
    long_section(option: string): string;
    cross_section(option: string, chainage: number): string;
    station_near(option: string, lon: number, lat: number): number;
    export_geojson(): string;
    export_csv(): string;
    reach_route(river: string, points: string, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_search(text: string, country: string, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_river(osm_id: string, also: string, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_cut(points: string): string;
    reach_route_only(): boolean;
    reach_add_tile(name: string, bytes: Uint8Array): string;
    reach_step(step: string, percentile: number | undefined, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    set_bed(points: string, season: string): string;
    run_fits(): string;
  }
  export function engine_version(): string;
  export default function init(input?: unknown): Promise<unknown>;
}
declare module "*.wasm?url" {
  const url: string;
  export default url;
}
