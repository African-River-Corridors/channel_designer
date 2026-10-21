//! The `fits` part of the document (spec 0012, tickets T8 and T9): the answer to "what vessel fits
//! this reach as it is?". Plan only (T8) reads width and radius. Full (T9), when the document has a
//! bed model, a water level and a DEM, adds depth and the spec 0007 verdict: each class is sized,
//! walked along the river centreline and screened by the spec 0007 path itself
//! ([`Session::screening_value`]), so `fits` adds no rule logic of its own. It is not a stage. It is never rerun by `run_stale`: a change to
//! the banks, the centreline or the project CRS makes it stale, and only an explicit run
//! (`owd fits`) replaces it. Its status is worked out on load from its recorded dependency hash.

use super::alignment::Terrain;
use super::hash::hash_value;
use super::session::{DesignError, ENGINE_VERSION, Session, StageStatus};
use owd_core::fits::{ClassFit, Fits, PLAN_ONLY_LABEL, plan_only_fits};
use serde_json::{Value, json};

/// The label of a full answer: what it read, and what it assumed.
pub const FULL_LABEL: &str = "width, radius and depth; depth assumed, not measured";

/// The ruleset a run uses when none is named.
pub const DEFAULT_FITS_RULESET: &str = "PIANC-B";

fn class_json(r: &ClassFit) -> Value {
    json!({
        "vessel_standard_id": r.vessel_standard_id,
        "class_name": r.class_name,
        "family": r.family,
        "formation": r.formation,
        "loa_m": r.loa_m,
        "beam_m": r.beam_m,
        "r_min_m": r.r_min_m,
        "bottom_width_straight_m": r.bottom_width_straight_m,
        "fits": r.fits,
        "worst_width_deficit_m": r.worst_width_deficit_m,
        "worst_radius_deficit_m": r.worst_radius_deficit_m,
        "stretch_count": r.stretches.len(),
        "stretches": r.stretches.iter().map(|s| json!({
            "kind": s.kind.as_str(),
            "start_chainage_m": s.start_chainage_m,
            "end_chainage_m": s.end_chainage_m,
            "start": s.start,
            "end": s.end,
            "required_m": s.required_m,
            "found_m": s.found_m,
            "deficit_m": s.deficit_m,
        })).collect::<Vec<_>>(),
        "warnings": r.warnings,
        // Every row says what it did not read.
        "label": PLAN_ONLY_LABEL,
    })
}

