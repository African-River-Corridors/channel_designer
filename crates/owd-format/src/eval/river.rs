//! The `centreline`, `widths`, `limits`, `meanders` and `bends` entry points (ticket 27): from two
//! bank lines to a classified river.

use super::geometry::{geom_from, geom_json};
use super::{
    EvalError, arg, bad, f64_arg, f64_list, f64_or, float_json, floats_json, list, ok, read_float,
};
use owd_core::alignment::{bends, centreline as cl, limits as lm, meanders as md, widths as wd};
use owd_core::geom::{Geometry, Polygon, Pt};
use serde_json::{Value, json};

fn value_error(message: String) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": "ValueError", "message": message } }))
}

/// A Python `ValueError` becomes its recorded form; anything else is the result.
fn or_value_error<T>(r: Result<T, String>, f: impl FnOnce(T) -> Value) -> Result<Value, EvalError> {
    match r {
        Ok(v) => ok(f(v)),
        Err(m) => value_error(m),
    }
}

fn pt(v: &Value, what: &str) -> Result<Pt, EvalError> {
    match v.as_array().map(Vec::as_slice) {
        Some([x, y, ..]) => Ok([read_float(x, what)?, read_float(y, what)?]),
        _ => Err(bad(format!("{what}: a point is [x, y]"))),
    }
}

fn pts_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    list(args, name)?
        .iter()
        .map(|p| pt(p, &format!("args.{name}")))
        .collect()
}

fn pt_arg(args: &Value, name: &str) -> Result<Pt, EvalError> {
    pt(arg(args, name)?, &format!("args.{name}"))
}

fn pt_json(p: Pt) -> Value {
    json!([float_json(p[0]), float_json(p[1])])
}

fn pts_json(ps: &[Pt]) -> Value {
    Value::Array(ps.iter().map(|&p| pt_json(p)).collect())
}

fn line_json(l: &[Pt]) -> Value {
    geom_json(&Geometry::LineString(l.to_vec()))
}

fn int_arg(args: &Value, name: &str) -> Result<i64, EvalError> {
    arg(args, name)?
        .as_i64()
        .ok_or_else(|| bad(format!("`args.{name}` must be an integer")))
}

fn int_or(args: &Value, name: &str, default: i64) -> Result<i64, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => int_arg(args, name),
    }
}

/// A LineString argument as its points.
fn line_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    match geom_from(arg(args, name)?, &format!("args.{name}"))? {
        Geometry::LineString(l) => Ok(l),
        Geometry::Empty("LineString") => Ok(vec![]),
        _ => Err(bad(format!("`args.{name}` must be a LineString"))),
    }
}

fn polys_arg(args: &Value, name: &str) -> Result<Vec<Polygon>, EvalError> {
    match geom_from(arg(args, name)?, &format!("args.{name}"))? {
        Geometry::Polygon(p) => Ok(vec![p]),
        Geometry::MultiPolygon(ps) => Ok(ps),
        _ => Err(bad(format!("`args.{name}` must be a polygon"))),
    }
}

/// `limits.extract_lines`: the non-empty LineStrings in a geometry.
fn extract_lines(g: Geometry) -> Result<Vec<Vec<Pt>>, EvalError> {
    Ok(match g {
        Geometry::Empty(_) => vec![],
        Geometry::LineString(l) if l.is_empty() => vec![],
        Geometry::LineString(l) => vec![l],
        Geometry::MultiLineString(ls) => ls.into_iter().filter(|l| !l.is_empty()).collect(),
        Geometry::Collection(gs) => {
            let mut out = Vec::new();
            for g in gs {
                out.extend(extract_lines(g)?);
            }
            out
        }
        _ => {
            return Err(bad(
                "extract_lines takes lines and collections of them".into()
            ));
        }
    })
}

fn lines_json(mut ls: Vec<Vec<Pt>>, multi: bool) -> Value {
    if !multi && ls.len() == 1 {
        return line_json(&ls.remove(0));
    }
    geom_json(&Geometry::MultiLineString(ls))
}

/// The value of an argument as the case gave it, or `default`: Python echoes these unchanged.
fn echo(args: &Value, name: &str, default: Value) -> Value {
    match args.get(name) {
        None | Some(Value::Null) => default,
        Some(v) => v.clone(),
    }
}

