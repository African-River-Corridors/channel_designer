//! The `cascade` entry point (ticket 31): the weir cascade search, and the level table behind its
//! earthworks cost.
//!
//! Cost callables are specs (or callbacks into the host): a reach cost is `bumpy` (a
//! trapezoid of a Gaussian bump over 9 points, times `exp(-decay * level)`) or `earthworks` (a
//! level table); a structure cost is `power` (`fixed + coeff * lift ** power`).

use super::terrain::terrain_from;
use super::volumes::{line_of, polygons_of};
use super::{
    EvalError, arg, bad, f64_arg, f64_list, f64_or, float_json, floats_json, list, ok, opt_f64,
    read_float,
};
use cd_core::optimise::cascade::{
    self as cs, Cascade, CascadeConfig, CascadeSearch, LevelTable, Refusal, SectionOptions,
};
use cd_core::volumes::sections::HalfWidth;
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

fn matrix(v: &Value, name: &str) -> Result<Vec<Vec<f64>>, EvalError> {
    v.as_array()
        .ok_or_else(|| bad(format!("`{name}` must be a list of lists")))?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| bad(format!("`{name}` must be a list of lists")))?
                .iter()
                .map(|x| read_float(x, name))
                .collect()
        })
        .collect()
}

fn float_vec(v: &Value, name: &str) -> Result<Vec<f64>, EvalError> {
    v.as_array()
        .ok_or_else(|| bad(format!("`{name}` must be a list")))?
        .iter()
        .map(|x| read_float(x, name))
        .collect()
}

fn table(spec: &Value) -> Result<LevelTable, EvalError> {
    let mut t = LevelTable::new(
        float_vec(arg(spec, "chainage_m")?, "table.chainage_m")?,
        float_vec(arg(spec, "levels_m")?, "table.levels_m")?,
        matrix(arg(spec, "dredge_m2")?, "table.dredge_m2")?,
        matrix(arg(spec, "excav_m2")?, "table.excav_m2")?,
        f64_arg(spec, "station_m")?,
    );
    if let Some(v) = spec.get("level_tol_m").filter(|v| !v.is_null()) {
        t.level_tol_m = read_float(v, "table.level_tol_m")?;
    }
    Ok(t)
}

fn config(d: &Value) -> Result<Result<CascadeConfig, Refusal>, EvalError> {
    let mut cfg = CascadeConfig::new(
        float_vec(arg(d, "candidate_sites_m")?, "cfg.candidate_sites_m")?,
        float_vec(arg(d, "lifts_m")?, "cfg.lifts_m")?,
        f64_arg(d, "terminal_m")?,
        f64_arg(d, "terminal_min_level_m")?,
    );
    cfg.base_level_m = f64_or(d, "base_level_m", 0.0)?;
    cfg.start_m = f64_or(d, "start_m", 0.0)?;
    cfg.min_spacing_m = f64_or(d, "min_spacing_m", 0.0)?;
    cfg.level_cap_m = opt_f64(d, "level_cap_m")?;
    if let Some(k) = d.get("k_best").filter(|v| !v.is_null()) {
        cfg.k_best = k
            .as_i64()
            .ok_or_else(|| bad("cfg.k_best must be a whole number".into()))?;
    }
    if let Some(ids) = d.get("site_ids").filter(|v| !v.is_null()) {
        cfg.site_ids = Some(
            ids.as_array()
                .ok_or_else(|| bad("cfg.site_ids must be a list".into()))?
                .iter()
                .map(|x| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| bad("site ids must be strings".into()))
                })
                .collect::<Result<_, _>>()?,
        );
    }
    Ok(cfg.validated())
}

type Reach = Box<dyn Fn(f64, f64, f64) -> Result<f64, Refusal>>;

/// numpy's `sum` of 8 doubles: one pass of its 8-way unrolled loop, then a fixed tree.
fn sum8(t: [f64; 8]) -> f64 {
    ((t[0] + t[1]) + (t[2] + t[3])) + ((t[4] + t[5]) + (t[6] + t[7]))
}

