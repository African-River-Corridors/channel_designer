//! The water level (spec 0002, ADR 0006): a profile along the river centreline. A design line's
//! station takes its level by projecting onto the river centreline and reading the profile at that
//! chainage, so two design lines through the same place see the same water.
//!
//! The profile is a table of `(river chainage, level)` points, read linearly between points and held
//! flat beyond the first and last. Two points at one chainage make a step (a weir): at that chainage
//! the later point holds. The optimise stage's pool levels are such a table and, once it has run,
//! they are the profile the volumes stage reads.

use super::session::{DesignError, Session, Stage};
use owd_core::geom::{Pt, linear};
use serde_json::{Value, json};

/// How a design line's station finds its place on the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaterSampling {
    /// Project the station onto the river centreline. The shipped way (ADR 0006).
    #[default]
    Projection,
    /// Read the profile at the design line's own chainage, as the frozen Python does. For tests of
    /// the port only: it is not offered by the CLI or the app.
    DesignLineChainage,
}

/// The level at `chainage_m` on a table of `(chainage, level)` points sorted by chainage.
pub fn level_at(points: &[[f64; 2]], chainage_m: f64) -> f64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return f64::NAN;
    };
    if chainage_m.is_nan() {
        return f64::NAN;
    }
    if chainage_m < first[0] {
        return first[1];
    }
    // The last point at or before the chainage: a step takes its later level.
    let i = points.partition_point(|p| p[0] <= chainage_m) - 1;
    if i + 1 >= points.len() {
        return last[1];
    }
    let (a, b) = (points[i], points[i + 1]);
    a[1] + (b[1] - a[1]) * (chainage_m - a[0]) / (b[0] - a[0])
}

fn points_of(v: &Value) -> Vec<[f64; 2]> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                .collect()
        })
        .unwrap_or_default()
}

impl Session {
    /// Read the profile at the stations of design lines by the test-only sampling of the frozen
    /// Python (the design line's own chainage). Not for the CLI or the app.
    #[doc(hidden)]
    pub fn sample_water_at_design_line_chainage(&mut self) {
        self.water_sampling = WaterSampling::DesignLineChainage;
    }

    /// Set the water level as a table of `(river chainage in metres, level in metres)` points. One
    /// point is a constant level. Chainages must not decrease.
    pub fn set_water_profile(&mut self, points: &[[f64; 2]]) -> Result<(), DesignError> {
        if points.is_empty() {
            return Err(DesignError::Input(
                "a water level profile needs at least one point".into(),
            ));
        }
        if points
            .iter()
            .any(|p| !(p[0].is_finite() && p[1].is_finite()))
        {
            return Err(DesignError::Input(
                "every point of the water level profile must be a finite chainage and level".into(),
            ));
        }
        if points.windows(2).any(|w| w[1][0] < w[0][0]) {
            return Err(DesignError::Input(
                "the chainages of the water level profile must not decrease".into(),
            ));
        }
        self.doc["inputs"]["water_level"] = json!({ "points": points });
        Ok(())
    }

    /// A straight fall: `start_m_level` at the river's start, falling `slope` per metre of river
    /// chainage to its end. Needs the river centreline (or both banks) to be imported first.
    pub fn set_water_fall(&mut self, start_level_m: f64, slope: f64) -> Result<(), DesignError> {
        let len = linear::length(&self.base_line()?);
        self.set_water_profile(&[[0.0, start_level_m], [len, start_level_m - slope * len]])
    }

    /// The water level the later stages read: the pool levels the optimise stage set, if it has run,
    /// else the profile of the inputs. The second value says which.
    pub fn water_profile(&self) -> Option<(Vec<[f64; 2]>, &'static str)> {
        let opt = &self.doc["stages"]["optimise"]["results"];
        if !opt.is_null() {
            let p = points_of(&opt["profile"]);
            if !p.is_empty() {
                return Some((p, "optimise"));
            }
        }
        self.input_water_profile().map(|p| (p, "input"))
    }

    /// The profile set by hand or by import.
    pub fn input_water_profile(&self) -> Option<Vec<[f64; 2]>> {
        let p = points_of(&self.doc["inputs"]["water_level"]["points"]);
        (!p.is_empty()).then_some(p)
    }

