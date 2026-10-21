//! The design session: the one interface between the engine and its users (the CLI and the app).
//! It takes and returns text and values. The callers do all file, network and clock access.
//!
//! The document is a JSON value (`schema/owd.schema.json`). A stage's status is never stored:
//! it is worked out on load from the stage's recorded dependency hash.

use super::alignment::Terrain;
use super::crs::{Crs, Utm};
use super::geojson::{self, Role};
use super::hash::{hash_value, sha256_hex};
use super::schema;
use owd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use owd_core::rules::vessel_catalogue;
use serde_json::{Value, json};

/// The engine version written into documents and stages.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The document format version this engine writes (2 since spec 0005; it opens 1 and moves it).
pub const FORMAT_VERSION: i64 = 2;

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
}

fn empty_stage() -> Value {
    json!({"settings": null, "results": null, "depends_on": null, "engine_version": null})
}

fn shape_value(
    ruleset_id: &str,
    vessel_standard_id: &str,
    draught_m: Option<f64>,
) -> Result<Value, DesignError> {
    if let Some(d) = draught_m
        && !(d.is_finite() && d > 0.0)
    {
        return Err(DesignError::Input(
            "the draught must be a positive number of metres".into(),
        ));
    }
    let reg = RulesetRegistry::new();
    let s = channel_shape(&reg, ruleset_id, vessel_standard_id, draught_m)
        .map_err(|e| DesignError::Input(e.raw_message().to_string()))?;
    Ok(json!({
        "ruleset_id": s.ruleset_id,
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
    }))
}

impl Session {
    /// An empty channel design with a title. `created` is `YYYY-MM-DD`, given by the caller.
    pub fn new(title: &str, created: &str) -> Session {
        Session {
            water_sampling: Default::default(),
            doc: json!({
                "format_version": FORMAT_VERSION,
                "engine_version": ENGINE_VERSION,
                "project": {"title": title, "created": created, "crs": null, "source_crs": {}},
                "inputs": {
                    "left_bank": null, "right_bank": null, "centreline": null,
                    "river_polygon": null, "dem": null, "water_level": null
                },
                "stages": {
                    "sizing": empty_stage(), "alignment": empty_stage(),
                    "optimise": empty_stage(), "volumes": empty_stage()
                },
                "options": []
            }),
        }
    }

    /// Parse and validate `.owd` text (a `.cdesign` file has the same contents).
    pub fn open(text: &str) -> Result<Session, DesignError> {
        let doc: Value = serde_json::from_str(text)
            .map_err(|e| DesignError::Input(format!("the file is not valid JSON: {e}")))?;
        let mut s = Session {
            doc,
            water_sampling: Default::default(),
        };
        if s.doc["inputs"].is_object() && s.validate_schema_only().is_empty() {
            s.migrate_water_settings();
            s.migrate_to_v2();
        }
        let errors = s.validate();
        if errors.is_empty() {
            Ok(s)
        } else {
            Err(DesignError::Document(errors))
        }
    }

    /// A version 1 document (before spec 0005) becomes version 2. Its rolling circle records
    /// `use_water: false`, so a rerun gives the results it already holds; a stage that was fresh
    /// stays fresh (its hash is taken again over the recorded setting).
    fn migrate_to_v2(&mut self) {
        if self.doc["format_version"] != 1 {
            return;
        }
        let fresh: Vec<Stage> = Stage::ALL
            .iter()
            .copied()
            .filter(|st| self.stage_status(*st) == StageStatus::Fresh)
            .collect();
        self.doc["format_version"] = json!(FORMAT_VERSION);
        let settings = &mut self.doc["stages"]["alignment"]["settings"];
        if !settings.is_object() {
            *settings = json!({});
        }
        if !settings["rolling_circle"].is_object() {
            settings["rolling_circle"] = json!({});
        }
        settings["rolling_circle"]["use_water"] = json!(false);
        for st in fresh {
            let h = self.dependency_hash(st);
            self.doc["stages"][st.name()]["depends_on"] = json!(h);
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
        }
        if matches!(stage, Stage::Optimise | Stage::Volumes) {
            dep["options"] = self.doc["options"].clone();
        }
        hash_value(&dep)
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

    /// The channel shape for a ruleset, vessel class and draught, without touching the document.
    /// The app calls this on every edit; `size` runs it and records the result.
    pub fn preview_sizing(
        ruleset_id: &str,
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<Value, DesignError> {
        shape_value(ruleset_id, vessel_standard_id, draught_m)
    }

    /// Set the sizing choices. Nothing runs: the sizing stage shows as stale, and so does every
    /// stage after it that had results. The choices are checked first, so a typo is refused here.
    pub fn set_sizing(
        &mut self,
        ruleset_id: &str,
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<(), DesignError> {
        shape_value(ruleset_id, vessel_standard_id, draught_m)?;
        self.doc["stages"]["sizing"]["settings"] = json!({
            "ruleset_id": ruleset_id,
            "vessel_standard_id": vessel_standard_id,
            "draught_m": draught_m,
        });
        Ok(())
    }

    fn run_sizing(&mut self) -> Result<(), DesignError> {
        let set = self.doc["stages"]["sizing"]["settings"].clone();
        let (Some(rs), Some(v)) = (
            set["ruleset_id"].as_str(),
            set["vessel_standard_id"].as_str(),
        ) else {
            return Err(DesignError::Input(
                "the sizing stage has no settings: choose a ruleset and a vessel class first"
                    .into(),
            ));
        };
        let shape = shape_value(rs, v, set["draught_m"].as_f64())?;
        self.doc["stages"]["sizing"]["results"] = shape;
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
                StageStatus::NotRun => !self.doc["stages"][stage.name()]["settings"].is_null(),
                StageStatus::Fresh => false,
            };
            if due {
                self.run_stage_with(stage, terrain)?;
                ran.push(stage);
            }
        }
        Ok(ran)
    }

    /// The rulesets and vessel classes a user can choose for sizing, for the lists in the app and
    /// for `--help`-style discovery: ids, display labels and the dimensions that matter.
    pub fn sizing_choices() -> Value {
        let rulesets: Vec<Value> = RulesetRegistry::new()
            .iter()
            .map(|r| json!({"id": r.ruleset_id, "label": r.label, "family": r.family}))
            .collect();
        let vessels: Vec<Value> = vessel_catalogue()
            .into_iter()
            .map(|(id, v)| {
                json!({
                    "id": id,
                    "label": format!(
                        "{} {} ({}, LOA {}-{} m)",
                        v.family, v.class_name, v.formation.replace('_', " "), v.loa_m.0, v.loa_m.1
                    ),
                    "family": v.family,
                    "formation": v.formation,
                    "loa_m": [v.loa_m.0, v.loa_m.1],
                    "beam_m": v.beam_m,
                    "draught_nominal_m": v.draught_nominal_m(),
                })
            })
            .collect();
        json!({"rulesets": rulesets, "vessels": vessels})
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

    fn with_sizing_fresh() -> Session {
        let mut s = Session::new("t", "2026-10-16");
        s.set_sizing("PIANC-B", "CEMT_IV", None).unwrap();
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
        s.set_sizing("PIANC-B", "CEMT_Va", None).unwrap();
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
        s.set_sizing("PIANC-B", "CEMT_Va", None).unwrap();
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
