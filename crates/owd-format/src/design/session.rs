//! The design session: the one interface between the engine and its users (the CLI and the app).
//! It takes and returns text and values. The callers do all file, network and clock access.
//!
//! The document is a JSON value (`schema/owd.schema.json`). A stage's status is never stored:
//! it is worked out on load from the stage's recorded dependency hash.

use super::alignment::Terrain;
use super::crs::{Crs, Utm};
use super::geojson::{self, Role};
use super::hash::{hash_value, sha256_hex};
use super::rule_sets::default_settings;
use super::schema;
use arc_rules::load::Library;
use owd_core::rules::engine;
use owd_core::rules::rulesets::ChannelShape;
use serde_json::{Value, json};

/// The engine version written into documents and stages.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The document format version this engine writes: the string `"0.4"` since spec 0015 (the custom
/// vessel). A file with the number `3` (format 0.3, spec 0007) opens and is saved as 0.4, with
/// nothing else changed. An older file is refused; there is no migration. Until format 1.0 (after
/// an external review) a format change raises the minor number.
pub const FORMAT_VERSION: &str = "0.4";

/// The format 0.3 files wrote as a number.
const FORMAT_0_3: i64 = 3;

/// A format version string as `(major, minor)`, such as `"0.4"` -> `(0, 4)`.
fn format_number(v: &str) -> Option<(u64, u64)> {
    let (a, b) = v.split_once('.')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// The UTC calendar date, `YYYY-MM-DD`, of a Unix time in seconds. The session has no clock: a caller
/// reads one and passes the seconds here when it needs a date to write into a new document.
pub fn date_from_unix(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A stage of a channel design, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Sizing,
    Alignment,
    Optimise,
    Volumes,
}

impl Stage {
    pub const ALL: [Stage; 4] = [
        Stage::Sizing,
        Stage::Alignment,
        Stage::Optimise,
        Stage::Volumes,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Sizing => "sizing",
            Stage::Alignment => "alignment",
            Stage::Optimise => "optimise",
            Stage::Volumes => "volumes",
        }
    }

    pub fn from_name(name: &str) -> Option<Stage> {
        Stage::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// Where a stage stands. Worked out on load, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    NotRun,
    Fresh,
    Stale,
}

impl StageStatus {
    pub fn name(self) -> &'static str {
        match self {
            StageStatus::NotRun => "not run",
            StageStatus::Fresh => "fresh",
            StageStatus::Stale => "stale",
        }
    }
}

/// Why a session call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignError {
    /// The caller gave something the tool cannot use (a bad file, an unknown ruleset, a CRS it
    /// does not support). The message says what to change.
    Input(String),
    /// The document breaks the schema or its own rules; one line per break.
    Document(Vec<String>),
    /// A stage that cannot run yet: a later ticket builds it.
    NotAvailable(String),
    /// A stage whose inputs are stale refuses to run, and names the stage to run first.
    StaleUpstream { stage: Stage, run_first: Stage },
}

impl core::fmt::Display for DesignError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DesignError::Input(m) | DesignError::NotAvailable(m) => f.write_str(m),
            DesignError::Document(errs) => {
                write!(f, "the channel design is not valid: {}", errs.join("; "))
            }
            DesignError::StaleUpstream { stage, run_first } => write!(
                f,
                "the {} stage cannot run: {} is stale. Run {} first.",
                stage.name(),
                run_first.name(),
                run_first.name()
            ),
        }
    }
}

impl std::error::Error for DesignError {}

/// Refuse a design file named with the old extension, `.cdesign` (spec 0006, T10). Its content is a
/// `.owd` file's, but the old name is no longer read: the message says to rename the file. Every
/// caller that opens or creates a design by file name checks the name here first.
pub fn check_file_name(name: &str) -> Result<(), DesignError> {
    let old = std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cdesign"));
    if old {
        return Err(DesignError::Input(format!(
            "{name}: `.cdesign` is the old name of the design file: rename it to `.owd` (the content is the same)"
        )));
    }
    Ok(())
}