fn reach_fn(spec: &Value) -> Result<Reach, EvalError> {
    if let Some(id) = crate::host::callback_id(spec) {
        let f = crate::host::triple_fn(id)?;
        return Ok(Box::new(move |a, b, level| Ok(f(a, b, level))));
    }
    let kind = spec
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("reach spec needs `kind`".into()))?;
    match kind {
        "bumpy" => {
            let (base, amp, centre, width, decay) = (
                f64_arg(spec, "base")?,
                f64_arg(spec, "amp")?,
                f64_arg(spec, "centre")?,
                f64_arg(spec, "width")?,
                f64_arg(spec, "decay")?,
            );
            Ok(Box::new(move |a, b, level| {
                // np.linspace(a, b, 9), then np.trapezoid(dens, x).
                let step = (b - a) / 8.0;
                let mut x: Vec<f64> = (0..9)
                    .map(|i| {
                        if step == 0.0 {
                            (i as f64 / 8.0) * (b - a) + a
                        } else {
                            i as f64 * step + a
                        }
                    })
                    .collect();
                x[8] = b;
                let dens: Vec<f64> = x
                    .iter()
                    .map(|&xi| {
                        let t = (xi - centre) / width;
                        base + amp * libm::exp(-(t * t))
                    })
                    .collect();
                let mut terms = [0.0; 8];
                for i in 0..8 {
                    terms[i] = (x[i + 1] - x[i]) * (dens[i + 1] + dens[i]) / 2.0;
                }
                Ok(sum8(terms) * libm::exp(-decay * level))
            }))
        }
        "earthworks" => {
            let t = table(arg(spec, "table")?)?;
            let (wet, dry) = (
                f64_or(spec, "wet_weight", 1.0)?,
                f64_or(spec, "dry_weight", 1.0)?,
            );
            Ok(Box::new(move |a, b, level| {
                cs::earthworks_reach_cost(&t, wet, dry)(a, b, level)
            }))
        }
        other => Err(bad(format!("unknown reach spec `{other}`"))),
    }
}

fn structure_fn(spec: &Value) -> Result<Box<dyn Fn(f64) -> f64>, EvalError> {
    if let Some(id) = crate::host::callback_id(spec) {
        return crate::host::scalar_fn(id);
    }
    let (fixed, coeff, power) = (
        f64_arg(spec, "fixed")?,
        f64_arg(spec, "coeff")?,
        f64_arg(spec, "power")?,
    );
    Ok(Box::new(move |lift: f64| {
        fixed + coeff * libm::pow(lift, power)
    }))
}

fn weir_json(w: &cs::CascadeWeir) -> Value {
    json!({
        "site_id": w.site_id,
        "chainage_m": float_json(w.chainage_m),
        "pool_level_m": float_json(w.pool_level_m),
        "lift_m": float_json(w.lift_m),
        "structure_cost": float_json(w.structure_cost),
    })
}

fn cascade_json(c: &Cascade) -> Value {
    json!({
        "total_cost": float_json(c.total_cost),
        "weirs": c.weirs.iter().map(weir_json).collect::<Vec<_>>(),
        "reaches": c.reaches.iter().map(|r| json!({
            "start_m": float_json(r.start_m),
            "end_m": float_json(r.end_m),
            "level_m": float_json(r.level_m),
            "cost": float_json(r.cost),
        })).collect::<Vec<_>>(),
    })
}

fn pools_json(c: &Cascade) -> Value {
    Value::Array(
        c.pools()
            .iter()
            .map(|&(a, b, l)| json!([float_json(a), float_json(b), float_json(l)]))
            .collect(),
    )
}

fn search_json(r: &CascadeSearch) -> Value {
    json!({
        "cascades": r.cascades.iter().map(cascade_json).collect::<Vec<_>>(),
        "levels_m": floats_json(&r.levels_m),
        "states_explored": r.states_explored,
        "feasible_endings": r.feasible_endings,
        "sites_dropped_beyond_terminal": r.sites_dropped_beyond_terminal,
    })
}

fn table_json(t: &LevelTable) -> Value {
    let m = |rows: &Vec<Vec<f64>>| Value::Array(rows.iter().map(|r| floats_json(r)).collect());
    json!({
        "chainage_m": floats_json(&t.chainage_m),
        "levels_m": floats_json(&t.levels_m),
        "dredge_m2": m(&t.dredge_m2),
        "excav_m2": m(&t.excav_m2),
        "station_m": float_json(t.station_m),
        "level_tol_m": float_json(t.level_tol_m),
    })
}

fn queries(args: &Value) -> Result<Vec<(f64, f64, f64)>, EvalError> {
    list(args, "queries")?
        .iter()
        .map(|q| match q.as_array().map(Vec::as_slice) {
            Some([a, b, c]) => Ok((
                read_float(a, "q")?,
                read_float(b, "q")?,
                read_float(c, "q")?,
            )),
            _ => Err(bad("a query is [start, end, level]".into())),
        })
        .collect()
}

