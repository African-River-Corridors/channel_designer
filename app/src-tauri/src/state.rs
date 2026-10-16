//! What the shell holds besides the design session: the file path, the unsaved flag and the
//! warnings from the last open.
//!
//! This module has no Tauri types, so it is tested like any other Rust. It holds no calculation,
//! validation or default logic: every answer comes from [`Session`].

use cd_format::design::{DesignError, Session, Stage, StageStatus};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct AppState {
    session: Session,
    path: Option<PathBuf>,
    dirty: bool,
    /// Warnings from the last open: for example a DEM that is missing or has changed.
    warnings: Vec<String>,
}

fn message(e: DesignError) -> String {
    e.to_string()
}

impl AppState {
    /// A new channel design. `created` is today's date, `YYYY-MM-DD`, read by the shell.
    pub fn new_design(title: &str, created: &str) -> AppState {
        AppState {
            session: Session::new(title, created),
            path: None,
            dirty: false,
            warnings: Vec::new(),
        }
    }

    /// Open `.cdesign` text that was read from `path`. `dem_bytes` is the DEM file the document
    /// names, if the shell found it next to the document.
    pub fn open(path: &Path, text: &str, dem_bytes: Option<&[u8]>) -> Result<AppState, String> {
        let session = Session::open(text).map_err(message)?;
        let warnings = session.dem_warning(dem_bytes).into_iter().collect();
        Ok(AppState {
            session,
            path: Some(path.to_path_buf()),
            dirty: false,
            warnings,
        })
    }

    /// The DEM path the document names, relative to the document, if any.
    pub fn dem_path(text: &str) -> Option<String> {
        let doc: Value = serde_json::from_str(text).ok()?;
        doc["inputs"]["dem"]["path"].as_str().map(str::to_string)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The draught as typed: empty is the vessel class's own; anything else must be a number. Whether
    /// the number is acceptable is the session's to say.
    pub fn parse_draught(text: &str) -> Result<Option<f64>, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        t.parse::<f64>()
            .map(Some)
            .map_err(|_| format!("the draught must be a number of metres, not `{t}`"))
    }

    /// Choose a ruleset, a vessel class and a draught, and size the channel at once. The
    /// stages after sizing that had results show as stale; nothing else runs. A choice the
    /// session refuses leaves the design as it was.
    pub fn set_sizing(
        &mut self,
        ruleset_id: &str,
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<(), String> {
        let before = self.session.clone();
        let outcome = self
            .session
            .set_sizing(ruleset_id, vessel_standard_id, draught_m)
            .and_then(|()| self.session.run_stage(Stage::Sizing));
        match outcome {
            Ok(()) => {
                self.dirty = true;
                Ok(())
            }
            Err(e) => {
                self.session = before;
                Err(message(e))
            }
        }
    }

    /// Rerun every stale stage, in order. Returns the names of the stages that ran.
    pub fn run_stale(&mut self) -> Result<Vec<&'static str>, String> {
        let before = self.session.clone();
        match self.session.run_stale() {
            Ok(ran) => {
                if !ran.is_empty() {
                    self.dirty = true;
                }
                Ok(ran.into_iter().map(Stage::name).collect())
            }
            Err(e) => {
                self.session = before;
                Err(message(e))
            }
        }
    }

    /// The plan map's geometry in WGS84, from the session.
    pub fn map_geometry(&self) -> Result<Value, String> {
        self.session.map_geometry().map_err(message)
    }

    /// The `.cdesign` text to write.
    pub fn save_text(&self) -> String {
        self.session.to_text()
    }