/// The state of a channel design.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub stages: Vec<(Stage, StageStatus)>,
}

/// What an import changed.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    /// The project CRS after the import (EPSG code).
    pub project_epsg: u32,
    /// `(input name, source CRS as EPSG:nnnn, positions read)` per input.
    pub inputs: Vec<(String, String, usize)>,
}

/// One channel design.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub(super) doc: Value,
    pub(super) water_sampling: super::water::WaterSampling,
    /// The Rule Sets the session can select: the built-in ones and any loaded at run time.
    pub(super) library: Library,
}

fn empty_stage() -> Value {
    json!({"settings": null, "results": null, "depends_on": null, "engine_version": null})
}

/// The sizing stage of a new design: the default Rule Set selection, no vessel yet.
fn sizing_stage() -> Value {
    let mut st = empty_stage();
    st["settings"] = default_settings();
    st
}

/// The sizing results of a channel shape. `label` names the selection, such as
/// `pianc-wg141@0.3.0 (se_quality B)`.
fn shape_value(s: &ChannelShape, label: &str) -> Value {
    let mut v = json!({
        "rule_sets": label,
        "vessel_standard_id": s.vessel_standard_id,
        "loa_m": s.loa_m,
        "beam_m": s.beam_m,
        "draught_m": s.draught_m,
        "formation": s.formation,
        "bottom_width_straight_m": s.bottom_width_straight_m,
        "min_bend_radius_m": s.min_bend_radius_m,
        "design_depth_m": s.design_depth_m,
        "bend_widening_at_rmin_m": s.bend_widening_at_rmin_m,
        "bottom_width_at_rmin_m": s.bottom_width_at_rmin_m,
        "width_basis": s.width_basis,
        "source": s.source,
        "warnings": s.warnings,
    });
    // A custom vessel has no class id; every output shows its label (spec 0015 T1).
    if s.custom {
        v["vessel_standard_id"] = Value::Null;
        v["vessel_label"] = json!(s.vessel_standard_id);
    }
    v
}

impl Session {
    /// An empty channel design with a title. `created` is `YYYY-MM-DD`, given by the caller.
    pub fn new(title: &str, created: &str) -> Session {
        Session {
            water_sampling: Default::default(),
            library: engine::built_in().clone(),
            doc: json!({
                "format_version": FORMAT_VERSION,
                "engine_version": ENGINE_VERSION,
                "project": {"title": title, "created": created, "crs": null, "source_crs": {}},
                "inputs": {
                    "left_bank": null, "right_bank": null, "centreline": null,
                    "river_polygon": null, "dem": null, "water_level": null
                },
                "stages": {
                    "sizing": sizing_stage(), "alignment": empty_stage(),
                    "optimise": empty_stage(), "volumes": empty_stage()
                },
                "options": []
            }),
        }
    }

    /// Parse and validate `.owd` text.
    pub fn open(text: &str) -> Result<Session, DesignError> {
        let mut doc: Value = serde_json::from_str(text)
            .map_err(|e| DesignError::Input(format!("the file is not valid JSON: {e}")))?;
        match &doc["format_version"] {
            // Format 0.3 reads as it is, and is saved as 0.4 (spec 0015).
            Value::Number(n) if n.as_i64() == Some(FORMAT_0_3) => {
                doc["format_version"] = json!(FORMAT_VERSION);
            }
            Value::Number(n) => {
                return Err(DesignError::Input(format!(
                    "the file is document format version {n}; this OWD opens format \
                     {FORMAT_VERSION} and 0.3 only, and moves no older file (spec 0007). Rebuild \
                     the design: start a new one, import its inputs, size it and run its stages \
                     again."
                )));
            }
            Value::String(v)
                if format_number(v).is_some_and(|n| Some(n) > format_number(FORMAT_VERSION)) =>
            {
                return Err(DesignError::Input(format!(
                    "the file is document format {v}, made by a newer OWD; this OWD opens format \
                     {FORMAT_VERSION} and older. Update OWD to open it."
                )));
            }
            // Anything else is for the schema to name.
            _ => {}
        }
        let mut s = Session {
            doc,
            water_sampling: Default::default(),
            library: engine::built_in().clone(),
        };
        if s.doc["inputs"].is_object() && s.validate_schema_only().is_empty() {
            s.migrate_water_settings();
            s.canonical_vessel_ids();
        }
        let errors = s.validate();
        if errors.is_empty() {
            Ok(s)
        } else {
            Err(DesignError::Document(errors))
        }
    }

