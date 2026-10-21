//! The `footprint` and `pools` entry points (ticket 30): the plan excavation footprint, and the
//! bed profile and weir cascade that set the pool levels.

use super::geometry::{geom_from, geom_json};
use super::terrain::terrain_from;
use super::{
    EvalError, arg, bad, f64_arg, f64_list, f64_or, float_json, floats_json, list, ok, opt_f64,
    read_float,
};
use owd_core::geom::{Geometry, Polygon, Pt};
use owd_core::volumes::footprint::{self as fp, FootprintParams};
use owd_core::volumes::pools::{self as pl, BedProfile, ExplicitWeir, ProfileParams, Weir};
use serde_json::{Value, json};

fn value_error(message: String) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": "ValueError", "message": message } }))
}

fn line_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    match geom_from(arg(args, name)?, &format!("args.{name}"))? {
        Geometry::LineString(l) => Ok(l),
        _ => Err(bad(format!("`args.{name}` must be a LineString"))),
    }
}

fn polys_of(g: Geometry) -> Vec<Polygon> {
    match g {
        Geometry::Polygon(p) if !p.exterior.is_empty() => vec![p],
        Geometry::MultiPolygon(ps) => ps.into_iter().filter(|p| !p.exterior.is_empty()).collect(),
        Geometry::Collection(gs) => gs.into_iter().flat_map(polys_of).collect(),
        _ => vec![],
    }
}

fn polys_arg(args: &Value, name: &str) -> Result<Vec<Polygon>, EvalError> {
    Ok(polys_of(geom_from(
        arg(args, name)?,
        &format!("args.{name}"),
    )?))
}

fn poly_json(p: &Polygon) -> Value {
    geom_json(&Geometry::Polygon(p.clone()))
}

/// One polygon as a Polygon, several as a MultiPolygon.
fn polys_json(mut ps: Vec<Polygon>) -> Value {
    if ps.len() == 1 {
        poly_json(&ps.remove(0))
    } else {
        geom_json(&Geometry::MultiPolygon(ps))
    }
}

fn line_json(l: &[Pt]) -> Value {
    geom_json(&Geometry::LineString(l.to_vec()))
}

