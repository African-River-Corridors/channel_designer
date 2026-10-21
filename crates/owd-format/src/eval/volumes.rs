//! The `sections`, `cut_model_grid` and `cut_model_extras` entry points (ticket 26).

use super::geometry::geom_from;
use super::terrain::{river_from, terrain_from};
use super::{
    EvalError, arg, bad, f64_arg, f64_list, f64_or, float_json, floats_json, list, ok, opt_f64,
    read_float,
};
use owd_core::cut_model::{self as cm, CUT_MODEL};
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::pyrepr::repr_str;
use owd_core::rules::rulesets::channel_shape;
use owd_core::volumes::cut_model_grid::{self as grid, HalfWidths, Mask, Matrix};
use owd_core::volumes::sections::{self as sec, HalfWidth, Level};
use serde_json::{Map, Value, json};

fn value_error(message: String) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": "ValueError", "message": message } }))
}

fn bools_json(xs: &[bool]) -> Value {
    json!(xs)
}

fn str_arg(args: &Value, name: &str) -> Result<String, EvalError> {
    arg(args, name)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| bad(format!("`args.{name}` must be a string")))
}

fn matrix(args: &Value, name: &str) -> Result<Matrix, EvalError> {
    list(args, name)?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| bad(format!("`args.{name}` must be a list of lists")))?
                .iter()
                .map(|v| read_float(v, name))
                .collect()
        })
        .collect()
}

fn mask(args: &Value, name: &str) -> Result<Mask, EvalError> {
    list(args, name)?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| bad(format!("`args.{name}` must be a list of lists")))?
                .iter()
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| bad(format!("`args.{name}` must hold booleans")))
                })
                .collect()
        })
        .collect()
}

fn matrix_json(m: &Matrix) -> Value {
    Value::Array(m.iter().map(|r| floats_json(r)).collect())
}

fn half_widths(args: &Value) -> Result<HalfWidths, EvalError> {
    let v = arg(args, "hb")?;
    match v {
        Value::Number(_) => Ok(HalfWidths::Scalar(read_float(v, "hb")?)),
        Value::Array(items) => {
            if items.iter().all(|x| x.is_array()) && !items.is_empty() {
                let cols = items[0].as_array().map_or(0, Vec::len);
                return Ok(HalfWidths::Invalid(vec![items.len(), cols]));
            }
            items
                .iter()
                .map(|x| read_float(x, "hb"))
                .collect::<Result<Vec<_>, _>>()
                .map(HalfWidths::PerSection)
        }
        _ => Err(bad("`args.hb` must be a number or a list".into())),
    }
}

pub(super) fn cut_model_grid(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "cut_model_a_template" | "cut_model_b_template" => {
            let tmpl2 = matrix(args, "tmpl2")?;
            let gz2 = matrix(args, "gz2")?;
            let finite2 = mask(args, "finite2")?;
            let offs = f64_list(args, "offs")?;
            let hb = half_widths(args)?;
            let (grade, wl) = (f64_arg(args, "grade")?, f64_arg(args, "wl")?);
            let wet_slope = f64_or(args, "wet_slope", grid::DEFAULT_WET_SLOPE)?;
            let result = if function == "cut_model_a_template" {
                grid::cut_model_a_template(&tmpl2, &gz2, &finite2, &offs, &hb, grade, wl, wet_slope)
            } else {
                grid::cut_model_b_template(
                    &tmpl2,
                    &gz2,
                    &finite2,
                    &offs,
                    &hb,
                    grade,
                    wl,
                    wet_slope,
                    f64_or(args, "run_m", grid::DEFAULT_RUN_M)?,
                    opt_f64(args, "step")?,
                )
            };
            match result {
                Ok(m) => ok(matrix_json(&m)),
                Err(e) => value_error(e),
            }
        }
        "beyond_works_mask" => {
            let comp = matrix(args, "comp")?;
            let gz2 = matrix(args, "gz2")?;
            let finite2 = mask(args, "finite2")?;
            let offs = f64_list(args, "offs")?;
            let o_wl = vec![f64_arg(args, "o_wl")?; gz2.len()];
            let m = grid::beyond_works_mask(
                &comp,
                &gz2,
                &finite2,
                &offs,
                &o_wl,
                f64_arg(args, "run_m")?,
                f64_arg(args, "step")?,
            );
            ok(Value::Array(m.iter().map(|r| bools_json(r)).collect()))
        }
        other => Err(bad(format!(
            "cut_model_grid has no parity function `{other}`"
        ))),
    }
}