    /// The water level along a design line, as a function of the line's own chainage.
    pub(super) fn water_level_along<'a>(
        &self,
        profile: &'a [[f64; 2]],
        line: &'a [Pt],
        centre: &'a [Pt],
    ) -> impl Fn(f64) -> f64 + 'a {
        let mode = self.water_sampling;
        move |s| match mode {
            WaterSampling::DesignLineChainage => level_at(profile, s),
            WaterSampling::Projection => {
                let at = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
                level_at(profile, linear::project(centre, at))
            }
        }
    }

    /// The water level at stations of a design line, given as chainages along that line. It is the
    /// level the stages read: the same place gives the same level whatever line asks.
    pub fn water_levels_along(
        &self,
        line: &[Pt],
        chainages_m: &[f64],
    ) -> Result<Vec<f64>, DesignError> {
        let (profile, _) = self
            .water_profile()
            .ok_or_else(|| self.needs_water_profile(Stage::Volumes))?;
        let centre = self.river_centreline()?;
        let level = self.water_level_along(&profile, line, &centre);
        Ok(chainages_m.iter().map(|&s| level(s)).collect())
    }

    /// The pool levels of the optimise stage as a profile on the river centreline: each pool holds its
    /// level between the river chainages of its two ends, with a step where one pool meets the next.
    pub(super) fn pool_profile(
        &self,
        baseline: &[Pt],
        pools: &[(f64, f64, f64)],
    ) -> Result<Vec<[f64; 2]>, DesignError> {
        let centre = self.base_line()?;
        let at = |c: f64| {
            let p = linear::interpolate(baseline, c).unwrap_or([f64::NAN; 2]);
            linear::project(&centre, p)
        };
        let mut out: Vec<[f64; 2]> = Vec::new();
        for &(a, b, l) in pools {
            let ca = at(a).max(out.last().map_or(f64::NEG_INFINITY, |p| p[0]));
            let cb = at(b).max(ca);
            out.push([ca, l]);
            out.push([cb, l]);
        }
        Ok(out)
    }

    /// Move the old per-stage water settings (a start level and a fall per metre of design-line
    /// chainage) into a two-point profile on the river centreline. The settings change, so every stage
    /// that held one shows as stale: its results came from the old model and are not kept as fresh.
    pub(super) fn migrate_water_settings(&mut self) {
        let mut start: Option<f64> = None;
        let mut slope = 0.0;
        let mut found = false;
        for (stage, holder, key) in [
            ("alignment", Some("reroute"), "water_level_start_m"),
            ("volumes", None, "water_level_start_m"),
        ] {
            for k in [key, "water_surface_slope"] {
                // `pointer_mut`, not `[]`: indexing a null for writing would turn it into an object.
                let path = match holder {
                    Some(h) => format!("/stages/{stage}/settings/{h}"),
                    None => format!("/stages/{stage}/settings"),
                };
                let obj = self.doc.pointer_mut(&path).and_then(Value::as_object_mut);
                let Some(o) = obj else { continue };
                if let Some(v) = o.remove(k) {
                    found = true;
                    if k == key && start.is_none() {
                        start = v.as_f64();
                    }
                    if k == "water_surface_slope" && slope == 0.0 {
                        slope = v.as_f64().unwrap_or(0.0);
                    }
                }
            }
        }
        if !found || self.input_water_profile().is_some() {
            return;
        }
        let Some(wl0) = start else { return };
        let len = self.base_line().map_or(0.0, |l| linear::length(&l));
        let pts = if len > 0.0 {
            vec![[0.0, wl0], [len, wl0 - slope * len]]
        } else {
            vec![[0.0, wl0]]
        };
        self.doc["inputs"]["water_level"] = json!({ "points": pts });
    }

    pub(super) fn river_centreline(&self) -> Result<Vec<Pt>, DesignError> {
        self.base_line()
    }

    pub(super) fn needs_water_profile(&self, stage: Stage) -> DesignError {
        DesignError::Input(format!(
            "the {} stage needs a water level: set one with `owd import --water-level LEVEL` \
             (a constant), `--water-slope` (a fall per metre of river chainage) or `--water-table FILE`",
            stage.name()
        ))
    }
}
