//! The Open Waterway Designer desktop app shell.
//!
//! Thin by design (spec 0003): each command forwards to the design session in `owd-format`. The
//! shell owns the native open and save dialogs, the unsaved-changes guard, the window title and
//! the clock, and nothing else. It holds no calculation, validation or default logic.

use base64::Engine;
use owd_format::design::{Session, date_from_unix};
use owd_shell::state::{self, AppState};
use owd_shell::{input, run};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

type Shared = Mutex<AppState>;
type Reply = Result<serde_json::Value, String>;

fn lock(state: &Shared) -> Result<std::sync::MutexGuard<'_, AppState>, String> {
    state
        .lock()
        .map_err(|_| "the app state is unusable".to_string())
}

/// Set the window title from the state and return the snapshot for the front end.
fn finish(window: &Window, app: &AppState) -> Reply {
    window.set_title(&app.title()).map_err(|e| e.to_string())?;
    Ok(app.snapshot())
}

fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    date_from_unix(secs)
}

/// Smoke-test hook, debug builds only. A native dialog cannot be driven by WebDriver, so when
/// `CD_TEST_DIALOG_DIR` is set, each dialog is answered from a file in that folder instead:
/// `open.txt` and `save.txt` hold the path to pick (no file means the user cancelled), and
/// `discard.txt` holds `no` to refuse a discard (anything else, or no file, agrees). Release
/// builds have no such hook.
#[cfg(debug_assertions)]
fn test_answer(file: &str) -> Option<Option<String>> {
    let dir = std::env::var_os("CD_TEST_DIALOG_DIR")?;
    let text = std::fs::read_to_string(Path::new(&dir).join(file)).ok();
    Some(text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
}

#[cfg(not(debug_assertions))]
fn test_answer(_file: &str) -> Option<Option<String>> {
    None
}

/// Ask whether to discard unsaved changes. `true` means go ahead.
fn ask_discard(app: &AppHandle, text: &str) -> bool {
    if let Some(answer) = test_answer("discard.txt") {
        return answer.as_deref() != Some("no");
    }
    app.dialog()
        .message(text)
        .title("Unsaved changes")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Discard".into(),
            "Cancel".into(),
        ))
        .blocking_show()
}

/// Ask before the unsaved channel design is lost. `true` means go ahead.
fn confirm_discard(app: &AppHandle, state: &Shared) -> Result<bool, String> {
    if !lock(state)?.is_dirty() {
        return Ok(true);
    }
    Ok(ask_discard(
        app,
        "The channel design has unsaved changes. Discard them?",
    ))
}

/// Ask for a file to open. `None` when the user cancels.
/// What the open dialog offers. A file with another name that is still picked is opened by
/// `AppState::open`, which refuses the old extension with a message to rename the file (spec 0006).
const OPEN_EXTENSIONS: [&str; 1] = ["owd"];
/// What a save writes.
const SAVE_EXTENSION: &str = "owd";

