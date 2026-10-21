//! What the app shells hold besides the design session: [`state::AppState`] and the stage runner.
//! No Tauri types. Shared by the desktop app and the web build (spec 0011).

pub mod bank_edit;
pub mod input;
pub mod run;
pub mod state;
