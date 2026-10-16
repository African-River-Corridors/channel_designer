//! Terrain samplers as case specs (`parity/common.py`, `terrain_from`): `synthetic_river`,
//! `plane`, `hills`, `valley` and `wrong_shape`. A sampler maps points to heights, NaN where
//! unknown.

use super::{EvalError, bad, f64_arg, f64_or, read_float};
use cd_core::synthetic::{self, RiverParams};
use serde_json::Value;

/// A terrain sampler: heights for the points `(xs[i], ys[i])`.
pub(super) type Sampler = Box<dyn Fn(&[f64], &[f64]) -> Vec<f64>>;

pub(super) fn river_params(p: &Value) -> Result<RiverParams, EvalError> {
    let d = RiverParams::default();
    let hill_at = match p.get("hill_at") {
        None | Some(Value::Null) => d.hill_at,
        Some(v) => match v.as_array().map(Vec::as_slice) {
            Some([x, y]) => [read_float(x, "hill_at")?, read_float(y, "hill_at")?],
            _ => return Err(bad("hill_at is [x, y]".into())),
        },
    };
    Ok(RiverParams {
        length_m: f64_or(p, "length_m", d.length_m)?,
        wavelength_m: f64_or(p, "wavelength_m", d.wavelength_m)?,
        amplitude_m: f64_or(p, "amplitude_m", d.amplitude_m)?,
        width_m: f64_or(p, "width_m", d.width_m)?,
        bed_depth_m: f64_or(p, "bed_depth_m", d.bed_depth_m)?,
        floodplain_above_wl_m: f64_or(p, "floodplain_above_wl_m", d.floodplain_above_wl_m)?,
        bed_slope: f64_or(p, "bed_slope", d.bed_slope)?,
        water_level_at_mouth_m: f64_or(p, "water_level_at_mouth_m", d.water_level_at_mouth_m)?,
        hill_height_m: f64_or(p, "hill_height_m", d.hill_height_m)?,
        hill_at,
        hill_radius_m: f64_or(p, "hill_radius_m", d.hill_radius_m)?,
    })
}

/// The synthetic river for the `params` object of a spec (`None` or `null`: the defaults).
pub(super) fn river_from(params: Option<&Value>) -> Result<synthetic::SyntheticRiver, EvalError> {
    let params = match params {
        Some(p) if !p.is_null() => river_params(p)?,
        _ => RiverParams::default(),
    };
    synthetic::synthetic_river(params).map_err(|e| bad(e.to_string()))
}

/// The sampler that a terrain spec names.
pub(super) fn terrain_from(spec: &Value) -> Result<Sampler, EvalError> {
    if let Some(id) = crate::host::callback_id(spec) {
        return crate::host::arrays_fn(id);
    }
    let kind = spec
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("terrain spec needs a string `kind`".into()))?;
    match kind {
        "synthetic_river" => {
            let params = match spec.get("params") {
                Some(p) if !p.is_null() => river_params(p)?,
                _ => RiverParams::default(),
            };
            let river = synthetic::synthetic_river(params).map_err(|e| bad(e.to_string()))?;
            Ok(Box::new(move |xs, ys| {
                xs.iter()
                    .zip(ys)
                    .map(|(&x, &y)| river.terrain(x, y))
                    .collect()
            }))
        }
        "plane" => {
            let (z0, gx, gy) = (
                f64_arg(spec, "z0")?,
                f64_arg(spec, "gx")?,
                f64_arg(spec, "gy")?,
            );
            Ok(Box::new(move |xs, ys| {
                xs.iter()
                    .zip(ys)
                    .map(|(&x, &y)| z0 + gx * x + gy * y)
                    .collect()
            }))
        }
        "hills" => {
            let (base, gx, gy) = (
                f64_arg(spec, "base")?,
                f64_arg(spec, "gx")?,
                f64_arg(spec, "gy")?,
            );
            let hills: Vec<[f64; 4]> = spec
                .get("hills")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("hills spec needs a list `hills`".into()))?
                .iter()
                .map(|h| {
                    let a = h
                        .as_array()
                        .filter(|a| a.len() == 4)
                        .ok_or_else(|| bad("a hill is [x, y, height, radius]".into()))?;
                    Ok([
                        read_float(&a[0], "hill")?,
                        read_float(&a[1], "hill")?,
                        read_float(&a[2], "hill")?,
                        read_float(&a[3], "hill")?,
                    ])
                })
                .collect::<Result<_, EvalError>>()?;
            Ok(Box::new(move |xs, ys| {
                xs.iter()
                    .zip(ys)
                    .map(|(&x, &y)| {
                        let mut z = base + gx * x + gy * y;
                        for [hx, hy, h, r] in &hills {
                            let (dx, dy) = (x - hx, y - hy);
                            z += h * libm::exp(-(dx * dx + dy * dy) / (2.0 * r * r));
                        }
                        z
                    })
                    .collect()
            }))
        }
        "valley" => {
            let (x0, run, depth) = (
                f64_arg(spec, "x0")?,
                f64_arg(spec, "run")?,
                f64_arg(spec, "depth")?,
            );
            let gap = match spec.get("gap") {
                None | Some(Value::Null) => None,
                Some(g) => match g.as_array().map(Vec::as_slice) {
                    Some([lo, hi]) => Some((read_float(lo, "gap")?, read_float(hi, "gap")?)),
                    _ => return Err(bad("gap is [lo, hi]".into())),
                },
            };
            Ok(Box::new(move |xs, _ys| {
                xs.iter()
                    .map(|&x| match gap {
                        Some((lo, hi)) if x > lo && x < hi => f64::NAN,
                        _ => depth * (x - x0).abs() / run,
                    })
                    .collect()
            }))
        }
        "wrong_shape" => {
            // A broken sampler: always `n` zeros, whatever it is asked.
            let n = spec
                .get("n")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad("wrong_shape needs a whole number `n`".into()))?
                as usize;
            Ok(Box::new(move |_xs, _ys| vec![0.0; n]))
        }
        other => Err(bad(format!("unknown terrain kind `{other}`"))),
    }
}
