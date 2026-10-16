//! The optimise stage (spec 0002): the weir cascade search sets the weir sites and pool levels, and
//! the pool lattice adds a design line option. The calls are those of the `optimise_pools` example:
//! `build_level_table`, `search_cascade` with an earthworks reach cost, then `optimise_pools`.
//!
//! The baseline is the rolling-circle option's line, else the river centreline. Chainages are
//! metres along it. Weights are settings, not prices: 1.0 / 1.0 minimises volume.

use super::alignment::Terrain;
use super::geojson::all_positions;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use cd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use cd_core::cut_model::CUT_MODEL;
use cd_core::geom::{Polygon, Pt, linear};
use cd_core::optimise::cascade::{
    CascadeConfig, SectionOptions, build_level_table, cascade_levels, earthworks_reach_cost,
    search_cascade,
};
use cd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pools,
};
use cd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use cd_core::volumes::sections::HalfWidth;
use serde_json::{Value, json};

/// `(chainage start, chainage end, water level)` of a pool.
pub type Pool = (f64, f64, f64);

/// The baseline, the pools along it and the level below the first pool.
pub(super) type PoolsResult = (Vec<Pt>, Vec<Pool>, f64);

const OPTION_ID: &str = "lattice";

fn defaults() -> Value {
    json!({
        "site_step_m": 500.0,
        "candidate_sites_m": null,
        "lifts_m": [0.5, 1.0],
        "terminal_min_level_m": 0.5,
        "min_spacing_m": 0.0,
        "base_level_m": null,
        "k_best": 5,
        "wet_weight": 1.0,
        "dry_weight": 1.0,
        "structure_fixed": 1.0e5,
        "structure_per_lift": 2.0e5,
        "structure_power": 1.0,
        "cut_model": CUT_MODEL,
        "station_m": 25.0,
        "maxw_m": 200.0,
        "side_slope": 3.0,
        "rake_half_m": 250.0,
        "rake_step_m": 4.0,
        "ds_step_m": 8.0,
        "pitch_m": 12.0,
        "n_heading": 16,
        "radius_multiples": [1.0, 1.5, 3.0],
        "corridor_half_m": 350.0,
        "cell_m": 2.0,
        "check_chord_m": null,
    })
}

fn num(v: &Value, key: &str) -> Result<f64, DesignError> {
    v[key]
        .as_f64()
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a number")))
}

fn nums(v: &Value, key: &str) -> Result<Vec<f64>, DesignError> {
    v[key]
        .as_array()
        .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>())
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a list of numbers")))
}

fn refuse(m: String) -> DesignError {
    DesignError::Input(m)
}

/// The water level of the pools along a line, as a function of the line's own chainage: the level
/// of the last pool that starts at or before the baseline chainage nearest the point. `base` is the
/// level before the first pool.
pub fn pool_level_fn<'a>(
    line: &'a [Pt],
    baseline: &'a [Pt],
    pools: &'a [(f64, f64, f64)],
    base: f64,
) -> impl Fn(f64) -> f64 + 'a {
    move |s| {
        let at = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
        let sp = linear::project(baseline, at);
        pools
            .iter()
            .filter(|(a, _, _)| *a <= sp)
            .map(|(_, _, l)| *l)
            .fold(None, |m: Option<f64>, l| match m {
                Some(m) if m >= l => Some(m),
                _ => Some(l),
            })
            .unwrap_or(base)
    }
}

