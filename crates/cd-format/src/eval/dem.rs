//! The `raster` entry point (ticket 33): the bilinear sampler and the raster volume integrator, on
//! rasters built from the case spec as `parity/modules/raster.py` (`tif_from`) builds them.

use super::geometry::geom_from;
use super::terrain::terrain_from;
use super::{
    EvalError, arg, bad, f64_arg, f64_list, f64_or, float_json, floats_json, list, ok, opt_f64,
    read_float,
};
use cd_core::geom::{Geometry, Polygon, Pt, linear};
use cd_core::volumes::pools::Weir;
use cd_core::volumes::raster::{self as rs, Affine, Raster, SlopeMode, VolumeParams};
use serde_json::{Value, json};

/// `tif_from(spec)`: heights from the spec's terrain at the pixel centres, holes set to nodata
/// (or NaN), stored as float32, with the transform `from_origin(x0, y_top, res, res)`.
fn raster_from(spec: &Value) -> Result<Raster, EvalError> {
    let int = |k: &str| {
        arg(spec, k)?
            .as_u64()
            .map(|v| v as usize)
            .ok_or_else(|| bad(format!("raster `{k}` must be an integer")))
    };
    let (w, h) = (int("width")?, int("height")?);
    let (res, x0, y_top) = (
        f64_arg(spec, "res")?,
        f64_arg(spec, "x0")?,
        f64_arg(spec, "y_top")?,
    );
    let xs: Vec<f64> = (0..w).map(|i| x0 + (i as f64 + 0.5) * res).collect();
    let ys: Vec<f64> = (0..h).map(|j| y_top - (j as f64 + 0.5) * res).collect();
    let (mut gx, mut gy) = (Vec::with_capacity(w * h), Vec::with_capacity(w * h));
    for &y in &ys {
        for &x in &xs {
            gx.push(x);
            gy.push(y);
        }
    }
    let mut z = terrain_from(arg(spec, "terrain")?)?(&gx, &gy);
    let nodata = opt_f64(spec, "nodata")?;
    if let Some(holes) = spec.get("holes").and_then(Value::as_array) {
        for hole in holes {
            let b: Vec<f64> = hole
                .as_array()
                .ok_or_else(|| bad("a hole is [x0, y0, x1, y1]".into()))?
                .iter()
                .map(|v| read_float(v, "hole"))
                .collect::<Result<_, _>>()?;
            for i in 0..z.len() {
                if gx[i] >= b[0] && gx[i] <= b[2] && gy[i] >= b[1] && gy[i] <= b[3] {
                    z[i] = nodata.unwrap_or(f64::NAN);
                }
            }
        }
    }
    Ok(Raster {
        width: w,
        height: h,
        transform: Affine {
            a: res,
            c: x0,
            e: -res,
            f: y_top,
        },
        data: z.into_iter().map(|v| f64::from(v as f32)).collect(),
        nodata,
    })
}

fn line_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    match geom_from(arg(args, name)?, &format!("args.{name}"))? {
        Geometry::LineString(l) => Ok(l),
        _ => Err(bad(format!("`args.{name}` must be a LineString"))),
    }
}

fn polys_arg(args: &Value, name: &str) -> Result<Vec<Polygon>, EvalError> {
    Ok(
        match geom_from(arg(args, name)?, &format!("args.{name}"))? {
            Geometry::Polygon(p) => vec![p],
            Geometry::MultiPolygon(ps) => ps,
            _ => return Err(bad(format!("`args.{name}` must be a polygon"))),
        },
    )
}

/// `_weirs(args)`: the weirs as the adapter rebuilds them.
fn weirs(args: &Value, line: &[Pt]) -> Result<Vec<Weir>, EvalError> {
    list(args, "weirs")?
        .iter()
        .map(|w| {
            let start = f64_arg(w, "pool_start_m")?;
            let wl = f64_arg(w, "pool_water_level_m")?;
            Ok(Weir {
                index: arg(w, "index")?
                    .as_u64()
                    .ok_or_else(|| bad("weir index".into()))? as usize,
                chainage_m: start,
                point: linear::interpolate(line, start).unwrap_or([f64::NAN; 2]),
                bed_m: f64_or(w, "bed_m", 0.0)?,
                foundation_m: f64_or(w, "foundation_m", 0.0)?,
                crest_m: wl,
                pool_water_level_m: wl,
                pool_start_m: start,
                pool_end_m: f64_arg(w, "pool_end_m")?,
            })
        })
        .collect()
}

fn params(args: &Value, chunk_default: f64) -> Result<VolumeParams, EvalError> {
    let mut p = VolumeParams::new(
        f64_arg(args, "navigable_depth_m")?,
        f64_or(args, "chunk_m", chunk_default)?,
    );
    p.channel_bottom_width_m = f64_or(args, "channel_bottom_width_m", p.channel_bottom_width_m)?;
    p.side_slope_h_per_v = f64_or(args, "side_slope_h_per_v", p.side_slope_h_per_v)?;
    p.overdepth_m = f64_or(args, "overdepth_m", p.overdepth_m)?;
    if let Some(d) = args.get("despike").and_then(Value::as_bool) {
        p.despike = d;
    }
    if let Some(v) = args.get("despike_window_px").and_then(Value::as_u64) {
        p.despike_window_px = v as usize;
    }
    p.despike_k = f64_or(args, "despike_k", p.despike_k)?;
    p.despike_floor_m = f64_or(args, "despike_floor_m", p.despike_floor_m)?;
    p.deep_cut_threshold_m = f64_or(args, "deep_cut_threshold_m", p.deep_cut_threshold_m)?;
    p.max_corridor_half_width_m = f64_or(
        args,
        "max_corridor_half_width_m",
        p.max_corridor_half_width_m,
    )?;
    p.downstream_water_level_m = opt_f64(args, "downstream_water_level_m")?;
    if let Some(v) = args.get("read_decimate").and_then(Value::as_u64) {
        p.read_decimate = (v as usize).max(1);
    }
    let pianc = args.get("section_mode").and_then(Value::as_str) == Some("pianc");
    let lim = |k: &str| match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => line_arg(args, k).map(Some),
    };
    if let (true, Some(l), Some(r)) = (pianc, lim("left_limit")?, lim("right_limit")?) {
        p.pianc_limits = Some((l, r));
    }
    Ok(p)
}

