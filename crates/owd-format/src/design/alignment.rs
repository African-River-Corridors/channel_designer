//! The alignment stage: design line options from the rolling circle and the least-cost re-route
//! (spec 0002). Each run adds or replaces the option of its own method, with its line and its
//! checks: `radius_ok`, the measured minimum radius and the chord it was read on.
//!
//! Spec 0005 (#100): the rolling circle is water-aware when a river polygon is present (setting
//! `use_water`, default on then; a document from before spec 0005 records it off on open). Its
//! option then carries the `fits_in_water` check. Optional `seed_legs` in the inputs are joined
//! first (`join_legs`, with the `fixed` pieces held), and the option carries the `join_guard` check.

use super::geojson::all_positions;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use owd_core::alignment::centreline::build_centreline;
use owd_core::alignment::fairway_fit::{Fixed, Pose};
use owd_core::alignment::join::{JoinParams, join_legs};
use owd_core::alignment::navigable::Stretch;
use owd_core::alignment::rolling_circle::{WaterInput, rolling_circle_with_water};
use owd_core::alignment::route::{self, GridParams, RouteParams};
use owd_core::geom::linear;
use owd_core::geom::{Geometry, Polygon, Pt};
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
                json!({
                    "step_m": 10.0, "passes": 2, "check_chord_m": null,
                    "use_water": null, "clearance_m": 0.0, "r_pref_m": null,
                })
            }
            AlignMethod::Reroute => json!({
                "corridor_sweep_m": [40.0, 80.0, 120.0],
                "station_step_m": 100.0,
                "offset_step_m": 10.0,
                "side_slope": 3.0,
                "wet_weight": 1.0,
                "dry_weight": 1.0,
                "check_chord_m": null,
                "fairness_weight": 0.0,
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
    pub(super) fn base_line(&self) -> Result<Vec<Pt>, DesignError> {
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
            AlignMethod::RollingCircle => self.water_aware_rolling_circle(&settings, r_min)?,
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
                let profile = self.input_water_profile().ok_or_else(|| {
                    DesignError::Input(
                        "the re-route needs a water level: set one with `owd import --water-level LEVEL` (a constant), `--water-slope` or `--water-table FILE`".into(),
                    )
                })?;
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
                let centre = self.river_centreline()?;
                let level = self.water_level_along(&profile, &start, &centre);
                let grade = move |s: f64| level(s) - depth;
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
                        fairness_weight: fairness_weight(&settings)?,
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
    /// A re-route needs a DEM: without terrain it is refused.
    pub(super) fn rerun_alignment(&mut self, terrain: Option<Terrain>) -> Result<(), DesignError> {
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
            if m == AlignMethod::Reroute && terrain.is_none() {
                return Err(DesignError::NotAvailable(
                    "the re-route needs a DEM: rerun it with `owd align --method reroute --dem FILE`".into(),
                ));
            }
            self.align(m, &Value::Null, terrain)?;
        }
        Ok(())
    }
}

/// The text every export of a water-aware option carries (spec 0005).
pub const WIDTH_ONLY: &str = "width only, no depth data";

/// What an option whose every bend is infeasible says (issue #366). Its line is the fit that
/// breaks the rules least, not a design: the volumes stage does not cost it.
pub const NO_FEASIBLE_LINE: &str = "no feasible line";

/// A stretch as the document records it.
fn stretch_json(line: &[Pt], s: &Stretch) -> Value {
    let (a, b) = (linear::project(line, s.start), linear::project(line, s.end));
    json!({
        "start_chainage_m": a.min(b), "end_chainage_m": a.max(b),
        "start": s.start, "end": s.end, "reason": s.reason.as_str(),
    })
}

/// A pose input: `{"point": [x, y], "heading_deg": d}`, degrees counter-clockwise from the
/// project CRS's +x (east) axis.
fn pose_of(v: &Value) -> Option<Pose> {
    let p = v["point"].as_array()?;
    Some(Pose {
        at: [p.first()?.as_f64()?, p.get(1)?.as_f64()?],
        heading_rad: v["heading_deg"].as_f64()?.to_radians(),
    })
}

impl Session {
    /// The river polygon(s), with holes, when the inputs have one.
    pub(super) fn river_polygons(&self) -> Option<Vec<Polygon>> {
        let poly = &self.doc["inputs"]["river_polygon"];
        if poly["type"] != "Polygon" {
            return None;
        }
        let rings: Vec<Vec<Pt>> = poly["coordinates"]
            .as_array()?
            .iter()
            .map(|r| {
                r.as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let (shell, holes) = rings.split_first()?;
        Some(vec![Polygon {
            exterior: shell.clone(),
            holes: holes.to_vec(),
        }])
    }

    /// The seed legs and fixed pieces of the inputs (spec 0005), when there are legs.
    fn seed_legs(&self) -> Option<(Vec<Vec<Pt>>, Fixed)> {
        let legs: Vec<Vec<Pt>> = self.doc["inputs"]["seed_legs"]
            .as_array()?
            .iter()
            .filter_map(pts_of)
            .collect();
        if legs.is_empty() {
            return None;
        }
        let f = &self.doc["inputs"]["fixed"];
        let fixed = Fixed {
            start_pose: pose_of(&f["start_pose"]),
            end_pose: pose_of(&f["end_pose"]),
            must_follow: f["must_follow"]
                .as_array()
                .map(|a| a.iter().filter_map(pts_of).collect())
                .unwrap_or_default(),
        };
        Some((legs, fixed))
    }

    /// The rolling-circle option: seed legs joined first when there are any, then the rolling
    /// circle, water-aware when `use_water` holds (default: when there is a river polygon).
    fn water_aware_rolling_circle(
        &self,
        settings: &Value,
        r_min: f64,
    ) -> Result<Value, DesignError> {
        let input = |e: owd_core::alignment::PyError| DesignError::Input(e.message);
        let chord = opt_num(settings, "check_chord_m")?;
        let river = self.river_polygons();
        let use_water = match &settings["use_water"] {
            Value::Bool(b) => *b,
            Value::Null => river.is_some(),
            _ => {
                return Err(DesignError::Input(
                    "the setting `use_water` must be true, false or null".into(),
                ));
            }
        };
        let clearance = opt_num(settings, "clearance_m")?.unwrap_or(0.0);
        let r_pref = opt_num(settings, "r_pref_m")?;
        let shape = if use_water || self.seed_legs().is_some() {
            Some(self.sized_shape()?)
        } else {
            None
        };
        let water: Option<Vec<Polygon>> = if use_water {
            Some(river.clone().ok_or_else(|| {
                DesignError::Input(
                    "`use_water` is on but there is no river polygon: import one, or set it off"
                        .into(),
                )
            })?)
        } else {
            None
        };
        let mut checks = serde_json::Map::new();
        let mut join_stretches: Vec<Stretch> = Vec::new();
        let base = match self.seed_legs() {
            None => self.base_line()?,
            Some((legs, fixed)) => {
                let shape = shape.as_ref().expect("a shape when there are seed legs");
                // Without water the legs are joined in open space: a box well round them.
                let open = || {
                    let all: Vec<Pt> = legs.iter().flatten().copied().collect();
                    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                    for p in &all {
                        for k in 0..2 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                    let m = 10.0 * r_min;
                    vec![Polygon {
                        exterior: vec![
                            [lo[0] - m, lo[1] - m],
                            [hi[0] + m, lo[1] - m],
                            [hi[0] + m, hi[1] + m],
                            [lo[0] - m, hi[1] + m],
                            [lo[0] - m, lo[1] - m],
                        ],
                        holes: vec![],
                    }]
                };
                let w = water.clone().unwrap_or_else(open);
                let mut jp = JoinParams::new(r_min);
                jp.clearance_m = clearance;
                let j = join_legs(&legs, &w, shape, &fixed, &jp).map_err(input)?;
                checks.insert(
                    "join_guard".into(),
                    json!({"ok": j.guard_failed.is_empty(), "joins": j.guard_failed}),
                );
                join_stretches = j.infeasible.clone();
                j.line
            }
        };
        let wi = match (&water, &shape) {
            (Some(w), Some(sh)) => Some(WaterInput {
                water: w,
                shape: sh,
                clearance_m: clearance,
                smooth_m: 0.0,
                r_pref_m: r_pref,
            }),
            _ => None,
        };
        let r = rolling_circle_with_water(
            &Geometry::LineString(base),
            r_min,
            num(settings, "step_m")?,
            settings["passes"].as_i64().unwrap_or(2),
            &["left".to_string(), "right".to_string()],
            None,
            chord,
            wi.as_ref(),
        )
        .map_err(input)?;
        let line = &r.result.centreline;
        let mut base_checks = json!({
            "r_min_m": r_min,
            "radius_ok": r.result.radius_ok,
            "measured_min_radius_m": r.result.measured_min_radius_m,
            "radius_check_chord_m": r.result.radius_check_chord_m,
        });
        if wi.is_some() {
            let stretches: Vec<Value> = r
                .infeasible
                .iter()
                .chain(&join_stretches)
                .map(|s| stretch_json(line, s))
                .collect();
            checks.insert(
                "fits_in_water".into(),
                json!({"ok": stretches.is_empty(), "stretches": stretches, "note": WIDTH_ONLY}),
            );
            // Issue #366: every bend infeasible is no feasible line, not a design.
            let mut feasible = json!({"ok": r.feasible});
            if !r.feasible {
                feasible["note"] = json!(NO_FEASIBLE_LINE);
            }
            checks.insert("feasible".into(), feasible);
        }
        for (k, v) in checks {
            base_checks[k] = v;
        }
        let mut details = json!({"arcs": r.result.circles.len()});
        if wi.is_some() {
            details["rungs"] = json!(r.rungs.iter().map(|b| b.rung.as_str()).collect::<Vec<_>>());
        }
        Ok(json!({
            "id": AlignMethod::RollingCircle.option_id(),
            "name": AlignMethod::RollingCircle.option_name(),
            "method": AlignMethod::RollingCircle.name(), "settings": settings,
            "line": line_json(line),
            "checks": base_checks,
            "details": details,
        }))
    }
}

/// The re-route's fairness weight: a number at or above 0, 0 when absent (a document from
/// before spec 0005).
fn fairness_weight(settings: &Value) -> Result<f64, DesignError> {
    match &settings["fairness_weight"] {
        Value::Null => Ok(0.0),
        v => match v.as_f64() {
            Some(w) if w.is_finite() && w >= 0.0 => Ok(w),
            _ => Err(DesignError::Input(
                "the setting `fairness_weight` must be a number at or above 0".into(),
            )),
        },
    }
}