fn widening(args: &Value) -> Result<lm::Widening, EvalError> {
    Ok(lm::Widening {
        loa: f64_arg(args, "loa")?,
        extra_width_coefficient: f64_or(args, "extra_width_coefficient", 0.6)?,
        min_radius_length_multiple: f64_or(args, "min_radius_length_multiple", 3.0)?,
        max_extra_width_radius: match args.get("max_extra_width_radius") {
            None => Some(2000.0),
            Some(Value::Null) => None,
            Some(v) => Some(read_float(v, "args.max_extra_width_radius")?),
        },
        radius_threshold: f64_or(args, "radius_threshold", 2000.0)?,
    })
}

fn circle_json(c: Option<(Pt, f64)>) -> Value {
    match c {
        None => Value::Null,
        Some((centre, r)) => json!({"center": pt_json(centre), "radius": float_json(r)}),
    }
}

fn ranges_json(rs: &[(usize, usize)]) -> Value {
    Value::Array(
        rs.iter()
            .map(|&(a, b)| json!((a..b).collect::<Vec<_>>()))
            .collect(),
    )
}

pub(super) fn centreline(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "ensure_orientation" => {
            let (l, r) = cl::ensure_orientation(&pts_arg(args, "left")?, &pts_arg(args, "right")?);
            ok(json!([pts_json(&l), pts_json(&r)]))
        }
        "resample_polyline" => or_value_error(
            cl::resample_polyline(&pts_arg(args, "points")?, f64_arg(args, "step")?),
            |p| pts_json(&p),
        ),
        "reparameterize_fraction" => or_value_error(
            cl::reparameterize_fraction(&pts_arg(args, "points")?),
            |(p, f)| json!([pts_json(&p), floats_json(&f)]),
        ),
        "moving_average" => ok(pts_json(&cl::moving_average(
            &pts_arg(args, "points")?,
            int_arg(args, "window")?,
        ))),
        "chaikin_smoothing" => ok(pts_json(&cl::chaikin_smoothing(
            &pts_arg(args, "points")?,
            int_arg(args, "iterations")?,
        ))),
        "map_points_monotonic" => ok(pts_json(&cl::map_points_monotonic(
            &pts_arg(args, "source")?,
            &line_arg(args, "target_line")?,
            f64_arg(args, "max_step")?,
        ))),
        "compute_paired_centreline" => or_value_error(
            cl::compute_paired_centreline(
                &pts_arg(args, "left")?,
                &pts_arg(args, "right")?,
                f64_arg(args, "resample_step")?,
                int_arg(args, "smooth_window")?,
                int_arg(args, "chaikin_iterations")?,
            ),
            |p| pts_json(&p),
        ),
        "build_centreline" => or_value_error(
            cl::build_centreline(
                &line_arg(args, "left_bank")?,
                &line_arg(args, "right_bank")?,
                f64_or(args, "resample_step", 20.0)?,
                int_or(args, "smooth_window", 5)?,
                int_or(args, "chaikin_iterations", 2)?,
            ),
            |p| line_json(&p),
        ),
        "trim_polyline" => or_value_error(
            cl::trim_polyline(&pts_arg(args, "points")?, f64_arg(args, "cut_length")?),
            |p| pts_json(&p),
        ),
        f => Err(bad(format!("centreline has no parity function `{f}`"))),
    }
}

pub(super) fn widths(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "resample_distances" => or_value_error(
            wd::resample_distances(f64_arg(args, "total_length")?, f64_arg(args, "spacing")?),
            |d| floats_json(&d),
        ),
        "measure_widths" => or_value_error(
            wd::measure_widths(
                &line_arg(args, "centreline")?,
                &line_arg(args, "left_bank")?,
                &line_arg(args, "right_bank")?,
                f64_arg(args, "spacing")?,
                f64_arg(args, "cross_span")?,
            ),
            |(rows, failures)| {
                let rows: Vec<Value> = rows
                    .iter()
                    .map(|r| {
                        json!({
                            "chainage_m": float_json(r.chainage_m),
                            "width_m": float_json(r.width_m),
                            "geometry": line_json(&[r.left, r.right]),
                            "sample_index": r.sample_index,
                            "left_x": float_json(r.left[0]),
                            "left_y": float_json(r.left[1]),
                            "right_x": float_json(r.right[0]),
                            "right_y": float_json(r.right[1]),
                        })
                    })
                    .collect();
                json!({"rows": rows, "failures": failures})
            },
        ),
        f => Err(bad(format!("widths has no parity function `{f}`"))),
    }
}