    /// Record that the design was written to `path`.
    pub fn saved(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.warnings.clear();
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    /// The window title: unsaved marker, file name, the study's title.
    pub fn title(&self) -> String {
        let marker = if self.dirty { "\u{25cf} " } else { "" };
        format!(
            "{marker}{} \u{2014} {} \u{2014} Channel Designer",
            self.file_name(),
            self.session.document()["project"]["title"]
                .as_str()
                .unwrap_or("")
        )
    }

    /// What the front end shows: the document as the session holds it, and each stage's status.
    pub fn snapshot(&self) -> Value {
        let status: Vec<Value> = self
            .session
            .status()
            .stages
            .iter()
            .map(|(stage, st)| {
                json!({
                    "name": stage.name(),
                    "status": match st {
                        StageStatus::NotRun => "not_run",
                        StageStatus::Fresh => "fresh",
                        StageStatus::Stale => "stale",
                    },
                })
            })
            .collect();
        json!({
            "engine_version": cd_format::design::session::ENGINE_VERSION,
            "document": self.session.document(),
            "stages": status,
            "file_name": self.file_name(),
            "path": self.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "dirty": self.dirty,
            "title": self.title(),
            "warnings": self.warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn worked_example() -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.cdesign"),
        )
        .expect("examples/synthetic-river.cdesign")
    }

    fn stage_status(state: &AppState, name: &str) -> String {
        state.snapshot()["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()["status"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_typed_draught_is_empty_or_a_number() {
        assert_eq!(AppState::parse_draught("  "), Ok(None));
        assert_eq!(AppState::parse_draught("2.5"), Ok(Some(2.5)));
        assert!(
            AppState::parse_draught("abc")
                .unwrap_err()
                .contains("`abc`")
        );
        // A number the session refuses is refused by the session, not here.
        assert_eq!(AppState::parse_draught("-1"), Ok(Some(-1.0)));
    }

    #[test]
    fn a_new_design_is_the_session_s_empty_design_and_is_not_unsaved() {
        let s = AppState::new_design("Study", "2026-10-16");
        assert_eq!(
            s.snapshot()["document"],
            *Session::new("Study", "2026-10-16").document()
        );
        assert_eq!(s.file_name(), "Untitled");
        assert!(!s.is_dirty());
    }

    #[test]
    fn the_title_shows_the_file_the_study_and_the_unsaved_marker() {
        let mut s = AppState::open(Path::new("/x/river.cdesign"), &worked_example(), None).unwrap();
        assert_eq!(
            s.title(),
            "river.cdesign \u{2014} Synthetic river \u{2014} Channel Designer"
        );
        s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
        assert!(s.title().starts_with("\u{25cf} river.cdesign"));
        s.saved(Path::new("/x/other.cdesign"));
        assert_eq!(
            s.title(),
            "other.cdesign \u{2014} Synthetic river \u{2014} Channel Designer"
        );
    }

    #[test]
    fn changing_the_vessel_sizes_at_once_and_marks_the_design_unsaved() {
        let mut s = AppState::open(Path::new("a.cdesign"), &worked_example(), None).unwrap();
        let before = s.snapshot()["document"]["stages"]["sizing"]["results"].clone();
        s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
        let after = s.snapshot()["document"]["stages"]["sizing"]["results"].clone();
        assert_ne!(before["min_bend_radius_m"], after["min_bend_radius_m"]);
        assert_eq!(after["vessel_standard_id"], "CEMT_Va");
        assert_eq!(stage_status(&s, "sizing"), "fresh");
        assert!(s.is_dirty());
    }

    #[test]
    fn a_refused_choice_changes_nothing_and_leaves_the_design_saved() {
        let mut s = AppState::open(Path::new("a.cdesign"), &worked_example(), None).unwrap();
        let before = s.snapshot();
        let e = s.set_sizing("PIANC-B", "NOPE", None).unwrap_err();
        assert!(e.contains("unknown vessel standard"), "{e}");
        assert!(s.set_sizing("PIANC-B", "CEMT_IV", Some(-1.0)).is_err());
        assert_eq!(s.snapshot(), before);
        assert!(!s.is_dirty());
    }

    #[test]
    fn a_saved_file_reopens_unchanged_and_runs_to_the_same_results() {
        let mut s = AppState::open(Path::new("a.cdesign"), &worked_example(), None).unwrap();
        s.set_sizing("PIANC-B", "CEMT_Va", Some(2.4)).unwrap();
        let text = s.save_text();
        // `cdesign validate`: the session's own validation, which the command runs.
        assert!(Session::open(&text).unwrap().validate().is_empty());
        // `cdesign run` on the saved file changes nothing: every stage is fresh.
        let mut again = Session::open(&text).unwrap();
        assert_eq!(again.run_stale().unwrap(), vec![]);
        assert_eq!(again.to_text(), text);
        let reopened = AppState::open(Path::new("a.cdesign"), &text, None).unwrap();
        assert_eq!(reopened.save_text(), text);
        assert_eq!(stage_status(&reopened, "sizing"), "fresh");
    }

    #[test]
    fn a_stale_sizing_in_an_opened_file_is_shown_and_run_reruns_it() {
        let text = worked_example().replacen(
            "\"vessel_standard_id\": \"CEMT_IV\",\n        \"draught_m\"",
            "\"vessel_standard_id\": \"CEMT_Va\",\n        \"draught_m\"",
            1,
        );
        let mut s = AppState::open(Path::new("a.cdesign"), &text, None).unwrap();
        assert_eq!(stage_status(&s, "sizing"), "stale");
        assert_eq!(s.run_stale().unwrap(), vec!["sizing"]);
        assert_eq!(stage_status(&s, "sizing"), "fresh");
        assert!(s.is_dirty());
    }

    #[test]
    fn a_file_that_is_not_a_channel_design_is_refused_with_the_reason() {
        assert!(
            AppState::open(Path::new("a"), "{", None)
                .unwrap_err()
                .contains("not valid JSON")
        );
        assert!(
            AppState::open(Path::new("a"), "{}", None)
                .unwrap_err()
                .contains("missing")
        );
    }

    #[test]
    fn a_missing_or_changed_dem_is_a_warning_on_open_and_saving_clears_it() {
        let mut doc: Value = serde_json::from_str(&worked_example()).unwrap();
        doc["inputs"]["dem"] = json!({
            "path": "dem.tif", "sha256": cd_format::design::hash::sha256_hex(b"abc"),
            "crs": "EPSG:4326", "nodata": -32768
        });
        let text = serde_json::to_string_pretty(&doc).unwrap();
        assert_eq!(AppState::dem_path(&text).as_deref(), Some("dem.tif"));
        let ok = AppState::open(Path::new("a"), &text, Some(b"abc")).unwrap();
        assert_eq!(ok.snapshot()["warnings"], json!([]));
        let mut missing = AppState::open(Path::new("a"), &text, None).unwrap();
        assert!(
            missing.snapshot()["warnings"][0]
                .as_str()
                .unwrap()
                .contains("missing")
        );
        let changed = AppState::open(Path::new("a"), &text, Some(b"abd")).unwrap();
        assert!(
            changed.snapshot()["warnings"][0]
                .as_str()
                .unwrap()
                .contains("changed")
        );
        missing.saved(Path::new("b"));
        assert_eq!(missing.snapshot()["warnings"], json!([]));
    }
}
