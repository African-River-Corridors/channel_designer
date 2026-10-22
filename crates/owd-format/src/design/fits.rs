//! The `fits` part of the document (spec 0012, tickets T8 and T9): the answer to "what vessel fits
//! this reach as it is?". Plan only (T8) reads width and radius. Full (T9), when the document has a
//! bed model, a water level and a DEM, adds depth and the spec 0007 verdict: each class is sized,
//! walked along the river centreline and screened by the spec 0007 path itself
//! ([`Session::screening_value`]), so `fits` adds no rule logic of its own. It is not a stage. It is never rerun by `run_stale`: a change to
//! the banks, the centreline or the project CRS makes it stale, and only an explicit run
//! (`owd fits`) replaces it. Its status is worked out on load from its recorded dependency hash.

/// A DEM sampler that the classes of a full answer can share across threads.
pub type SyncTerrain<'a> = &'a (dyn Fn(&[f64], &[f64]) -> Vec<f64> + Sync);

/// Run `f` on every job and give the answers in the jobs' order: on as many threads as the machine
/// has when `parallel` (native), or one after another (always in wasm, which has no threads here).
fn run_all<J: Sync, T: Send>(jobs: &[J], f: impl Fn(&J) -> T + Sync, parallel: bool) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    if parallel {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(jobs.len().max(1));
        let next = AtomicUsize::new(0);
        let done: Vec<std::sync::Mutex<Option<T>>> =
            jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(k) else { break };
                        let answer = f(job);
                        *done[k].lock().expect("no worker panicked") = Some(answer);
                    }
                });
            }
        });
        done.into_iter()
            .map(|m| {
                m.into_inner()
                    .expect("no worker panicked")
                    .expect("every job ran")
            })
            .collect()
    } else {
        jobs.iter().map(f).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = parallel;
        jobs.iter().map(f).collect()
    }
}
use super::hash::hash_value;
use super::session::{DesignError, ENGINE_VERSION, Session, StageStatus};
use owd_core::fits::{ClassFit, Fits, PLAN_ONLY_LABEL, plan_only_fits};
use owd_core::rules::engine::old_name_choice;
use serde_json::{Value, json};

/// The label of a full answer: what it read, and what it assumed.
pub const FULL_LABEL: &str = "width, radius and depth; depth assumed, not measured";

/// The Rule Set choice (an old ruleset name) a run uses when none is named.
pub const DEFAULT_FITS_CHOICE: &str = "PIANC-B";

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

/// The Rule Set and set input an old name stands for, so a caller can size to it with
/// `set_rule_sets`. CN-reduced needs a canal reach as well, which this does not say.
fn selection_json(name: &str) -> Value {
    match old_name_choice(name) {
        Ok((set, input, value)) => json!({
            "rule_sets": [set],
            "set_inputs": [{"name": input, "value": value}],
        }),
        Err(_) => Value::Null,
    }
}

