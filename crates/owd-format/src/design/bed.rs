//! The bed model of the document (spec 0012, ticket T7): the depth profile a user gives when there
//! is no survey. The calculation is `owd_core::bed`; here are the input, the label and the terrain
//! the volumes read. The depth is assumed, not measured, and every output that used it says so.

use super::alignment::Terrain;
use super::session::{DesignError, Session};
use super::water::level_at;
use owd_core::bed::BedModel;
use owd_core::geom::Pt;
use serde_json::{Value, json};

pub use owd_core::bed::DEPTH_ASSUMED_LABEL;

impl Session {
    /// Set the bed model: `points` of `(river chainage in metres, depth in metres)` along the river
    /// centreline (one point is a constant depth), and a season label. It replaces a bed model
    /// the document holds; nothing else in the document changes, and the volumes show as stale.
    pub fn set_bed_model(
        &mut self,
        points: &[[f64; 2]],
        season: Option<&str>,
    ) -> Result<(), DesignError> {
        if points.is_empty() {
            return Err(DesignError::Input(
                "a bed model needs at least one depth".into(),
            ));
        }
        if points
            .iter()
            .any(|p| !(p[0].is_finite() && p[1].is_finite()))
        {
            return Err(DesignError::Input(
                "every point of the depth profile must be a finite chainage and depth".into(),
            ));
        }
        if points.iter().any(|p| p[1] < 0.0) {
            return Err(DesignError::Input("a depth must not be negative".into()));
        }
        if points.windows(2).any(|w| w[1][0] < w[0][0]) {
            return Err(DesignError::Input(
                "the chainages of the depth profile must not decrease".into(),
            ));
        }
        self.doc["inputs"]["bed_model"] = json!({
            "points": points,
            "shape": "parabola",
            "season": season,
        });
        Ok(())
    }

    /// Take the bed model out, as a survey does when it replaces it. Nothing else changes.
    pub fn clear_bed_model(&mut self) {
        if let Some(inputs) = self.doc["inputs"].as_object_mut() {
            inputs.remove("bed_model");
        }
    }

    /// The bed model as the document holds it.
    pub fn bed_model_value(&self) -> Option<&Value> {
        let b = &self.doc["inputs"]["bed_model"];
        b.is_object().then_some(b)
    }

    /// The text every output carries while a bed model is used; `None` when the document has none.
    pub fn depth_note(&self) -> Option<&'static str> {
        self.bed_model_value().map(|_| DEPTH_ASSUMED_LABEL)
    }

    /// `v` (an object) with `depth_basis` set while the document has a bed model. A document with
    /// none gets `v` back as it was.
    pub(super) fn with_depth_basis(&self, mut v: Value) -> Value {
        if let (Some(note), Some(o)) = (self.depth_note(), v.as_object_mut()) {
            o.insert("depth_basis".into(), json!(note));
        }
        v
    }

    /// The bed model on the river, ready to read; `None` when the document has none. It needs the
    /// river polygon and the river centreline.
    pub(super) fn bed_on_river(&self) -> Result<Option<BedModel>, DesignError> {
        let Some(b) = self.bed_model_value() else {
            return Ok(None);
        };
        let table: Vec<[f64; 2]> = b["points"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                    .collect()
            })
            .unwrap_or_default();
        let water = self.river_polygons().ok_or_else(|| {
            DesignError::Input(
                "the bed model needs the river polygon: import it first (`owd import --polygon FILE`)"
                    .into(),
            )
        })?;
        let centre = self.river_centreline()?;
        BedModel::new(&table, &centre, &water)
            .map(Some)
            .map_err(|e| DesignError::Input(e.message))
    }

    /// The water level the bed hangs from: the river's own profile (not the pools of the optimise
    /// stage, which raise the water above the natural low water).
    pub(super) fn bed_water_profile(&self) -> Option<Vec<[f64; 2]>> {
        self.input_water_profile()
            .or_else(|| self.water_profile().map(|(p, _)| p))
    }
}

/// `terrain` with the bed model laid over it: inside the water the bed is the water level less the
/// depth; elsewhere it is the DEM.
pub(super) fn with_bed<'a>(
    bed: &'a BedModel,
    level: &'a [[f64; 2]],
    terrain: Terrain<'a>,
) -> impl Fn(&[f64], &[f64]) -> Vec<f64> + 'a {
    move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        let dem = terrain(xs, ys);
        xs.iter()
            .zip(ys)
            .zip(dem)
            .map(|((&x, &y), z)| {
                let p: Pt = [x, y];
                let wl = level_at(level, bed.chainage_of(p));
                bed.bed_elevation(p, wl).unwrap_or(z)
            })
            .collect()
    }
}
