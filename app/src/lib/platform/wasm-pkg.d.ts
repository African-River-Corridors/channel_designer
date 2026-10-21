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
    set_sizing(ruleset_id: string, vessel_standard_id: string, draught: string): string;
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
  }
  export function engine_version(): string;
  export default function init(input?: unknown): Promise<unknown>;
}
declare module "*.wasm?url" {
  const url: string;
  export default url;
}
