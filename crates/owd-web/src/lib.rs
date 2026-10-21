//! The design session in the browser (spec 0011).
//!
//! A thin forwarder: each method calls `owd_shell::state::AppState` and returns the same JSON the
//! desktop app's commands return. It holds no calculation, validation or default logic. Errors are
//! thrown to JS as plain strings. A stage runs to its end inside one call: the page runs this in a
//! Web Worker, and Cancel ends the Worker (wasm cannot unwind out of a stage).

use owd_format::design::date_from_unix;
use owd_shell::input;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use wasm_bindgen::prelude::*;

type Js<T> = Result<T, JsValue>;

fn err(message: String) -> JsValue {
    JsValue::from_str(&message)
}

fn ok<T>(result: Result<T, String>) -> Js<T> {
    result.map_err(err)
}

fn today() -> String {
    date_from_unix((js_sys::Date::now() / 1000.0) as u64)
}

fn new_design() -> AppState {
    AppState::new_design("Untitled channel design", &today())
}

/// The network for the reach steps (spec 0012): a JS function `(url, start, length)` that returns
/// the bytes as a `Uint8Array`, or throws. `start` is -1 for the whole file. The page's Worker gives
/// one that makes a synchronous request, which a Worker may do.
struct JsFetch(js_sys::Function);

impl JsFetch {
    fn call(&self, url: &str, start: f64, len: f64) -> Result<Vec<u8>, owd_reach::FetchError> {
        self.0
            .call3(
                &JsValue::NULL,
                &JsValue::from_str(url),
                &JsValue::from_f64(start),
                &JsValue::from_f64(len),
            )
            .map(|v| js_sys::Uint8Array::new(&v).to_vec())
            .map_err(|e| {
                let reason = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                if reason.contains("404") {
                    owd_reach::FetchError::NotFound(url.to_string())
                } else {
                    owd_reach::FetchError::Failed {
                        url: url.to_string(),
                        reason,
                    }
                }
            })
    }
}

impl owd_reach::Fetch for JsFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, owd_reach::FetchError> {
        self.call(url, -1.0, -1.0)
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, owd_reach::FetchError> {
        self.call(url, start as f64, len as f64)
    }
}

fn endpoints(json: &str) -> Result<owd_shell::reach::ReachEndpoints, String> {
    if json.trim().is_empty() {
        return Ok(Default::default());
    }
    let v: Value = serde_json::from_str(json).map_err(|e| format!("not endpoints: {e}"))?;
    Ok(owd_shell::reach::ReachEndpoints::from_json(&v))
}

fn point(v: &Value) -> Result<[f64; 2], String> {
    match (v[0].as_f64(), v[1].as_f64()) {
        (Some(a), Some(b)) => Ok([a, b]),
        _ => Err(format!("`{v}` is not [lon, lat]")),
    }
}

/// The engine version, as written in a `.owd` file.
#[wasm_bindgen]
pub fn engine_version() -> String {
    owd_format::design::session::ENGINE_VERSION.to_string()
}

/// One open channel design, as in the desktop app.
#[wasm_bindgen]
pub struct Session {
    state: AppState,
}

impl Session {
    /// Edit, then return the snapshot JSON.
    fn edit(&mut self, f: impl FnOnce(&mut AppState) -> Result<(), String>) -> Js<String> {
        ok(f(&mut self.state))?;
        Ok(self.snapshot())
    }

    /// The station walk of an option, made now or taken from the cache.
    fn walked(&mut self, option: &str) -> Js<owd_shell::state::Walked> {
        let walked = ok(self.state.begin_walk(option).run())?;
        self.state.keep_walk(&walked);
        Ok(walked)
    }
}

