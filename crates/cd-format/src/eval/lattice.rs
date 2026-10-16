//! The `lattice` entry point (ticket 32): the pool-by-pool lattice alignment search.
//!
//! A section's half-width is a number or `{"kind": "width_fn", "loa_m", "beam_m", "draft_m",
//! "rule_set"}`, which is `make_width_fn(...).half_width` (the width profile of ticket 25).
//! `seconds` is wall-clock time and is not an output.

use super::geometry::{geom_from, geom_json};
use super::terrain::terrain_from;
use super::volumes::{line_of, polygons_of};
use super::{
    EvalError, arg, bad, f64_arg, f64_list, f64_or, float_json, floats_json, list, ok, opt_f64,
    read_float,
};
use cd_core::geom::{Geometry, Polygon, Pt};
use cd_core::optimise::lattice::{
    self as lt, FieldScorer, HalfWidthSpec, LatticeConfig, LatticeResult, PoolAlignment,
    PoolOptions, Pose, Prim, SectionSpec, TerrainGrid,
};
use cd_core::rules::methods::MethodRegistry;
use cd_core::rules::rulesets::ChannelRuleInputs;
use cd_core::rules::width_profile::WidthFn;
use serde_json::{Value, json};

fn refused(message: String) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": "ValueError", "message": message } }))
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(m) => return refused(m),
        }
    };
}

fn pose(v: &Value, what: &str) -> Result<Pose, EvalError> {
    match v.as_array().map(Vec::as_slice) {
        Some([x, y, t]) => Ok((
            read_float(x, what)?,
            read_float(y, what)?,
            read_float(t, what)?,
        )),
        _ => Err(bad(format!("{what}: a pose is [x, y, heading]"))),
    }
}

fn pose_json(p: Pose) -> Value {
    floats_json(&[p.0, p.1, p.2])
}

fn string_or(v: &Value, name: &str, default: &str) -> Result<String, EvalError> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(default.to_string()),
        Some(s) => s
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| bad(format!("`{name}` must be a string"))),
    }
}

fn section(d: &Value) -> Result<Result<SectionSpec, String>, EvalError> {
    let hw = arg(d, "half_width")?;
    let half_width = match hw {
        h if crate::host::callback_id(h).is_some() => {
            HalfWidthSpec::Func(crate::host::scalar_fn(crate::host::callback_id(h).unwrap_or(0))?)
        }
        Value::Object(_) => {
            let mut inp = ChannelRuleInputs::new(
                f64_arg(hw, "beam_m")?,
                f64_arg(hw, "loa_m")?,
                f64_arg(hw, "draft_m")?,
                0.0,
            );
            inp.rule_set = string_or(hw, "rule_set", "pianc")?;
            match WidthFn::new(&MethodRegistry::new(), inp) {
                Ok(f) => {
                    HalfWidthSpec::Func(Box::new(move |r| f.half_width(r).unwrap_or(f64::NAN)))
                }
                Err(e) => return Ok(Err(e.python_message())),
            }
        }
        _ => HalfWidthSpec::Fixed(read_float(hw, "half_width")?),
    };
    let mut s = SectionSpec::new(half_width);
    s.side_slope_h_per_v = f64_or(d, "side_slope_h_per_v", s.side_slope_h_per_v)?;
    s.footprint_corr_m = f64_or(d, "footprint_corr_m", s.footprint_corr_m)?;
    s.wet_weight = f64_or(d, "wet_weight", s.wet_weight)?;
    s.dry_weight = f64_or(d, "dry_weight", s.dry_weight)?;
    s.deep_threshold_m = f64_or(d, "deep_threshold_m", s.deep_threshold_m)?;
    s.deep_multiplier = f64_or(d, "deep_multiplier", s.deep_multiplier)?;
    s.rake_half_m = f64_or(d, "rake_half_m", s.rake_half_m)?;
    s.rake_step_m = f64_or(d, "rake_step_m", s.rake_step_m)?;
    s.ds_step_m = f64_or(d, "ds_step_m", s.ds_step_m)?;
    s.min_valid_fraction = f64_or(d, "min_valid_fraction", s.min_valid_fraction)?;
    Ok(Ok(s))
}