pub(super) fn limits(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "extract_lines" => {
            let ls = extract_lines(geom_from(arg(args, "geom")?, "args.geom")?)?;
            ok(Value::Array(ls.iter().map(|l| line_json(l)).collect()))
        }
        "merge_lines" => {
            let lines: Vec<Vec<Pt>> = list(args, "lines")?
                .iter()
                .map(|l| match geom_from(l, "args.lines")? {
                    Geometry::LineString(l) => Ok(l),
                    _ => Err(bad("`args.lines` must hold LineStrings".into())),
                })
                .collect::<Result<_, _>>()?;
            or_value_error(lm::merge_lines(&lines), |m| lines_json(m, false))
        }
        "ensure_multilinestring" => match geom_from(arg(args, "geom")?, "args.geom")? {
            Geometry::LineString(l) => ok(lines_json(vec![l], true)),
            Geometry::MultiLineString(ls) => ok(lines_json(ls, true)),
            _ => Err(bad("ensure_multilinestring takes lines".into())),
        },
        "resample_line" => or_value_error(
            lm::resample_line(&line_arg(args, "line")?, f64_arg(args, "step")?),
            |p| pts_json(&p),
        ),
        "circumcircle" => ok(circle_json(lm::circumcircle(
            pt_arg(args, "p0")?,
            pt_arg(args, "p1")?,
            pt_arg(args, "p2")?,
        ))),
        "compute_curvature_radius" => ok(floats_json(&lm::compute_curvature_radius(&pts_arg(
            args, "points",
        )?))),
        "compute_normals" => ok(pts_json(&lm::compute_normals(&pts_arg(args, "points")?))),
        "compute_chainages" => ok(floats_json(&lm::compute_chainages(&pts_arg(
            args, "points",
        )?))),
        "rolling_average_radius" => ok(floats_json(&lm::rolling_average_radius(
            &f64_list(args, "radii")?,
            &f64_list(args, "chainages")?,
            f64_arg(args, "window")?,
        ))),
        "compute_extra_width" => ok(floats_json(&lm::compute_extra_width(
            &f64_list(args, "radii")?,
            &f64_list(args, "chainages")?,
            f64_arg(args, "total_length")?,
            &widening(args)?,
        ))),
        "build_variable_offset_line" => or_value_error(
            lm::build_variable_offset_line(
                &line_arg(args, "line")?,
                f64_arg(args, "base_offset")?,
                &widening(args)?,
                f64_or(args, "radius_sample_length", 10.0)?,
            ),
            |lm::OffsetLines {
                 left: l,
                 right: r,
                 smoothed_radii: radii,
                 extra,
             }| json!({"left": line_json(&l), "right": line_json(&r), "smoothed_radii": floats_json(&radii), "extra": floats_json(&extra)}),
        ),
        "build_channel_limits" => {
            let mut lines = Vec::new();
            for g in list(args, "centreline_geoms")? {
                if g.is_null() {
                    continue;
                }
                lines.extend(extract_lines(geom_from(g, "args.centreline_geoms")?)?);
            }
            or_value_error(
                lm::build_channel_limits(
                    &lines,
                    f64_arg(args, "base_offset")?,
                    &widening(args)?,
                    f64_or(args, "radius_sample_length", 10.0)?,
                ),
                |lm::ChannelLimits {
                     left: l,
                     right: r,
                     diagnostics: d,
                 }| {
                    json!({
                        "left": lines_json(l, true),
                        "right": lines_json(r, true),
                        "diagnostics": {
                            "min_radius": d.min_radius.map_or(Value::Null, float_json),
                            "max_extra_width": float_json(d.max_extra_width),
                            "mean_extra_width": float_json(d.mean_extra_width),
                        },
                    })
                },
            )
        }
        f => Err(bad(format!("limits has no parity function `{f}`"))),
    }
}

