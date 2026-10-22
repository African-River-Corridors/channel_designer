//! The reference line in the study (spec 0016 T1): `inputs.reference_line`. One smooth line in the
//! water that belongs to no vessel (see `owd_core::alignment::reference_line`), made from the two
//! banks and the river polygon. It records what it was made from as a hash (`depends_on`), so a
//! change to the banks, the polygon or the project CRS makes it stale. The stages do not read it:
//! adding it leaves them fresh.

use super::alignment::pts_of;
use super::hash::hash_value;
use super::session::{DesignError, Session, StageStatus};
use owd_core::alignment::reference_line::{
    CHAIKIN_PASSES, CLEARANCE_MIN_M, CLEARANCE_SHARE, END_TRIM_M, REFIT_CHORD_M, REFIT_SCALE_M,
    SEED_MAX_STEPS_AHEAD, SEED_PULL_WEIGHT, SEED_STEP_M, TAUBIN_ITERATIONS, TAUBIN_LAMBDA,
    TAUBIN_MU, carry_by_chainage, reference_line,
};
use owd_core::geom::Pt;
use serde_json::{Value, json};

/// The settings the line is made with, as the study records them.
pub fn reference_line_parameters() -> Value {
    json!({
        "seed_step_m": SEED_STEP_M,
        "seed_max_steps_ahead": SEED_MAX_STEPS_AHEAD,
        "chaikin_passes": CHAIKIN_PASSES,
        "taubin": {"lambda": TAUBIN_LAMBDA, "mu": TAUBIN_MU, "iterations": TAUBIN_ITERATIONS},
        "clearance": {"min_m": CLEARANCE_MIN_M, "share_of_width": CLEARANCE_SHARE},
        "seed_pull_weight": SEED_PULL_WEIGHT,
        "refit_scale_m": REFIT_SCALE_M,
        "refit_chord_m": REFIT_CHORD_M,
        "end_trim_m": END_TRIM_M,
    })
}

impl Session {
    /// What the reference line is made from, hashed: the banks, the river polygon, the project
    /// CRS and the settings.
    fn reference_line_basis(&self) -> String {
        let inp = &self.doc["inputs"];
        hash_value(&json!({
            "left_bank": inp["left_bank"],
            "right_bank": inp["right_bank"],
            "river_polygon": inp["river_polygon"],
            "crs": self.doc["project"]["crs"],
            "parameters": reference_line_parameters(),
        }))
    }

