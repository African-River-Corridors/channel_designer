//! The `fairway` entry point: `fairway_fit` (the minimum-curvature corridor solve, ADR 0007) for the
//! Python wrapper in `src/channel_designer/alignment/fairway.py`.

use super::geometry::{geom_from, polys_of, pts};
use super::{EvalError, arg, bad, f64_arg, float_json, ok, opt_f64, value_error_of};
use cd_core::alignment::fairway_fit::{self as ff, FitParams, Fixed, Pose};
use cd_core::geom::{Geometry, Pt};
use serde_json::{Value, json};

fn pt_json(p: Pt) -> Value {
    json!([float_json(p[0]), float_json(p[1])])
}

fn pose_arg(args: &Value, name: &str) -> Result<Option<Pose>, EvalError> {
    match args.get(name).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => {
            let at = pts(&json!([arg(v, "at")?]), &format!("args.{name}.at"))?[0];
            Ok(Some(Pose {
                at,
                heading_rad: f64_arg(v, "heading_rad")?,
            }))
        }
    }
}

/// `args.params`: `r_min` and any `FitParams` field to override; the rest are `FitParams::new`'s.
fn params_arg(args: &Value) -> Result<FitParams, EvalError> {
    let p = arg(args, "params")?;
    let mut f = FitParams::new(f64_arg(p, "r_min")?);
    macro_rules! num {
        ($($k:ident),*) => {$(
            if let Some(x) = opt_f64(p, stringify!($k))? { f.$k = x; }
        )*};
    }
    num!(
        chord_m,
        pref_weight,
        spacing_m,
        converged_m,
        reach_m,
        fold_share,
        reweight_cap,
        free_cap_m,
        pull_weight,
        contact_margin_m,
        max_growth,
        check_step_m,
        inside_tol_m
    );
    f.r_pref_m = opt_f64(p, "r_pref_m")?.or(f.r_pref_m);
    if let Some(n) = p.get("passes").filter(|v| !v.is_null()) {
        f.passes = n
            .as_u64()
            .ok_or_else(|| bad("`params.passes` must be a whole number".into()))?
            as usize;
    }
    Ok(f)
}

pub(super) fn fairway(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "fairway_fit" => {
            let line = match geom_from(arg(args, "line")?, "args.line")? {
                Geometry::LineString(l) => l,
                _ => return Err(bad("`args.line` must be a LineString".into())),
            };
            let nav = polys_of(
                geom_from(arg(args, "navigable")?, "args.navigable")?,
                "args.navigable",
            )?;
            let must_follow = match args.get("must_follow").filter(|v| !v.is_null()) {
                None => Vec::new(),
                Some(v) => v
                    .as_array()
                    .ok_or_else(|| bad("`args.must_follow` must be a list".into()))?
                    .iter()
                    .map(|g| match geom_from(g, "args.must_follow")? {
                        Geometry::LineString(l) => Ok(l),
                        _ => Err(bad("each must-follow piece must be a LineString".into())),
                    })
                    .collect::<Result<_, _>>()?,
            };
            let fixed = Fixed {
                start_pose: pose_arg(args, "start_pose")?,
                end_pose: pose_arg(args, "end_pose")?,
                must_follow,
            };
            match ff::fairway_fit(&line, &nav, &fixed, &params_arg(args)?) {
                Err(e) => value_error_of(e),
                Ok(r) => ok(json!({
                    "line": {"geom": "LineString", "coords": r.line.iter().map(|&p| pt_json(p)).collect::<Vec<_>>()},
                    "infeasible": r.infeasible.iter().map(|s| json!({
                        "start_chainage_m": float_json(s.start_chainage_m),
                        "end_chainage_m": float_json(s.end_chainage_m),
                        "reason": s.reason.as_str(),
                    })).collect::<Vec<_>>(),
                    "measured_min_radius_m": float_json(r.measured_min_radius_m),
                    "radius_check_chord_m": float_json(r.radius_check_chord_m),
                    "radius_ok": r.radius_ok,
                    "start_heading_error_rad": r.start_heading_error_rad.map(float_json),
                    "end_heading_error_rad": r.end_heading_error_rad.map(float_json),
                    "notes": r.notes,
                })),
            }
        }
        other => Err(bad(format!("fairway has no function `{other}`"))),
    }
}