/// Python `repr` of a float as an f-string prints it (`2.0`, `0.5`, `1e-05`, `1e+16`).
fn py_float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    // The decimal exponent, from the shortest round-trip digits.
    let sci = format!("{x:e}");
    let exp: i32 = sci
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    if (-4..16).contains(&exp) {
        let s = format!("{x}");
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        let (m, e) = sci.split_once('e').unwrap_or((&sci, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    }
}

/// A number argument as Python prints it: an int as an int, a float by `repr`.
fn py_number_text(v: &Value) -> String {
    match v.as_i64() {
        Some(i) if v.is_i64() => i.to_string(),
        _ => py_float_repr(v.as_f64().unwrap_or(f64::NAN)),
    }
}

/// `limits.extract_lines` for a centreline geometry; a point is refused as shapely refuses it.
fn extract_lines(g: Geometry) -> Result<Result<Vec<Vec<Pt>>, String>, EvalError> {
    Ok(Ok(match g {
        Geometry::Empty(_) => vec![],
        Geometry::LineString(l) if l.is_empty() => vec![],
        Geometry::LineString(l) => vec![l],
        Geometry::MultiLineString(ls) => ls.into_iter().filter(|l| !l.is_empty()).collect(),
        Geometry::Point(p) => {
            let t = |x: f64| {
                if x.fract() == 0.0 && x.abs() < 1e15 {
                    format!("{}", x as i64)
                } else {
                    format!("{x}")
                }
            };
            return Ok(Err(format!(
                "Cannot linemerge POINT ({} {})",
                t(p[0]),
                t(p[1])
            )));
        }
        _ => return Err(bad("the centreline must be lines".into())),
    }))
}

fn excavation_json(es: &[fp::Excavation]) -> Value {
    Value::Array(
        es.iter()
            .map(|e| json!({"geometry": poly_json(&e.polygon), "bank_side": e.bank_side, "area_m2": float_json(e.area_m2)}))
            .collect(),
    )
}

pub(super) fn footprint(function: &str, args: &Value) -> Result<Value, EvalError> {
    let res = |r: Result<Value, String>| match r {
        Ok(v) => ok(v),
        Err(m) => value_error(m),
    };
    match function {
        "orient_like" => ok(line_json(&fp::orient_like(
            &line_arg(args, "reference")?,
            &line_arg(args, "candidate")?,
        ))),
        "polygon_from_bank_pair" => res(fp::polygon_from_bank_pair(
            &line_arg(args, "left")?,
            &line_arg(args, "right")?,
        )
        .map(polys_json)),
        "iter_polygons" => {
            let ps = polys_of(geom_from(arg(args, "geom")?, "args.geom")?);
            ok(Value::Array(ps.iter().map(poly_json).collect()))
        }
        "subtract_polygons" => res(fp::subtract_polygons(
            &polys_arg(args, "channel_poly")?,
            &polys_arg(args, "river_poly")?,
            f64_or(args, "min_area", 0.5)?,
        )
        .map(|ps| Value::Array(ps.iter().map(poly_json).collect()))),
        "classify_sides" => {
            let polys: Vec<Polygon> = list(args, "polygons")?
                .iter()
                .map(|p| geom_from(p, "args.polygons").map(polys_of))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            ok(json!(fp::classify_sides(
                &polys,
                &line_arg(args, "left_line")?,
                &line_arg(args, "right_line")?
            )))
        }
        "compute_excavation" => res(fp::compute_excavation(
            &line_arg(args, "channel_left")?,
            &line_arg(args, "channel_right")?,
            &line_arg(args, "bank_left")?,
            &line_arg(args, "bank_right")?,
            f64_or(args, "min_area", 0.5)?,
        )
        .map(|es| excavation_json(&es))),
        "channel_footprint" => {
            let lines =
                match extract_lines(geom_from(arg(args, "centreline")?, "args.centreline")?)? {
                    Ok(l) => l,
                    Err(m) => return value_error(m),
                };
            let mut p = FootprintParams::new(
                f64_arg(args, "loa")?,
                f64_arg(args, "beam")?,
                f64_arg(args, "depth")?,
            );
            p.side_slope = f64_or(args, "side_slope", p.side_slope)?;
            p.width_beam_multiple = f64_or(args, "width_beam_multiple", p.width_beam_multiple)?;
            p.min_radius_length_multiple = f64_or(
                args,
                "min_radius_length_multiple",
                p.min_radius_length_multiple,
            )?;
            p.extra_width_coefficient =
                f64_or(args, "extra_width_coefficient", p.extra_width_coefficient)?;
            p.radius_sample_length = f64_or(args, "radius_sample_length", p.radius_sample_length)?;
            p.max_extra_width_radius =
                f64_or(args, "max_extra_width_radius", p.max_extra_width_radius)?;
            p.min_area = f64_or(args, "min_area", p.min_area)?;
            res(fp::channel_footprint(
                lines,
                &line_arg(args, "bank_left")?,
                &line_arg(args, "bank_right")?,
                &p,
            )
            .map(|f| {
                let lines =
                    |ls: &[Vec<Pt>]| Value::Array(ls.iter().map(|l| line_json(l)).collect());
                let d = &f.diagnostics;
                json!({
                    "footprint": {
                        "centreline": lines(&f.centreline),
                        "channel_left": lines(&f.channel_left),
                        "channel_right": lines(&f.channel_right),
                        "excavation": excavation_json(&f.excavation),
                        "diagnostics": {
                            "min_radius": d.min_radius.map_or(Value::Null, float_json),
                            "max_extra_width": float_json(d.max_extra_width),
                            "mean_extra_width": float_json(d.mean_extra_width),
                        },
                    },
                    "total_excavation_area_m2": float_json(f.total_excavation_area_m2()),
                })
            }))
        }
        f => Err(bad(format!("footprint has no parity function `{f}`"))),
    }
}

/// `_profile`: the bed profile a spec describes.
fn profile(spec: &Value) -> Result<BedProfile, EvalError> {
    // A profile the host already has (`BedProfile` fields): used as it is.
    if spec.get("z_overall").is_some() {
        let pts = |name: &str| -> Result<Vec<Pt>, EvalError> {
            list(spec, name)?
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([x, y]) => Ok([read_float(x, name)?, read_float(y, name)?]),
                    _ => Err(bad(format!("`{name}` entries must be [x, y]"))),
                })
                .collect()
        };
        return Ok(BedProfile {
            s: f64_list(spec, "s")?,
            z_raw: f64_list(spec, "z_raw")?,
            z_overall: f64_list(spec, "z_overall")?,
            points: pts("points")?,
            line: line_arg(spec, "line")?,
        });
    }
    let d = ProfileParams::default();
    let p = ProfileParams {
        spacing_m: f64_or(spec, "spacing_m", d.spacing_m)?,
        slope_percentile: f64_or(spec, "slope_percentile", d.slope_percentile)?,
        slope_perc_window_m: f64_or(spec, "slope_perc_window_m", d.slope_perc_window_m)?,
        slope_smooth_window_m: f64_or(spec, "slope_smooth_window_m", d.slope_smooth_window_m)?,
    };
    let terrain = terrain_from(arg(spec, "terrain")?)?;
    Ok(pl::sample_bed_profile(
        &line_arg(spec, "centerline")?,
        &|xs, ys| terrain(xs, ys),
        &p,
    ))
}