    /// Where the stored reference line stands: not made, fresh, or stale.
    pub fn reference_line_status(&self) -> StageStatus {
        let r = &self.doc["inputs"]["reference_line"];
        if !r.is_object() {
            return StageStatus::NotRun;
        }
        if r["depends_on"].as_str() == Some(self.reference_line_basis().as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    /// The stored reference line, fresh or not.
    pub fn reference_line_value(&self) -> Option<&Value> {
        let r = &self.doc["inputs"]["reference_line"];
        r.is_object().then_some(r)
    }

    /// The stored reference line's points, fresh or not.
    pub fn reference_line(&self) -> Option<Vec<Pt>> {
        self.reference_line_value().and_then(|r| pts_of(&r["line"]))
    }

    /// Make the reference line from the banks and the river polygon and store it, with the banks'
    /// low-confidence stretches carried onto it by position.
    pub fn run_reference_line(&mut self) -> Result<(), DesignError> {
        let inp = &self.doc["inputs"];
        let (Some(left), Some(right)) = (pts_of(&inp["left_bank"]), pts_of(&inp["right_bank"]))
        else {
            return Err(DesignError::Input(
                "the reference line needs both bank lines: import them first".into(),
            ));
        };
        let water = self.river_polygons().ok_or_else(|| {
            DesignError::Input(
                "the reference line needs the river polygon: import it first (`owd import --polygon FILE`)"
                    .into(),
            )
        })?;
        let rl = reference_line(&left, &right, &water)
            .map_err(|e| DesignError::Input(format!("the reference line: {}", e.message)))?;
        // The low-confidence stretches are by chainage on the study's centreline.
        let low: Vec<Value> = match (
            self.low_confidence_width(),
            pts_of(&self.doc["inputs"]["centreline"]),
        ) {
            (Some(lc), Some(centre)) => {
                let st: Vec<&Value> = lc["stretches"].as_array().into_iter().flatten().collect();
                let ranges: Vec<(f64, f64)> = st
                    .iter()
                    .map(|s| {
                        (
                            s["from_m"].as_f64().unwrap_or(0.0),
                            s["to_m"].as_f64().unwrap_or(0.0),
                        )
                    })
                    .collect();
                // Carried one by one, so each keeps its kind.
                st.iter()
                    .zip(&ranges)
                    .flat_map(|(s, r)| {
                        carry_by_chainage(&centre, &rl.line, &[*r])
                            .into_iter()
                            .map(|(a, b)| json!({"from_m": a, "to_m": b, "kind": s["kind"]}))
                    })
                    .collect()
            }
            _ => vec![],
        };
        let mut low = low;
        low.sort_by(|a, b| {
            a["from_m"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&b["from_m"].as_f64().unwrap_or(0.0))
        });
        let outside: Vec<Value> = rl
            .outside_water
            .iter()
            .map(|s| json!({"from_m": s.start_chainage_m, "to_m": s.end_chainage_m}))
            .collect();
        self.doc["inputs"]["reference_line"] = json!({
            "line": {"type": "LineString", "coordinates": rl.line},
            "parameters": reference_line_parameters(),
            "depends_on": self.reference_line_basis(),
            "low_confidence": low,
            "outside_water": outside,
            "notes": rl.notes,
        });
        Ok(())
    }

    /// The reference line, made again first when it is missing or stale.
    pub fn fresh_reference_line(&mut self) -> Result<Vec<Pt>, DesignError> {
        if self.reference_line_status() != StageStatus::Fresh {
            self.run_reference_line()?;
        }
        self.reference_line()
            .ok_or_else(|| DesignError::Input("the reference line was not stored".into()))
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::design::{Role, Stage};

    const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

    fn at(x: f64, y: f64) -> [f64; 2] {
        [x + 500_000.0, y + 600_000.0]
    }

    fn line(pts: &[[f64; 2]]) -> String {
        format!(r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#, json!(pts))
    }

    /// A straight river 100 m wide and 2 km long, with its banks, polygon and centreline.
    fn study(shift: f64) -> Session {
        let mut s = Session::new("t", "2026-10-22");
        let left: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, 50.0 + shift)).collect();
        let right: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, -50.0)).collect();
        let centre: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, shift / 2.0)).collect();
        let mut ring = right.clone();
        ring.extend(left.iter().rev());
        ring.push(ring[0]);
        let polygon = format!(
            r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
            json!(ring)
        );
        s.import(&[
            (Role::RiverPolygon, polygon.as_str()),
            (Role::LeftBank, line(&left).as_str()),
            (Role::RightBank, line(&right).as_str()),
            (Role::Centreline, line(&centre).as_str()),
        ])
        .unwrap();
        s
    }

    #[test]
    fn it_is_stored_and_reopens_fresh() {
        let mut s = study(0.0);
        assert_eq!(s.reference_line_status(), StageStatus::NotRun);
        s.run_reference_line().unwrap();
        let back = Session::open(&s.to_text()).unwrap();
        assert_eq!(back.reference_line_status(), StageStatus::Fresh);
        let l = back.reference_line().unwrap();
        assert!(l.iter().all(|p| (p[1] - 600_000.0).abs() < 0.5));
        assert_eq!(
            back.reference_line_value().unwrap()["parameters"]["seed_pull_weight"],
            json!(SEED_PULL_WEIGHT)
        );
    }

    #[test]
    fn a_change_to_the_banks_makes_it_stale() {
        let mut s = study(0.0);
        s.run_reference_line().unwrap();
        let moved: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, 60.0)).collect();
        s.import(&[(Role::LeftBank, line(&moved).as_str())]).unwrap();
        assert_eq!(s.reference_line_status(), StageStatus::Stale);
        let l = s.fresh_reference_line().unwrap();
        assert_eq!(s.reference_line_status(), StageStatus::Fresh);
        // Now midway between y = -50 and y = 60 (the polygon still ends at 50: the clearance holds
        // the line inside it).
        assert!(l.iter().all(|p| p[1] - 600_000.0 > 0.0));
    }

    #[test]
    fn storing_it_leaves_the_stages_as_they_were() {
        let mut s = study(0.0);
        let before: Vec<String> = Stage::ALL.iter().map(|st| s.dependency_hash(*st)).collect();
        s.run_reference_line().unwrap();
        let after: Vec<String> = Stage::ALL.iter().map(|st| s.dependency_hash(*st)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn low_confidence_stretches_are_carried_onto_it() {
        let mut s = study(0.0);
        s.set_low_confidence_width(json!({"min_width_m": 90.0, "stretches": [
            {"from_m": 500.0, "to_m": 700.0, "kind": "narrow"}]}))
            .unwrap();
        s.run_reference_line().unwrap();
        let low = &s.reference_line_value().unwrap()["low_confidence"];
        assert_eq!(low[0]["kind"], "narrow");
        // The line starts `END_TRIM_M` along the centreline: the stretch keeps its place.
        assert!((low[0]["from_m"].as_f64().unwrap() - (500.0 - END_TRIM_M)).abs() < 1.0);
        assert!((low[0]["to_m"].as_f64().unwrap() - (700.0 - END_TRIM_M)).abs() < 1.0);
    }
}