fn sampled(args: &Value) -> Result<Value, EvalError> {
    let r = raster_from(arg(args, "raster")?)?;
    Ok(floats_json(&rs::bilinear(
        &r,
        &f64_list(args, "xs")?,
        &f64_list(args, "ys")?,
    )))
}

/// A 2-D list of numbers or booleans, row by row.
fn grid<T>(
    args: &Value,
    name: &str,
    cell: impl Fn(&Value) -> Result<T, EvalError>,
) -> Result<(Vec<T>, usize, usize), EvalError> {
    let rows = list(args, name)?;
    let mut out = Vec::new();
    let mut w = 0;
    for row in rows {
        let r = row
            .as_array()
            .ok_or_else(|| bad(format!("`args.{name}` must be a 2-D list")))?;
        w = r.len();
        for v in r {
            out.push(cell(v)?);
        }
    }
    Ok((out, w, rows.len()))
}

pub(super) fn raster(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "bilinear" | "raster_terrain" => ok(sampled(args)?),
        "layer_volumes" => ok(floats_json(&rs::layer_volumes(&f64_list(args, "vals")?))),
        "despike" => {
            let (z, w, h) = grid(args, "z", |v| read_float(v, "z"))?;
            let (valid, _, _) = grid(args, "valid", |v| {
                v.as_bool().ok_or_else(|| bad("valid".into()))
            })?;
            let window = arg(args, "window")?
                .as_u64()
                .ok_or_else(|| bad("window".into()))? as usize;
            let (zc, spike) = rs::despike(
                &z,
                &valid,
                w,
                h,
                f64_arg(args, "fill")?,
                window,
                f64_arg(args, "k")?,
                f64_arg(args, "floor")?,
            );
            let rows = |v: Vec<Value>| {
                Value::Array(
                    v.chunks(w.max(1))
                        .map(|c| Value::Array(c.to_vec()))
                        .collect(),
                )
            };
            ok(json!({
                "z": rows(zc.iter().map(|&x| float_json(x)).collect()),
                "spike": rows(spike.iter().map(|&b| Value::Bool(b)).collect()),
            }))
        }
        "compute_pool_volumes" | "compute_chainage_volumes" => {
            let line = line_arg(args, "centerline")?;
            let river = polys_arg(args, "river_poly")?;
            let r = raster_from(arg(args, "raster")?)?;
            let ws = weirs(args, &line)?;
            if function == "compute_pool_volumes" {
                let p = params(args, 2000.0)?;
                let out = rs::compute_pool_volumes(&ws, &line, &river, &r, &p);
                ok(Value::Array(
                    out.iter()
                        .map(|v| {
                            json!({
                                "weir_index": v.weir_index, "pool_water_level_m": float_json(v.pool_water_level_m),
                                "design_grade_m": float_json(v.design_grade_m), "dredge_volume_m3": float_json(v.dredge_volume_m3),
                                "excavation_volume_m3": float_json(v.excavation_volume_m3), "dredge_area_m2": float_json(v.dredge_area_m2),
                                "excavation_area_m2": float_json(v.excavation_area_m2), "max_cut_m": float_json(v.max_cut_m),
                                "mean_cut_m": float_json(v.mean_cut_m), "despiked_cells": v.despiked_cells,
                                "deep_excavation_volume_m3": float_json(v.deep_excavation_volume_m3),
                                "deep_excavation_area_m2": float_json(v.deep_excavation_area_m2),
                                "dredge_hist": floats_json(&v.dredge_hist), "excav_hist": floats_json(&v.excav_hist),
                            })
                        })
                        .collect(),
                ))
            } else {
                let p = params(args, 100.0)?;
                let mode = match args
                    .get("slope_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("always")
                {
                    "always" => SlopeMode::Always,
                    "dry_bank_only" => SlopeMode::DryBankOnly,
                    "fairway_only" => SlopeMode::FairwayOnly,
                    m => return Err(bad(format!("unknown slope_mode `{m}`"))),
                };
                let out = rs::compute_chainage_volumes(&ws, &line, &river, &r, &p, mode);
                ok(Value::Array(
                    out.iter()
                        .map(|c| {
                            json!({
                                "s0_m": float_json(c.s0_m), "s1_m": float_json(c.s1_m), "grade_m": float_json(c.grade_m),
                                "wl_m": float_json(c.wl_m), "pool": c.pool, "len_m": float_json(c.len_m),
                                "is_cut_through": c.is_cut_through, "design_width_m": float_json(c.design_width_m),
                                "dredge_m3": float_json(c.dredge_m3), "excav_m3": float_json(c.excav_m3),
                                "widening_m3": float_json(c.widening_m3), "cut_through_m3": float_json(c.cut_through_m3),
                                "deep_m3": float_json(c.deep_m3), "river_width_m": float_json(c.river_width_m),
                            })
                        })
                        .collect(),
                ))
            }
        }
        f => Err(bad(format!("raster has no parity function `{f}`"))),
    }
}