/// `_weirs`: the cascade a spec describes, or the Python `ValueError` message.
fn weirs(spec: &Value) -> Result<Result<Vec<Weir>, String>, EvalError> {
    let prof = profile(arg(spec, "profile")?)?;
    let place = arg(spec, "place")?;
    if place.get("kind").and_then(Value::as_str) == Some("explicit") {
        let cascade = list(place, "cascade")?
            .iter()
            .map(|w| {
                Ok(ExplicitWeir {
                    chainage_m: f64_arg(w, "chainage_m")?,
                    pool_mwl_m: f64_arg(w, "pool_mwl_m")?,
                    structure_height_m: opt_f64(w, "structure_height_m")?,
                })
            })
            .collect::<Result<Vec<_>, EvalError>>()?;
        return Ok(Ok(pl::place_weirs_explicit(&prof, &cascade)));
    }
    let (h, d) = (
        arg(place, "weir_height_m")?,
        arg(place, "implantation_depth_m")?,
    );
    let r = pl::place_weirs(
        &prof,
        read_float(h, "weir_height_m")?,
        read_float(d, "implantation_depth_m")?,
        opt_f64(place, "start_chainage_m")?,
        f64_or(place, "start_elevation_m", 0.0)?,
        opt_f64(place, "max_weir_elevation_m")?,
    );
    Ok(r.map_err(|lift| {
        // An int minus an int stays an int in Python; anything else is a float.
        let lift_text = if h.is_i64() && d.is_i64() {
            format!("{}", lift as i64)
        } else {
            py_float_repr(lift)
        };
        format!(
            "pool lift must be > 0 (H={}, d={} -> lift={lift_text})",
            py_number_text(h),
            py_number_text(d)
        )
    }))
}

fn weir_json(w: &Weir) -> Value {
    json!({
        "index": w.index, "chainage_m": float_json(w.chainage_m),
        "point": [float_json(w.point[0]), float_json(w.point[1])], "bed_m": float_json(w.bed_m),
        "foundation_m": float_json(w.foundation_m), "crest_m": float_json(w.crest_m),
        "pool_water_level_m": float_json(w.pool_water_level_m), "pool_start_m": float_json(w.pool_start_m),
        "pool_end_m": float_json(w.pool_end_m), "pool_length_m": float_json(w.pool_length_m()),
    })
}

pub(super) fn pools(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "overall_bottom_slope" => ok(floats_json(&pl::overall_bottom_slope(
            &f64_list(args, "z_raw")?,
            f64_arg(args, "spacing_m")?,
            f64_or(args, "percentile", 20.0)?,
            f64_or(args, "perc_window_m", 2500.0)?,
            f64_or(args, "smooth_window_m", 5000.0)?,
        ))),
        "sample_bed_profile" => {
            let p = profile(args)?;
            ok(json!({
                "s": floats_json(&p.s), "z_raw": floats_json(&p.z_raw), "z_overall": floats_json(&p.z_overall),
                "points": p.points.iter().map(|q| json!([float_json(q[0]), float_json(q[1])])).collect::<Vec<_>>(),
                "line": line_json(&p.line),
                "length_m": float_json(owd_core::geom::linear::length(&p.line)),
            }))
        }
        "place_weirs" => match weirs(args)? {
            Ok(ws) => ok(Value::Array(ws.iter().map(weir_json).collect())),
            Err(m) => value_error(m),
        },
        "water_levels" => {
            let ws = match weirs(arg(args, "weirs")?)? {
                Ok(ws) => ws,
                Err(m) => return value_error(m),
            };
            let down = f64_or(args, "downstream_water_level_m", 0.0)?;
            let depth = f64_arg(args, "depth_m")?;
            let over = f64_or(args, "overdepth_m", 0.0)?;
            let out = list(args, "chainages")?
                .iter()
                .map(|sv| {
                    let s = read_float(sv, "chainages")?;
                    let wl = pl::water_level_at(&ws, s, down);
                    Ok(json!({"s": sv.clone(), "water_level": float_json(wl), "water_level_fn": float_json(wl),
                              "grade": float_json(pl::grade_at(&ws, s, depth, down, over))}))
                })
                .collect::<Result<Vec<_>, EvalError>>()?;
            ok(Value::Array(out))
        }
        f => Err(bad(format!("pools has no parity function `{f}`"))),
    }
}