pub(super) fn cut_model_extras(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "normalise_model" => {
            let names = list(args, "names")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(cm::normalise_model)
                        .ok_or_else(|| bad("names must be strings".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            ok(json!(names))
        }
        "local_radii" => {
            let xs = f64_list(args, "xs")?;
            let ys = f64_list(args, "ys")?;
            let stride = match args.get("stride") {
                None | Some(Value::Null) => 2,
                Some(v) => v
                    .as_u64()
                    .ok_or_else(|| bad("`args.stride` must be a whole number".into()))?
                    as usize,
            };
            ok(floats_json(&cm::local_radii(&xs, &ys, stride)))
        }
        "widened_hb_from_shape" => {
            let xy = xy_list(args, "xy")?;
            let rmax = f64_or(args, "rmax", cm::WIDEN_RMAX)?;
            match channel_shape(
                &crate::eval_rules::ruleset_registry(args)?,
                &str_arg(args, "ruleset_id")?,
                &str_arg(args, "vessel_standard_id")?,
                None,
            ) {
                Ok(shape) => ok(floats_json(&cm::widened_hb_from_shape(&xy, &shape, rmax))),
                Err(e) => Ok(json!({ "error": {
                    "type": e.type_name(), "message": e.python_message() } })),
            }
        }
        "cut_model_a_section_face" => {
            let face = cm::cut_model_a_section_face(
                &f64_list(args, "offsets")?,
                &f64_list(args, "terrain")?,
                f64_arg(args, "hb")?,
                f64_arg(args, "grade")?,
                f64_arg(args, "wl")?,
                f64_arg(args, "depth")?,
                f64_or(args, "wet_slope", cm::WET_SLOPE_H_PER_V)?,
                &match args.get("model") {
                    None | Some(Value::Null) => CUT_MODEL.to_string(),
                    Some(v) => v
                        .as_str()
                        .ok_or_else(|| bad("`args.model` must be a string".into()))?
                        .to_string(),
                },
                opt_f64(args, "run_m")?,
            );
            ok(Value::Array(
                face.iter()
                    .map(|f| f.map_or(Value::Null, float_json))
                    .collect(),
            ))
        }
        other => Err(bad(format!(
            "cut_model_extras has no parity function `{other}`"
        ))),
    }
}

fn xy_list(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    list(args, name)?
        .iter()
        .map(|p| match p.as_array().map(Vec::as_slice) {
            Some([x, y]) => Ok([read_float(x, name)?, read_float(y, name)?]),
            _ => Err(bad(format!("`args.{name}` entries must be [x, y]"))),
        })
        .collect()
}

pub(super) fn polygons_of(v: &Value) -> Result<Vec<Polygon>, EvalError> {
    match geom_from(v, "river_polygon")? {
        Geometry::Polygon(p) => Ok(vec![p]),
        Geometry::MultiPolygon(ps) => Ok(ps),
        _ => Err(bad("`river_polygon` must be a polygon".into())),
    }
}

fn section_json(s: &sec::CrossSection) -> Value {
    json!({
        "chainage_m": float_json(s.chainage_m),
        "x": float_json(s.x),
        "y": float_json(s.y),
        "heading_rad": float_json(s.heading_rad),
        "hb": float_json(s.hb),
        "wl": float_json(s.wl),
        "grade": float_json(s.grade),
        "depth": float_json(s.depth),
        "model": s.model,
        "dredge_area_m2": float_json(s.dredge_area_m2),
        "excav_area_m2": float_json(s.excav_area_m2),
        "max_face_h_m": float_json(s.max_face_h_m),
        "bench_left": s.bench_left,
        "bench_right": s.bench_right,
        "tag": s.tag,
    })
}

pub(super) fn sections(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "station_frame" => {
            let line = line_of(args)?;
            match sec::station_frame(&line, f64_or(args, "station_m", sec::DEFAULT_STATION_M)?) {
                Ok((d, xy, th)) => ok(json!({
                    "chainages": floats_json(&d),
                    "xy": Value::Array(xy.iter().map(|p| json!([float_json(p[0]), float_json(p[1])])).collect()),
                    "headings": floats_json(&th),
                })),
                Err(e) => value_error(e),
            }
        }
        "trapezoid_area" => ok(float_json(sec::trapezoid_area(
            f64_arg(args, "hb")?,
            f64_arg(args, "depth")?,
            f64_arg(args, "side_slope")?,
        ))),
        "section_volumes" => section_volumes(args),
        other => Err(bad(format!("sections has no parity function `{other}`"))),
    }
}

