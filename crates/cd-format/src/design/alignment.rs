//! The alignment stage: design line options from the rolling circle and the least-cost re-route
//! (spec 0002). Each run adds or replaces the option of its own method, with its line and its
//! checks: `radius_ok`, the measured minimum radius and the chord it was read on. The rolling
//! circle ignores the water (a known limit, spec 0005), and nothing here changes that.

use super::geojson::all_positions;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use cd_core::alignment::centreline::build_centreline;
use cd_core::alignment::rolling_circle::rolling_circle;
use cd_core::alignment::route::{self, GridParams, RouteParams};
use cd_core::geom::{Geometry, Polygon, Pt};
use serde_json::{Value, json};

/// A terrain sampler: heights for the points `(xs[i], ys[i])` in the project CRS, NaN where unknown.
pub type Terrain<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// A way of finding a design line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMethod {
    RollingCircle,
    Reroute,
}

impl AlignMethod {
    pub fn name(self) -> &'static str {
        match self {
            AlignMethod::RollingCircle => "rolling_circle",
            AlignMethod::Reroute => "reroute",
        }
    }

    pub fn from_name(s: &str) -> Option<AlignMethod> {
        match s {
            "rolling_circle" | "rolling-circle" => Some(AlignMethod::RollingCircle),
            "reroute" => Some(AlignMethod::Reroute),
            _ => None,
        }
    }

    fn option_id(self) -> &'static str {
        match self {
            AlignMethod::RollingCircle => "rolling-circle",
            AlignMethod::Reroute => "reroute",
        }
    }

    fn option_name(self) -> &'static str {
        match self {
            AlignMethod::RollingCircle => "Rolling circle",
            AlignMethod::Reroute => "Least-cost re-route",
        }
    }

    /// The settings of a method, with the defaults the Python has.
    fn defaults(self) -> Value {
        match self {
            AlignMethod::RollingCircle => {
                json!({"step_m": 10.0, "passes": 2, "check_chord_m": null})
            }
            AlignMethod::Reroute => json!({
                "corridor_sweep_m": [40.0, 80.0, 120.0],
                "station_step_m": 100.0,
                "offset_step_m": 10.0,
                "side_slope": 3.0,
                "wet_weight": 1.0,
                "dry_weight": 1.0,
                "check_chord_m": null,
                "water_level_start_m": null,
                "water_surface_slope": 0.0,
            }),
        }
    }
}

fn pts_of(line: &Value) -> Option<Vec<Pt>> {
    (line["type"] == "LineString").then(|| all_positions(line))
}

fn line_json(pts: &[Pt]) -> Value {
    json!({"type": "LineString", "coordinates": pts})
}

fn num(v: &Value, key: &str) -> Result<f64, DesignError> {
    v[key]
        .as_f64()
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a number")))
}

fn opt_num(v: &Value, key: &str) -> Result<Option<f64>, DesignError> {
    match &v[key] {
        Value::Null => Ok(None),
        x => x.as_f64().map(Some).ok_or_else(|| {
            DesignError::Input(format!("the setting `{key}` must be a number or null"))
        }),
    }
}

impl Session {
    fn upstream_ready(&self) -> Result<(), DesignError> {
        match self.stage_status(Stage::Sizing) {
            StageStatus::Stale => Err(DesignError::StaleUpstream {
                stage: Stage::Alignment,
                run_first: Stage::Sizing,
            }),
            StageStatus::NotRun => Err(DesignError::Input(
                "the sizing stage has not been run: choose a ruleset and a vessel class first"
                    .into(),
            )),
            StageStatus::Fresh => Ok(()),
        }
    }

    /// The river centreline the methods start from: the imported one, or the smoothed line midway
    /// between the two banks.
    fn base_line(&self) -> Result<Vec<Pt>, DesignError> {
        let inp = &self.doc["inputs"];
        if let Some(c) = pts_of(&inp["centreline"]) {
            return Ok(c);
        }
        match (pts_of(&inp["left_bank"]), pts_of(&inp["right_bank"])) {
            (Some(l), Some(r)) => build_centreline(&l, &r, 20.0, 5, 2).map_err(DesignError::Input),
            _ => Err(DesignError::Input(
                "the alignment needs a river centreline or both bank lines: import them first"
                    .into(),
            )),
        }
    }

    /// Settings of a method: its defaults, then what the document holds, then `overrides`.
    fn method_settings(&self, method: AlignMethod, overrides: &Value) -> Value {
        let mut s = method.defaults();
        let stored = &self.doc["stages"]["alignment"]["settings"][method.name()];
        for source in [stored, overrides] {
            if let Some(o) = source.as_object() {
                for (k, v) in o {
                    s[k] = v.clone();
                }
            }
        }
        s
    }