    /// Older studies name a vessel class by an id the shared catalogue replaced (`CEMT_Va`). The
    /// class still works; the study moves to the catalogue id (`CEMT-Va`), and stages that were fresh
    /// stay fresh (spec 0015 T0).
    fn canonical_vessel_ids(&mut self) {
        let fresh: Vec<Stage> = Stage::ALL
            .iter()
            .copied()
            .filter(|s| self.stage_status(*s) == StageStatus::Fresh)
            .collect();
        let mut moved = false;
        for part in ["settings", "results"] {
            // `get_mut`, not indexing: indexing would turn a missing part into an empty object.
            let Some(at) = self
                .doc
                .pointer_mut(&format!("/stages/sizing/{part}/vessel_standard_id"))
            else {
                continue;
            };
            if let Some(id) = at.as_str()
                && let Some(new) = owd_core::rules::classes::canonical_id(id)
                && new != id
            {
                *at = json!(new);
                moved = true;
            }
        }
        if moved {
            for s in fresh {
                let h = self.dependency_hash(s);
                self.doc["stages"][s.name()]["depends_on"] = json!(h);
            }
        }
    }

    /// The document text: two-space indented JSON with a final newline.
    pub fn to_text(&self) -> String {
        let mut t = serde_json::to_string_pretty(&self.doc).expect("a JSON value serialises");
        t.push('\n');
        t
    }

    /// The document as a value.
    pub fn document(&self) -> &Value {
        &self.doc
    }