fn config(d: &Value) -> Result<Result<LatticeConfig, String>, EvalError> {
    let mut c = LatticeConfig::new(f64_arg(d, "r_min_m")?);
    if let Some(m) = d.get("radius_multiples").filter(|v| !v.is_null()) {
        c.radius_multiples = m
            .as_array()
            .ok_or_else(|| bad("radius_multiples must be a list".into()))?
            .iter()
            .map(|x| read_float(x, "radius_multiples"))
            .collect::<Result<_, _>>()?;
    }
    c.pitch_m = f64_or(d, "pitch_m", c.pitch_m)?;
    if let Some(n) = d.get("n_heading").filter(|v| !v.is_null()) {
        c.n_heading = n
            .as_i64()
            .ok_or_else(|| bad("n_heading must be a whole number".into()))?;
    }
    c.straight_len_m = opt_f64(d, "straight_len_m")?;
    c.arc_step_m = f64_or(d, "arc_step_m", c.arc_step_m)?;
    c.goal_pos_tol_m = opt_f64(d, "goal_pos_tol_m")?;
    c.goal_head_tol_bins = f64_or(d, "goal_head_tol_bins", c.goal_head_tol_bins)?;
    if let Some(h) = d.get("heuristic").filter(|v| !v.is_null()) {
        c.heuristic = h
            .as_str()
            .ok_or_else(|| bad("heuristic must be a string".into()))?
            .to_string();
    }
    c.heuristic_weight = f64_or(d, "heuristic_weight", c.heuristic_weight)?;
    c.prune_bound = f64_or(d, "prune_bound", c.prune_bound)?;
    if let Some(n) = d.get("max_expand").filter(|v| !v.is_null()) {
        c.max_expand = n
            .as_i64()
            .ok_or_else(|| bad("max_expand must be a whole number".into()))?;
    }
    c.wall_cap_s = opt_f64(d, "wall_cap_s")?;
    c.bounds_margin_m = f64_or(d, "bounds_margin_m", c.bounds_margin_m)?;
    Ok(c.validated())
}

fn config_json(c: &LatticeConfig) -> Value {
    json!({
        "r_min_m": float_json(c.r_min_m),
        "radius_multiples": floats_json(&c.radius_multiples),
        "pitch_m": float_json(c.pitch_m),
        "n_heading": c.n_heading,
        "straight_len_m": c.straight_len_m.map_or(Value::Null, float_json),
        "arc_step_m": float_json(c.arc_step_m),
        "goal_pos_tol_m": c.goal_pos_tol_m.map_or(Value::Null, float_json),
        "goal_head_tol_bins": float_json(c.goal_head_tol_bins),
        "heuristic": c.heuristic,
        "heuristic_weight": float_json(c.heuristic_weight),
        "prune_bound": float_json(c.prune_bound),
        "max_expand": c.max_expand,
        "wall_cap_s": c.wall_cap_s.map_or(Value::Null, float_json),
        "bounds_margin_m": float_json(c.bounds_margin_m),
    })
}

fn grid_of(g: &Value) -> Result<Result<TerrainGrid, String>, EvalError> {
    let sampler = terrain_from(arg(g, "terrain")?)?;
    let river = polygons_of(arg(g, "river_polygon")?)?;
    let corridor = polygons_of(arg(g, "corridor")?)?;
    Ok(lt::build_terrain_grid(
        &*sampler,
        &river,
        &corridor,
        f64_or(g, "cell_m", 2.0)?,
        f64_or(g, "margin_m", 20.0)?,
    ))
}

fn matrix_json(m: &[Vec<f64>]) -> Value {
    Value::Array(m.iter().map(|r| floats_json(r)).collect())
}

fn mask_json(m: &[Vec<bool>]) -> Value {
    Value::Array(m.iter().map(|r| json!(r)).collect())
}

