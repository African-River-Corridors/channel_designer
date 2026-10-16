//! `eval(entry_point, json) -> json`: the parity seam. One dispatch over every parity entry
//! point. Input and output use the case and Reference Result shapes in `parity/README.md`.
//!
//! Number encoding, both ways: a finite float is a JSON number; NaN and infinity are the strings
//! `"float:nan"`, `"float:inf"` and `"float:-inf"`. In an input list of heights, `null` also means
//! "no ground" (Python `None`) and is read as NaN.

use cd_core::cut_model::{self as cm, PerSide};
use serde_json::{Number, Value, json};

mod alignment;
mod cascade;
mod geometry;
mod lattice;
mod river;
mod route;
mod terrain;
mod volumes;

/// Why `eval` could not produce a result. A Python `ValueError` is not an `EvalError`: it is a
/// normal result, `{"error": {"type": "ValueError", "message": ...}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// No such parity entry point.
    UnknownEntryPoint(String),
    /// The entry point exists in the spec but is not ported yet.
    NotPorted(String),
    /// The case input does not have the shape the entry point needs.
    BadInput(String),
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EvalError::UnknownEntryPoint(ep) => write!(f, "unknown entry point `{ep}`"),
            EvalError::NotPorted(ep) => write!(f, "entry point `{ep}` is not ported yet"),
            EvalError::BadInput(msg) => write!(f, "bad input: {msg}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Every parity entry point in spec 0001: one per Python module, named after it (`cut_model`
/// keeps the name it had before the others were added). Each port ticket adds its dispatch arm and
/// its name to `parity/ported.json`; until then `eval` says `NotPorted`.
pub const ENTRY_POINTS: &[&str] = &[
    "bends",
    "cascade",
    "centreline",
    "cut_model",
    "cut_model_extras",
    "cut_model_grid",
    "design",
    "footprint",
    "geometry",
    "lattice",
    "limits",
    "loops",
    "meanders",
    "methods",
    "pools",
    "primitives",
    "radius_check",
    "raster",
    "rolling_circle",
    "route",
    "rulesets",
    "sections",
    "simple",
    "synthetic",
    "terrain_profile",
    "width_profile",
    "widths",
];

/// Run one parity case through the Rust code.
pub fn eval(entry_point: &str, input: &Value) -> Result<Value, EvalError> {
    let function = input
        .get("fn")
        .and_then(Value::as_str)
        .ok_or_else(|| EvalError::BadInput("case input needs a string `fn`".into()))?;
    let args = input.get("args").unwrap_or(&Value::Null);
    match entry_point {
        "cut_model" => cut_model(function, args),
        "rulesets" | "methods" | "design" | "simple" | "width_profile" => {
            crate::eval_rules::dispatch(entry_point, function, args)
        }
        "radius_check" => alignment::radius_check(function, args),
        "loops" => alignment::loops(function, args),
        "rolling_circle" => alignment::rolling_circle(function, args),
        "route" => route::route(function, args),
        "cascade" => cascade::cascade(function, args),
        "lattice" => lattice::lattice(function, args),
        "sections" => volumes::sections(function, args),
        "cut_model_grid" => volumes::cut_model_grid(function, args),
        "cut_model_extras" => volumes::cut_model_extras(function, args),
        "centreline" => river::centreline(function, args),
        "widths" => river::widths(function, args),
        "limits" => river::limits(function, args),
        "meanders" => river::meanders(function, args),
        "bends" => river::bends(function, args),
        "geometry" => geometry::geometry(function, args),
        "synthetic" => geometry::synthetic(function, args),
        ep if ENTRY_POINTS.contains(&ep) => Err(EvalError::NotPorted(ep.into())),
        ep => Err(EvalError::UnknownEntryPoint(ep.into())),
    }
}

pub(crate) fn ok(value: Value) -> Result<Value, EvalError> {
    Ok(json!({ "ok": value }))
}

/// A Python `ValueError`, as the frozen Python driver records it.
fn value_error(message: String) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": "ValueError", "message": message } }))
}

/// A refusal the Python raised, as the frozen driver records it.
pub(crate) fn value_error_of(e: cd_core::alignment::PyError) -> Result<Value, EvalError> {
    Ok(json!({ "error": { "type": e.kind, "message": e.message } }))
}