    /// Run one method and add its option, or replace the option that method made before. The
    /// least-cost re-route needs a terrain sampler (a DEM) and a water level; it starts from the
    /// rolling-circle option's line when there is one, else from the river centreline.
    pub fn align(
        &mut self,
        method: AlignMethod,
        overrides: &Value,
        terrain: Option<Terrain>,
    ) -> Result<(), DesignError> {
        self.upstream_ready()?;
        let shape = self.doc["stages"]["sizing"]["results"].clone();
        let r_min = shape["min_bend_radius_m"].as_f64().unwrap_or(f64::NAN);
        let settings = self.method_settings(method, overrides);
        let option = match method {
            AlignMethod::RollingCircle => {
                let line = Geometry::LineString(self.base_line()?);
                let chord = opt_num(&settings, "check_chord_m")?;
                let r = rolling_circle(
                    &line,
                    r_min,
                    num(&settings, "step_m")?,
                    settings["passes"].as_i64().unwrap_or(2),
                    &["left".to_string(), "right".to_string()],
                    None,
                    chord,
                )
                .map_err(|e| DesignError::Input(e.message))?;
                json!({
                    "id": method.option_id(), "name": method.option_name(),
                    "method": method.name(), "settings": settings,
                    "line": line_json(&r.centreline),
                    "checks": {
                        "r_min_m": r_min,
                        "radius_ok": r.radius_ok,
                        "measured_min_radius_m": r.measured_min_radius_m,
                        "radius_check_chord_m": r.radius_check_chord_m,
                    },
                    "details": {"arcs": r.circles.len()},
                })
            }
            AlignMethod::Reroute => {
                let terrain = terrain.ok_or_else(|| {
                    DesignError::Input("the re-route needs terrain: give it a DEM".into())
                })?;
                let polygon = &self.doc["inputs"]["river_polygon"];
                if polygon.is_null() {
                    return Err(DesignError::Input(
                        "the re-route needs the river polygon: import it first".into(),
                    ));
                }
                let ring: Vec<Pt> = polygon["coordinates"][0]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                            .collect()
                    })
                    .unwrap_or_default();
                let river = [Polygon {
                    exterior: ring,
                    holes: vec![],
                }];
                let wl0 = opt_num(&settings, "water_level_start_m")?.ok_or_else(|| {
                    DesignError::Input(
                        "the re-route needs `water_level_start_m`: the water surface level at the start of the reach, m".into(),
                    )
                })?;
                let slope = num(&settings, "water_surface_slope")?;
                let depth = shape["design_depth_m"].as_f64().unwrap_or(f64::NAN);
                // The line it starts from: the rolling-circle option's, if there is one.
                let start = self.doc["options"]
                    .as_array()
                    .and_then(|o| o.iter().find(|o| o["method"] == "rolling_circle"))
                    .and_then(|o| pts_of(&o["line"]))
                    .map_or_else(|| self.base_line(), Ok)?;
                let sweep: Vec<f64> = settings["corridor_sweep_m"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                let widest = sweep.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let params = GridParams {
                    bottom_width: shape["bottom_width_straight_m"]
                        .as_f64()
                        .unwrap_or(f64::NAN),
                    depth,
                    side_slope: num(&settings, "side_slope")?,
                    corridor_half: widest,
                    station_step: num(&settings, "station_step_m")?,
                    offset_step: num(&settings, "offset_step_m")?,
                    transect_step: 2.0,
                    daylight_m: None,
                };
                let grade = move |s: f64| wl0 - slope * s - depth;
                let grid = route::build_cost_grid(&start, terrain, &river, &grade, &params)
                    .map_err(|e| DesignError::Input(e.message))?;
                let r = route::optimise_route(
                    &grid,
                    &RouteParams {
                        r_min,
                        wet_weight: num(&settings, "wet_weight")?,
                        dry_weight: num(&settings, "dry_weight")?,
                        corridor_sweep: sweep,
                        check_chord_m: opt_num(&settings, "check_chord_m")?,
                    },
                )
                .map_err(|e| DesignError::Input(e.message))?;
                json!({
                    "id": method.option_id(), "name": method.option_name(),
                    "method": method.name(), "settings": settings,
                    "line": line_json(&r.line),
                    "checks": {
                        "r_min_m": r_min,
                        "radius_ok": r.radius_ok,
                        "measured_min_radius_m": r.measured_min_radius_m,
                        "radius_check_chord_m": r.radius_check_chord_m,
                    },
                    "details": {
                        "corridor_half_m": r.corridor_half_m,
                        "approx_saving_pct": r.approx_saving_pct,
                    },
                })
            }
        };
        // Replace the option this method made before, or add one.
        let options = self.doc["options"]
            .as_array_mut()
            .expect("options is a list");
        match options.iter().position(|o| o["method"] == method.name()) {
            Some(i) => options[i] = option,
            None => options.push(option),
        }
        // The stage's settings hold each method's settings; its results list the methods run.
        self.doc["stages"]["alignment"]["settings"][method.name()] = settings;
        let mut ran: Vec<Value> = self.doc["stages"]["alignment"]["results"]["methods"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !ran.iter().any(|m| m == method.name()) {
            ran.push(json!(method.name()));
        }
        self.doc["stages"]["alignment"]["results"] = json!({"methods": ran});
        self.doc["stages"]["alignment"]["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Alignment);
        self.doc["stages"]["alignment"]["depends_on"] = json!(h);
        Ok(())
    }

    /// `run_stage(Alignment)`: rerun the methods that were run, or the rolling circle if none was.
    /// A re-route needs a DEM, which a bare stage run does not have.
    pub(super) fn rerun_alignment(&mut self) -> Result<(), DesignError> {
        let ran: Vec<AlignMethod> = self.doc["stages"]["alignment"]["results"]["methods"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.as_str().and_then(AlignMethod::from_name))
                    .collect()
            })
            .unwrap_or_default();
        if ran.is_empty() {
            return self.align(AlignMethod::RollingCircle, &Value::Null, None);
        }
        for m in ran {
            if m == AlignMethod::Reroute {
                return Err(DesignError::NotAvailable(
                    "the re-route needs a DEM: rerun it with `cdesign align --method reroute --dem FILE`".into(),
                ));
            }
            self.align(m, &Value::Null, None)?;
        }
        Ok(())
    }
}