fn prim_json(p: &Prim) -> Value {
    json!({
        "R": float_json(p.r),
        "sign": p.sign,
        "pts": Value::Array(p.pts.iter().map(|q| floats_json(q)).collect()),
        "end": floats_json(&[p.end.0, p.end.1, p.end.2]),
        "arc_len": float_json(p.arc_len),
    })
}

fn line_json(line: &Option<Vec<Pt>>) -> Value {
    line.as_ref()
        .map_or(Value::Null, |l| geom_json(&Geometry::LineString(l.clone())))
}

fn result_json(r: &LatticeResult) -> Value {
    let seq: Vec<Value> = r
        .prim_seq
        .iter()
        .map(|&(radius, sign, len)| {
            json!([
                radius.map_or(Value::Null, float_json),
                sign,
                float_json(len)
            ])
        })
        .collect();
    let diagnostics = match r.c_min_per_m {
        Some(c) => json!({ "c_min_per_m": float_json(c) }),
        None => json!({}),
    };
    json!({
        "converged": r.converged,
        "line": line_json(&r.line),
        "cost": float_json(r.cost),
        "prim_seq": seq,
        "start_pose": pose_json(r.start_pose),
        "goal_pose": pose_json(r.goal_pose),
        "end_err_m": float_json(r.end_err_m),
        "expansions": r.expansions,
        "n_states": r.n_states,
        "hit_cap": r.hit_cap,
        "timed_out": r.timed_out,
        "diagnostics": diagnostics,
    })
}

fn pool_json(p: &PoolAlignment) -> Value {
    json!({
        "start_m": float_json(p.start_m),
        "end_m": float_json(p.end_m),
        "water_level_m": float_json(p.water_level_m),
        "grade_m": float_json(p.grade_m),
        "result": result_json(&p.result),
        "baseline_cost": float_json(p.baseline_cost),
    })
}

fn pool_options(args: &Value) -> Result<PoolOptions, EvalError> {
    let d = PoolOptions::default();
    Ok(PoolOptions {
        corridor_half_m: f64_or(args, "corridor_half_m", d.corridor_half_m)?,
        cell_m: f64_or(args, "cell_m", d.cell_m)?,
    })
}

fn optional_line(args: &Value, name: &str) -> Result<Option<Vec<Pt>>, EvalError> {
    match args.get(name).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => match geom_from(v, name)? {
            Geometry::LineString(l) => Ok(Some(l)),
            _ => Err(bad(format!("`{name}` must be a line string"))),
        },
    }
}

