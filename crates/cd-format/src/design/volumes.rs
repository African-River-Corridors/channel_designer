//! The volumes stage and the DEM reference (spec 0002). The DEM is named by relative path, SHA-256,
//! CRS and nodata value; its bytes are never in the document. The stage gives every design line
//! option its dredge, dry cut and tallest face by calling the core's `section_volumes` on the DEM,
//! under the cut model chosen in the settings.

use super::alignment::Terrain;
use super::geojson::all_positions;
use super::hash::sha256_hex;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use crate::geotiff::Dem;
use cd_core::cut_model::{CUT_MODEL, WIDEN_RMAX, widened_hb_from_shape};
use cd_core::geom::{Polygon, Pt};
use cd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use cd_core::volumes::sections::{
    DEFAULT_DX_M, DEFAULT_MAXW_M, DEFAULT_STATION_M, DEFAULT_WET_SLOPE, HalfWidth, Level,
    section_volumes,
};
use serde_json::{Value, json};

/// How the DEM file the caller found compares with the document's reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemCheck {
    /// The document names no DEM.
    NoDem,
    /// The file is the one whose hash was recorded.
    Matches,
    /// No file at the recorded path.
    Missing,
    /// A file whose hash differs from the recorded one.
    Changed,
}

fn defaults() -> Value {
    json!({
        "cut_model": CUT_MODEL,
        "station_m": DEFAULT_STATION_M,
        "maxw_m": DEFAULT_MAXW_M,
        "dx_m": DEFAULT_DX_M,
        "wet_slope": DEFAULT_WET_SLOPE,
        "half_width_m": null,
    })
}

fn setting(v: &Value, key: &str) -> Result<f64, DesignError> {
    v[key]
        .as_f64()
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a number")))
}

impl Session {
    /// The path of the DEM the document names, relative to the document.
    pub fn dem_path(&self) -> Option<&str> {
        self.doc["inputs"]["dem"]["path"].as_str()
    }

    /// Reference a DEM: its path relative to the document, the SHA-256 of its bytes, its CRS and its
    /// nodata value. The bytes are not kept. The stages that depend on the inputs show as stale.
    pub fn set_dem(&mut self, path: &str, bytes: &[u8], dem: &Dem) {
        self.doc["inputs"]["dem"] = json!({
            "path": path,
            "sha256": sha256_hex(bytes),
            "crs": super::terrain::dem_crs_name(dem),
            "nodata": dem.nodata,
        });
    }

    /// Say where the DEM file now is, relative to the document, when the design is saved to another
    /// folder. The file's hash, CRS and nodata value stay as recorded, and no stage goes stale.
    pub fn set_dem_path(&mut self, path: &str) {
        if self.doc["inputs"]["dem"].is_object() {
            self.doc["inputs"]["dem"]["path"] = json!(path);
        }
    }

    /// Compare the file the caller found at [`Session::dem_path`] with the recorded hash.
    pub fn dem_check(&self, bytes: Option<&[u8]>) -> DemCheck {
        let Some(dem) = self.doc["inputs"]["dem"].as_object() else {
            return DemCheck::NoDem;
        };
        match bytes {
            None => DemCheck::Missing,
            Some(b) if Some(sha256_hex(b).as_str()) == dem["sha256"].as_str() => DemCheck::Matches,
            Some(_) => DemCheck::Changed,
        }
    }

    /// A stage's status given the DEM file the caller found. The recorded hash is in the document, so
    /// a changed or missing file is not visible to [`Session::stage_status`]: here a fresh `volumes`
    /// stage whose DEM does not match shows as stale.
    pub fn stage_status_with_dem(&self, stage: Stage, bytes: Option<&[u8]>) -> StageStatus {
        let status = self.stage_status(stage);
        let dem_bad = matches!(self.dem_check(bytes), DemCheck::Missing | DemCheck::Changed);
        if stage == Stage::Volumes && status == StageStatus::Fresh && dem_bad {
            StageStatus::Stale
        } else {
            status
        }
    }

    /// Every stage's status, given the DEM file the caller found.
    pub fn status_with_dem(&self, bytes: Option<&[u8]>) -> super::session::Status {
        super::session::Status {
            stages: Stage::ALL
                .iter()
                .map(|s| (*s, self.stage_status_with_dem(*s, bytes)))
                .collect(),
        }
    }

