//! The `route` entry point (ticket 29): the seam for `alignment/route.py`, over
//! `cd_core::alignment::route`.

use super::geometry::{geom_from, geom_json, polys_of, pts};
use super::terrain::{river_params, terrain_from};
use super::{EvalError, arg, bad, f64_arg, f64_or, float_json, floats_json, list, ok, opt_f64};
use super::{read_float, value_error_of};
use cd_core::alignment::route as rt;
use cd_core::geom::{Geometry, Pt};
use cd_core::synthetic;
use serde_json::{Value, json};

fn pt_json(p: Pt) -> Value {
    json!([float_json(p[0]), float_json(p[1])])
}

fn pts_json(ps: &[Pt]) -> Value {
    Value::Array(ps.iter().map(|&p| pt_json(p)).collect())
}

fn matrix_json(m: &[Vec<f64>]) -> Value {
    Value::Array(m.iter().map(|r| floats_json(r)).collect())
}

fn floats_arg(args: &Value, name: &str) -> Result<Vec<f64>, EvalError> {
    list(args, name)?
        .iter()
        .map(|v| read_float(v, &format!("args.{name}")))
        .collect()
}

fn matrix_arg(v: &Value, what: &str) -> Result<Vec<Vec<f64>>, EvalError> {
    v.as_array()
        .ok_or_else(|| bad(format!("{what} must be a list of lists")))?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| bad(format!("{what} rows must be lists")))?
                .iter()
                .map(|x| read_float(x, what))
                .collect()
        })
        .collect()
}

fn pts_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    pts(arg(args, name)?, &format!("args.{name}"))
}

fn int_or(args: &Value, name: &str, default: i64) -> Result<i64, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_i64()
            .ok_or_else(|| bad(format!("`args.{name}` must be an integer"))),
    }
}

fn grid_from(v: &Value) -> Result<rt::CostGrid, EvalError> {
    let f = |k: &str| -> Result<Vec<f64>, EvalError> {
        floats_arg(&json!({ k: v.get(k).cloned().unwrap_or(Value::Null) }), k)
    };
    let p = |k: &str| -> Result<Vec<Pt>, EvalError> {
        pts(v.get(k).ok_or_else(|| bad(format!("grid.{k} missing")))?, k)
    };
    let m = |k: &str| -> Result<Vec<Vec<f64>>, EvalError> {
        matrix_arg(v.get(k).ok_or_else(|| bad(format!("grid.{k} missing")))?, k)
    };
    Ok(rt::CostGrid {
        offsets: f("offsets")?,
        stations_m: f("stations_m")?,
        centres: p("centres")?,
        normals: p("normals")?,
        grades: f("grades")?,
        vol: m("vol")?,
        wet: m("wet")?,
        station_step: f64_arg(v, "station_step")?,
        offset_step: f64_arg(v, "offset_step")?,
    })
}

fn grid_json(g: &rt::CostGrid) -> Value {
    json!({
        "offsets": floats_json(&g.offsets),
        "stations_m": floats_json(&g.stations_m),
        "centres": pts_json(&g.centres),
        "normals": pts_json(&g.normals),
        "grades": floats_json(&g.grades),
        "vol": matrix_json(&g.vol),
        "wet": matrix_json(&g.wet),
        "station_step": float_json(g.station_step),
        "offset_step": float_json(g.offset_step),
    })
}

fn grid_params(args: &Value, corridor_half: f64) -> Result<rt::GridParams, EvalError> {
    Ok(rt::GridParams {
        bottom_width: f64_arg(args, "bottom_width")?,
        depth: f64_arg(args, "depth")?,
        side_slope: f64_arg(args, "side_slope")?,
        corridor_half,
        station_step: f64_or(args, "station_step", 100.0)?,
        offset_step: f64_or(args, "offset_step", 10.0)?,
        transect_step: f64_or(args, "transect_step", 2.0)?,
        daylight_m: opt_f64(args, "daylight_m")?,
    })
}

/// Build the cost grid from the case's line, terrain, river and grade.
fn built_grid(
    args: &Value,
    p: &rt::GridParams,
) -> Result<Result<rt::CostGrid, cd_core::alignment::PyError>, EvalError> {
    let line = match geom_from(arg(args, "line")?, "args.line")? {
        Geometry::LineString(l) => l,
        _ => return Err(bad("`args.line` must be a LineString".into())),
    };
    let river = polys_of(
        geom_from(arg(args, "river_polygon")?, "args.river_polygon")?,
        "args.river_polygon",
    )?;
    let sampler = terrain_from(arg(args, "terrain")?)?;
    let grade: Box<dyn Fn(f64) -> f64> = match arg(args, "grade")? {
        g if crate::host::callback_id(g).is_some() => {
            crate::host::scalar_fn(crate::host::callback_id(g).unwrap_or(0))?
        }
        g if g.is_object() => {
            // `{"kind": "water_level_minus", "depth": d, "params": {...}}`
            if g.get("kind").and_then(Value::as_str) != Some("water_level_minus") {
                return Err(bad("grade spec: unknown kind".into()));
            }
            let depth = f64_arg(g, "depth")?;
            let params = match g.get("params") {
                Some(p) if !p.is_null() => river_params(p)?,
                _ => synthetic::RiverParams::default(),
            };
            let river = synthetic::synthetic_river(params).map_err(|e| bad(e.to_string()))?;
            Box::new(move |s| river.water_level(s) - depth)
        }
        g => {
            let v = read_float(g, "args.grade")?;
            Box::new(move |_| v)
        }
    };
    Ok(rt::build_cost_grid(&line, &*sampler, &river, &*grade, p))
}