fn fits_json(f: &Fits) -> Value {
    json!({
        "label": f.label,
        "length_m": f.length_m,
        "rulesets": f.rulesets.iter().map(|rs| json!({
            "ruleset_id": rs.ruleset_id,
            "largest_fitting": rs.largest_fitting,
            "classes": rs.rows.iter().map(class_json).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

impl Session {
    /// What the answer depends on: the water and the line it reads, the project CRS, and the
    /// settings. Not the sizing, the DEM or any stage.
    fn fits_dependency_hash(&self) -> String {
        let inp = &self.doc["inputs"];
        // A full answer also read the bed, the water and the DEM; a plan-only one did not, and its
        // hash stays what it was.
        let full = (self.doc["fits"]["results"]["mode"] == "full").then(|| {
            json!({
                "bed_model": inp["bed_model"],
                "water_level": inp["water_level"],
                "dem": inp["dem"]["sha256"],
                "sizing": self.doc["stages"]["sizing"]["settings"]["rule_sets"],
            })
        });
        hash_value(&json!({
            "full": full,
            "left_bank": inp["left_bank"],
            "right_bank": inp["right_bank"],
            "centreline": inp["centreline"],
            "river_polygon": inp["river_polygon"],
            "crs": self.doc["project"]["crs"],
            "settings": self.doc["fits"]["settings"],
        }))
    }

    /// Where the answer stands: not run, fresh, or stale.
    pub fn fits_status(&self) -> StageStatus {
        let f = &self.doc["fits"];
        if f["results"].is_null() {
            return StageStatus::NotRun;
        }
        if f["depends_on"].as_str() == Some(self.fits_dependency_hash().as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    /// The answer the document holds, if it has run.
    pub fn fits_result(&self) -> Option<&Value> {
        let r = &self.doc["fits"]["results"];
        (!r.is_null()).then_some(r)
    }

    /// The rulesets the last run (or the next one) uses.
    pub fn fits_rulesets(&self) -> Vec<String> {
        self.doc["fits"]["settings"]["rulesets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec![DEFAULT_FITS_RULESET.to_string()])
    }

    /// Run the plan-only answer over every vessel class, under each of `rulesets` (the ones the
    /// document holds, else PIANC-B, when `None`). It reads the river polygon and the river
    /// centreline (or the line midway between the banks). Replaces the answer the document holds.
    pub fn run_fits(&mut self, rulesets: Option<&[String]>) -> Result<(), DesignError> {
        self.run_fits_with(rulesets, None)
    }

    /// [`Session::run_fits`] with the DEM. With a bed model and a water level too, the answer is
    /// full: each class row also carries the spec 0007 verdict and constraint stretches (depth among
    /// them, with the works and the dredge and dry cut of each), and says the depth is assumed.
    pub fn run_fits_with(
        &mut self,
        rulesets: Option<&[String]>,
        terrain: Option<Terrain>,
    ) -> Result<(), DesignError> {
        let ids: Vec<String> = match rulesets {
            Some(r) if !r.is_empty() => r.to_vec(),
            _ => self.fits_rulesets(),
        };
        let water = self.river_polygons().ok_or_else(|| {
            DesignError::Input(
                "the answer needs the river polygon: import it first (`owd import --polygon FILE`)"
                    .into(),
            )
        })?;
        let line = self.river_centreline()?;
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let fits =
            plan_only_fits(&line, &water, &refs).map_err(|e| DesignError::Input(e.message))?;
        let mut results = fits_json(&fits);
        if let Some(t) =
            terrain.filter(|_| self.bed_model_value().is_some() && self.water_profile().is_some())
        {
            self.add_full(&mut results, &line, t)?;
        }
        self.doc["fits"] = json!({
            "settings": {"rulesets": ids},
            "results": results,
            "depends_on": null,
            "engine_version": ENGINE_VERSION,
        });
        let h = self.fits_dependency_hash();
        self.doc["fits"]["depends_on"] = json!(h);
        Ok(())
    }

    /// Make a plan-only answer full: screen each class along the river centreline by the spec 0007
    /// path, on a copy of the design sized for that class.
    fn add_full(
        &self,
        results: &mut Value,
        centre: &[[f64; 2]],
        terrain: Terrain,
    ) -> Result<(), DesignError> {
        let option = json!({
            "id": "river",
            "line": {"type": "LineString", "coordinates": centre},
            // The line is the river itself: a bend tighter than the minimum radius is a stretch
            // that needs a cut-through, not a line that cannot be built.
            "checks": {"radius_ok": true},
        });
        let note = self.depth_note().unwrap_or("");
        for rs in results["rulesets"].as_array_mut().into_iter().flatten() {
            let id = rs["ruleset_id"].as_str().unwrap_or_default().to_string();
            let mut largest: Option<String> = None;
            for c in rs["classes"].as_array_mut().into_iter().flatten() {
                let class = c["vessel_standard_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let mut sized = self.clone();
                sized.set_sizing(&id, &class, None)?;
                let plan = sized.section_plan(&json!({}))?;
                let walk = sized.option_sections(&plan, &option, terrain)?;
                let screening = sized.screening_value(&option, &walk, &plan.river, &plan.centre)?;
                let as_is = screening["verdict"] == "navigable-as-is";
                c["fits"] = json!(as_is);
                c["verdict"] = screening["verdict"].clone();
                c["constraint_stretches"] = screening["constraint_stretches"].clone();
                c["blocking_not_evaluated"] = screening["blocking_not_evaluated"].clone();
                c["stretch_count"] =
                    json!(c["constraint_stretches"].as_array().map_or(0, Vec::len));
                c["label"] = json!(FULL_LABEL);
                c["depth_basis"] = json!(note);
                if as_is {
                    largest = Some(class);
                }
            }
            rs["largest_fitting"] = json!(largest);
        }
        results["mode"] = json!("full");
        results["label"] = json!(FULL_LABEL);
        results["notice"] = json!(super::screening::SCREENING_NOTICE);
        results["rules_notice"] = json!(super::screening::UNVERIFIED_NOTICE);
        Ok(())
    }
}