pub(super) fn lattice(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "section_half_width_at" => {
            let s = tri!(section(arg(args, "section")?)?);
            let out: Vec<f64> = f64_list(args, "radii")?
                .into_iter()
                .map(|r| s.half_width_at(r))
                .collect();
            ok(floats_json(&out))
        }
        "config" => {
            let c = tri!(config(arg(args, "cfg")?)?);
            ok(config_json(&c))
        }
        "build_terrain_grid" => {
            let g = tri!(grid_of(arg(args, "grid")?)?);
            let (rows, cols): (Vec<usize>, Vec<usize>) = match args.get("rc") {
                Some(Value::Array(rc)) if rc.len() == 2 => {
                    let ix = |v: &Value| -> Result<Vec<usize>, EvalError> {
                        v.as_array()
                            .ok_or_else(|| bad("rc entries must be lists".into()))?
                            .iter()
                            .map(|x| {
                                x.as_u64()
                                    .map(|n| n as usize)
                                    .ok_or_else(|| bad("rc indices are whole numbers".into()))
                            })
                            .collect()
                    };
                    (ix(&rc[0])?, ix(&rc[1])?)
                }
                _ => (vec![0], vec![0]),
            };
            let (cx, cy) = g.cell_centres(&rows, &cols);
            let probe = |name: &str| -> Result<Vec<f64>, EvalError> {
                match args.get(name) {
                    None | Some(Value::Null) => Ok(Vec::new()),
                    Some(_) => f64_list(args, name),
                }
            };
            let (sx, sy) = (probe("probe_xs")?, probe("probe_ys")?);
            let b = g.bounds();
            ok(json!({
                "ground": matrix_json(&g.ground),
                "wet": mask_json(&g.wet),
                "valid": mask_json(&g.valid),
                "x0": float_json(g.x0),
                "y0": float_json(g.y0),
                "cell_m": float_json(g.cell_m),
                "bounds": floats_json(&[b.0, b.1, b.2, b.3]),
                "cell_centres": [floats_json(&cx), floats_json(&cy)],
                "sample_ground": floats_json(&g.sample_ground(&sx, &sy)),
                "sample_wet": json!(g.sample_wet(&sx, &sy)),
            }))
        }
        "lattice_primitives" => {
            let c = tri!(config(arg(args, "cfg")?)?);
            ok(Value::Array(
                lt::lattice_primitives(&c).iter().map(prim_json).collect(),
            ))
        }
        "score" => {
            let g = tri!(grid_of(arg(args, "grid")?)?);
            let spec = tri!(section(arg(args, "section")?)?);
            let sc = FieldScorer::new(&g, f64_arg(args, "grade_m")?, f64_arg(args, "hb_m")?, &spec);
            let mut out = Vec::new();
            for q in list(args, "queries")? {
                let p = arg(q, "prim")?;
                let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
                let prim = if kind == "straight" {
                    let l = f64_arg(p, "length_m")?;
                    let step = f64_or(p, "step_m", 1.0)?;
                    let n = (l / step) as usize;
                    Prim {
                        r: f64::INFINITY,
                        sign: 0,
                        pts: (1..=n)
                            .map(|k| {
                                let v = l * k as f64 / n as f64;
                                [v, v, 0.0, 0.0]
                            })
                            .collect(),
                        end: (l, 0.0, 0.0),
                        arc_len: l,
                    }
                } else {
                    let c = tri!(config(arg(p, "cfg")?)?);
                    let idx = p
                        .get("index")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| bad("prim.index".into()))?
                        as usize;
                    lt::lattice_primitives(&c)
                        .into_iter()
                        .nth(idx)
                        .ok_or_else(|| bad("prim.index out of range".into()))?
                };
                let (cost, okk) = sc.score(&prim, pose(arg(q, "pose")?, "pose")?);
                out.push(json!({ "cost": float_json(cost), "ok": okk }));
            }
            ok(Value::Array(out))
        }
        "min_cost_per_metre" => {
            let g = tri!(grid_of(arg(args, "grid")?)?);
            let spec = tri!(section(arg(args, "section")?)?);
            let sc = FieldScorer::new(&g, f64_arg(args, "grade_m")?, f64_arg(args, "hb_m")?, &spec);
            let n_heading = arg(args, "n_heading")?
                .as_i64()
                .ok_or_else(|| bad("n_heading must be a whole number".into()))?;
            let max_samples = match args.get("max_samples").filter(|v| !v.is_null()) {
                Some(v) => v.as_u64().ok_or_else(|| bad("max_samples".into()))? as usize,
                None => lt::DEFAULT_C_MIN_MAX_SAMPLES,
            };
            ok(float_json(lt::min_cost_per_metre(
                &g,
                &sc,
                n_heading,
                f64_arg(args, "length_m")?,
                f64_or(args, "stride_m", lt::DEFAULT_C_MIN_STRIDE_M)?,
                max_samples,
            )))
        }
        "reverse_dijkstra_heuristic" => {
            let g = tri!(grid_of(arg(args, "grid")?)?);
            let spec = tri!(section(arg(args, "section")?)?);
            let sc = FieldScorer::new(&g, f64_arg(args, "grade_m")?, f64_arg(args, "hb_m")?, &spec);
            let goal = f64_list(args, "goal_xy")?;
            let n_heading = arg(args, "n_heading")?
                .as_i64()
                .ok_or_else(|| bad("n_heading must be a whole number".into()))?;
            let h = lt::reverse_dijkstra_heuristic(
                &g,
                &sc,
                [goal[0], goal[1]],
                n_heading,
                f64_arg(args, "length_m")?,
                f64_arg(args, "stride_m")?,
            );
            let mut out = Vec::new();
            for p in list(args, "probes")? {
                let xy = f64_list(&json!({ "p": p }), "p")?;
                out.push(h.h(xy[0], xy[1]));
            }
            ok(floats_json(&out))
        }
        "solve_lattice" => {
            let g = tri!(grid_of(arg(args, "grid")?)?);
            let spec = tri!(section(arg(args, "section")?)?);
            let cfg = tri!(config(arg(args, "cfg")?)?);
            let res = lt::solve_lattice(
                &g,
                pose(arg(args, "start_pose")?, "start_pose")?,
                pose(arg(args, "goal_pose")?, "goal_pose")?,
                f64_arg(args, "grade_m")?,
                &spec,
                &cfg,
            );
            ok(result_json(&res))
        }
        "heading_at" => {
            let line = line_of(args)?;
            let arm = f64_or(args, "arm_m", 20.0)?;
            let out: Vec<f64> = f64_list(args, "chainages")?
                .into_iter()
                .map(|s| lt::heading_at(&line, s, arm))
                .collect();
            ok(floats_json(&out))
        }
        "optimise_pool" => {
            let baseline = match geom_from(arg(args, "baseline")?, "baseline")? {
                Geometry::LineString(l) => l,
                _ => return Err(bad("`baseline` must be a line string".into())),
            };
            let sampler = terrain_from(arg(args, "terrain")?)?;
            let river: Vec<Polygon> = polygons_of(arg(args, "river_polygon")?)?;
            let spec = tri!(section(arg(args, "section")?)?);
            let cfg = tri!(config(arg(args, "cfg")?)?);
            let corridor_line = optional_line(args, "corridor_line")?;
            let pa = tri!(lt::optimise_pool(
                &baseline,
                &*sampler,
                &river,
                f64_arg(args, "start_m")?,
                f64_arg(args, "end_m")?,
                f64_arg(args, "water_level_m")?,
                f64_arg(args, "depth_m")?,
                &spec,
                &cfg,
                corridor_line.as_deref(),
                pool_options(args)?,
            ));
            ok(pool_json(&pa))
        }
        "optimise_pools" => {
            let baseline = match geom_from(arg(args, "baseline")?, "baseline")? {
                Geometry::LineString(l) => l,
                _ => return Err(bad("`baseline` must be a line string".into())),
            };
            let sampler = terrain_from(arg(args, "terrain")?)?;
            let river: Vec<Polygon> = polygons_of(arg(args, "river_polygon")?)?;
            let spec = tri!(section(arg(args, "section")?)?);
            let cfg = tri!(config(arg(args, "cfg")?)?);
            let corridor_line = optional_line(args, "corridor_line")?;
            let pools: Vec<(f64, f64, f64)> = list(args, "pools")?
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([a, b, w]) => Ok((
                        read_float(a, "pool")?,
                        read_float(b, "pool")?,
                        read_float(w, "pool")?,
                    )),
                    _ => Err(bad("a pool is [start, end, water level]".into())),
                })
                .collect::<Result<_, _>>()?;
            let keep = match args.get("keep_if_dearer") {
                None | Some(Value::Null) => true,
                Some(v) => v.as_bool().ok_or_else(|| bad("keep_if_dearer".into()))?,
            };
            let pr = tri!(lt::optimise_pools(
                &baseline,
                &*sampler,
                &river,
                &pools,
                f64_arg(args, "depth_m")?,
                &spec,
                &cfg,
                corridor_line.as_deref(),
                pool_options(args)?,
                keep,
            ));
            ok(json!({
                "line": geom_json(&Geometry::LineString(pr.line.clone())),
                "n_replaced": pr.n_replaced(),
                "pools": Value::Array(pr.pools.iter().map(pool_json).collect()),
            }))
        }
        other => Err(bad(format!("lattice has no parity function `{other}`"))),
    }
}
