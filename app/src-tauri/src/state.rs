//! What the shell holds besides the design session: the file path, the unsaved flag and the
//! warnings from the last open.
//!
//! This module has no Tauri types, so it is tested like any other Rust. It holds no calculation,
//! validation or default logic: every answer comes from [`Session`].

use crate::run::{Outcome, RunControl, RunJob};
use cd_core::volumes::sections::SectionVolumes;
use cd_format::design::{AlignMethod, DemCheck, DesignError, Role, Session, Stage, StageStatus};
use cd_format::geotiff::{Dem, read_bytes};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Debug)]
pub struct AppState {
    session: Session,
    path: Option<PathBuf>,
    dirty: bool,
    /// Warnings from the last open: for example a DEM that is missing or has changed.
    warnings: Vec<String>,
    /// The DEM the document names, when the shell found it next to the document and it reads.
    dem: Option<Arc<Dem>>,
    /// The DEM file's bytes, for the hash check, and where it was read from.
    dem_bytes: Option<Arc<Vec<u8>>>,
    dem_file: Option<PathBuf>,
    /// The stage now running on a worker, if any.
    running: Option<(Stage, Arc<RunControl>)>,
    /// The station walk last drawn, with the key it was made under (option and volumes inputs).
    walk: Option<(String, Arc<SectionVolumes>)>,
}

/// A station walk to fetch or make off the main thread.
pub struct WalkJob {
    session: Session,
    dem: Option<Arc<Dem>>,
    epsg: Option<u32>,
    option: String,
    key: String,
    cached: Option<Arc<SectionVolumes>>,
}

/// A station walk and the design it was made from, to draw from.
#[derive(Debug)]
pub struct Walked {
    pub session: Session,
    pub walk: Arc<SectionVolumes>,
    key: String,
}

impl WalkJob {
    /// The cached walk, or one made now on the DEM. A refusal is the session's reason.
    pub fn run(self) -> Result<Walked, String> {
        let walk = match self.cached {
            Some(w) => w,
            None => {
                let dem = self
                    .dem
                    .ok_or("the sections need the DEM the volumes were run on: it is missing or has changed")?;
                let sampler =
                    cd_format::design::terrain::dem_sampler(&dem, self.epsg).map_err(message)?;
                Arc::new(
                    self.session
                        .option_section_walk(&self.option, &sampler)
                        .map_err(message)?,
                )
            }
        };
        Ok(Walked {
            session: self.session,
            walk,
            key: self.key,
        })
    }
}