pub(crate) fn bad(msg: String) -> EvalError {
    EvalError::BadInput(msg)
}

/// A float as JSON: a number, or the tagged string for NaN and infinity.
pub(crate) fn float_json(x: f64) -> Value {
    if x.is_nan() {
        json!("float:nan")
    } else if x == f64::INFINITY {
        json!("float:inf")
    } else if x == f64::NEG_INFINITY {
        json!("float:-inf")
    } else {
        Value::Number(Number::from_f64(x).expect("finite"))
    }
}

pub(crate) fn floats_json(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|&x| float_json(x)).collect())
}

pub(crate) fn read_float(v: &Value, what: &str) -> Result<f64, EvalError> {
    match v {
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| bad(format!("{what}: not a float"))),
        Value::String(s) => match s.as_str() {
            "float:nan" => Ok(f64::NAN),
            "float:inf" => Ok(f64::INFINITY),
            "float:-inf" => Ok(f64::NEG_INFINITY),
            _ => Err(bad(format!("{what}: `{s}` is not a number"))),
        },
        _ => Err(bad(format!("{what}: must be a number"))),
    }
}

pub(crate) fn arg<'a>(args: &'a Value, name: &str) -> Result<&'a Value, EvalError> {
    match args.get(name) {
        Some(v) if !v.is_null() => Ok(v),
        _ => Err(bad(format!("`args.{name}` is required"))),
    }
}

pub(crate) fn f64_arg(args: &Value, name: &str) -> Result<f64, EvalError> {
    read_float(arg(args, name)?, &format!("args.{name}"))
}

/// An optional float: absent or `null` gives `default`.
pub(crate) fn f64_or(args: &Value, name: &str, default: f64) -> Result<f64, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => read_float(v, &format!("args.{name}")),
    }
}

pub(crate) fn opt_f64(args: &Value, name: &str) -> Result<Option<f64>, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => read_float(v, &format!("args.{name}")).map(Some),
    }
}

pub(crate) fn list<'a>(args: &'a Value, name: &str) -> Result<&'a Vec<Value>, EvalError> {
    arg(args, name)?
        .as_array()
        .ok_or_else(|| bad(format!("`args.{name}` must be a list")))
}

/// A list of floats; `null` entries read as NaN (no ground).
fn f64_list(args: &Value, name: &str) -> Result<Vec<f64>, EvalError> {
    list(args, name)?
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if v.is_null() {
                Ok(f64::NAN)
            } else {
                read_float(v, &format!("args.{name}[{i}]"))
            }
        })
        .collect()
}

fn sides(args: &Value, name: &str, n: usize) -> Result<Vec<i8>, EvalError> {
    let out: Vec<i8> = list(args, name)?
        .iter()
        .map(|v| match v.as_i64() {
            Some(1) => Ok(1),
            Some(-1) => Ok(-1),
            _ => Err(bad(format!("`args.{name}` entries must be 1 or -1"))),
        })
        .collect::<Result<_, _>>()?;
    same_len(name, out.len(), n)?;
    Ok(out)
}

fn same_len(name: &str, got: usize, want: usize) -> Result<(), EvalError> {
    if got == want {
        Ok(())
    } else {
        Err(bad(format!(
            "`args.{name}` has {got} items, expected {want}"
        )))
    }
}

fn opt_sides(args: &Value, name: &str, n: usize) -> Result<Option<Vec<i8>>, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => sides(args, name, n).map(Some),
    }
}

fn per_side_bool(b: PerSide<bool>) -> Value {
    json!({ "1": b.plus, "-1": b.minus })
}

fn per_side_float(l: PerSide<f64>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("1".into(), float_json(l.plus));
    m.insert("-1".into(), float_json(l.minus));
    Value::Object(m)
}

fn bool_arg(args: &Value, name: &str) -> Result<bool, EvalError> {
    arg(args, name)?
        .as_bool()
        .ok_or_else(|| bad(format!("`args.{name}` must be true or false")))
}

fn bench_arg(args: &Value, name: &str) -> Result<PerSide<bool>, EvalError> {
    let v = arg(args, name)?;
    let side = |key: &str| {
        v.get(key)
            .and_then(Value::as_bool)
            .ok_or_else(|| bad(format!("`args.{name}` needs a boolean `{key}`")))
    };
    Ok(PerSide {
        plus: side("1")?,
        minus: side("-1")?,
    })
}