pub(super) fn route(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "build_cost_grid" => {
            let p = grid_params(args, f64_or(args, "corridor_half", 300.0)?)?;
            match built_grid(args, &p)? {
                Ok(g) => ok(grid_json(&g)),
                Err(e) => value_error_of(e),
            }
        }
        "radii" => ok(floats_json(&rt::radii(&pts_arg(args, "pts")?))),
        "min_radius_of_line" => {
            let (r, i) = rt::min_radius_of_line(&pts_arg(args, "pts")?);
            ok(json!({"radius": float_json(r), "index": i}))
        }
        "baseline_turn_angles" => ok(floats_json(&rt::baseline_turn_angles(&pts_arg(
            args, "centres",
        )?))),
        "offset_bounds" => {
            let (lo, hi) =
                rt::offset_bounds(&pts_arg(args, "centres")?, f64_arg(args, "target_R")?);
            ok(json!({"lo": floats_json(&lo), "hi": floats_json(&hi)}))
        }
        "dp_optimise" => {
            let cost = matrix_arg(arg(args, "cost_grid")?, "args.cost_grid")?;
            match rt::dp_optimise(
                &floats_arg(args, "offsets")?,
                &pts_arg(args, "centres")?,
                &cost,
                f64_arg(args, "r_min")?,
                f64_arg(args, "station_step")?,
                f64_arg(args, "offset_step")?,
            ) {
                Ok((path, c)) => ok(json!({"path": path, "cost": float_json(c)})),
                Err(e) => value_error_of(e),
            }
        }
        "build_line_from_offsets" => ok(pts_json(&rt::build_line_from_offsets(
            &pts_arg(args, "centres")?,
            &pts_arg(args, "normals")?,
            &floats_arg(args, "offset_vals")?,
        ))),
        "smooth_offsets" => ok(floats_json(&rt::smooth_offsets(
            &floats_arg(args, "offset_vals")?,
            f64_arg(args, "heading_budget")?,
            &floats_arg(args, "beta")?,
            f64_arg(args, "ds")?,
        ))),
        "enforce_min_radius" => {
            let (o, passes, rmin) = rt::enforce_min_radius(
                &floats_arg(args, "offset_vals")?,
                &pts_arg(args, "centres")?,
                &pts_arg(args, "normals")?,
                f64_arg(args, "target_R")?,
                int_or(args, "max_iter", 600)?,
                int_or(args, "taper_stations", 8)?,
            );
            ok(
                json!({"offsets": floats_json(&o), "passes": passes, "min_radius": float_json(rmin)}),
            )
        }
        "approx_saving_pct" => {
            let grid = grid_from(arg(args, "grid")?)?;
            let cost = matrix_arg(arg(args, "cost")?, "args.cost")?;
            ok(float_json(rt::approx_saving_pct(
                &grid,
                &cost,
                &floats_arg(args, "ovals")?,
            )))
        }
        "solve_at_corridor" => {
            let grid = grid_from(arg(args, "grid")?)?;
            let cost = matrix_arg(arg(args, "cost")?, "args.cost")?;
            match rt::solve_at_corridor(
                &grid,
                &cost,
                f64_arg(args, "corridor_half")?,
                f64_arg(args, "r_min")?,
            ) {
                Ok((o, passes, rmin, sav)) => ok(json!({
                    "offsets": floats_json(&o), "passes": passes,
                    "min_radius": float_json(rmin), "saving_pct": float_json(sav),
                })),
                Err(e) => value_error_of(e),
            }
        }
        "optimise_route" => {
            let sweep: Vec<f64> = match args.get("corridor_sweep") {
                None | Some(Value::Null) => rt::DEFAULT_CORRIDOR_SWEEP.to_vec(),
                Some(_) => floats_arg(args, "corridor_sweep")?,
            };
            let widest = sweep.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let grid = match args.get("grid") {
                Some(g) if !g.is_null() => grid_from(g)?,
                _ => match built_grid(args, &grid_params(args, widest)?)? {
                    Ok(g) => g,
                    Err(e) => return value_error_of(e),
                },
            };
            let params = rt::RouteParams {
                r_min: f64_arg(args, "r_min")?,
                wet_weight: f64_or(args, "wet_weight", 1.0)?,
                dry_weight: f64_or(args, "dry_weight", 1.0)?,
                corridor_sweep: sweep,
                check_chord_m: opt_f64(args, "check_chord_m")?,
            };
            match rt::optimise_route(&grid, &params) {
                Err(e) => value_error_of(e),
                Ok(r) => ok(json!({
                    "line": geom_json(&Geometry::LineString(r.line.clone())),
                    "offsets": floats_json(&r.offsets),
                    "min_radius_m": float_json(r.min_radius_m),
                    "corridor_half_m": float_json(r.corridor_half_m),
                    "approx_saving_pct": float_json(r.approx_saving_pct),
                    "smooth_passes": r.smooth_passes,
                    "sweep": r.sweep.iter()
                        .map(|&(c, s, m)| json!([float_json(c), float_json(s), float_json(m)]))
                        .collect::<Vec<_>>(),
                    "measured_min_radius_m": float_json(r.measured_min_radius_m),
                    "radius_check_chord_m": float_json(r.radius_check_chord_m),
                    "radius_ok": r.radius_ok,
                })),
            }
        }
        other => Err(bad(format!("route has no parity function `{other}`"))),
    }
}