impl Session {
    /// The pools the optimise stage set, with the baseline they are measured along and the level
    /// below the first pool, if the stage has run.
    pub(super) fn optimise_pools_result(&self) -> Option<PoolsResult> {
        let r = &self.doc["stages"]["optimise"]["results"];
        if r.is_null() {
            return None;
        }
        let pools = r["pools"]
            .as_array()?
            .iter()
            .filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?, p[2].as_f64()?)))
            .collect();
        Some((
            all_positions(&r["baseline"]),
            pools,
            r["base_level_m"].as_f64()?,
        ))
    }

    /// Run the optimise stage: choose weir sites and pool levels, then add the lattice design line
    /// option (replacing the one an earlier run made). `terrain` is the DEM in the project CRS.
    pub fn run_optimise(
        &mut self,
        overrides: &Value,
        terrain: Option<Terrain>,
    ) -> Result<(), DesignError> {
        match self.stage_status(Stage::Sizing) {
            StageStatus::Stale => {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Optimise,
                    run_first: Stage::Sizing,
                });
            }
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "the sizing stage has not been run: choose a ruleset and a vessel class first"
                        .into(),
                ));
            }
            StageStatus::Fresh => {}
        }
        if self.stage_status(Stage::Alignment) == StageStatus::Stale {
            return Err(DesignError::StaleUpstream {
                stage: Stage::Optimise,
                run_first: Stage::Alignment,
            });
        }
        let Some(dem_path) = self.dem_path().map(str::to_string) else {
            return Err(DesignError::Input(
                "the optimise stage needs a DEM: import one first (`cdesign import --dem FILE`)"
                    .into(),
            ));
        };
        let terrain = terrain.ok_or_else(|| {
            DesignError::Input(format!(
                "the DEM `{dem_path}` is missing: the optimise stage cannot run without it"
            ))
        })?;

        let mut settings = defaults();
        let reroute = self.doc["stages"]["alignment"]["settings"]["reroute"].clone();
        if !reroute["water_level_start_m"].is_null() {
            settings["base_level_m"] = reroute["water_level_start_m"].clone();
        }
        for source in [
            &self.doc["stages"]["optimise"]["settings"].clone(),
            overrides,
        ] {
            if let Some(o) = source.as_object() {
                for (k, v) in o {
                    settings[k] = v.clone();
                }
            }
        }
        let base_level = settings["base_level_m"].as_f64().ok_or_else(|| {
            DesignError::Input(
                "the optimise stage needs `base_level_m`: the water level below the first weir, m"
                    .into(),
            )
        })?;

        let sizing = &self.doc["stages"]["sizing"]["settings"];
        let (Some(rs), Some(vessel)) = (
            sizing["ruleset_id"].as_str(),
            sizing["vessel_standard_id"].as_str(),
        ) else {
            return Err(DesignError::Input(
                "the sizing stage has no settings".into(),
            ));
        };
        let shape = channel_shape(
            &RulesetRegistry::new(),
            rs,
            vessel,
            sizing["draught_m"].as_f64(),
        )
        .map_err(|e| DesignError::Input(e.raw_message().to_string()))?;
        let (depth, r_min) = (shape.design_depth_m, shape.min_bend_radius_m);
        let half_width = shape.bottom_width_straight_m / 2.0;

        let baseline: Vec<Pt> = self.doc["options"]
            .as_array()
            .and_then(|o| o.iter().find(|o| o["method"] == "rolling_circle"))
            .map(|o| all_positions(&o["line"]))
            .map_or_else(|| self.base_line(), Ok)?;
        let length = linear::length(&baseline);

        let polygon = &self.doc["inputs"]["river_polygon"];
        if polygon.is_null() {
            return Err(DesignError::Input(
                "the optimise stage needs the river polygon: import it first".into(),
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

        // The cascade search.
        let sites: Vec<f64> = match &settings["candidate_sites_m"] {
            Value::Null => {
                let step = num(&settings, "site_step_m")?;
                if !(step.is_finite() && step > 0.0) {
                    return Err(DesignError::Input(
                        "the setting `site_step_m` must be a positive number".into(),
                    ));
                }
                (1..)
                    .map(|k| k as f64 * step)
                    .take_while(|c| *c < length)
                    .collect()
            }
            _ => nums(&settings, "candidate_sites_m")?,
        };
        let mut cfg = CascadeConfig::new(
            sites,
            nums(&settings, "lifts_m")?,
            length,
            num(&settings, "terminal_min_level_m")?,
        );
        cfg.base_level_m = base_level;
        cfg.min_spacing_m = num(&settings, "min_spacing_m")?;
        cfg.k_best = settings["k_best"].as_i64().unwrap_or(5);
        let cfg = cfg.validated().map_err(refuse)?;
        let (wet, dry) = (num(&settings, "wet_weight")?, num(&settings, "dry_weight")?);
        let table = build_level_table(
            &baseline,
            terrain,
            &river,
            &cascade_levels(&cfg),
            depth,
            &HalfWidth::Fixed(half_width),
            settings["cut_model"].as_str(),
            Some(num(&settings, "station_m")?),
            Some(num(&settings, "maxw_m")?),
            None,
            SectionOptions::default(),
        )
        .map_err(refuse)?;
        let (fixed, coeff, power) = (
            num(&settings, "structure_fixed")?,
            num(&settings, "structure_per_lift")?,
            num(&settings, "structure_power")?,
        );
        let reach = earthworks_reach_cost(&table, wet, dry);
        let structure = move |lift: f64| fixed + coeff * libm::pow(lift, power);
        let search = search_cascade(&cfg, &reach, &structure).map_err(refuse)?;
        let Some(best) = search.best() else {
            return Err(DesignError::Input(
                "no cascade reaches the terminal level: raise the lifts or the candidate sites"
                    .into(),
            ));
        };
        let pools = best.pools();

        // The pool lattice.
        let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(half_width));
        spec.side_slope_h_per_v = num(&settings, "side_slope")?;
        spec.dry_weight = dry;
        spec.wet_weight = wet;
        spec.rake_half_m = num(&settings, "rake_half_m")?;
        spec.rake_step_m = num(&settings, "rake_step_m")?;
        spec.ds_step_m = num(&settings, "ds_step_m")?;
        let mut lattice = LatticeConfig::new(r_min);
        lattice.pitch_m = num(&settings, "pitch_m")?;
        lattice.n_heading = settings["n_heading"].as_i64().unwrap_or(16);
        lattice.radius_multiples = nums(&settings, "radius_multiples")?;
        let lattice = lattice.validated().map_err(refuse)?;
        let out = optimise_pools(
            &baseline,
            terrain,
            &river,
            &pools,
            depth,
            &spec,
            &lattice,
            None,
            PoolOptions {
                corridor_half_m: num(&settings, "corridor_half_m")?,
                cell_m: num(&settings, "cell_m")?,
            },
            true,
        )
        .map_err(refuse)?;

        let chord = match &settings["check_chord_m"] {
            Value::Null => default_check_chord(r_min),
            v => v.as_f64().ok_or_else(|| {
                DesignError::Input("the setting `check_chord_m` must be a number or null".into())
            })?,
        };
        let measured =
            min_radius_on_chord(&out.line, chord).map_err(|e| DesignError::Input(e.message))?;
        let option = json!({
            "id": OPTION_ID, "name": "Pool lattice",
            "method": "lattice", "settings": settings,
            "line": {"type": "LineString", "coordinates": out.line},
            "checks": {
                "r_min_m": r_min,
                "radius_ok": radius_ok(measured, r_min),
                "measured_min_radius_m": measured,
                "radius_check_chord_m": chord,
            },
            "details": {
                "n_replaced": out.n_replaced(),
                "pools": out.pools.iter().map(|p| json!({
                    "start_m": p.start_m, "end_m": p.end_m, "water_level_m": p.water_level_m,
                    "converged": p.result.converged, "cost": p.result.cost,
                    "baseline_cost": p.baseline_cost,
                })).collect::<Vec<_>>(),
            },
        });
        let options = self.doc["options"]
            .as_array_mut()
            .expect("options is a list");
        match options.iter().position(|o| o["method"] == "lattice") {
            Some(i) => options[i] = option,
            None => options.push(option),
        }

        let st = &mut self.doc["stages"]["optimise"];
        st["settings"] = settings;
        st["results"] = json!({
            "baseline": {"type": "LineString", "coordinates": baseline},
            "base_level_m": base_level,
            "pools": pools.iter().map(|&(a, b, l)| json!([a, b, l])).collect::<Vec<_>>(),
            "cascade": {
                "total_cost": best.total_cost,
                "reach_cost": best.reach_cost(),
                "structure_cost": best.structure_cost(),
                "weirs": best.weirs.iter().map(|w| json!({
                    "site_id": w.site_id, "chainage_m": w.chainage_m,
                    "pool_level_m": w.pool_level_m, "lift_m": w.lift_m,
                    "structure_cost": w.structure_cost,
                })).collect::<Vec<_>>(),
            },
            "option": OPTION_ID,
        });
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Optimise);
        self.doc["stages"]["optimise"]["depends_on"] = json!(h);
        Ok(())
    }
}