pub(super) fn meanders(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "resample_polyline" => or_value_error(
            md::resample_polyline(&pts_arg(args, "points")?, f64_arg(args, "step")?),
            |p| pts_json(&p),
        ),
        "circumcircle" => ok(circle_json(lm::circumcircle(
            pt_arg(args, "p0")?,
            pt_arg(args, "p1")?,
            pt_arg(args, "p2")?,
        ))),
        "curvature_radius" => or_value_error(
            md::curvature_radius(&pts_arg(args, "points")?),
            |md::Curvature {
                 curvature: c,
                 radius: r,
                 signed: s,
             }| json!({"curvature": floats_json(&c), "radius": floats_json(&r), "signed_curvature": floats_json(&s)}),
        ),
        "compute_heading" => ok(floats_json(&md::compute_heading(&pts_arg(args, "points")?))),
        "detect_curvature_runs" => ok(ranges_json(&md::detect_curvature_runs(
            &f64_list(args, "signed_curvature")?,
            int_arg(args, "min_points")?,
        ))),
        "find_meander_runs" => ok(ranges_json(&md::find_meander_runs(
            &f64_list(args, "radius")?,
            &f64_list(args, "signed_curvature")?,
            f64_arg(args, "threshold")?,
            int_arg(args, "min_points")?,
        ))),
        "tangent_angle" => {
            let h = f64_list(args, "heading")?;
            let out = list(args, "indices")?
                .iter()
                .map(|i| {
                    i.as_i64()
                        .map(|i| float_json(md::tangent_angle(&h, i)))
                        .ok_or_else(|| bad("`args.indices` must be integers".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            ok(Value::Array(out))
        }
        "find_meanders" => or_value_error(
            md::find_meanders(
                &line_arg(args, "centreline")?,
                f64_arg(args, "max_radius")?,
                f64_or(args, "sample_step", 10.0)?,
                int_or(args, "min_points", 8)?,
                f64_or(args, "min_deflection_deg", 45.0)?,
            ),
            |rs| {
                Value::Array(
                    rs.iter()
                        .map(|r| {
                            json!({
                                "apex_index": r.apex_index,
                                "apex_distance_m": float_json(r.apex_distance_m),
                                "apex_location": {"x": float_json(r.apex_location[0]), "y": float_json(r.apex_location[1])},
                                "radius": float_json(r.radius),
                                "deflection_deg": float_json(r.deflection_deg),
                                "tangent_angle_deg": float_json(r.tangent_angle_deg),
                                "tangent_direction": if r.left { "left" } else { "right" },
                                "circle_center": {"x": float_json(r.circle_center[0]), "y": float_json(r.circle_center[1])},
                                "id_number": r.id_number,
                            })
                        })
                        .collect(),
                )
            },
        ),
        f => Err(bad(format!("meanders has no parity function `{f}`"))),
    }
}

fn unit_json(u: &bends::BendUnit, with_indices: bool) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    if with_indices {
        m.insert("i0".into(), json!(u.i0));
        m.insert("i1".into(), json!(u.i1));
    }
    m.insert("s0".into(), float_json(u.s0));
    m.insert("s1".into(), float_json(u.s1));
    m.insert("length_m".into(), float_json(u.length_m));
    m.insert("sign".into(), json!(u.sign));
    m.insert("kappa_max".into(), float_json(u.kappa_max));
    m.insert("r_min_natural_m".into(), float_json(u.r_min_natural_m));
    m.insert("deflection_rad".into(), float_json(u.deflection_rad));
    m
}

pub(super) fn bends(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "constants" => ok(json!({
            "STEP_M": float_json(bends::STEP_M), "N_PSI": bends::N_PSI, "PSI_MAX_DEG": float_json(bends::PSI_MAX_DEG),
            "N_Y": bends::N_Y, "E_MAX_M": float_json(bends::E_MAX_M), "E_TOL_M": float_json(bends::E_TOL_M),
        })),
        "resample" => {
            let (s, xy) = bends::resample(&line_arg(args, "line")?, f64_arg(args, "step")?);
            ok(json!({"s": floats_json(&s), "xy": pts_json(&xy)}))
        }
        "curvature" => ok(floats_json(&bends::curvature(&pts_arg(args, "xy")?))),
        "bend_units" => {
            let n = int_or(args, "smooth_n", 5)? as usize;
            let us = bends::bend_units(&f64_list(args, "s")?, &f64_list(args, "kappa")?, n);
            ok(Value::Array(
                us.iter()
                    .map(|u| Value::Object(unit_json(u, true)))
                    .collect(),
            ))
        }
        "feasible_within" => ok(Value::Bool(bends::feasible_within(
            &f64_list(args, "kappa")?,
            f64_arg(args, "ds")?,
            f64_arg(args, "half_width")?,
            f64_arg(args, "r_min")?,
            int_or(args, "n_y", bends::N_Y as i64)? as usize,
            int_or(args, "n_psi", bends::N_PSI as i64)? as usize,
        ))),
        "e_exact" => ok(float_json(bends::e_exact(
            &f64_list(args, "kappa")?,
            f64_arg(args, "ds")?,
            f64_arg(args, "r_min")?,
        ))),
        "e_closed_form" => ok(float_json(bends::e_closed_form(
            f64_arg(args, "r_nat")?,
            f64_arg(args, "defl")?,
            f64_arg(args, "r_min")?,
        ))),
        "contiguous_wetted_width" => ok(float_json(bends::contiguous_wetted_width(
            &polys_arg(args, "river")?,
            pt_arg(args, "centre")?,
            f64_arg(args, "nx")?,
            f64_arg(args, "ny")?,
            f64_or(args, "max_half_m", 200.0)?,
            f64_or(args, "probe_m", 1.0)?,
        ))),
        "classify_bends" => {
            let bank_cut = f64_or(
                args,
                "bank_cut_fraction",
                owd_core::rules::rulesets::BANK_CUT_FRACTION,
            )?;
            let step = f64_or(args, "step", bends::STEP_M)?;
            let rows = bends::classify_bends(
                &line_arg(args, "natural")?,
                &polys_arg(args, "river")?,
                f64_arg(args, "r_min")?,
                f64_arg(args, "top_width")?,
                f64_arg(args, "bottom_width")?,
                bank_cut,
                step,
            );
            let count = |k: &str| rows.iter().filter(|r| r.klass == k).count();
            // Python `sum` of no values is the integer 0.
            let total = |pick: &dyn Fn(&bends::BendRow) -> bool| -> (Value, f64) {
                let xs: Vec<f64> = rows
                    .iter()
                    .filter(|r| pick(r))
                    .map(|r| r.unit.length_m)
                    .collect();
                if xs.is_empty() {
                    (json!(0), 0.0)
                } else {
                    let s = bends::py_sum(xs);
                    (float_json(s), s)
                }
            };
            let (lt_json, lt) = total(&|r| r.klass == "T");
            let (tot_json, tot) = total(&|_| true);
            let bends_json: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let mut m = unit_json(&r.unit, false);
                    m.insert("e_m".into(), r.e_m.map_or(Value::Null, float_json));
                    m.insert("e_closed_form_m".into(), float_json(r.e_closed_form_m));
                    m.insert("wetted_width_m".into(), float_json(r.wetted_width_m));
                    m.insert("room_m".into(), float_json(r.room_m));
                    m.insert("klass".into(), json!(r.klass));
                    Value::Object(m)
                })
                .collect();
            ok(json!({
                "params": {
                    "r_min_m": echo(args, "r_min", Value::Null),
                    "top_width_m": echo(args, "top_width", Value::Null),
                    "bottom_width_m": echo(args, "bottom_width", Value::Null),
                    "bank_cut_fraction": echo(args, "bank_cut_fraction", float_json(owd_core::rules::rulesets::BANK_CUT_FRACTION)),
                    "step_m": echo(args, "step", float_json(bends::STEP_M)),
                    "n_y": bends::N_Y, "n_psi": bends::N_PSI, "psi_max_deg": float_json(bends::PSI_MAX_DEG),
                    "e_method": "exact forward-reachability DP + bisection",
                    "room_rule": "(w - W_top)/2 + bank_cut_fraction * W_bottom",
                },
                "summary": {
                    "n_bends": rows.len(), "n_straight": count("S"), "n_gentle": count("G"), "n_tight": count("T"),
                    "tight_length_m": lt_json, "total_length_m": tot_json,
                    "tight_fraction": float_json(if tot != 0.0 { lt / tot } else { 0.0 }),
                },
                "bends": bends_json,
            }))
        }
        f => Err(bad(format!("bends has no parity function `{f}`"))),
    }
}