pub(super) fn cascade(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "cascade_levels" => {
            let cfg = tri!(config(arg(args, "cfg")?)?);
            ok(floats_json(&cs::cascade_levels(&cfg)))
        }
        "search_cascade" => {
            let cfg = tri!(config(arg(args, "cfg")?)?);
            let reach = reach_fn(arg(args, "reach")?)?;
            let structure = structure_fn(arg(args, "structure")?)?;
            let res = tri!(cs::search_cascade(&cfg, &*reach, &structure));
            let height = opt_f64(args, "structure_height_m")?;
            ok(json!({
                "search": search_json(&res),
                "best": res.best().map_or(Value::Null, cascade_json),
                "cascades_pools": res.cascades.iter().map(pools_json).collect::<Vec<_>>(),
                "cascades_explicit": res.cascades.iter().map(|c| Value::Array(
                    c.as_explicit_weirs(height).iter().map(|w| json!({
                        "chainage_m": float_json(w.chainage_m),
                        "pool_mwl_m": float_json(w.pool_mwl_m),
                        "structure_height_m": w.structure_height_m.map_or(Value::Null, float_json),
                    })).collect())).collect::<Vec<_>>(),
                "reach_cost": floats_json(&res.cascades.iter().map(Cascade::reach_cost).collect::<Vec<_>>()),
                "structure_cost": floats_json(&res.cascades.iter().map(Cascade::structure_cost).collect::<Vec<_>>()),
            }))
        }
        "score_cascade" => {
            let cfg = tri!(config(arg(args, "cfg")?)?);
            let reach = reach_fn(arg(args, "reach")?)?;
            let structure = structure_fn(arg(args, "structure")?)?;
            let weirs: Vec<(f64, f64)> = list(args, "weirs")?
                .iter()
                .map(|w| match w.as_array().map(Vec::as_slice) {
                    Some([c, l]) => Ok((read_float(c, "weir")?, read_float(l, "weir")?)),
                    _ => Err(bad("a weir is [chainage, lift]".into())),
                })
                .collect::<Result<_, _>>()?;
            let c = tri!(cs::score_cascade(&cfg, &weirs, &*reach, &structure));
            ok(json!({
                "cascade": cascade_json(&c),
                "reach_cost": float_json(c.reach_cost()),
                "structure_cost": float_json(c.structure_cost()),
                "pools": pools_json(&c),
            }))
        }
        "reach_volumes" => {
            let t = table(arg(args, "table")?)?;
            let mut out = Vec::new();
            for (a, b, lv) in queries(args)? {
                let (d, e) = tri!(t.reach_volumes(a, b, lv));
                out.push(floats_json(&[d, e]));
            }
            ok(Value::Array(out))
        }
        "level_index" => {
            let t = table(arg(args, "table")?)?;
            let mut out = Vec::new();
            for lv in f64_list(args, "levels")? {
                out.push(json!(tri!(t.level_index(lv))));
            }
            ok(Value::Array(out))
        }
        "earthworks_reach_cost" => {
            let t = table(arg(args, "table")?)?;
            let f = cs::earthworks_reach_cost(
                &t,
                f64_or(args, "wet_weight", 1.0)?,
                f64_or(args, "dry_weight", 1.0)?,
            );
            let mut out = Vec::new();
            for (a, b, lv) in queries(args)? {
                out.push(float_json(tri!(f(a, b, lv))));
            }
            ok(Value::Array(out))
        }
        "build_level_table" => {
            let line = line_of(args)?;
            let sampler = terrain_from(arg(args, "terrain")?)?;
            let river = polygons_of(arg(args, "river_polygon")?)?;
            let hb = HalfWidth::Fixed(f64_arg(args, "hb")?);
            let levels = f64_list(args, "levels_m")?;
            let chainage = match args.get("chainage_m").filter(|v| !v.is_null()) {
                Some(v) => Some(float_vec(v, "chainage_m")?),
                None => None,
            };
            let mut opts = SectionOptions::default();
            opts.wet_slope = f64_or(args, "wet_slope", opts.wet_slope)?;
            opts.dx_m = f64_or(args, "dx_m", opts.dx_m)?;
            opts.daylight_run_m = opt_f64(args, "daylight_run_m")?;
            let model = args.get("model").and_then(Value::as_str);
            let t = tri!(cs::build_level_table(
                &line,
                &*sampler,
                &river,
                &levels,
                f64_arg(args, "depth_m")?,
                &hb,
                model,
                opt_f64(args, "station_m")?,
                opt_f64(args, "maxw_m")?,
                chainage.as_deref(),
                opts,
            ));
            ok(table_json(&t))
        }
        other => Err(bad(format!("cascade has no parity function `{other}`"))),
    }
}