pub(super) fn line_of(args: &Value) -> Result<Vec<Pt>, EvalError> {
    match geom_from(arg(args, "line")?, "line")? {
        Geometry::LineString(l) => Ok(l),
        _ => Err(bad("`line` must be a line string".into())),
    }
}

fn section_volumes(args: &Value) -> Result<Value, EvalError> {
    let line = line_of(args)?;
    let sampler = terrain_from(arg(args, "terrain")?)?;
    let river = polygons_of(arg(args, "river_polygon")?)?;
    let depth = f64_arg(args, "depth")?;

    // wl: a number, or a spec: {"kind": "linear", "wl0", "slope"}, {"kind": "synthetic_water_level",
    // "params"} (`river.water_level`), or {"kind": "pool_levels", "line", "pools"} (the pool level
    // at the baseline chainage nearest this line's chainage `s`, as examples/optimise_pools.py).
    let wl_spec = arg(args, "wl")?;
    let wl_fn: Option<Box<dyn Fn(f64) -> f64>> = match wl_spec {
        w if crate::host::callback_id(w).is_some() => Some(crate::host::scalar_fn(
            crate::host::callback_id(w).unwrap_or(0),
        )?),
        Value::Object(_) => {
            let kind = wl_spec
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("linear");
            match kind {
                "linear" => {
                    let (w0, k) = (f64_arg(wl_spec, "wl0")?, f64_arg(wl_spec, "slope")?);
                    Some(Box::new(move |s| w0 + k * s))
                }
                "synthetic_water_level" => {
                    let river = river_from(wl_spec.get("params"))?;
                    Some(Box::new(move |s| river.water_level(s)))
                }
                "pool_levels" => {
                    let base = match geom_from(arg(wl_spec, "line")?, "wl.line")? {
                        Geometry::LineString(l) => l,
                        _ => return Err(bad("`wl.line` must be a line string".into())),
                    };
                    let pools: Vec<(f64, f64)> = list(wl_spec, "pools")?
                        .iter()
                        .map(|p| match p.as_array().map(Vec::as_slice) {
                            Some([a, _b, l]) => {
                                Ok((read_float(a, "pool")?, read_float(l, "pool")?))
                            }
                            _ => Err(bad("a pool is [start, end, level]".into())),
                        })
                        .collect::<Result<_, _>>()?;
                    let section_line = line.clone();
                    Some(Box::new(move |s| {
                        let at = linear::interpolate(&section_line, s).unwrap_or([f64::NAN; 2]);
                        let sp = linear::project(&base, at);
                        let x = linear::interpolate(&base, sp).map_or(f64::NAN, |p| p[0]);
                        pools
                            .iter()
                            .filter(|(a, _)| *a <= x)
                            .map(|(_, l)| *l)
                            .fold(None, |m: Option<f64>, l| match m {
                                Some(m) if m >= l => Some(m),
                                _ => Some(l),
                            })
                            .unwrap_or(0.0)
                    }))
                }
                other => return Err(bad(format!("unknown water-level kind `{other}`"))),
            }
        }
        _ => None,
    };
    let wl = match &wl_fn {
        Some(f) => Level::AtChainage(&**f),
        None => Level::Fixed(read_float(wl_spec, "wl")?),
    };

    // hb: a number, a list (one per station), or {"kind": "widened_from_shape", ...}
    let hb_spec = arg(args, "hb")?;
    let hb_host = match crate::host::callback_id(hb_spec) {
        Some(id) => Some(crate::host::arrays_fn(id)?),
        None => None,
    };
    let shape = match hb_spec {
        Value::Object(_) if hb_host.is_none() => {
            match channel_shape(
                &crate::eval_rules::ruleset_registry(args)?,
                &str_arg(hb_spec, "ruleset_id")?,
                &str_arg(hb_spec, "vessel_standard_id")?,
                None,
            ) {
                Ok(s) => Some(s),
                Err(e) => {
                    return Ok(json!({ "error": {
                        "type": e.type_name(), "message": e.python_message() } }));
                }
            }
        }
        _ => None,
    };
    let from_shape = |xy: &[Pt]| match (&hb_host, &shape) {
        (Some(f), _) => {
            let xs: Vec<f64> = xy.iter().map(|p| p[0]).collect();
            let ys: Vec<f64> = xy.iter().map(|p| p[1]).collect();
            f(&xs, &ys)
        }
        (None, Some(s)) => cm::widened_hb_from_shape(xy, s, cm::WIDEN_RMAX),
        (None, None) => Vec::new(),
    };
    let hb = match hb_spec {
        Value::Object(_) => HalfWidth::FromStations(&from_shape),
        Value::Array(_) => HalfWidth::PerStation(f64_list(args, "hb")?),
        _ => HalfWidth::Fixed(read_float(hb_spec, "hb")?),
    };

    let tag_spec = args.get("tag").filter(|v| !v.is_null());
    let tag_host = match tag_spec.and_then(crate::host::callback_id) {
        Some(id) => Some(crate::host::text_fn(id)?),
        None => None,
    };
    let tag_fn = |s: f64| -> String {
        if let Some(f) = &tag_host {
            return f(s);
        }
        let t = tag_spec.expect("tag spec");
        let key = if s < t["at"].as_f64().unwrap_or(f64::NAN) {
            "before"
        } else {
            "after"
        };
        t[key].as_str().unwrap_or("").to_string()
    };
    let tag: Option<&dyn Fn(f64) -> String> = tag_spec.map(|_| &tag_fn as &dyn Fn(f64) -> String);

    let model = match args.get("model") {
        None | Some(Value::Null) => CUT_MODEL.to_string(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| bad("`args.model` must be a string".into()))?
            .to_string(),
    };
    let chunk = match args.get("chunk") {
        None | Some(Value::Null) => sec::DEFAULT_CHUNK,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| bad("`args.chunk` must be a whole number".into()))?
            as usize,
    };
    let result = sec::section_volumes(
        &line,
        &*sampler,
        &river,
        &wl,
        depth,
        &hb,
        &model,
        f64_or(args, "wet_slope", sec::DEFAULT_WET_SLOPE)?,
        f64_or(args, "station_m", sec::DEFAULT_STATION_M)?,
        f64_or(args, "maxw_m", sec::DEFAULT_MAXW_M)?,
        f64_or(args, "dx_m", sec::DEFAULT_DX_M)?,
        opt_f64(args, "daylight_run_m")?,
        tag,
        chunk,
    );
    let sv = match result {
        Ok(sv) => sv,
        Err(e) => return value_error(e),
    };
    // `"all"`: the arrays of every station (the Python `CrossSection` carries them).
    let wanted: Option<Vec<Value>> = match args.get("detail_stations") {
        Some(Value::Array(idx)) => Some(idx.clone()),
        Some(Value::String(s)) if s == "all" => {
            Some((0..sv.stations.len()).map(|i| json!(i)).collect())
        }
        _ => None,
    };
    let detail: Vec<Value> = match &wanted {
        Some(idx) => idx
            .iter()
            .map(|i| {
                let i = i.as_u64().ok_or_else(|| bad("detail index".into()))? as usize;
                let st = sv
                    .stations
                    .get(i)
                    .ok_or_else(|| bad("detail station out of range".into()))?;
                Ok(json!({
                    "index": i,
                    "offsets": floats_json(&st.offsets),
                    "terrain": floats_json(&st.terrain),
                    "wet": bools_json(&st.wet),
                    "cutface": floats_json(&st.cutface),
                }))
            })
            .collect::<Result<_, EvalError>>()?,
        None => Vec::new(),
    };
    let mut by_tag = Map::new();
    for (t, v) in sv.by_tag() {
        by_tag.insert(
            t,
            json!({ "dredge_m3": float_json(v.dredge_m3), "excav_m3": float_json(v.excav_m3) }),
        );
    }
    let _ = repr_str; // quoted by the core's messages
    ok(json!({
        "station_m": float_json(sv.station_m),
        "dredge_m3": float_json(sv.dredge_m3()),
        "excav_m3": float_json(sv.excav_m3()),
        "total_m3": float_json(sv.total_m3()),
        "max_face_h_m": float_json(sv.max_face_h_m()),
        "by_tag": Value::Object(by_tag),
        "stations": Value::Array(sv.stations.iter().map(section_json).collect()),
        "detail": detail,
    }))
}