    /// The published JSON Schema text.
    pub fn schema_text() -> &'static str {
        schema::SCHEMA_TEXT
    }

    fn validate_schema_only(&self) -> Vec<String> {
        schema::validate(&schema::schema(), &self.doc)
    }

    /// Every way the document breaks the schema or its own rules. Empty when it is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = schema::validate(&schema::schema(), &self.doc);
        if !errors.is_empty() {
            return errors;
        }
        for stage in Stage::ALL {
            let st = &self.doc["stages"][stage.name()];
            if !st["results"].is_null() && st["depends_on"].is_null() {
                errors.push(format!(
                    "/stages/{}: has results but no `depends_on` hash",
                    stage.name()
                ));
            }
            if !st["results"].is_null() && st["engine_version"].is_null() {
                errors.push(format!(
                    "/stages/{}: has results but no `engine_version`",
                    stage.name()
                ));
            }
        }
        errors
    }

    // --- status -------------------------------------------------------------------------

    /// What a stage depended on, hashed: the inputs, the project CRS, and the settings and results
    /// of the stage and of every stage before it.
    pub(super) fn dependency_hash(&self, stage: Stage) -> String {
        let st = |s: Stage| &self.doc["stages"][s.name()];
        let part = |s: Stage| json!({"settings": st(s)["settings"], "results": st(s)["results"]});
        let upto = |s: Stage| Stage::ALL.iter().take_while(|x| **x != s).count();
        let mut chain = serde_json::Map::new();
        for s in Stage::ALL.iter().take(upto(stage) + 1) {
            if *s == stage {
                chain.insert(s.name().into(), json!({"settings": st(*s)["settings"]}));
            } else {
                chain.insert(s.name().into(), part(*s));
            }
        }
        let mut dep = json!({
            "stage": stage.name(),
            "chain": Value::Object(chain),
        });
        // The content of every selected Rule Set, as the engine holds it now: a changed set makes the
        // sizing stage, and so every stage after it, stale.
        dep["rule_sets"] = self.selected_set_hashes();
        // The marked reaches decide which radius Rule applies where, so sizing depends on them.
        if let Some(r) = self.doc["inputs"].get("waterway_reaches") {
            dep["waterway_reaches"] = r.clone();
        }
        if stage != Stage::Sizing {
            dep["inputs"] = self.doc["inputs"].clone();
            // The DEM is identified by its hash, not by where the file sits: moving a design and its
            // DEM to another folder must not make its results stale.
            if let Some(dem) = dep["inputs"]["dem"].as_object_mut() {
                dem.remove("path");
            }
            // The rolling circle reads neither the DEM nor the water level: one added after it ran
            // leaves it fresh. The re-route, the optimise stage and the volumes read them.
            let reads_dem = match stage {
                Stage::Alignment => self.doc["stages"]["alignment"]["results"]["methods"]
                    .as_array()
                    .is_some_and(|m| m.iter().any(|x| x == "reroute")),
                _ => true,
            };
            if !reads_dem && let Some(inputs) = dep["inputs"].as_object_mut() {
                inputs.remove("dem");
                inputs.remove("water_level");
            }
            dep["crs"] = self.doc["project"]["crs"].clone();
            // Only the volumes read the bed model (spec 0012): a new depth leaves the stages
            // before them fresh.
            if stage != Stage::Volumes
                && let Some(inputs) = dep["inputs"].as_object_mut()
            {
                inputs.remove("bed_model");
            }
            // No stage reads the reference line (spec 0016): storing it leaves them fresh. The
            // volumes read the assumed bed, which is centred on it (#398).
            if let Some(inputs) = dep["inputs"].as_object_mut() {
                inputs.remove("reference_line");
            }
            if stage == Stage::Volumes
                && self.bed_model_value().is_some()
                && let Some(a) = self.bed_axis_hash()
            {
                dep["bed_axis"] = a;
            }
        }
        if matches!(stage, Stage::Optimise | Stage::Volumes) {
            dep["options"] = self.doc["options"].clone();
        }
        hash_value(&dep)
    }

    /// The hash of what `stage` depends on now: a stage is fresh while it equals the one recorded.
    pub fn stage_hash(&self, stage: Stage) -> String {
        self.dependency_hash(stage)
    }

    pub fn stage_status(&self, stage: Stage) -> StageStatus {
        let st = &self.doc["stages"][stage.name()];
        if st["results"].is_null() {
            return StageStatus::NotRun;
        }
        if st["depends_on"].as_str() == Some(self.dependency_hash(stage).as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    pub fn status(&self) -> Status {
        Status {
            stages: Stage::ALL
                .iter()
                .map(|s| (*s, self.stage_status(*s)))
                .collect(),
        }
    }

    /// A warning when the DEM the document names is missing or is not the file whose hash was
    /// recorded. `bytes` is the file the caller found at the recorded path, if any.
    pub fn dem_warning(&self, bytes: Option<&[u8]>) -> Option<String> {
        let dem = self.doc["inputs"]["dem"].as_object()?;
        let path = dem["path"].as_str().unwrap_or("?");
        match bytes {
            None => Some(format!(
                "the DEM `{path}` is missing: the volumes may no longer hold"
            )),
            Some(b) if Some(sha256_hex(b).as_str()) != dem["sha256"].as_str() => Some(format!(
                "the DEM `{path}` has changed since its hash was recorded: the volumes may no longer hold"
            )),
            Some(_) => None,
        }
    }

    // --- sizing -------------------------------------------------------------------------

    /// The sizing results for a Rule Set selection (set ids, and set inputs as `(name, value)`),
    /// a vessel class and a draught, without touching any document. `owd` is always selected.
    pub fn preview_sizing(
        rule_sets: &[&str],
        set_inputs: &[(&str, &str)],
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<Value, DesignError> {
        let mut s = Session::new("", "");
        s.set_rule_sets(rule_sets, set_inputs)?;
        s.set_vessel(vessel_standard_id, draught_m)?;
        s.sizing_results()
    }

    /// The sizing results of the sizing settings: the channel shape, and B, the design basis (spec
    /// 0007), each quantity with its governing Rule.
    fn sizing_results(&self) -> Result<Value, DesignError> {
        let settings = self.sizing_settings();
        let shape = self.shape_for_settings(&settings)?;
        let mut results = shape_value(&shape, &Session::selection_label(&settings));
        results["design_basis"] = self.design_basis_value()?;
        results["rules_notice"] = json!(super::screening::UNVERIFIED_NOTICE);
        Ok(results)
    }

    fn run_sizing(&mut self) -> Result<(), DesignError> {
        self.doc["stages"]["sizing"]["results"] = self.sizing_results()?;
        self.doc["stages"]["sizing"]["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Sizing);
        self.doc["stages"]["sizing"]["depends_on"] = json!(h);
        Ok(())
    }

    /// Run one stage with no terrain. See [`Session::run_stage_with`].
    pub fn run_stage(&mut self, stage: Stage) -> Result<(), DesignError> {
        self.run_stage_with(stage, None)
    }

    /// Run one stage. A stage whose earlier stages are stale refuses, and names the one to run
    /// first. `terrain` is the DEM in the project CRS, for the stages that need it (the re-route and
    /// the volumes); the caller reads the file.
    pub fn run_stage_with(
        &mut self,
        stage: Stage,
        terrain: Option<Terrain>,
    ) -> Result<(), DesignError> {
        for earlier in Stage::ALL.iter().take_while(|s| **s != stage) {
            if self.stage_status(*earlier) == StageStatus::Stale {
                return Err(DesignError::StaleUpstream {
                    stage,
                    run_first: *earlier,
                });
            }
        }
        match stage {
            Stage::Sizing => self.run_sizing(),
            Stage::Alignment => self.rerun_alignment(terrain),
            Stage::Volumes => self.run_volumes(&Value::Null, terrain),
            Stage::Optimise => self.run_optimise(&Value::Null, terrain),
        }
    }

    /// Run every stage that is stale, or has settings but no results, in order. Returns the stages
    /// that ran. A stale stage that cannot run yet (a later ticket builds it) is an error.
    pub fn run_stale(&mut self) -> Result<Vec<Stage>, DesignError> {
        self.run_stale_with(None)
    }

    /// [`Session::run_stale`] with the DEM as a terrain sampler.
    pub fn run_stale_with(&mut self, terrain: Option<Terrain>) -> Result<Vec<Stage>, DesignError> {
        let mut ran = Vec::new();
        for stage in Stage::ALL {
            let due = match self.stage_status(stage) {
                StageStatus::Stale => true,
                StageStatus::NotRun => {
                    let settings = &self.doc["stages"][stage.name()]["settings"];
                    // A new design has a Rule Set selection but no vessel: nothing to size yet.
                    !settings.is_null()
                        && (stage != Stage::Sizing
                            || !settings["vessel_standard_id"].is_null()
                            || settings["custom_vessel"].is_object())
                }
                StageStatus::Fresh => false,
            };
            if due {
                self.run_stage_with(stage, terrain)?;
                ran.push(stage);
            }
        }
        Ok(ran)
    }

    /// The Rule Sets, their set inputs and the vessel classes a user can choose for sizing, for
    /// the lists in the app: ids, display labels and the dimensions that matter. `owd` is listed
    /// apart, as `always`: it is always selected.
    pub fn sizing_choices() -> Value {
        let (rule_sets, always) = super::rule_sets::rule_set_choices();
        // Every class a channel can be sized for (spec 0015 T0), with its family for the picker's
        // groups and the words its search matches; `default` marks the 12 `owd fits` checks.
        let defaults = owd_core::rules::classes::default_class_ids();
        let vessels: Vec<Value> = owd_core::rules::full_catalogue()
            .into_iter()
            .map(|(id, v)| {
                let (lo, hi) = v.loa_m.unwrap_or((f64::NAN, f64::NAN));
                let loa = if lo == hi {
                    format!("{hi}")
                } else {
                    format!("{lo}-{hi}")
                };
                json!({
                    "id": id,
                    "label": format!(
                        "{id} ({}, LOA {loa} m, beam {} m)",
                        v.formation.replace('_', " "),
                        v.beam_m.unwrap_or(f64::NAN)
                    ),
                    "family": v.family,
                    "region": v.region,
                    "formation": v.formation,
                    "loa_m": v.loa_m.map(|(a, b)| [a, b]),
                    "beam_m": v.beam_m,
                    "draught_nominal_m": v.draught_nominal_m(),
                    "default": defaults.contains(&id.as_str()),
                })
            })
            .collect();
        json!({"rule_sets": rule_sets, "always": always, "vessels": vessels})
    }

    // --- inputs -------------------------------------------------------------------------

    /// Import inputs from GeoJSON text. Reads every input first and changes nothing if one is
    /// refused. The first import chooses the project CRS: the UTM zone of a UTM input, or the zone
    /// and hemisphere at the centroid of the bank lines for WGS84 input. Later imports use it.
    pub fn import(&mut self, items: &[(Role, &str)]) -> Result<ImportReport, DesignError> {
        if items.is_empty() {
            return Err(DesignError::Input("nothing to import".into()));
        }
        let mut parsed = Vec::new();
        for (role, text) in items {
            let p = geojson::parse(*role, text)
                .map_err(|e| DesignError::Input(format!("{}: {e}", role.name())))?;
            parsed.push((*role, p));
        }
        let existing: Option<Utm> = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        let target = match existing {
            Some(u) => u,
            None => {
                // A UTM input names the zone; otherwise the centroid of the bank lines (any line
                // or polygon if there are no banks in this import) picks it.
                let utm_input = parsed.iter().find_map(|(_, p)| match p.crs {
                    Crs::Utm(u) => Some(u),
                    Crs::Wgs84 => None,
                });
                match utm_input {
                    Some(u) => u,
                    None => {
                        let pick = |banks_only: bool| {
                            let pts: Vec<[f64; 2]> = parsed
                                .iter()
                                .filter(|(r, _)| !banks_only || r.is_bank())
                                .flat_map(|(_, p)| geojson::all_positions(&p.geometry))
                                .collect();
                            (!pts.is_empty()).then_some(pts)
                        };
                        let pts = pick(true).or_else(|| pick(false)).ok_or_else(|| {
                            DesignError::Input("the input has no positions".into())
                        })?;
                        let n = pts.len() as f64;
                        let lon = pts.iter().map(|p| p[0]).sum::<f64>() / n;
                        let lat = pts.iter().map(|p| p[1]).sum::<f64>() / n;
                        if !(lon.abs() <= 180.0 && lat.abs() <= 84.0) {
                            return Err(DesignError::Input(format!(
                                "the input is marked WGS84 but its centre ({lon:.4}, {lat:.4}) is not a \
                                 longitude and latitude inside the UTM limits (84 N to 80 S). If the \
                                 data is projected, declare its CRS with a `crs` member naming \
                                 WGS84 / UTM."
                            )));
                        }
                        Utm::for_point(lon, lat)
                    }
                }
            }
        };
        let mut converted = Vec::new();
        for (role, p) in &parsed {
            let g = geojson::to_project_crs(p, target)
                .map_err(|e| DesignError::Input(format!("{}: {e}", role.name())))?;
            converted.push((*role, p.crs, g));
        }
        let mut report = ImportReport {
            project_epsg: target.epsg(),
            inputs: Vec::new(),
        };
        self.doc["project"]["crs"] = json!({"epsg": target.epsg(), "name": target.name()});
        for (role, source, g) in converted {
            report.inputs.push((
                role.name().to_string(),
                format!("EPSG:{}", source.epsg()),
                geojson::all_positions(&g).len(),
            ));
            self.doc["project"]["source_crs"][role.name()] =
                json!(format!("EPSG:{}", source.epsg()));
            self.doc["inputs"][role.name()] = g;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn dates_come_from_unix_seconds() {
        assert_eq!(date_from_unix(0), "1970-01-01");
        assert_eq!(date_from_unix(951_782_400), "2000-02-29"); // a leap day
        assert_eq!(date_from_unix(1_792_000_000), "2026-10-14");
        assert_eq!(date_from_unix(4_107_542_400), "2100-03-01"); // 2100 is not a leap year
    }

    #[test]
    fn a_cdesign_file_name_is_refused_with_a_message_to_rename_it_to_owd() {
        for name in ["x.cdesign", "dir/river.CDesign"] {
            let e = check_file_name(name).unwrap_err().to_string();
            assert!(e.starts_with(name), "{e}");
            assert!(e.contains("rename it to `.owd`"), "{e}");
        }
        for name in ["x.owd", "-", "cdesign", "x.cdesign.owd", "notes.txt"] {
            assert!(check_file_name(name).is_ok(), "{name}");
        }
    }

    fn with_sizing_fresh() -> Session {
        let mut s = Session::new("t", "2026-10-16");
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_IV", None).unwrap();
        s.run_stage(Stage::Sizing).unwrap();
        s
    }

    /// Give `stage` results that match what it depends on now, as its own ticket will.
    fn fake_run(s: &mut Session, stage: Stage) {
        s.doc["stages"][stage.name()]["settings"] = json!({});
        let h = s.dependency_hash(stage);
        let st = &mut s.doc["stages"][stage.name()];
        st["results"] = json!({"n": 1});
        st["depends_on"] = json!(h);
        st["engine_version"] = json!(ENGINE_VERSION);
    }

    #[test]
    fn an_edit_marks_exactly_the_stages_that_depend_on_it_stale() {
        let mut s = with_sizing_fresh();
        s.import(&[(
            Role::LeftBank,
            r#"{"type":"LineString","crs":{"type":"name","properties":{"name":"EPSG:32630"}},
                "coordinates":[[500000,100],[500100,100]]}"#,
        )])
        .unwrap();
        // Alignment ran after the import and the sizing.
        s.run_stage(Stage::Sizing).unwrap();
        fake_run(&mut s, Stage::Alignment);
        fake_run(&mut s, Stage::Optimise);
        let fresh = |s: &Session| Stage::ALL.map(|st| s.stage_status(st));
        use StageStatus::*;
        assert_eq!(fresh(&s), [Fresh, Fresh, Fresh, NotRun]);
        // A new vessel: sizing and everything after it that has results is stale. Nothing reruns.
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_Va", None).unwrap();
        assert_eq!(fresh(&s), [Stale, Stale, Stale, NotRun]);
        // Running sizing again changes its results, so the later stages stay stale.
        s.run_stage(Stage::Sizing).unwrap();
        assert_eq!(fresh(&s), [Fresh, Stale, Stale, NotRun]);
    }

    #[test]
    fn a_new_input_marks_alignment_stale_but_not_sizing() {
        let mut s = with_sizing_fresh();
        let line = |y: u32| {
            format!(
                r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"EPSG:32630"}}}},
                "coordinates":[[500000,{y}],[500100,{y}]]}}"#
            )
        };
        s.import(&[(Role::LeftBank, &line(100))]).unwrap();
        fake_run(&mut s, Stage::Alignment);
        assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
        s.import(&[(Role::LeftBank, &line(120))]).unwrap();
        assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
        assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    }

    #[test]
    fn a_stage_whose_inputs_are_stale_refuses_to_run_and_names_the_stage_to_run_first() {
        let mut s = with_sizing_fresh();
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_Va", None).unwrap();
        let e = s.run_stage(Stage::Alignment).unwrap_err();
        assert_eq!(
            e,
            DesignError::StaleUpstream {
                stage: Stage::Alignment,
                run_first: Stage::Sizing
            }
        );
        assert_eq!(
            e.to_string(),
            "the alignment stage cannot run: sizing is stale. Run sizing first."
        );
    }

    #[test]
    fn results_without_a_hash_make_the_document_invalid() {
        let mut s = with_sizing_fresh();
        s.doc["stages"]["alignment"]["results"] = json!({"n": 1});
        let e = s.validate();
        assert!(
            e.iter()
                .any(|m| m.contains("alignment") && m.contains("depends_on")),
            "{e:?}"
        );
    }
}
