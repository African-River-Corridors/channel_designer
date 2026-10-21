//! The `fits` part of the document (spec 0012, ticket T8): the plan-only answer to "what vessel
//! fits this reach as it is?". It is not a stage. It is never rerun by `run_stale`: a change to
//! the banks, the centreline or the project CRS makes it stale, and only an explicit run
//! (`owd fits`) replaces it. Its status is worked out on load from its recorded dependency hash.

use super::hash::hash_value;
use super::session::{DesignError, ENGINE_VERSION, Session, StageStatus};
use owd_core::fits::{ClassFit, Fits, PLAN_ONLY_LABEL, plan_only_fits};
use serde_json::{Value, json};

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
        hash_value(&json!({
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
        let results = fits_json(&fits);
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
}
