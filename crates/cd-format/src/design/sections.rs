//! The long section and the cross-section (spec 0003): the station walk of one design line option,
//! shaped for drawing. The walk is the one the volumes stage sums, with the same settings, so what
//! is drawn is what the table counts.

use super::alignment::Terrain;
use super::crs::{Crs, Utm};
use super::export::SCREENING_LABEL;
use super::session::{DesignError, Session, Stage, StageStatus};
use cd_core::geom::Pt;
use cd_core::volumes::sections::{CrossSection, SectionVolumes};
use serde_json::{Value, json};

impl Session {
    /// The station walk of the option `option_id`, on the DEM. Needs the volumes stage fresh: the
    /// walk must be the one whose sums the table shows.
    pub fn option_section_walk(
        &self,
        option_id: &str,
        terrain: Terrain,
    ) -> Result<SectionVolumes, DesignError> {
        match self.stage_status(Stage::Volumes) {
            StageStatus::Fresh => {}
            StageStatus::Stale => {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Volumes,
                    run_first: Stage::Volumes,
                });
            }
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "run the volumes stage first: the sections are drawn from its station walk"
                        .into(),
                ));
            }
        }
        let option = self.doc["options"]
            .as_array()
            .and_then(|a| a.iter().find(|o| o["id"] == option_id))
            .ok_or_else(|| DesignError::Input(format!("there is no option `{option_id}`")))?;
        let plan = self.section_plan(&Value::Null)?;
        self.option_sections(&plan, option, terrain)
    }

    fn utm(&self) -> Result<Utm, DesignError> {
        self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            })
            .ok_or_else(|| DesignError::Input("the channel design has no project CRS".into()))
    }

    /// The long section as series along the chainage of the option's line: the bed on the line,
    /// the water level (steps are the pools), the design invert and the cut depth below the bed,
    /// with the station's quantities and its place on the map (`[lon, lat]`).
    pub fn long_section(
        &self,
        option_id: &str,
        walk: &SectionVolumes,
    ) -> Result<Value, DesignError> {
        let xy: Vec<Pt> = walk.stations.iter().map(|s| [s.x, s.y]).collect();
        let positions = self.utm()?.inverse_all(&xy).map_err(DesignError::Input)?;
        let col = |f: &dyn Fn(&CrossSection) -> f64| -> Vec<Value> {
            walk.stations.iter().map(|s| json!(f(s))).collect()
        };
        Ok(json!({
            "screening": SCREENING_LABEL,
            "option_id": option_id,
            "station_m": walk.station_m,
            "chainage_m": col(&|s| s.chainage_m),
            "bed_m": col(&bed),
            "water_level_m": col(&|s| s.wl),
            "design_invert_m": col(&|s| s.grade),
            "cut_depth_m": col(&|s| (bed(s) - s.grade).max(0.0)),
            "dredge_area_m2": col(&|s| s.dredge_area_m2),
            "excav_area_m2": col(&|s| s.excav_area_m2),
            "max_face_h_m": col(&|s| s.max_face_h_m),
            "positions": positions,
        }))
    }

    /// The cross-section at the station nearest to `chainage_m`.
    pub fn cross_section(
        &self,
        walk: &SectionVolumes,
        chainage_m: f64,
    ) -> Result<Value, DesignError> {
        let s = nearest(walk, |s| (s.chainage_m - chainage_m).abs())?;
        let [lon, lat] = self.utm()?.inverse(s.x, s.y).map_err(DesignError::Input)?;
        Ok(json!({
            "screening": SCREENING_LABEL,
            "chainage_m": s.chainage_m,
            "position": [lon, lat],
            "offsets_m": s.offsets,
            "terrain_m": s.terrain,
            "wet": s.wet,
            "cut_face_m": s.cutface,
            "water_level_m": s.wl,
            "design_invert_m": s.grade,
            "half_width_m": s.hb,
            "depth_m": s.depth,
            "model": s.model,
            "dredge_area_m2": s.dredge_area_m2,
            "excav_area_m2": s.excav_area_m2,
            "max_face_h_m": s.max_face_h_m,
            "bench_left": s.bench_left,
            "bench_right": s.bench_right,
        }))
    }

    /// The chainage of the station nearest to a point picked on the map (`lon`, `lat`).
    pub fn station_near(
        &self,
        walk: &SectionVolumes,
        lon: f64,
        lat: f64,
    ) -> Result<f64, DesignError> {
        let [x, y] = self.utm()?.forward(lon, lat).map_err(DesignError::Input)?;
        nearest(walk, |s| libm::hypot(s.x - x, s.y - y)).map(|s| s.chainage_m)
    }
}

/// The terrain on the option's line: the sample at the offset nearest zero.
fn bed(s: &CrossSection) -> f64 {
    s.offsets
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or(f64::NAN, |(i, _)| s.terrain[i])
}

fn nearest(
    walk: &SectionVolumes,
    dist: impl Fn(&CrossSection) -> f64,
) -> Result<&CrossSection, DesignError> {
    walk.stations
        .iter()
        .filter(|s| dist(s).is_finite())
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .ok_or_else(|| DesignError::Input("the option has no stations".into()))
}