/// `target` relative to the folder `base`, using `..` where it must climb. Both are made absolute
/// first, so the document stays portable when it is saved somewhere else.
fn relative_to(base: &Path, target: &Path) -> String {
    let (Ok(b), Ok(t)) = (std::fs::canonicalize(base), std::fs::canonicalize(target)) else {
        return target.to_string_lossy().replace('\\', "/");
    };
    let (bc, tc): (Vec<_>, Vec<_>) = (b.components().collect(), t.components().collect());
    let common = bc.iter().zip(&tc).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); bc.len() - common];
    parts.extend(
        tc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
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
            dem: None,
            dem_bytes: None,
            dem_file: None,
            running: None,
            walk: None,
        }
    }

    /// Open `.cdesign` text that was read from `path`. `dem_bytes` is the DEM file the document
    /// names, if the shell found it next to the document.
    pub fn open(path: &Path, text: &str, dem_bytes: Option<&[u8]>) -> Result<AppState, String> {
        let session = Session::open(text).map_err(message)?;
        let warnings = session.dem_warning(dem_bytes).into_iter().collect();
        let dem_file = session
            .dem_path()
            .map(|rel| path.parent().unwrap_or(Path::new(".")).join(rel));
        Ok(AppState {
            session,
            path: Some(path.to_path_buf()),
            dirty: false,
            warnings,
            dem: dem_bytes.and_then(|b| read_bytes(b).ok()).map(Arc::new),
            dem_bytes: dem_bytes.map(|b| Arc::new(b.to_vec())),
            dem_file,
            running: None,
            walk: None,
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
        self.refuse_while_running()?;
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
        self.refuse_while_running()?;
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

    /// The option comparison table and the checks list, from the session.
    pub fn option_table(&self) -> Value {
        self.session
            .option_table(self.dem_bytes.as_deref().map(|b| b.as_slice()))
    }

    /// The plan map's geometry in WGS84, from the session.
    pub fn map_geometry(&self) -> Result<Value, String> {
        self.session.map_geometry().map_err(message)
    }

    /// The DEM, to draw the offline hillshade from. `None` when there is none to read.
    pub fn dem(&self) -> Option<Arc<Dem>> {
        self.dem.clone()
    }

    fn refuse_while_running(&self) -> Result<(), String> {
        match &self.running {
            Some((stage, _)) => Err(format!(
                "the {} stage is running: wait for it, or press Cancel",
                stage.name()
            )),
            None => Ok(()),
        }
    }

    /// A fetch rewrites the file on disk, so unsaved changes would be lost: refuse them.
    pub fn refuse_unsaved_fetch(&self) -> Result<(), String> {
        self.refuse_while_running()?;
        if self.dirty {
            Err("save the design first: fetching the DEM rewrites the file".into())
        } else {
            Ok(())
        }
    }

    /// Import vector inputs (GeoJSON text, already read by the shell). A refusal changes nothing.
    pub fn load_inputs(&mut self, items: &[(Role, String)]) -> Result<(), String> {
        self.refuse_while_running()?;
        let list: Vec<(Role, &str)> = items.iter().map(|(r, t)| (*r, t.as_str())).collect();
        let before = self.session.clone();
        match self.session.import(&list) {
            Ok(_) => {
                self.dirty = true;
                Ok(())
            }
            Err(e) => {
                self.session = before;
                Err(message(e))
            }
        }
    }

    /// Reference a DEM file and keep it for the stages. A DEM the stages could not sample is
    /// refused, with the CRS named.
    pub fn load_dem(&mut self, file: &Path, bytes: Vec<u8>) -> Result<(), String> {
        self.refuse_while_running()?;
        let dem = read_bytes(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        let epsg = self.project_epsg();
        if epsg.is_some() {
            cd_format::design::terrain::dem_sampler(&dem, epsg)
                .map(|_| ())
                .map_err(message)?;
        }
        let base = self.path.as_deref().and_then(Path::parent);
        let rel = match base {
            Some(b) => relative_to(b, file),
            None => file.to_string_lossy().into_owned(),
        };
        self.session.set_dem(&rel, &bytes, &dem);
        self.dem = Some(Arc::new(dem));
        self.dem_bytes = Some(Arc::new(bytes));
        self.dem_file = Some(file.to_path_buf());
        self.warnings.clear();
        self.dirty = true;
        Ok(())
    }

    fn project_epsg(&self) -> Option<u32> {
        self.session.document()["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32)
    }

    /// Set the water level: a level at the river's start, and optionally a fall per metre of
    /// river. A refusal changes nothing.
    pub fn set_water(&mut self, level_m: f64, fall_per_m: Option<f64>) -> Result<(), String> {
        self.refuse_while_running()?;
        let before = self.session.clone();
        let done = match fall_per_m {
            Some(f) => self.session.set_water_fall(level_m, f),
            None => self.session.set_water_profile(&[[0.0, level_m]]),
        };
        match done {
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

    /// The DEM, only when it is the file whose hash the design recorded.
    fn usable_dem(&self) -> Option<Arc<Dem>> {
        match (
            self.session
                .dem_check(self.dem_bytes.as_deref().map(|b| b.as_slice())),
            &self.dem,
        ) {
            (DemCheck::Matches, Some(d)) => Some(d.clone()),
            _ => None,
        }
    }

    /// Get ready to draw the sections of an option: the cached walk if the design has not changed
    /// since, else a job that makes it.
    pub fn begin_walk(&self, option: &str) -> WalkJob {
        let key = format!(
            "{option}|{}",
            self.session.document()["stages"]["volumes"]["depends_on"]
        );
        // A kept walk holds only while the volumes are fresh on this design: a stale one has no walk to draw.
        let fresh = self.session.stage_status(Stage::Volumes) == StageStatus::Fresh;
        let cached = self
            .walk
            .as_ref()
            .filter(|(k, _)| fresh && *k == key)
            .map(|(_, w)| w.clone());
        WalkJob {
            session: self.session.clone(),
            dem: self.usable_dem(),
            epsg: self.project_epsg(),
            option: option.to_string(),
            key,
            cached,
        }
    }

    /// Keep a walk made off-thread for the next draw.
    pub fn keep_walk(&mut self, walked: &Walked) {
        self.walk = Some((walked.key.clone(), walked.walk.clone()));
    }

    /// Begin running a stage: a job over a copy of the design. The design itself changes only when
    /// [`AppState::finish_run`] gets a finished stage.
    pub fn begin_run(
        &mut self,
        stage: &str,
        method: Option<&str>,
    ) -> Result<(RunJob, Arc<RunControl>), String> {
        self.refuse_while_running()?;
        let stage = Stage::from_name(stage).ok_or_else(|| format!("`{stage}` is not a stage"))?;
        let method = match method {
            Some(m) => Some(
                AlignMethod::from_name(m)
                    .ok_or_else(|| format!("`{m}` is not an alignment method"))?,
            ),
            None => None,
        };
        // The DEM is used only when it is the file whose hash was recorded.
        let dem = match (
            self.session
                .dem_check(self.dem_bytes.as_deref().map(|b| b.as_slice())),
            &self.dem,
        ) {
            (DemCheck::Matches, Some(d)) => Some(d.clone()),
            _ => None,
        };
        let control = Arc::new(RunControl::default());
        self.running = Some((stage, control.clone()));
        let job = RunJob {
            session: self.session.clone(),
            stage,
            method,
            dem,
            project_epsg: self.project_epsg(),
            control: control.clone(),
        };
        Ok((job, control))
    }

    /// Ask the running stage to stop. `false` when nothing runs.
    pub fn cancel_run(&self) -> bool {
        match &self.running {
            Some((_, c)) => {
                c.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Take the end of a run. A finished stage replaces the design; a cancelled or failed one
    /// leaves it as it was. Returns the outcome's name, or the failure's reason.
    pub fn finish_run(&mut self, outcome: Outcome) -> Result<&'static str, String> {
        self.running = None;
        match outcome {
            Outcome::Done(session) => {
                self.session = *session;
                self.dirty = true;
                Ok("done")
            }
            Outcome::Cancelled => Ok("cancelled"),
            Outcome::Failed(reason) => Err(reason),
        }
    }

    /// The `.cdesign` text to write.
    pub fn save_text(&self) -> String {
        self.session.to_text()
    }

    /// Record that the design was written to `path`.
    pub fn saved(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        // The DEM path is stored relative to the document: say it again from the new folder.
        if let (Some(file), Some(bytes), Some(dem)) = (&self.dem_file, &self.dem_bytes, &self.dem) {
            let rel = relative_to(path.parent().unwrap_or(Path::new(".")), file);
            self.session.set_dem(&rel, bytes, dem);
        }
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
            .status_with_dem(self.dem_bytes.as_deref().map(|b| b.as_slice()))
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
            "running": self.running.as_ref().map(|(s, _)| s.name()),
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

    #[test]
    fn the_dem_next_to_the_document_is_kept_for_the_hillshade_when_it_reads() {
        let tif = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../crates/cd-format/tests/fixtures/dem/f32_geographic.tif"),
        )
        .unwrap();
        let with = AppState::open(Path::new("a"), &worked_example(), Some(&tif)).unwrap();
        let dem = with.dem().expect("a DEM that reads is kept");
        let shade = cd_format::design::hillshade::hillshade(&dem).unwrap();
        assert!(shade.png.starts_with(b"\x89PNG"));
        // No file, or a file that is not a GeoTIFF: no DEM, and no error.
        assert!(
            AppState::open(Path::new("a"), &worked_example(), None)
                .unwrap()
                .dem()
                .is_none()
        );
        assert!(
            AppState::open(Path::new("a"), &worked_example(), Some(b"not a tiff"))
                .unwrap()
                .dem()
                .is_none()
        );
    }

    // --- running stages (ticket 70) --------------------------------------------------------

    /// A gently rolling DEM over the worked example's river, as GeoTIFF bytes (WGS 84 / UTM 30N).
    fn dem_bytes() -> Vec<u8> {
        let (w, h) = (80usize, 12usize);
        let data = (0..w * h)
            .map(|i| 40.0 + 6.0 * ((i % w) as f64 / 6.0).sin() + 4.0 * ((i / w) as f64 / 2.0).cos())
            .collect();
        let dem = Dem {
            width: w,
            height: h,
            data,
            geotransform: [100.0, 0.0, 499_800.0, 0.0, -100.0, 600_600.0],
            nodata: Some(-9999.0),
            epsg: Some(32630),
        };
        cd_format::design::dem_source::write_geotiff(&dem).unwrap()
    }

    /// The worked example with a DEM and a water level, opened from a folder that holds the DEM.
    fn ready() -> (AppState, tempdir::Dir) {
        let dir = tempdir::Dir::new();
        let file = dir.path().join("dem.tif");
        std::fs::write(&file, dem_bytes()).unwrap();
        let mut s =
            AppState::open(&dir.path().join("river.cdesign"), &worked_example(), None).unwrap();
        s.load_dem(&file, dem_bytes()).unwrap();
        s.set_water(38.0, None).unwrap();
        (s, dir)
    }

    /// Run a stage to the end on this thread, as the worker does.
    fn run(s: &mut AppState, stage: &str, method: Option<&str>) -> Result<&'static str, String> {
        let (job, _) = s.begin_run(stage, method)?;
        let outcome = job.execute(&|_| {});
        s.finish_run(outcome)
    }

    #[test]
    fn alignment_then_volumes_turn_fresh() {
        let (mut s, _dir) = ready();
        assert_eq!(stage_status(&s, "alignment"), "not_run");
        assert_eq!(run(&mut s, "alignment", Some("rolling_circle")), Ok("done"));
        assert_eq!(stage_status(&s, "alignment"), "fresh");
        assert_eq!(run(&mut s, "volumes", None), Ok("done"));
        assert_eq!(stage_status(&s, "volumes"), "fresh");
        assert!(s.is_dirty());
        assert!(s.snapshot()["running"].is_null());
    }

    #[test]
    fn editing_the_vessel_after_a_run_marks_the_later_stages_stale() {
        let (mut s, _dir) = ready();
        run(&mut s, "alignment", Some("rolling_circle")).unwrap();
        run(&mut s, "volumes", None).unwrap();
        s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
        assert_eq!(stage_status(&s, "sizing"), "fresh");
        assert_eq!(stage_status(&s, "alignment"), "stale");
        assert_eq!(stage_status(&s, "volumes"), "stale");
    }

    #[test]
    fn a_cancelled_run_leaves_the_design_exactly_as_it_was() {
        let (mut s, _dir) = ready();
        run(&mut s, "alignment", Some("rolling_circle")).unwrap();
        let before = s.snapshot();
        let (job, control) = s.begin_run("volumes", None).unwrap();
        // Cancel before the first terrain read: the stage stops at its first look at the DEM.
        assert!(s.cancel_run());
        assert!(control.cancel.load(Ordering::Relaxed));
        assert_eq!(s.finish_run(job.execute(&|_| {})), Ok("cancelled"));
        let mut after = s.snapshot();
        after["running"] = before["running"].clone();
        assert_eq!(after, before);
        assert_eq!(stage_status(&s, "volumes"), "not_run");
    }

    #[test]
    fn cancel_pressed_mid_run_stops_it_after_it_started() {
        let (mut s, _dir) = ready();
        run(&mut s, "alignment", Some("rolling_circle")).unwrap();
        let before = s.snapshot();
        let (job, control) = s.begin_run("volumes", None).unwrap();
        // The user presses Cancel while the stage is already reading the DEM.
        let c = control.clone();
        let outcome = job.execute(&move |n| {
            if n > 500 {
                c.cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(
            control.samples.load(Ordering::Relaxed) > 500,
            "it had started"
        );
        assert_eq!(s.finish_run(outcome), Ok("cancelled"));
        let mut after = s.snapshot();
        after["running"] = before["running"].clone();
        assert_eq!(after, before);
    }

    #[test]
    fn a_failed_run_says_why_and_changes_nothing() {
        let (mut s, _dir) = ready();
        let before = s.snapshot();
        // Volumes before alignment: the session refuses, with its own reason.
        let e = run(&mut s, "volumes", None).unwrap_err();
        assert!(!e.is_empty());
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn nothing_else_runs_or_changes_while_a_stage_runs() {
        let (mut s, _dir) = ready();
        let (_job, _) = s.begin_run("alignment", None).unwrap();
        assert!(
            s.begin_run("alignment", None)
                .unwrap_err()
                .contains("is running")
        );
        assert!(
            s.set_sizing("PIANC-B", "CEMT_Va", None)
                .unwrap_err()
                .contains("is running")
        );
        assert!(s.set_water(1.0, None).unwrap_err().contains("is running"));
        assert_eq!(s.snapshot()["running"], "alignment");
    }

    #[test]
    fn loading_wgs84_banks_moves_them_to_a_utm_zone_and_a_refusal_changes_nothing() {
        let mut s = AppState::new_design("Study", "2026-10-16");
        let line = |pts: &str| format!(r#"{{"type":"LineString","coordinates":{pts}}}"#);
        s.load_inputs(&[
            (Role::LeftBank, line("[[-3.0,5.40],[-2.99,5.41]]")),
            (Role::RightBank, line("[[-3.0,5.401],[-2.99,5.411]]")),
        ])
        .unwrap();
        let doc = &s.snapshot()["document"];
        assert_eq!(doc["project"]["crs"]["name"], "WGS 84 / UTM zone 30N");
        assert_eq!(doc["project"]["source_crs"]["left_bank"], "EPSG:4326");
        let before = s.snapshot();
        assert!(s.load_inputs(&[(Role::Centreline, "{".into())]).is_err());
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn a_dem_is_referenced_by_a_path_relative_to_the_document_and_follows_a_save_as() {
        let (mut s, dir) = ready();
        assert_eq!(s.snapshot()["document"]["inputs"]["dem"]["path"], "dem.tif");
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        s.saved(&sub.join("moved.cdesign"));
        assert_eq!(
            s.snapshot()["document"]["inputs"]["dem"]["path"],
            "../dem.tif"
        );
    }

    #[test]
    fn fetching_a_dem_needs_a_saved_design() {
        let (mut s, _dir) = ready();
        assert!(
            s.refuse_unsaved_fetch()
                .unwrap_err()
                .contains("save the design first")
        );
        s.saved(Path::new("/nonexistent/x.cdesign"));
        assert!(s.refuse_unsaved_fetch().is_ok());
    }

    #[test]
    fn the_sections_come_from_the_walk_the_volumes_summed_and_the_walk_is_kept_until_the_design_changes()
     {
        let (mut s, _dir) = ready();
        run(&mut s, "alignment", Some("rolling_circle")).unwrap();
        // Before volumes have run there is no walk to draw.
        assert!(s.begin_walk("rolling-circle").run().is_err());
        run(&mut s, "volumes", None).unwrap();
        let first = s.begin_walk("rolling-circle").run().unwrap();
        let ls = first
            .session
            .long_section("rolling-circle", &first.walk)
            .unwrap();
        let row = &s.snapshot()["document"]["stages"]["volumes"]["results"]["options"][0];
        assert_eq!(
            ls["chainage_m"].as_array().unwrap().len() as u64,
            row["stations"].as_u64().unwrap()
        );
        s.keep_walk(&first);
        // Kept: the next job already holds it. A new water level changes the volumes' inputs: it is dropped.
        assert!(s.begin_walk("rolling-circle").cached.is_some());
        assert!(s.begin_walk("other").cached.is_none());
        s.set_water(37.0, None).unwrap();
        assert!(s.begin_walk("rolling-circle").cached.is_none());
        // And volumes are stale now, so there is no walk to make.
        assert!(
            s.begin_walk("rolling-circle")
                .run()
                .unwrap_err()
                .contains("stale")
        );
    }

    /// A temporary folder that goes away with the test.
    mod tempdir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                let n = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let p = std::env::temp_dir().join(
                    format!("cd-app-test-{n}-{:?}", std::thread::current().id())
                        .replace(['(', ')'], ""),
                );
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