    /// Run the volumes stage: dredge, dry cut and tallest face for every design line option.
    ///
    /// The water level is the document's profile along the river centreline (ADR 0006), or the pool
    /// levels of the optimise stage once it has run. `terrain` is the DEM in the project CRS, or `None`
    /// when the caller could not read it, which refuses with the DEM's path.
    pub fn run_volumes(
        &mut self,
        overrides: &Value,
        terrain: Option<Terrain>,
    ) -> Result<(), DesignError> {
        match self.stage_status(Stage::Sizing) {
            StageStatus::Stale => {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Volumes,
                    run_first: Stage::Sizing,
                });
            }
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "the sizing stage has not been run: choose a ruleset and a vessel class first"
                        .into(),
                ));
            }
            StageStatus::Fresh => {}
        }
        for earlier in [Stage::Alignment, Stage::Optimise] {
            if self.stage_status(earlier) == StageStatus::Stale {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Volumes,
                    run_first: earlier,
                });
            }
        }
        let Some(dem_path) = self.dem_path().map(str::to_string) else {
            return Err(DesignError::Input(
                "the volumes stage needs a DEM: import one first (`cdesign import --dem FILE`)"
                    .into(),
            ));
        };
        let terrain = terrain.ok_or_else(|| {
            DesignError::Input(format!(
                "the DEM `{dem_path}` is missing: the volumes stage cannot run without it"
            ))
        })?;
        let options: Vec<Value> = self.doc["options"].as_array().cloned().unwrap_or_default();
        if options.is_empty() {
            return Err(DesignError::Input(
                "there are no design line options: run the alignment stage first".into(),
            ));
        }

        let plan = self.section_plan(overrides)?;
        let mut rows = Vec::new();
        for o in &options {
            let id = o["id"].as_str().unwrap_or("?");
            let v = self.option_sections(&plan, o, terrain)?;
            rows.push(json!({
                "id": id,
                "name": o["name"],
                "dredge_m3": v.dredge_m3(),
                "excav_m3": v.excav_m3(),
                "total_m3": v.total_m3(),
                "max_face_h_m": v.max_face_h_m(),
                "stations": v.stations.len(),
                // A failed check stays with the option's numbers.
                "radius_ok": o["checks"]["radius_ok"],
                "water_source": plan.water_source,
            }));
        }
        let settings = plan.settings;
        let st = &mut self.doc["stages"]["volumes"];
        st["settings"] = settings;
        st["results"] = json!({ "options": rows });
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Volumes);
        self.doc["stages"]["volumes"]["depends_on"] = json!(h);
        Ok(())
    }
}

/// What every option's station walk shares: the settings, the water, the sizing and the river.
pub(super) struct SectionPlan {
    pub(super) settings: Value,
    profile: Vec<[f64; 2]>,
    pub(super) water_source: &'static str,
    centre: Vec<Pt>,
    model: String,
    shape: cd_core::rules::rulesets::ChannelShape,
    depth: f64,
    river: Vec<Polygon>,
    fixed_hb: Option<f64>,
}

impl Session {
    /// Read the volumes settings (the document's, then `overrides`), the water level, the sizing and
    /// the river, once, for the station walk of every option.
    pub(super) fn section_plan(&self, overrides: &Value) -> Result<SectionPlan, DesignError> {
        let mut settings = defaults();
        for source in [
            &self.doc["stages"]["volumes"]["settings"].clone(),
            overrides,
        ] {
            if let Some(o) = source.as_object() {
                for (k, v) in o {
                    settings[k] = v.clone();
                }
            }
        }
        let (profile, water_source) = self
            .water_profile()
            .ok_or_else(|| self.needs_water_profile(Stage::Volumes))?;
        let centre = self.river_centreline()?;
        let model = settings["cut_model"]
            .as_str()
            .unwrap_or(CUT_MODEL)
            .to_string();

        let sizing = &self.doc["stages"]["sizing"]["settings"];
        let (Some(rs), Some(vessel)) = (
            sizing["ruleset_id"].as_str(),
            sizing["vessel_standard_id"].as_str(),
        ) else {
            return Err(DesignError::Input(
                "the sizing stage has no settings".into(),
            ));
        };
        let shape = channel_shape(
            &RulesetRegistry::new(),
            rs,
            vessel,
            sizing["draught_m"].as_f64(),
        )
        .map_err(|e| DesignError::Input(e.raw_message().to_string()))?;
        let depth = shape.design_depth_m;

        let polygon = &self.doc["inputs"]["river_polygon"];
        let ring: Vec<Pt> = polygon["coordinates"][0]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                    .collect()
            })
            .unwrap_or_default();
        let river: Vec<Polygon> = if ring.is_empty() {
            Vec::new()
        } else {
            vec![Polygon {
                exterior: ring,
                holes: vec![],
            }]
        };

        // The bottom half-width: widened in bends from the sizing, unless the settings fix one.
        let fixed_hb = match &settings["half_width_m"] {
            Value::Null => None,
            v => Some(v.as_f64().filter(|h| *h > 0.0).ok_or_else(|| {
                DesignError::Input(
                    "the setting `half_width_m` must be a positive number of metres or null".into(),
                )
            })?),
        };
        Ok(SectionPlan {
            settings,
            profile,
            water_source,
            centre,
            model,
            shape,
            depth,
            river,
            fixed_hb,
        })
    }

    /// The station walk of one option: every cross-section, with its terrain, water level, design
    /// invert and cut face. The volumes stage sums these; the app draws them.
    pub(super) fn option_sections(
        &self,
        plan: &SectionPlan,
        o: &Value,
        terrain: Terrain,
    ) -> Result<cd_core::volumes::sections::SectionVolumes, DesignError> {
        let settings = &plan.settings;
        let id = o["id"].as_str().unwrap_or("?");
        let line: Vec<Pt> = all_positions(&o["line"]);
        let wl = self.water_level_along(&plan.profile, &line, &plan.centre);
        let level = Level::AtChainage(&wl);
        let shape = &plan.shape;
        let from_shape = |xy: &[Pt]| widened_hb_from_shape(xy, shape, WIDEN_RMAX);
        let half_width = match plan.fixed_hb {
            Some(h) => HalfWidth::Fixed(h),
            None => HalfWidth::FromStations(&from_shape),
        };
        section_volumes(
            &line,
            terrain,
            &plan.river,
            &level,
            plan.depth,
            &half_width,
            &plan.model,
            setting(settings, "wet_slope")?,
            setting(settings, "station_m")?,
            setting(settings, "maxw_m")?,
            setting(settings, "dx_m")?,
            None,
            None,
            cd_core::volumes::sections::DEFAULT_CHUNK,
        )
        .map_err(|e| DesignError::Input(format!("option `{id}`: {e}")))
    }
}