#[wasm_bindgen]
impl Session {
    /// A new, empty channel design.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Session {
        Session {
            state: new_design(),
        }
    }

    pub fn snapshot(&self) -> String {
        self.state.snapshot().to_string()
    }

    pub fn sizing_choices(&self) -> String {
        owd_format::design::Session::sizing_choices().to_string()
    }

    pub fn new_design(&mut self) -> String {
        self.state = new_design();
        self.snapshot()
    }

    /// Open `.owd` text. `dem` is the DEM file the document names, when the page has it.
    pub fn open_text(&mut self, name: &str, text: &str, dem: Option<Vec<u8>>) -> Js<String> {
        self.state = ok(AppState::open(Path::new(name), text, dem.as_deref()))?;
        Ok(self.snapshot())
    }

    /// The `.owd` text to save as `name` (the DEM path is said from there).
    pub fn save_text(&mut self, name: &str) -> String {
        self.state.save_text_for(Path::new(name))
    }

    /// Record that the design was saved as `name`.
    pub fn saved(&mut self, name: &str) -> String {
        self.state.saved(Path::new(name));
        self.snapshot()
    }

    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    /// Choose the Rule Sets and set inputs. `ids` is a JSON list of set ids, `inputs` a JSON object
    /// of input name to value.
    pub fn set_rule_sets(&mut self, ids: &str, inputs: &str) -> Js<String> {
        let ids: Vec<String> =
            ok(serde_json::from_str(ids).map_err(|e| format!("not a list of Rule Sets: {e}")))?;
        let inputs: std::collections::BTreeMap<String, String> =
            ok(serde_json::from_str(inputs).map_err(|e| format!("not set inputs: {e}")))?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let inputs: Vec<(&str, &str)> = inputs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.edit(|s| s.set_rule_sets(&ids, &inputs))
    }

    pub fn set_vessel(&mut self, vessel_standard_id: &str, draught: &str) -> Js<String> {
        let draught = ok(AppState::parse_draught(draught))?;
        self.edit(|s| s.set_vessel(vessel_standard_id, draught))
    }

    /// The constraint stretches as GeoJSON lines in WGS84, for the map.
    pub fn stretches_geojson(&self) -> Js<String> {
        Ok(ok(self.state.stretches_geojson())?.to_string())
    }

    pub fn run_stale(&mut self) -> Js<String> {
        self.edit(|s| s.run_stale().map(|_| ()))
    }

    /// Load one vector input from the bytes of a GeoJSON or GeoPackage file.
    pub fn load_input(
        &mut self,
        role: &str,
        name: &str,
        bytes: Vec<u8>,
        layer: Option<String>,
    ) -> Js<String> {
        let role = ok(input::role_of(role))?;
        let text = ok(input::vector_text(name, bytes, layer.as_deref()))?;
        let date = today();
        self.edit(|s| s.load_inputs(&[(role, text)], &date))
    }

    /// Edit a bank (spec 0012). `edit` is JSON, see `owd_shell::input::bank_edit_of`.
    pub fn edit_bank(&mut self, side: &str, edit: &str) -> Js<String> {
        let side = ok(input::role_of(side))?;
        let edit: Value =
            ok(serde_json::from_str(edit).map_err(|e| format!("not a bank edit: {e}")))?;
        let edit = ok(input::bank_edit_of(&edit))?;
        let date = today();
        self.edit(|s| s.edit_bank(side, edit, &date))
    }

    /// Plan a reach and start a new study from it (spec 0012). `points` is JSON
    /// `{"start": [lon, lat], "end": [lon, lat], "via": [[lon, lat], ...]}`.
    pub fn reach_route(
        &mut self,
        river: &str,
        points: &str,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<String> {
        let v: Value =
            ok(serde_json::from_str(points).map_err(|e| format!("not reach points: {e}")))?;
        let start = ok(point(&v["start"]))?;
        let end = ok(point(&v["end"]))?;
        let via = ok(v["via"]
            .as_array()
            .map(|a| a.iter().map(point).collect::<Result<Vec<_>, _>>())
            .unwrap_or(Ok(Vec::new())))?;
        let at = ok(endpoints(at))?;
        let date = today();
        let river = Some(river).filter(|r| !r.trim().is_empty());
        self.edit(|s| s.reach_route(&JsFetch(fetch), &at, river, start, end, &via, &date))
    }

    /// Give a JRC occurrence tile the user downloaded and dropped in.
    pub fn reach_add_tile(&mut self, name: &str, bytes: Vec<u8>) -> Js<String> {
        self.edit(|s| s.reach_add_tile(name, bytes))
    }

    /// Run one reach step: `banks`, `edge_fit` or `level` (with an optional percentile).
    pub fn reach_step(
        &mut self,
        step: &str,
        percentile: Option<f64>,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<String> {
        let step = ok(owd_shell::reach::ReachStep::of(step, percentile))?;
        let at = ok(endpoints(at))?;
        let date = today();
        let job = ok(self.state.begin_reach(step))?;
        let job = ok(job.run(&JsFetch(fetch), &at, &date))?;
        self.edit(|s| s.keep_reach(job))
    }

    /// Give the depth of a river with no survey. `points` is JSON `[[chainage, depth], ...]`.
    pub fn set_bed(&mut self, points: &str, season: &str) -> Js<String> {
        let pts: Vec<[f64; 2]> =
            ok(serde_json::from_str(points).map_err(|e| format!("not a depth table: {e}")))?;
        let date = today();
        self.edit(|s| s.set_bed(&pts, Some(season), &date))
    }

    /// Answer "what fits" over every vessel class.
    pub fn run_fits(&mut self) -> Js<String> {
        self.edit(|s| s.run_fits())
    }

    /// Undo the last bank edit.
    pub fn undo_bank_edit(&mut self) -> Js<String> {
        self.edit(|s| s.undo_bank_edit())
    }

    /// Load the DEM from the bytes of a GeoTIFF.
    pub fn load_dem(&mut self, name: &str, bytes: Vec<u8>) -> Js<String> {
        let date = today();
        self.edit(|s| s.load_dem(Path::new(name), bytes, &date))
    }

    pub fn set_water(&mut self, level: &str, fall: &str) -> Js<String> {
        let num = |t: &str, what: &str| -> Result<Option<f64>, String> {
            let t = t.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>()
                .map(Some)
                .map_err(|_| format!("the {what} must be a number, not `{t}`"))
        };
        let level = ok(num(level, "water level"))?;
        let level = ok(level.ok_or_else(|| "give a water level in metres".to_string()))?;
        let fall = ok(num(fall, "fall per metre"))?;
        self.edit(|s| s.set_water(level, fall))
    }

    /// Run one stage to its end. `progress`, when given, is called with the number of terrain reads
    /// so far, at most four times a second. The reply is the snapshot, with `run` saying how it ended.
    pub fn run_stage(
        &mut self,
        stage: &str,
        method: Option<String>,
        progress: Option<js_sys::Function>,
    ) -> Js<String> {
        let (job, _control) = ok(self.state.begin_run(stage, method.as_deref()))?;
        // The wasm module has one thread, so the runner's `Sync` bound holds for the JS callback.
        struct OneThread<T>(T);
        // SAFETY: nothing in this module ever starts a second thread.
        unsafe impl<T> Sync for OneThread<T> {}
        let progress = OneThread(progress);
        let last = AtomicU64::new(js_sys::Date::now().to_bits());
        let outcome = job.execute(&|n| {
            if let Some(f) = &progress.0 {
                let now = js_sys::Date::now();
                if now - f64::from_bits(last.load(Ordering::Relaxed)) >= 250.0 {
                    last.store(now.to_bits(), Ordering::Relaxed);
                    let _ = f.call1(&JsValue::NULL, &JsValue::from_f64(n as f64));
                }
            }
        });
        let ended = ok(self.state.finish_run(outcome))?;
        let mut snap: Value = self.state.snapshot();
        snap["run"] = json!({"stage": stage, "outcome": ended});
        Ok(snap.to_string())
    }

    pub fn option_table(&self) -> String {
        self.state.option_table().to_string()
    }

    pub fn map_geometry(&self) -> Js<String> {
        Ok(ok(self.state.map_geometry())?.to_string())
    }

    /// The hillshade of the DEM as a PNG data URL with its corners, or `null` with no DEM.
    pub fn hillshade(&self) -> Js<String> {
        use base64::Engine;
        let Some(dem) = self.state.dem() else {
            return Ok("null".to_string());
        };
        let shade = ok(owd_format::design::hillshade::hillshade(&dem).map_err(|e| e.to_string()))?;
        Ok(json!({
            "url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&shade.png)),
            "corners": shade.corners,
            "width": shade.width,
            "height": shade.height,
        })
        .to_string())
    }

    pub fn long_section(&mut self, option: &str) -> Js<String> {
        let w = self.walked(option)?;
        Ok(ok(w
            .session
            .long_section(option, &w.walk)
            .map_err(|e| e.to_string()))?
        .to_string())
    }

    pub fn cross_section(&mut self, option: &str, chainage: f64) -> Js<String> {
        let w = self.walked(option)?;
        Ok(ok(w
            .session
            .cross_section(&w.walk, chainage)
            .map_err(|e| e.to_string()))?
        .to_string())
    }

    pub fn station_near(&mut self, option: &str, lon: f64, lat: f64) -> Js<f64> {
        let w = self.walked(option)?;
        ok(w.session
            .station_near(&w.walk, lon, lat)
            .map_err(|e| e.to_string()))
    }

    /// The options as GeoJSON in longitude and latitude.
    pub fn export_geojson(&self) -> Js<String> {
        ok(self.state.export_geojson())
    }

    /// The quantity table as CSV.
    pub fn export_csv(&self) -> String {
        self.state.export_csv()
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}