fn fits_json(f: &Fits) -> Value {
    json!({
        "label": f.label,
        "length_m": f.length_m,
        "rule_sets": f.rule_sets.iter().map(|rs| json!({
            "rule_set": rs.rule_set,
            "selection": selection_json(&rs.rule_set),
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

    /// The Rule Set choices (old ruleset names) the last run (or the next one) uses.
    pub fn fits_choices(&self) -> Vec<String> {
        self.doc["fits"]["settings"]["rule_sets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec![DEFAULT_FITS_CHOICE.to_string()])
    }

    /// Run the plan-only answer over every vessel class, under each of `choices` (the ones the
    /// document holds, else PIANC-B, when `None`). It reads the river polygon and the river
    /// centreline (or the line midway between the banks). Replaces the answer the document holds.
    pub fn run_fits(&mut self, choices: Option<&[String]>) -> Result<(), DesignError> {
        self.run_fits_with(choices, None)
    }

    /// [`Session::run_fits`] with the DEM. With a bed model and a water level too, the answer is
    /// full: each class row also carries the spec 0007 verdict and constraint stretches (depth among
    /// them, with the works and the dredge and dry cut of each), and says the depth is assumed.
    pub fn run_fits_with(
        &mut self,
        choices: Option<&[String]>,
        terrain: Option<SyncTerrain>,
    ) -> Result<(), DesignError> {
        let ids: Vec<String> = match choices {
            Some(r) if !r.is_empty() => r.to_vec(),
            _ => self.fits_choices(),
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
            self.add_full(&mut results, &line, t, true)?;
        }
        self.doc["fits"] = json!({
            "settings": {"rule_sets": ids},
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
        terrain: SyncTerrain,
        parallel: bool,
    ) -> Result<(), DesignError> {
        let option = json!({
            "id": "river",
            "line": {"type": "LineString", "coordinates": centre},
            // The line is the river itself: a bend tighter than the minimum radius is a stretch
            // that needs a cut-through, not a line that cannot be built.
            "checks": {"radius_ok": true},
        });
        let note = self.depth_note().unwrap_or("");
        // One job per class under each Rule Set choice. Each runs on its own copy of the design, so the
        // jobs share nothing but read-only inputs and can run at once; the answer is written back
        // in the same order whatever order they finish in.
        let jobs: Vec<(usize, usize, String, String)> = results["rule_sets"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .flat_map(|(i, rs)| {
                let id = rs["rule_set"].as_str().unwrap_or_default().to_string();
                rs["classes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(move |(j, c)| {
                        let class = c["vessel_standard_id"].as_str().unwrap_or_default();
                        (i, j, id.clone(), class.to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let screen = |id: &str, class: &str| -> Result<Value, DesignError> {
            // The old name's selection, as the plan-only answer sizes it: the reaches of the
            // document do not apply; CN-reduced is one claimed canal reach over the whole line.
            let (set, input, value) =
                old_name_choice(id).map_err(|e| DesignError::Input(e.raw_message().into()))?;
            let mut sized = self.clone();
            if let Some(o) = sized.doc["inputs"].as_object_mut() {
                o.remove("waterway_reaches");
            }
            if id == "CN-reduced" {
                sized.add_waterway_reach(0.0, None, "canal", true)?;
            }
            sized.set_rule_sets(&[set], &[(input, value)])?;
            sized.set_vessel(class, None)?;
            let plan = sized.section_plan(&json!({}))?;
            let walk = sized.option_sections(&plan, &option, terrain)?;
            sized.screening_value(&option, &walk, &plan.river, &plan.centre)
        };
        let answers = run_all(&jobs, |(_, _, id, class)| screen(id, class), parallel);
        for ((i, j, _, _), screening) in jobs.iter().zip(answers) {
            let screening = screening?;
            let c = &mut results["rule_sets"][*i]["classes"][*j];
            let as_is = screening["verdict"] == "navigable-as-is";
            c["fits"] = json!(as_is);
            c["verdict"] = screening["verdict"].clone();
            c["constraint_stretches"] = screening["constraint_stretches"].clone();
            c["blocking_not_evaluated"] = screening["blocking_not_evaluated"].clone();
            c["stretch_count"] = json!(c["constraint_stretches"].as_array().map_or(0, Vec::len));
            c["label"] = json!(FULL_LABEL);
            c["depth_basis"] = json!(note);
        }
        for rs in results["rule_sets"].as_array_mut().into_iter().flatten() {
            let largest = rs["classes"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .find(|c| c["fits"] == true)
                .map(|c| c["vessel_standard_id"].clone());
            rs["largest_fitting"] = largest.unwrap_or(Value::Null);
        }
        results["mode"] = json!("full");
        results["label"] = json!(FULL_LABEL);
        results["notice"] = json!(super::screening::SCREENING_NOTICE);
        results["rules_notice"] = json!(super::screening::UNVERIFIED_NOTICE);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::design::Role;

    #[test]
    fn the_answers_come_back_in_the_jobs_order_whatever_order_they_finish_in() {
        let jobs: Vec<u64> = (0..40).collect();
        // Later jobs finish first.
        let slow = |k: &u64| {
            std::thread::sleep(std::time::Duration::from_millis(40 - *k));
            k * k
        };
        assert_eq!(
            run_all(&jobs, slow, true),
            jobs.iter().map(|k| k * k).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_classes_run_at_once_give_the_answer_they_give_one_after_another() {
        const UTM30: &str =
            r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
        let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
        let mut s = Session::new("t", "2026-10-22");
        let polygon = format!(
            r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
            json!([
                at(0.0, -150.0),
                at(2000.0, -150.0),
                at(2000.0, 150.0),
                at(0.0, 150.0),
                at(0.0, -150.0)
            ])
        );
        // A bend, so the width and radius parts of the walk are not trivial.
        let centre = format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!([
                at(0.0, 0.0),
                at(800.0, 0.0),
                at(1000.0, 40.0),
                at(1200.0, 0.0),
                at(2000.0, 0.0)
            ])
        );
        s.import(&[
            (Role::RiverPolygon, polygon.as_str()),
            (Role::Centreline, centre.as_str()),
        ])
        .unwrap();
        s.set_water_profile(&[[0.0, 10.0]]).unwrap();
        s.set_bed_model(
            &[[0.0, 4.0], [900.0, 1.8], [1100.0, 4.0]],
            Some("dry"),
            "2026-10-22",
        )
        .unwrap();
        let land = |xs: &[f64], _: &[f64]| vec![12.0; xs.len()];
        let line = s.river_centreline().unwrap();
        let base = {
            let mut probe = s.clone();
            probe.run_fits(None).unwrap();
            probe.doc["fits"]["results"].clone()
        };
        let (mut a, mut b) = (base.clone(), base);
        s.add_full(&mut a, &line, &land, true).unwrap();
        s.add_full(&mut b, &line, &land, false).unwrap();
        assert_eq!(a.to_string(), b.to_string());
        assert_eq!(a["mode"], "full");
    }
}
