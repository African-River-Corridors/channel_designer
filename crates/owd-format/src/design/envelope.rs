//! The largest vessel the river allows (spec 0016 T2): `stages.envelope`. A sweep of single-hull
//! sizes on the reference line (see `owd_core::envelope`), kept in the study. Like `fits`, it is
//! never rerun by `run_stale`: a change to the banks, the river polygon, the reference line, the
//! custom vessel or the settings makes it stale, and only `owd envelope` replaces it.

use super::hash::hash_value;
use super::screening::SCREENING_NOTICE;
use super::session::{DesignError, ENGINE_VERSION, Session, StageStatus};
use owd_core::envelope::{Envelope, Settings, Size, combine, size_result};
use owd_core::geom::Pt;
use serde_json::{Value, json};

/// Said on every output: the sweep reads width and radius on lines fitted in the banks as found.
pub const BANK_CAVEAT: &str = "the answer is only as good as the banks: a narrow or flagged \
     stretch of bank can set the limit for the whole river (see the low-confidence stretches)";

/// Said on every output: what the sweep reads.
pub const ENVELOPE_LABEL: &str = "single-hull motor vessels; width and radius only, no depth";

fn size_json(s: &Size) -> Value {
    json!({"label": s.label(), "loa_m": s.loa_m, "beam_m": s.beam_m})
}

/// The answer as the study keeps it. `positions` gives the point at a chainage of the plan line.
fn envelope_json(e: &Envelope, at: &dyn Fn(f64) -> Pt) -> Value {
    let label = |i: Option<usize>| i.map_or(Value::Null, |i| size_json(&e.sizes[i].size));
    json!({
        "label": ENVELOPE_LABEL,
        "notice": SCREENING_NOTICE,
        "bank_caveat": BANK_CAVEAT,
        "rule_set": e.settings.rule_set,
        "draught_m": e.settings.draught_m,
        "clearance_m": e.settings.clearance_m,
        "length_m": e.length_m,
        "largest": label(e.largest),
        "sizes": e.sizes.iter().map(|s| json!({
            "label": s.size.label(),
            "loa_m": s.size.loa_m,
            "beam_m": s.size.beam_m,
            "r_min_m": s.r_min_m,
            "bottom_width_straight_m": s.bottom_width_straight_m,
            "passes": s.passes,
            "stretch_count": s.stretches.len(),
            "stretches": s.stretches.iter().map(|st| json!({
                "kind": st.kind.as_str(),
                "start_chainage_m": st.start_chainage_m,
                "end_chainage_m": st.end_chainage_m,
                "start": st.start,
                "end": st.end,
                "required_m": st.required_m,
                "found_m": st.found_m,
                "deficit_m": st.deficit_m,
            })).collect::<Vec<_>>(),
            "notes": s.notes,
        })).collect::<Vec<_>>(),
        "per_km": e.per_km.iter().map(|k| json!({
            "from_m": k.from_m,
            "to_m": k.to_m,
            "from": at(k.from_m),
            "to": at(k.to_m),
            "largest": label(k.largest),
        })).collect::<Vec<_>>(),
        "marks": e.marks.iter().map(|m| json!({
            "id": m.id,
            "custom": m.custom,
            "loa_m": m.loa_m,
            "beam_m": m.beam_m,
            "within": m.within,
        })).collect::<Vec<_>>(),
    })
}

impl Session {
    /// What the sweep depends on: the banks, the river polygon, the CRS, the reference line, the
    /// custom vessel and the settings.
    fn envelope_dependency_hash(&self) -> String {
        let inp = &self.doc["inputs"];
        hash_value(&json!({
            "left_bank": inp["left_bank"],
            "right_bank": inp["right_bank"],
            "river_polygon": inp["river_polygon"],
            "centreline": inp["centreline"],
            "crs": self.doc["project"]["crs"],
            "reference_line": inp["reference_line"]["depends_on"],
            "custom_vessel": self.doc["stages"]["sizing"]["settings"]["custom_vessel"],
            "settings": self.doc["stages"]["envelope"]["settings"],
        }))
    }

    /// Where the sweep stands: not run, fresh, or stale.
    pub fn envelope_status(&self) -> StageStatus {
        let e = &self.doc["stages"]["envelope"];
        if e["results"].is_null() {
            return StageStatus::NotRun;
        }
        if e["depends_on"].as_str() == Some(self.envelope_dependency_hash().as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    /// The sweep the study holds, if it has run.
    pub fn envelope_result(&self) -> Option<&Value> {
        let r = &self.doc["stages"]["envelope"]["results"];
        (!r.is_null()).then_some(r)
    }

    /// Run the sweep on the reference line (made again first when missing or stale) and keep it.
    /// The sizes run at once when `parallel`. Without both banks, the centreline is read, and the
    /// answer says so.
    pub fn run_envelope(&mut self, settings: Settings, parallel: bool) -> Result<(), DesignError> {
        if settings.sizes.is_empty() {
            return Err(DesignError::Input(
                "the series needs at least one size".into(),
            ));
        }
        if !(settings.draught_m.is_finite() && settings.draught_m > 0.0) {
            return Err(DesignError::Input(
                "the draught must be a positive number".into(),
            ));
        }
        let water = self.river_polygons().ok_or_else(|| {
            DesignError::Input(
                "the sweep needs the river polygon: import it first (`owd import --polygon FILE`)"
                    .into(),
            )
        })?;
        let (line, plan_line, note) = match self.fresh_reference_line() {
            Ok(l) => (l, "reference", None),
            Err(e) => (
                self.river_centreline()?,
                "centreline",
                Some(format!(
                    "the reference line could not be made ({e}): the sizes are seeded from the centreline"
                )),
            ),
        };
        let answers = super::fits::run_all(
            &settings.sizes,
            |s| size_result(&line, &water, &settings, *s),
            parallel,
        );
        let sizes = answers
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DesignError::Input(format!("the sweep: {}", e.message)))?;
        let custom = self.custom_vessel();
        let e = combine(&line, settings.clone(), sizes, custom.as_ref());
        let chained = owd_core::geom::linear::length(&line);
        let at = |s: f64| {
            owd_core::geom::linear::interpolate(&line, s.min(chained)).unwrap_or([f64::NAN; 2])
        };
        let mut results = envelope_json(&e, &at);
        results["plan_line"] = json!(plan_line);
        if let Some(n) = note {
            results["plan_line_note"] = json!(n);
        }
        // The draught limit from the bed model, beside the answer: the sweep is plan only.
        if let Some(b) = self.bed_model_value() {
            let depths: Vec<f64> = b["points"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p[1].as_f64())
                .collect();
            let shallowest = depths.iter().copied().fold(f64::INFINITY, f64::min);
            if shallowest.is_finite() {
                results["bed_model"] = json!({
                    "shallowest_depth_m": shallowest,
                    "draught_m": settings.draught_m,
                    "note": "the depth is assumed, not measured, and not read by the sweep: \
                             compare the draught with it",
                });
            }
        }
        if !self.doc["stages"].is_object() {
            self.doc["stages"] = json!({});
        }
        self.doc["stages"]["envelope"] = json!({
            "settings": {
                "sizes": settings.sizes.iter().map(size_json).collect::<Vec<_>>(),
                "draught_m": settings.draught_m,
                "rule_set": settings.rule_set,
                "clearance_m": settings.clearance_m,
            },
            "results": results,
            "depends_on": null,
            "engine_version": ENGINE_VERSION,
        });
        let h = self.envelope_dependency_hash();
        self.doc["stages"]["envelope"]["depends_on"] = json!(h);
        Ok(())
    }
}