fn pick_open(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    if let Some(answer) = test_answer("open.txt") {
        return Ok(answer.map(PathBuf::from));
    }
    match app
        .dialog()
        .file()
        .add_filter("Open Waterway Designer file", &OPEN_EXTENSIONS)
        .blocking_pick_file()
    {
        Some(picked) => Ok(Some(picked.into_path().map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

/// Ask where to save. `None` when the user cancels.
fn pick_save(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let picked = match test_answer("save.txt") {
        Some(answer) => answer.map(PathBuf::from),
        None => match app
            .dialog()
            .file()
            .add_filter("Open Waterway Designer file", &[SAVE_EXTENSION])
            .set_file_name(format!("waterway-design.{SAVE_EXTENSION}"))
            .blocking_save_file()
        {
            Some(p) => Some(p.into_path().map_err(|e| e.to_string())?),
            None => None,
        },
    };
    Ok(picked.map(|mut p| {
        if p.extension().is_none() {
            p.set_extension(SAVE_EXTENSION);
        }
        p
    }))
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, data: &[u8]) -> Result<(), String> {
    // Write beside the target and rename, so a failed write cannot leave half a file.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[tauri::command]
fn snapshot(window: Window, state: State<Shared>) -> Reply {
    let guard = lock(&state)?;
    finish(&window, &guard)
}

/// The Rule Sets with their set inputs, and the vessel classes, to choose from.
#[tauri::command]
fn sizing_choices() -> serde_json::Value {
    Session::sizing_choices()
}

/// `null` when the user cancels.
#[tauri::command]
async fn new_file(app: AppHandle, window: Window, state: State<'_, Shared>) -> Reply {
    if !confirm_discard(&app, &state)? {
        return Ok(serde_json::Value::Null);
    }
    let mut guard = lock(&state)?;
    *guard = AppState::new_design("Untitled channel design", &today());
    finish(&window, &guard)
}

/// `null` when the user cancels.
#[tauri::command]
async fn open_file(app: AppHandle, window: Window, state: State<'_, Shared>) -> Reply {
    if !confirm_discard(&app, &state)? {
        return Ok(serde_json::Value::Null);
    }
    let Some(path) = pick_open(&app)? else {
        return Ok(serde_json::Value::Null);
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    // The DEM is read only to check its hash against the one the file recorded.
    let dem = AppState::dem_path(&text)
        .and_then(|rel| std::fs::read(path.parent().unwrap_or(Path::new(".")).join(rel)).ok());
    let opened = AppState::open(&path, &text, dem.as_deref())?;
    let mut guard = lock(&state)?;
    *guard = opened;
    finish(&window, &guard)
}

fn save_to(app: &AppHandle, window: &Window, state: &Shared, ask_for_path: bool) -> Reply {
    let known = lock(state)?.path().map(Path::to_path_buf);
    let path = match known {
        Some(p) if !ask_for_path => p,
        _ => {
            let Some(p) = pick_save(app)? else {
                return Ok(serde_json::Value::Null);
            };
            p
        }
    };
    // The text is made for this path: the DEM's relative path is said from the folder it lands in.
    let text = lock(state)?.save_text_for(&path);
    write_file(&path, &text)?;
    let mut guard = lock(state)?;
    guard.saved(&path);
    finish(window, &guard)
}

/// `null` when the user cancels the file dialog.
#[tauri::command]
async fn save(app: AppHandle, window: Window, state: State<'_, Shared>) -> Reply {
    save_to(&app, &window, &state, false)
}

#[tauri::command]
async fn save_as(app: AppHandle, window: Window, state: State<'_, Shared>) -> Reply {
    save_to(&app, &window, &state, true)
}

/// Choose the Rule Sets and the set inputs (name to value); with a vessel class chosen, the
/// channel is sized at once.
#[tauri::command]
fn set_rule_sets(
    window: Window,
    state: State<Shared>,
    ids: Vec<String>,
    inputs: std::collections::BTreeMap<String, String>,
) -> Reply {
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let inputs: Vec<(&str, &str)> = inputs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut guard = lock(&state)?;
    guard.set_rule_sets(&ids, &inputs)?;
    finish(&window, &guard)
}

/// Choose the vessel class and the draught (as typed); the channel is sized at once.
#[tauri::command]
fn set_vessel(
    window: Window,
    state: State<Shared>,
    vessel_standard_id: String,
    draught: String,
) -> Reply {
    let mut guard = lock(&state)?;
    guard.set_vessel(&vessel_standard_id, AppState::parse_draught(&draught)?)?;
    finish(&window, &guard)
}

/// Choose a custom vessel from the form's fields (spec 0015 T2); the channel is sized at once.
#[tauri::command]
fn set_custom_vessel(window: Window, state: State<Shared>, fields: serde_json::Value) -> Reply {
    let mut guard = lock(&state)?;
    guard.set_custom_vessel(&fields)?;
    finish(&window, &guard)
}

/// Check a custom vessel's fields: the refusal and the field it is on, or the warnings.
#[tauri::command]
fn check_custom_vessel(state: State<Shared>, fields: serde_json::Value) -> Reply {
    Ok(lock(&state)?.check_custom_vessel(&fields))
}

/// The constraint stretches as GeoJSON lines in WGS84, for the map.
#[tauri::command]
fn stretches_geojson(state: State<Shared>) -> Reply {
    lock(&state)?.stretches_geojson()
}

/// Make or fetch the station walk of an option off the main thread, and remember it.
async fn walked(state: &State<'_, Shared>, option: String) -> Result<state::Walked, String> {
    let job = lock(state)?.begin_walk(&option);
    let walked = tauri::async_runtime::spawn_blocking(move || job.run())
        .await
        .map_err(|e| e.to_string())??;
    lock(state)?.keep_walk(&walked);
    Ok(walked)
}

/// The long section of an option: bed, water level, design invert and cut depth along its chainage.
#[tauri::command]
async fn long_section(state: State<'_, Shared>, option: String) -> Reply {
    let w = walked(&state, option.clone()).await?;
    w.session
        .long_section(&option, &w.walk)
        .map_err(|e| e.to_string())
}

/// The cross-section at the station nearest to a chainage.
#[tauri::command]
async fn cross_section(state: State<'_, Shared>, option: String, chainage: f64) -> Reply {
    let w = walked(&state, option).await?;
    w.session
        .cross_section(&w.walk, chainage)
        .map_err(|e| e.to_string())
}

/// The chainage of the station nearest to a point picked on the map.
#[tauri::command]
async fn station_near(
    state: State<'_, Shared>,
    option: String,
    lon: f64,
    lat: f64,
) -> Result<f64, String> {
    let w = walked(&state, option).await?;
    w.session
        .station_near(&w.walk, lon, lat)
        .map_err(|e| e.to_string())
}

/// Export the options as GeoJSON (`kind` = `geojson`, WGS84) or the quantity table as CSV (`csv`),
/// through the session, to a file the user picks. Returns the path, or `null` on cancel. The text is
/// the same as `owd export` gives for the saved file.
#[tauri::command]
async fn export_file(
    app: AppHandle,
    state: State<'_, Shared>,
    kind: String,
) -> Result<Option<String>, String> {
    let (text, ext, label) = {
        let guard = lock(&state)?;
        match kind.as_str() {
            "geojson" => (guard.export_geojson()?, "geojson", "GeoJSON"),
            "csv" => (guard.export_csv(), "csv", "CSV"),
            other => return Err(format!("`{other}` is not an export: use geojson or csv")),
        }
    };
    let picked = match test_answer("export.txt") {
        Some(answer) => answer.map(PathBuf::from),
        None => app
            .dialog()
            .file()
            .add_filter(label, &[ext])
            .set_file_name(format!("waterway-design.{ext}"))
            .blocking_save_file()
            .map(|p| p.into_path().map_err(|e| e.to_string()))
            .transpose()?,
    };
    let Some(mut path) = picked else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension(ext);
    }
    write_file(&path, &text)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// The drawing sheets of an option (spec 0004, ticket 8): `[{number, title, svg, warnings}]`, made by
/// the same builder as `owd draw`, off the main thread.
#[tauri::command]
async fn sheets(state: State<'_, Shared>, option: String) -> Reply {
    let job = lock(&state)?.sheets_job(&option);
    tauri::async_runtime::spawn_blocking(move || job.pages())
        .await
        .map_err(|e| e.to_string())?
}

/// Write each sheet's PDF of an option into a folder the user picks, named as `owd draw` names them.
/// Returns the folder, or `null` on cancel. The PDFs have the bytes `owd draw` writes.
#[tauri::command]
async fn export_sheets(
    app: AppHandle,
    state: State<'_, Shared>,
    option: String,
) -> Result<Option<String>, String> {
    let job = lock(&state)?.sheets_job(&option);
    let picked = match test_answer("sheets.txt") {
        Some(answer) => answer.map(PathBuf::from),
        None => app
            .dialog()
            .file()
            .blocking_pick_folder()
            .map(|p| p.into_path().map_err(|e| e.to_string()))
            .transpose()?,
    };
    let Some(folder) = picked else {
        return Ok(None);
    };
    let pdfs = tauri::async_runtime::spawn_blocking(move || job.pdfs())
        .await
        .map_err(|e| e.to_string())??;
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    for (name, bytes) in &pdfs {
        write_bytes(&folder.join(name), bytes)?;
    }
    Ok(Some(folder.to_string_lossy().into_owned()))
}

/// Write the reports of the design (spec 0014, ticket 6) into a folder the user picks, named as
/// `owd report` names them. `ask` is `{kind, format, revision?, status?, prepared?, date}`. Returns
/// the folder, or `null` on cancel. The files have the bytes `owd report` writes.
#[tauri::command]
async fn export_report(
    app: AppHandle,
    state: State<'_, Shared>,
    option: Option<String>,
    ask: serde_json::Value,
) -> Result<Option<String>, String> {
    let ask = owd_shell::report::ReportAsk::from_json(
        &ask,
        &format!(
            "Open Waterway Designer {} (desktop)",
            env!("CARGO_PKG_VERSION")
        ),
    )?;
    let job = lock(&state)?.report_job(option.as_deref(), ask);
    let picked = match test_answer("report.txt") {
        Some(answer) => answer.map(PathBuf::from),
        None => app
            .dialog()
            .file()
            .blocking_pick_folder()
            .map(|p| p.into_path().map_err(|e| e.to_string()))
            .transpose()?,
    };
    let Some(folder) = picked else {
        return Ok(None);
    };
    let files = tauri::async_runtime::spawn_blocking(move || job.files())
        .await
        .map_err(|e| e.to_string())??;
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    for (name, bytes) in &files {
        write_bytes(&folder.join(name), bytes)?;
    }
    Ok(Some(folder.to_string_lossy().into_owned()))
}

/// The QA overlay of the study's banks (spec 0013): a PNG and a KMZ in a folder the user picks.
/// `from_km` and `to_km` cut the stretch along the centreline. The scenes are read over the network.
#[tauri::command]
async fn export_qa_overlay(
    app: AppHandle,
    state: State<'_, Shared>,
    from_km: Option<f64>,
    to_km: Option<f64>,
) -> Result<Option<String>, String> {
    let job = lock(&state)?.qa_job()?;
    let picked = match test_answer("qa.txt") {
        Some(answer) => answer.map(PathBuf::from),
        None => app
            .dialog()
            .file()
            .blocking_pick_folder()
            .map(|p| p.into_path().map_err(|e| e.to_string()))
            .transpose()?,
    };
    let Some(folder) = picked else {
        return Ok(None);
    };
    let at = reach_endpoints();
    let files = tauri::async_runtime::spawn_blocking(move || {
        job.files(&owd_cli::reach_fetch::HttpFetch, &at, from_km, to_km)
    })
    .await
    .map_err(|e| e.to_string())??;
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    for (name, bytes) in &files {
        write_bytes(&folder.join(name), bytes)?;
    }
    Ok(Some(folder.to_string_lossy().into_owned()))
}

/// The option comparison table and the checks list: failed options first, numbers kept.
#[tauri::command]
fn option_table(state: State<Shared>) -> Reply {
    Ok(lock(&state)?.option_table())
}

/// The plan map's geometry in WGS84 (banks, centreline, options with tight bends and ticks).
#[tauri::command]
fn map_geometry(state: State<Shared>) -> Reply {
    lock(&state)?.map_geometry()
}

/// The offline map fallback: a hillshade of the DEM as a PNG data URL with the four corners it goes
/// on, or `null` when the design has no DEM that can be read. Drawn on a worker thread, with the
/// state unlocked while it is.
#[tauri::command]
async fn hillshade(state: State<'_, Shared>) -> Reply {
    let dem = lock(&state)?.dem();
    let Some(dem) = dem else {
        return Ok(serde_json::Value::Null);
    };
    let shade = owd_format::design::hillshade::hillshade(&dem).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&shade.png)),
        "corners": shade.corners,
        "width": shade.width,
        "height": shade.height,
    }))
}

/// Smoke-test hook, debug builds only: `CD_TEST_TILE_URL` replaces the basemap tile address, so a
/// test can make every tile request fail. Release builds always answer `null` (the real basemap).
#[tauri::command]
fn basemap_tiles() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("CD_TEST_TILE_URL")
            .ok()
            .filter(|u| !u.is_empty())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Read a vector file as GeoJSON text. A GeoPackage is converted; `file.gpkg#layer` names a layer.
fn read_vector(spec: &str) -> Result<String, String> {
    let (path, layer) = if Path::new(spec).exists() {
        (spec, None)
    } else {
        match spec.rsplit_once('#') {
            Some((p, l)) if !l.is_empty() => (p, Some(l)),
            _ => (spec, None),
        }
    };
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    input::vector_text(path, bytes, layer)
}

/// Ask for a data file. `None` when the user cancels. A test answers from `load.txt`.
fn pick_data(app: &AppHandle, dem: bool) -> Result<Option<PathBuf>, String> {
    if let Some(answer) = test_answer("load.txt") {
        return Ok(answer.map(PathBuf::from));
    }
    let (name, ext): (&str, &[&str]) = if dem {
        ("GeoTIFF DEM", &["tif", "tiff"])
    } else {
        ("Vector data", &["geojson", "json", "gpkg"])
    };
    match app
        .dialog()
        .file()
        .add_filter(name, ext)
        .blocking_pick_file()
    {
        Some(p) => Ok(Some(p.into_path().map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

/// Load one input: a vector role (`left_bank`, `right_bank`, `centreline`, `river_polygon`) or the
/// `dem`. With no `path` the user is asked for a file; `null` comes back when they cancel. Dropping a
/// file on the window passes its path.
#[tauri::command]
async fn load_input(
    app: AppHandle,
    window: Window,
    state: State<'_, Shared>,
    role: String,
    path: Option<String>,
) -> Reply {
    let file = match path {
        Some(p) => PathBuf::from(p),
        None => match pick_data(&app, role == "dem")? {
            Some(p) => p,
            None => return Ok(serde_json::Value::Null),
        },
    };
    if role == "dem" {
        let bytes =
            std::fs::read(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let mut guard = lock(&state)?;
        guard.load_dem(&file, bytes, &today())?;
        return finish(&window, &guard);
    }
    let role = input::role_of(&role)?;
    let text = read_vector(&file.to_string_lossy())?;
    let mut guard = lock(&state)?;
    guard.load_inputs(&[(role, text)], &today())?;
    finish(&window, &guard)
}

/// Edit a bank (spec 0012): `edit` is JSON, see `owd_shell::input::bank_edit_of`.
#[tauri::command]
fn edit_bank(window: Window, state: State<Shared>, side: String, edit: serde_json::Value) -> Reply {
    let side = input::role_of(&side)?;
    let edit = input::bank_edit_of(&edit)?;
    let mut guard = lock(&state)?;
    guard.edit_bank(side, edit, &today())?;
    finish(&window, &guard)
}

/// Undo the last bank edit.
#[tauri::command]
fn undo_bank_edit(window: Window, state: State<Shared>) -> Reply {
    let mut guard = lock(&state)?;
    guard.undo_bank_edit()?;
    finish(&window, &guard)
}

/// Set the water level: at the river's start, and optionally a fall per metre of river.
#[tauri::command]
fn set_water(window: Window, state: State<Shared>, level: String, fall: String) -> Reply {
    let num = |t: &str, what: &str| -> Result<Option<f64>, String> {
        let t = t.trim();
        if t.is_empty() {
            return Ok(None);
        }
        t.parse::<f64>()
            .map(Some)
            .map_err(|_| format!("the {what} must be a number, not `{t}`"))
    };
    let level = num(&level, "water level")?.ok_or("give a water level in metres")?;
    let fall = num(&fall, "fall per metre")?;
    let mut guard = lock(&state)?;
    guard.set_water(level, fall)?;
    finish(&window, &guard)
}

/// Run one stage on a worker thread. `method` picks the alignment method (`rolling_circle` or
/// `reroute`). The design changes only if the stage finishes: a cancel or a failure leaves it as it
/// was. The reply is the snapshot, with `run` saying how it ended.
#[tauri::command]
async fn run_stage(
    app: AppHandle,
    window: Window,
    state: State<'_, Shared>,
    stage: String,
    method: Option<String>,
) -> Reply {
    let (job, _control) = lock(&state)?.begin_run(&stage, method.as_deref())?;
    let ticker = app.clone();
    let last = std::sync::Mutex::new(std::time::Instant::now());
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        job.execute(&|n| {
            // At most four progress events a second.
            if let Ok(mut t) = last.lock()
                && t.elapsed() >= std::time::Duration::from_millis(250)
            {
                *t = std::time::Instant::now();
                let _ = ticker.emit("stage-progress", n);
            }
        })
    })
    .await
    .map_err(|e| e.to_string());
    let mut guard = lock(&state)?;
    let ended = match outcome {
        Ok(o) => guard.finish_run(o),
        Err(e) => guard.finish_run(run::Outcome::Failed(e)),
    }?;
    let mut snap = finish(&window, &guard)?;
    snap["run"] = serde_json::json!({"stage": stage, "outcome": ended});
    Ok(snap)
}

/// Ask the running stage to stop.
#[tauri::command]
fn cancel_run(state: State<Shared>) -> Result<bool, String> {
    Ok(lock(&state)?.cancel_run())
}

/// Fetch a Copernicus DEM for the reach with `owd fetch-dem`, beside the saved design, then
/// reload the design from the file. The study is saved first: the tiles go next to it.
#[tauri::command]
async fn fetch_dem(app: AppHandle, window: Window, state: State<'_, Shared>) -> Reply {
    // The study is saved first, never asked about (#322): in its folder in the projects folder if
    // it has no file yet.
    let path = {
        let mut guard = lock(&state)?;
        guard.refuse_while_running()?;
        if guard.path().is_none() {
            autosave_new(&mut guard, None)?;
        } else if guard.is_dirty() {
            autosave(&mut guard)?;
        }
        guard
            .path()
            .map(Path::to_path_buf)
            .ok_or("the study could not be saved")?
    };
    let doc = path.to_string_lossy().into_owned();
    let mirror = test_answer("dem-mirror.txt").flatten();
    let turn = REACH_TURN.load(std::sync::atomic::Ordering::SeqCst);
    let ticker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let report = progress_events(ticker);
        let stop = || REACH_TURN.load(std::sync::atomic::Ordering::SeqCst) != turn;
        owd_cli::commands::fetch_dem_watched(
            &doc,
            1000.0,
            mirror.as_deref(),
            None,
            true,
            &owd_cli::fetch::Mirror,
            &report,
            &stop,
        )
        .map_err(|f| f.message)
    })
    .await
    .map_err(|e| e.to_string())??;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let dem = AppState::dem_path(&text)
        .and_then(|rel| std::fs::read(path.parent().unwrap_or(Path::new(".")).join(rel)).ok());
    let opened = AppState::open(&path, &text, dem.as_deref())?;
    let mut guard = lock(&state)?;
    // The same study, read again: the panel keeps its route and steps (#318).
    let next = guard.reopened(opened);
    *guard = next;
    finish(&window, &guard)
}

/// Rerun every stale stage.
#[tauri::command]
fn run_stale(window: Window, state: State<Shared>) -> Reply {
    let mut guard = lock(&state)?;
    guard.run_stale()?;
    finish(&window, &guard)
}

// --- Start from a river (spec 0012, T10) -------------------------------------------------------

/// Where the reach sources are. A debug build reads `reach-endpoints.json` from the test folder,
/// so the smoke test can point them at recorded answers. `OWD_PHOTON_URL` and `OWD_NOMINATIM_URL`
/// name another river search server (for example a Photon of your own); `OWD_OVERPASS_URLS` the
/// Overpass servers, separated by commas, asked in that order (#319).
fn reach_endpoints() -> owd_shell::reach::ReachEndpoints {
    let mut at = test_answer("reach-endpoints.json")
        .flatten()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| owd_shell::reach::ReachEndpoints::from_json(&v))
        .unwrap_or_default();
    if let Ok(u) = std::env::var("OWD_PHOTON_URL") {
        at.photon = u;
    }
    if let Ok(u) = std::env::var("OWD_NOMINATIM_URL") {
        at.nominatim = u;
    }
    if let Ok(u) = std::env::var("OWD_OVERPASS_URLS") {
        at.overpass = u;
    }
    at
}

// --- The projects folder (#322) ---------------------------------------------------------------

/// Where studies are saved: `OWD_PROJECTS_DIR`, else `~/Documents/Open Waterway Designer`. A debug
/// build reads `projects.txt` from the test folder.
fn projects_dir() -> Result<PathBuf, String> {
    if let Some(Some(p)) = test_answer("projects.txt") {
        return Ok(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("OWD_PROJECTS_DIR") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("there is no home folder for the projects folder: set OWD_PROJECTS_DIR")?;
    Ok(PathBuf::from(home).join(owd_shell::projects::DEFAULT_PROJECTS))
}

/// The file of the study about to be replaced, when the new one may take its folder: a study saved
/// in the projects folder that holds nothing but its reach, alone in its folder. A reach cut again
/// out of the same river keeps its folder and name.
fn handed_over(guard: &AppState) -> Option<PathBuf> {
    let file = guard.path()?;
    let dir = file.parent()?;
    if dir.parent()? != projects_dir().ok()? || !guard.reach_route_only() || guard.is_dirty() {
        return None;
    }
    (std::fs::read_dir(dir).ok()?.count() == 1).then(|| file.to_path_buf())
}

/// Write the study where it is saved, if it is saved.
fn autosave(guard: &mut AppState) -> Result<(), String> {
    let Some(path) = guard.path().map(Path::to_path_buf) else {
        return Ok(());
    };
    let text = guard.save_text_for(&path);
    write_file(&path, &text)?;
    guard.saved(&path);
    Ok(())
}

/// Save a new study at once, in its own folder in the projects folder, named for the minute it
/// started (local time) and its river: `2026-10-22 1435 Rhine`, with a suffix when the name is
/// taken (#322). A study of the same river that held only its reach hands its folder over.
fn autosave_new(guard: &mut AppState, previous: Option<PathBuf>) -> Result<(), String> {
    let river = guard.study_river();
    let projects = projects_dir()?;
    std::fs::create_dir_all(&projects)
        .map_err(|e| format!("cannot create {}: {e}", projects.display()))?;
    let same_river = |dir: &Path| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .is_some_and(|n| {
                let n = n.split(" (").next().unwrap_or(&n).to_string();
                n.ends_with(&format!(" {}", owd_shell::projects::safe_name(&river)))
            })
    };
    let reused = previous.as_deref().and_then(|old| {
        let dir = old.parent()?;
        same_river(dir).then(|| {
            let _ = std::fs::remove_file(old);
            dir.to_path_buf()
        })
    });
    let folder = match reused {
        Some(dir) => dir,
        None => {
            let started = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
            let name = owd_shell::projects::study_folder_name(&started, &river);
            let mut folder = projects.join(&name);
            let mut n = 2;
            while folder.exists() {
                folder = projects.join(format!("{name} ({n})"));
                n += 1;
            }
            folder
        }
    };
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    let stem = folder
        .file_name()
        .map_or_else(|| "study".to_string(), |n| n.to_string_lossy().into_owned());
    let path = folder.join(format!("{stem}.{SAVE_EXTENSION}"));
    let text = guard.save_text_for(&path);
    write_file(&path, &text)?;
    guard.saved(&path);
    Ok(())
}

/// Bumped by Cancel: a reach step that began before it is not kept.
static REACH_TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Progress for the window as `reach-progress` events (#320): every counted step, and at most four
/// other reports a second.
fn progress_events(app: AppHandle) -> impl Fn(&owd_shell::reach::Step) {
    let last = std::cell::Cell::new(None::<std::time::Instant>);
    move |s: &owd_shell::reach::Step| {
        let now = std::time::Instant::now();
        if s.total > 0
            || last
                .get()
                .is_none_or(|t| now - t >= std::time::Duration::from_millis(250))
        {
            last.set(Some(now));
            let _ = app.emit(
                "reach-progress",
                serde_json::json!({"done": s.done, "total": s.total, "what": s.line()}),
            );
        }
    }
}

/// Network work for a reach, off the main thread (#320): it reports its progress to the window, and
/// Cancel stops it before its next request. Work Cancel came during is dropped.
async fn watched<T, F>(app: &AppHandle, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&dyn owd_shell::reach::Fetch) -> Result<T, String> + Send + 'static,
{
    use std::sync::atomic::Ordering;
    let turn = REACH_TURN.load(Ordering::SeqCst);
    let ticker = app.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let report = progress_events(ticker);
        let stop = || REACH_TURN.load(Ordering::SeqCst) != turn;
        work(&owd_shell::reach::Reporting::new(
            &owd_cli::reach_fetch::HttpFetch,
            &report,
            &stop,
        ))
    })
    .await
    .map_err(|e| e.to_string())?;
    if REACH_TURN.load(Ordering::SeqCst) != turn {
        return Err("cancelled".into());
    }
    out
}

/// Plan the reach and start a new study from it. Asks first when the design has unsaved changes.
#[tauri::command]
async fn reach_route(
    app: AppHandle,
    window: Window,
    state: State<'_, Shared>,
    river: Option<String>,
    start: [f64; 2],
    end: [f64; 2],
    via: Vec<[f64; 2]>,
) -> Reply {
    if !confirm_discard(&app, &state)? {
        return Ok(serde_json::Value::Null);
    }
    let at = reach_endpoints();
    let plan = watched(&app, move |f| {
        owd_shell::reach::plan_reach(f, &at, river.as_deref(), start, end, &via)
    })
    .await?;
    let mut guard = lock(&state)?;
    let previous = handed_over(&guard);
    guard.start_reach(plan, &today())?;
    autosave_new(&mut guard, previous)?;
    finish(&window, &guard)
}

/// The rivers and canals whose name starts with or matches a text (spec 0012, T13).
#[tauri::command]
async fn reach_search(text: String, country: Option<String>, once: Option<bool>) -> Reply {
    let at = reach_endpoints();
    tauri::async_runtime::spawn_blocking(move || {
        owd_shell::reach::search_rivers(
            &owd_cli::reach_fetch::HttpFetch,
            &at,
            &text,
            country.as_deref(),
            once.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Start a new study from a whole river chosen from the search: Start at its mouth, End at its
/// upstream end. Asks first when the design has unsaved changes.
#[tauri::command]
async fn reach_river(
    app: AppHandle,
    window: Window,
    state: State<'_, Shared>,
    osm_id: String,
    also: Vec<String>,
    search: Option<String>,
) -> Reply {
    if !confirm_discard(&app, &state)? {
        return Ok(serde_json::Value::Null);
    }
    let at = reach_endpoints();
    let river = watched(&app, move |f| {
        owd_shell::reach::read_river(f, &at, &osm_id, &also)
    })
    .await?;
    let mut guard = lock(&state)?;
    let previous = handed_over(&guard);
    guard.start_river(river, search.as_deref(), &today())?;
    autosave_new(&mut guard, previous)?;
    finish(&window, &guard)
}

/// Cut a reach out of the chosen river (Start or End dragged or picked). Asks first only when the
/// study holds more than its reach.
#[tauri::command]
fn reach_cut(
    app: AppHandle,
    window: Window,
    state: State<Shared>,
    start: [f64; 2],
    end: [f64; 2],
    via: Vec<[f64; 2]>,
) -> Reply {
    if !lock(&state)?.reach_route_only() && !confirm_discard(&app, &state)? {
        return Ok(serde_json::Value::Null);
    }
    let mut guard = lock(&state)?;
    let previous = handed_over(&guard);
    guard.reach_cut(start, end, &via, &today())?;
    autosave_new(&mut guard, previous)?;
    finish(&window, &guard)
}

/// Draw the reach of an opened study again: its route is planned once more from OSM (#318). The
/// document is not changed.
#[tauri::command]
async fn reach_restore(app: AppHandle, window: Window, state: State<'_, Shared>) -> Reply {
    let doc = {
        let guard = lock(&state)?;
        if !guard.reach_needs_restore() {
            return finish(&window, &guard);
        }
        guard.snapshot()["document"].clone()
    };
    let at = reach_endpoints();
    let restored = watched(&app, move |f| owd_shell::reach::plan_restore(f, &at, &doc)).await?;
    let mut guard = lock(&state)?;
    guard.restore_reach(restored)?;
    finish(&window, &guard)
}

/// Give a JRC occurrence tile the user downloaded.
#[tauri::command]
fn reach_add_tile(app: AppHandle, window: Window, state: State<Shared>) -> Reply {
    let file = match test_answer("jrc.txt") {
        Some(answer) => answer.map(PathBuf::from),
        None => app
            .dialog()
            .file()
            .add_filter("JRC occurrence tile", &["tif"])
            .blocking_pick_file()
            .map(|p| p.into_path().map_err(|e| e.to_string()))
            .transpose()?,
    };
    let Some(file) = file else {
        return Ok(serde_json::Value::Null);
    };
    let bytes = std::fs::read(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let name = file
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut guard = lock(&state)?;
    guard.reach_add_tile(&name, bytes)?;
    finish(&window, &guard)
}

/// Download the JRC tiles the reach needs into `<study>/jrc/` (#321), with progress and Cancel, and
/// the source's notice beside them. `None` for a study with no folder. A debug build reads the
/// tiles from the folder or URL in `jrc-mirror.txt`.
async fn fetch_jrc_tiles(
    app: &AppHandle,
    state: &State<'_, Shared>,
) -> Result<Option<PathBuf>, String> {
    use std::sync::atomic::Ordering;
    let (dir, missing) = {
        let guard = lock(state)?;
        let Some(dir) = guard.path().and_then(Path::parent).map(|d| d.join("jrc")) else {
            return Ok(None);
        };
        (dir, guard.reach_jrc_missing())
    };
    let mirror = test_answer("jrc-mirror.txt").flatten();
    let turn = REACH_TURN.load(Ordering::SeqCst);
    let ticker = app.clone();
    let into = dir.clone();
    let names = tauri::async_runtime::spawn_blocking(move || {
        let report = progress_events(ticker);
        let stop = || REACH_TURN.load(Ordering::SeqCst) != turn;
        let n = missing.len();
        let mut got = Vec::new();
        for (i, (name, url)) in missing.iter().enumerate() {
            let dest = into.join(name);
            if !dest.is_file() {
                let what = format!("downloading JRC tile {} of {n} ({name}):", i + 1);
                match mirror.as_deref() {
                    Some(m) if !m.contains("://") => {
                        std::fs::create_dir_all(&into).map_err(|e| e.to_string())?;
                        std::fs::copy(Path::new(m).join(name), &dest).map_err(|e| e.to_string())?;
                    }
                    Some(m) => {
                        let from = format!("{}/{name}", m.trim_end_matches('/'));
                        owd_cli::reach_fetch::download(&from, &dest, &what, &report, &stop)?;
                    }
                    None => {
                        owd_cli::reach_fetch::download(url, &dest, &what, &report, &stop)?;
                    }
                }
            }
            got.push(name.clone());
        }
        if !got.is_empty() {
            write_file(&into.join("NOTICE.txt"), &owd_shell::reach::jrc_notice())?;
        }
        Ok::<_, String>(got)
    })
    .await
    .map_err(|e| e.to_string())??;
    if REACH_TURN.load(Ordering::SeqCst) != turn {
        return Err("cancelled".into());
    }
    lock(state)?.reach_mark_local(&names);
    Ok(Some(dir))
}

/// Run one step of the reach (`banks`, `edge_fit` or `level`) off the main thread, and keep its
/// result unless Cancel came first. Before the banks, the JRC tiles are downloaded into the study's
/// folder; every step reads them from there.
#[tauri::command]
async fn reach_step(
    app: AppHandle,
    window: Window,
    state: State<'_, Shared>,
    step: String,
    percentile: Option<f64>,
) -> Reply {
    let step = owd_shell::reach::ReachStep::of(&step, percentile)?;
    let local = if step == owd_shell::reach::ReachStep::Banks {
        fetch_jrc_tiles(&app, &state).await?
    } else {
        lock(&state)?
            .path()
            .and_then(Path::parent)
            .map(|d| d.join("jrc"))
    };
    let job = lock(&state)?.begin_reach(step)?;
    let at = reach_endpoints();
    let date = today();
    let job = watched(&app, move |f| match local {
        Some(dir) => job.run(
            &owd_cli::reach_fetch::LocalFirst { dir, inner: f },
            &at,
            &date,
        ),
        None => job.run(f, &at, &date),
    })
    .await?;
    let mut guard = lock(&state)?;
    guard.keep_reach(job)?;
    autosave(&mut guard)?;
    finish(&window, &guard)
}

/// Cancel the network work that is running (a route, a reach step, a DEM fetch): it stops before
/// its next request, and its result is dropped.
#[tauri::command]
fn reach_cancel() {
    REACH_TURN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Give the depth of a river with no survey: `points` are `[chainage, depth]`.
#[tauri::command]
fn set_bed(
    window: Window,
    state: State<Shared>,
    points: Vec<[f64; 2]>,
    season: Option<String>,
) -> Reply {
    let mut guard = lock(&state)?;
    guard.set_bed(&points, season.as_deref(), &today())?;
    finish(&window, &guard)
}

/// Answer "what fits" over every vessel class.
#[tauri::command]
fn run_fits(window: Window, state: State<Shared>) -> Reply {
    let mut guard = lock(&state)?;
    guard.run_fits()?;
    finish(&window, &guard)
}

pub fn run() {
    let initial = AppState::new_design("Untitled channel design", &today());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(initial))
        .invoke_handler(tauri::generate_handler![
            snapshot,
            sizing_choices,
            new_file,
            open_file,
            save,
            save_as,
            set_rule_sets,
            set_vessel,
            set_custom_vessel,
            check_custom_vessel,
            stretches_geojson,
            run_stale,
            map_geometry,
            option_table,
            export_file,
            sheets,
            export_qa_overlay,
            export_sheets,
            export_report,
            long_section,
            cross_section,
            station_near,
            hillshade,
            load_input,
            set_water,
            edit_bank,
            undo_bank_edit,
            run_stage,
            cancel_run,
            fetch_dem,
            basemap_tiles,
            reach_route,
            reach_search,
            reach_river,
            reach_cut,
            reach_restore,
            reach_add_tile,
            reach_step,
            reach_cancel,
            set_bed,
            run_fits,
        ])
        .on_window_event(|window, event| {
            // The unsaved-changes guard on close: ask first, then close for real.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let dirty = app
                    .state::<Shared>()
                    .lock()
                    .map(|s| s.is_dirty())
                    .unwrap_or(false);
                if dirty {
                    api.prevent_close();
                    let window = window.clone();
                    std::thread::spawn(move || {
                        let discard = ask_discard(
                            &app,
                            "The channel design has unsaved changes. Discard them and close?",
                        );
                        if discard {
                            let _ = window.destroy();
                        }
                    });
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running the Open Waterway Designer app");
}

#[cfg(test)]
mod tests {
    use super::{OPEN_EXTENSIONS, SAVE_EXTENSION};

    /// Spec 0006: the app is Open Waterway Designer, with its own bundle identifier. The old
    /// identifier would give the app a second data folder; the leftover-names check
    /// (`tools/old_names.py`) keeps it, and the old name, out of every file.
    #[test]
    fn the_bundle_config_holds_the_new_name_and_identifier() {
        let text = include_str!("../tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(conf["productName"], "Open Waterway Designer");
        assert_eq!(
            conf["identifier"],
            "org.africanrivercorridors.openwaterwaydesigner"
        );
        assert_eq!(conf["app"]["windows"][0]["title"], "Open Waterway Designer");
    }

    #[test]
    fn the_dialogs_open_and_save_owd_only() {
        assert_eq!(OPEN_EXTENSIONS, ["owd"]);
        assert_eq!(SAVE_EXTENSION, "owd");
    }
}