fn cut_model(function: &str, args: &Value) -> Result<Value, EvalError> {
    let wet_slope = f64_or(args, "wet_slope", cm::WET_SLOPE_H_PER_V)?;
    match function {
        "cut_model_a_face" => ok(float_json(cm::cut_model_a_face(
            f64_arg(args, "o")?,
            bool_arg(args, "side_bench")?,
            f64_arg(args, "hb")?,
            f64_arg(args, "grade")?,
            f64_arg(args, "wl")?,
            f64_arg(args, "depth")?,
            wet_slope,
        ))),
        "cut_model_a_bench_gate" => {
            let offsets = f64_list(args, "offsets")?;
            let terrain = f64_list(args, "terrain")?;
            same_len("terrain", terrain.len(), offsets.len())?;
            let sidewhich = opt_sides(args, "sidewhich", offsets.len())?;
            ok(per_side_bool(cm::cut_model_a_bench_gate(
                &offsets,
                &terrain,
                f64_arg(args, "hb")?,
                f64_arg(args, "wl")?,
                f64_arg(args, "depth")?,
                sidewhich.as_deref(),
                wet_slope,
            )))
        }
        "cut_model_b_daylight_limit" => {
            let offsets = f64_list(args, "offsets")?;
            let terrain = f64_list(args, "terrain")?;
            same_len("terrain", terrain.len(), offsets.len())?;
            let sidewhich = opt_sides(args, "sidewhich", offsets.len())?;
            ok(per_side_float(cm::cut_model_b_daylight_limit(
                &offsets,
                &terrain,
                f64_arg(args, "hb")?,
                f64_arg(args, "grade")?,
                f64_arg(args, "wl")?,
                f64_arg(args, "depth")?,
                bench_arg(args, "bench")?,
                sidewhich.as_deref(),
                wet_slope,
                opt_f64(args, "run_m")?,
            )))
        }
        "section_area" => {
            let xs = f64_list(args, "xs")?;
            let zs = f64_list(args, "zs")?;
            same_len("zs", zs.len(), xs.len())?;
            let sidewhich = sides(args, "sidewhich", xs.len())?;
            let wet: Vec<bool> = list(args, "wet")?
                .iter()
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| bad("`args.wet` entries must be booleans".into()))
                })
                .collect::<Result<_, _>>()?;
            same_len("wet", wet.len(), xs.len())?;
            let model = arg(args, "model")?
                .as_str()
                .ok_or_else(|| bad("`args.model` must be a string".into()))?;
            match cm::section_area(
                &zs,
                &xs,
                &sidewhich,
                f64_arg(args, "hb")?,
                f64_arg(args, "grade")?,
                f64_arg(args, "wl")?,
                f64_arg(args, "depth")?,
                model,
                &wet,
                wet_slope,
                f64_or(args, "dx", cm::DX_M)?,
                opt_f64(args, "run_m")?,
            ) {
                Ok((dredge, excav, max_face_h)) => ok(floats_json(&[dredge, excav, max_face_h])),
                Err(message) => value_error(message),
            }
        }
        "widened_hb" => {
            let xy: Vec<[f64; 2]> = list(args, "xy")?
                .iter()
                .map(|p| {
                    let pair = p
                        .as_array()
                        .filter(|a| a.len() == 2)
                        .ok_or_else(|| bad("`args.xy` entries must be [x, y]".into()))?;
                    Ok([
                        read_float(&pair[0], "args.xy")?,
                        read_float(&pair[1], "args.xy")?,
                    ])
                })
                .collect::<Result<_, EvalError>>()?;
            ok(floats_json(&cm::widened_hb(
                &xy,
                f64_arg(args, "loa")?,
                f64_arg(args, "beam")?,
                f64_arg(args, "radius_multiple")?,
                f64_arg(args, "width_mult")?,
                f64_arg(args, "c_c")?,
                f64_or(args, "rmax", cm::WIDEN_RMAX)?,
            )))
        }
        other => Err(bad(format!("cut_model has no parity function `{other}`"))),
    }
}
