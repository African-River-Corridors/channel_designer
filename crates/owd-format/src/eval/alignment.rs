//! The `radius_check`, `loops` and `rolling_circle` entry points (ticket 28): the adapters of
//! the seam for `radius_check`, `loops` and `rolling_circle`, over `owd_core::alignment`.

use super::geometry::{geom_from, geom_json, pt, pts};
use super::{EvalError, arg, bad, f64_arg, float_json, floats_json, list, ok, opt_f64};
use owd_core::alignment::{PyError, loops as lp, radius_check as rc, rolling_circle as rcl};
use owd_core::geom::{Geometry, Pt};
use serde_json::{Value, json};

fn error(e: PyError) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": e.kind, "message": e.message } }))
}

fn pt_json(p: Pt) -> Value {
    json!([float_json(p[0]), float_json(p[1])])
}

fn pts_json(ps: &[Pt]) -> Value {
    Value::Array(ps.iter().map(|&p| pt_json(p)).collect())
}

fn pts_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    pts(arg(args, name)?, &format!("args.{name}"))
}

fn pt_arg(args: &Value, name: &str) -> Result<Pt, EvalError> {
    pt(arg(args, name)?, &format!("args.{name}"))
}

fn line_geom(args: &Value, name: &str) -> Result<Geometry, EvalError> {
    geom_from(arg(args, name)?, &format!("args.{name}"))
}

fn line_pts(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    match line_geom(args, name)? {
        Geometry::LineString(l) => Ok(l),
        Geometry::Empty("LineString") => Ok(vec![]),
        _ => Err(bad(format!("`args.{name}` must be a LineString"))),
    }
}

fn int_arg(args: &Value, name: &str) -> Result<i64, EvalError> {
    arg(args, name)?
        .as_i64()
        .ok_or_else(|| bad(format!("`args.{name}` must be an integer")))
}

fn int_or(args: &Value, name: &str, default: i64) -> Result<i64, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_i64()
            .ok_or_else(|| bad(format!("`args.{name}` must be an integer"))),
    }
}

fn str_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, EvalError> {
    arg(args, name)?
        .as_str()
        .ok_or_else(|| bad(format!("`args.{name}` must be a string")))
}

fn result<T>(r: Result<T, PyError>, f: impl FnOnce(T) -> Value) -> Result<Value, EvalError> {
    match r {
        Ok(v) => ok(f(v)),
        Err(e) => error(e),
    }
}

pub(super) fn radius_check(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "default_check_chord" => ok(float_json(rc::default_check_chord(f64_arg(args, "r_min")?))),
        "min_radius_on_chord" => result(
            rc::min_radius_on_chord(&line_pts(args, "line")?, f64_arg(args, "chord_m")?),
            float_json,
        ),
        "min_radius_on_vertices" => result(
            rc::min_radius_on_vertices(&pts_arg(args, "pts")?, int_or(args, "k", 1)?),
            float_json,
        ),
        "radius_ok" => ok(json!(rc::radius_ok(
            f64_arg(args, "measured_m")?,
            f64_arg(args, "r_min")?
        ))),
        "constants" => ok(json!({
            "MAX_STRIDE_M": float_json(rc::MAX_STRIDE_M),
            "RADIUS_TOL": float_json(rc::RADIUS_TOL),
        })),
        other => Err(bad(format!(
            "radius_check has no parity function `{other}`"
        ))),
    }
}

pub(super) fn loops(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "constants" => ok(json!({
            "REJECT_SELF_INTERSECTING": lp::REJECT_SELF_INTERSECTING,
            "TURN_LIMIT_DEG": float_json(lp::TURN_LIMIT_DEG),
            "LEN_RATIO_BACKSTOP": float_json(lp::LEN_RATIO_BACKSTOP),
        })),
        "cumulative_turn_deg" => ok(float_json(lp::cumulative_turn_deg(&pts_arg(
            args, "coords",
        )?))),
        "is_self_intersecting" => ok(json!(lp::is_self_intersecting(&pts_arg(args, "coords")?))),
        "loop_verdict" => {
            let (reject, reason, m) =
                lp::loop_verdict(&pts_arg(args, "coords")?, f64_arg(args, "window_len_m")?);
            ok(json!([
                reject,
                reason,
                {
                    "path_len_m": float_json(m.path_len_m),
                    "window_len_m": float_json(m.window_len_m),
                    "len_ratio": float_json(m.len_ratio),
                    "turn_deg": float_json(m.turn_deg),
                    "self_intersecting": m.self_intersecting,
                }
            ]))
        }
        "is_loop_artefact" => ok(json!(lp::is_loop_artefact(
            &pts_arg(args, "coords")?,
            f64_arg(args, "window_len_m")?
        ))),
        other => Err(bad(format!("loops has no parity function `{other}`"))),
    }
}

