//! The Channel Designer desktop app shell.
//!
//! Thin by design (spec 0003): each command forwards to the design session in `cd-format`. The
//! shell owns the native open and save dialogs, the unsaved-changes guard, the window title and
//! the clock, and nothing else. It holds no calculation, validation or default logic.

mod state;

use base64::Engine;
use cd_format::design::{Session, date_from_unix};
use state::AppState;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State, Window, WindowEvent};
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
fn pick_open(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    if let Some(answer) = test_answer("open.txt") {
        return Ok(answer.map(PathBuf::from));
    }
    match app
        .dialog()
        .file()
        .add_filter("Channel design", &["cdesign"])
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
            .add_filter("Channel design", &["cdesign"])
            .set_file_name("channel-design.cdesign")
            .blocking_save_file()
        {
            Some(p) => Some(p.into_path().map_err(|e| e.to_string())?),
            None => None,
        },
    };
    Ok(picked.map(|mut p| {
        if p.extension().is_none() {
            p.set_extension("cdesign");
        }
        p
    }))
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    // Write beside the target and rename, so a failed write cannot leave half a file.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[tauri::command]
fn snapshot(window: Window, state: State<Shared>) -> Reply {
    let guard = lock(&state)?;
    finish(&window, &guard)
}

/// The rulesets and vessel classes to choose from, for the two lists.
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
    let (known, text) = {
        let guard = lock(state)?;
        (guard.path().map(Path::to_path_buf), guard.save_text())
    };
    let path = match known {
        Some(p) if !ask_for_path => p,
        _ => {
            let Some(p) = pick_save(app)? else {
                return Ok(serde_json::Value::Null);
            };
            p
        }
    };
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

/// Choose ruleset, vessel class and draught; the channel is sized at once.
#[tauri::command]
fn set_sizing(
    window: Window,
    state: State<Shared>,
    ruleset_id: String,
    vessel_standard_id: String,
    draught: String,
) -> Reply {
    let mut guard = lock(&state)?;
    guard.set_sizing(
        &ruleset_id,
        &vessel_standard_id,
        AppState::parse_draught(&draught)?,
    )?;
    finish(&window, &guard)
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
    let shade = cd_format::design::hillshade::hillshade(&dem).map_err(|e| e.to_string())?;
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

/// Rerun every stale stage.
#[tauri::command]
fn run_stale(window: Window, state: State<Shared>) -> Reply {
    let mut guard = lock(&state)?;
    guard.run_stale()?;
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
            set_sizing,
            run_stale,
            map_geometry,
            hillshade,
            basemap_tiles,
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
        .expect("error while running the Channel Designer app");
}