fn chainages_arg(args: &Value, name: &str) -> Result<Vec<f64>, EvalError> {
    list(args, name)?
        .iter()
        .map(|v| super::read_float(v, &format!("args.{name}")))
        .collect()
}

pub(super) fn rolling_circle(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "primary_line" => result(rcl::primary_line(&line_geom(args, "geom")?), |l| {
            geom_json(&Geometry::LineString(l))
        }),
        "resample_coordinate_sequence" => result(
            rcl::resample_coordinate_sequence(&pts_arg(args, "points")?, f64_arg(args, "step")?),
            |p| pts_json(&p),
        ),
        "resample_linestring" => result(
            rcl::resample_coordinate_sequence(&line_pts(args, "line")?, f64_arg(args, "step")?),
            |p| pts_json(&p),
        ),
        "cumulative_chainages" => ok(floats_json(&rcl::cumulative_chainages(&pts_arg(
            args, "points",
        )?))),
        "compute_unit_tangent" => ok(pt_json(rcl::compute_unit_tangent(
            &pts_arg(args, "points")?,
            int_arg(args, "index")?,
        ))),
        "place_circle_at_point" => result(
            rcl::place_circle_at_point(
                &pts_arg(args, "points")?,
                int_arg(args, "index")?,
                f64_arg(args, "radius")?,
                str_arg(args, "side")?,
            ),
            |(centre, angle)| json!({"centre": pt_json(centre), "tangent_angle": float_json(angle)}),
        ),
        "create_arc_between_points" => ok(pts_json(&rcl::create_arc_between_points(
            pt_arg(args, "centre")?,
            f64_arg(args, "radius")?,
            pt_arg(args, "start_point")?,
            pt_arg(args, "end_point")?,
            str_arg(args, "side")?,
            int_or(args, "num_segments", 32)?,
        ))),
        "interpolate_point_on_line" => result(
            rcl::interpolate_point_on_line(
                &pts_arg(args, "coords")?,
                &chainages_arg(args, "chainages")?,
                f64_arg(args, "distance")?,
            ),
            pt_json,
        ),
        "replace_segment_with_arc" => result(
            rcl::replace_segment_with_arc(
                &line_pts(args, "line")?,
                f64_arg(args, "start_distance")?,
                f64_arg(args, "end_distance")?,
                &pts_arg(args, "arc_points")?,
            ),
            |l| geom_json(&Geometry::LineString(l)),
        ),
        "find_forward_circle_contact" => result(
            rcl::find_forward_circle_contact(
                &line_pts(args, "centreline")?,
                pt_arg(args, "tangent_point")?,
                f64_arg(args, "tangent_chainage")?,
                pt_arg(args, "centre")?,
                f64_arg(args, "radius")?,
                f64_arg(args, "min_chainage_gap")?,
            ),
            |c| match c {
                None => Value::Null,
                Some(c) => json!({
                    "contact_point": pt_json(c.contact_point),
                    "centre": pt_json(c.centre),
                    "tangent_point": pt_json(c.tangent_point),
                    "contact_chainage": float_json(c.contact_chainage),
                }),
            },
        ),
        "rolling_circle" => {
            let sides: Vec<String> = match args.get("sides") {
                None | Some(Value::Null) => vec!["left".into(), "right".into()],
                Some(_) => list(args, "sides")?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| bad("`args.sides` entries must be strings".into()))
                    })
                    .collect::<Result<_, _>>()?,
            };
            let res = rcl::rolling_circle(
                &line_geom(args, "centreline")?,
                f64_arg(args, "min_radius")?,
                f64_arg(args, "step")?,
                int_or(args, "passes", 2)?,
                &sides,
                opt_f64(args, "min_chainage_gap")?,
                opt_f64(args, "check_chord_m")?,
            );
            result(res, |r| {
                let circles: Vec<Value> = r
                    .circles
                    .iter()
                    .map(|c| {
                        json!({
                            "pass": c.pass,
                            "side": c.side,
                            "centre": pt_json(c.centre),
                            "radius_m": float_json(c.radius_m),
                            "tangent_point": pt_json(c.tangent_point),
                            "contact_point": pt_json(c.contact_point),
                            "exit_point": pt_json(c.exit_point),
                            "tangent_exit": c.tangent_exit,
                            "start_chainage_m": float_json(c.start_chainage_m),
                            "end_chainage_m": float_json(c.end_chainage_m),
                        })
                    })
                    .collect();
                json!({
                    "result": {
                        "centreline": geom_json(&Geometry::LineString(r.centreline.clone())),
                        "min_radius_m": float_json(r.min_radius_m),
                        "circles": circles,
                        "measured_min_radius_m": float_json(r.measured_min_radius_m),
                        "radius_check_chord_m": float_json(r.radius_check_chord_m),
                        "radius_ok": r.radius_ok,
                    },
                    "n_arcs": r.circles.len(),
                })
            })
        }
        other => Err(bad(format!(
            "rolling_circle has no parity function `{other}`"
        ))),
    }
}
