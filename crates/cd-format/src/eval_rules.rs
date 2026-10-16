//! The parity dispatch for the rules entry points: `rulesets`, `methods`, `design`, `simple` and
//! `width_profile` (spec 0001, ticket 25). Case input and output use the shapes in
//! `parity/README.md`; the function names and argument names are those of the frozen Python and
//! of its adapters in `parity/modules/`.
//!
//! A case may carry a `toy_method` spec. It is registered, from this crate and through the public
//! registry API, into a registry made for that one case, so cases never see each other's methods.

use crate::eval::{
    EvalError, arg, bad, f64_arg, f64_or, float_json, floats_json, list, ok, opt_f64, read_float,
};
use cd_core::rules::design::{self as ds, Centreline, ChannelDesignInputs, ChannelDesignParams};
use cd_core::rules::methods::{BreakdownFn, MethodRegistry, OptValue, Options, WidthMethod};
use cd_core::rules::rulesets::{self as rs, ChannelRuleInputs, Ruleset, RulesetRegistry};
use cd_core::rules::simple::{self, PiancInputs};
use cd_core::rules::width_profile::{self as wp, WidthFn};
use cd_core::rules::{Check, RuleError};
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) fn dispatch(
    entry_point: &str,
    function: &str,
    args: &Value,
) -> Result<Value, EvalError> {
    let methods = method_registry(args)?;
    match entry_point {
        "rulesets" => rulesets(&methods, function, args),
        "methods" => methods_ep(methods, function, args),
        "design" => design(&methods, function, args),
        "simple" => simple_ep(&methods, function, args),
        "width_profile" => width_profile(&methods, function, args),
        other => Err(bad(format!("not a rules entry point: `{other}`"))),
    }
}

/// A Python refusal is a normal result, `{"error": {"type": ..., "message": ...}}`.
fn rule<T>(r: Result<T, RuleError>, f: impl FnOnce(T) -> Value) -> Result<Value, EvalError> {
    match r {
        Ok(v) => ok(f(v)),
        Err(e) => Ok(refusal(&e)),
    }
}

fn refusal(e: &RuleError) -> Value {
    json!({ "error": { "type": e.type_name(), "message": e.python_message() } })
}

/// Unwrap a `RuleError` into the refusal result, or carry on.
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return Ok(refusal(&e)),
        }
    };
}

// --- a case's own width method --------------------------------------------------------------

/// The built-in registry, plus the case's toy method: straight = k_beam x beam, bend = straight +
/// loa^2 / (bend_div x R), R_min = rmin_mult x loa. It is data-defined and built with the public
/// API only, as a plug-in crate would.
fn method_registry(args: &Value) -> Result<MethodRegistry, EvalError> {
    let mut reg = MethodRegistry::new();
    if let Some(spec) = args.get("toy_method").filter(|v| !v.is_null()) {
        let name = str_field(spec, "name")?;
        let k_beam = f64_arg(spec, "k_beam")?;
        let bend_div = f64_arg(spec, "bend_div")?;
        let rmin_mult = f64_arg(spec, "rmin_mult")?;
        let description = match spec.get("description").and_then(Value::as_str) {
            Some(d) => d.to_string(),
            None => "toy method".to_string(),
        };
        let method = WidthMethod {
            name,
            straight_width: Arc::new(move |_loa, beam, _o| Ok(k_beam * beam)),
            bend_width: Arc::new(move |loa, beam, _o, r, _rd, _d| {
                Ok(k_beam * beam + loa * loa / (bend_div * r))
            }),
            min_radius: Arc::new(move |loa, _o| Ok(rmin_mult * loa)),
            breakdown: None,
            description,
        };
        reg.register(method, true).map_err(|e| bad(e.to_string()))?;
    }
    // The host's own methods (Python callables, ticket 35): `host_methods` is a list of
    // `{name, description, id, breakdown}`; each call goes to the host as `{op, ...}`.
    if let Some(list) = args.get("host_methods").and_then(Value::as_array) {
        for spec in list {
            reg.register(host_method(spec)?, true).map_err(|e| bad(e.to_string()))?;
        }
    }
    Ok(reg)
}

/// A width method whose callables live in the host.
fn host_method(spec: &Value) -> Result<WidthMethod, EvalError> {
    let id = spec
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad("a host method needs a whole number `id`".into()))?;
    let call = Arc::new(crate::host::json_fn(id)?);
    let number = |v: Option<Value>| -> f64 {
        v.and_then(|v| read_float(&v, "host method").ok())
            .unwrap_or(f64::NAN)
    };
    let (c1, c2, c3, c4) = (call.clone(), call.clone(), call.clone(), call.clone());
    let breakdown: Option<BreakdownFn> = if spec.get("breakdown").and_then(Value::as_bool) == Some(true) {
        Some(Arc::new(move |loa, beam, o| {
            let r = c4(json!({"op": "breakdown", "loa": float_json(loa), "beam": float_json(beam),
                              "options": options_json(o)}));
            Ok(r.and_then(|v| read_options(&v, "breakdown").ok()).unwrap_or_default())
        }))
    } else {
        None
    };
    Ok(WidthMethod {
        name: str_field(spec, "name")?,
        straight_width: Arc::new(move |loa, beam, o| {
            Ok(number(c1(json!({"op": "straight", "loa": float_json(loa), "beam": float_json(beam),
                                 "options": options_json(o)}))))
        }),
        bend_width: Arc::new(move |loa, beam, o, r, rd, d| {
            Ok(number(c2(json!({"op": "bend", "loa": float_json(loa), "beam": float_json(beam),
                                 "options": options_json(o), "bend_radius": float_json(r),
                                 "design_radius": float_json(rd), "deflection_deg": float_json(d)}))))
        }),
        min_radius: Arc::new(move |loa, o| {
            Ok(number(c3(json!({"op": "min_radius", "loa": float_json(loa),
                                 "options": options_json(o)}))))
        }),
        breakdown,
        description: spec.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

/// The built-in rulesets, plus the host's own (`host_rulesets`: every ruleset the host registry
/// holds, as data; a built-in one the host edited replaces ours).
pub(crate) fn ruleset_registry(args: &Value) -> Result<RulesetRegistry, EvalError> {
    let mut reg = RulesetRegistry::new();
    if let Some(list) = args.get("host_rulesets").and_then(Value::as_array) {
        for spec in list {
            reg.register(read_ruleset(spec)?, true)
                .map_err(|e| bad(e.to_string()))?;
        }
    }
    Ok(reg)
}

// --- reading --------------------------------------------------------------------------------

fn str_field(v: &Value, name: &str) -> Result<String, EvalError> {
    v.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| bad(format!("`{name}` must be a string")))
}

fn read_bool(v: &Value, what: &str) -> Result<bool, EvalError> {
    v.as_bool()
        .ok_or_else(|| bad(format!("{what}: must be true or false")))
}

fn read_str(v: &Value, what: &str) -> Result<String, EvalError> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| bad(format!("{what}: must be a string")))
}

fn read_options(v: &Value, what: &str) -> Result<Options, EvalError> {
    let obj = v
        .as_object()
        .ok_or_else(|| bad(format!("{what}: must be an object")))?;
    let mut out = Options::new();
    for (k, x) in obj {
        out.set(k, read_opt_value(x, &format!("{what}.{k}"))?);
    }
    Ok(out)
}

fn read_opt_value(v: &Value, what: &str) -> Result<OptValue, EvalError> {
    match v {
        Value::Bool(b) => Ok(OptValue::Bool(*b)),
        Value::Number(_) => Ok(OptValue::Float(read_float(v, what)?)),
        Value::String(s) => match s.as_str() {
            "float:nan" | "float:inf" | "float:-inf" => Ok(OptValue::Float(read_float(v, what)?)),
            _ => Ok(OptValue::Str(s.clone())),
        },
        Value::Object(_) => Ok(OptValue::Map(read_options(v, what)?)),
        _ => Err(bad(format!("{what}: not an option value"))),
    }
}

/// A case's `options` mapping; absent or `null` is empty.
fn opt_options(args: &Value, name: &str) -> Result<Options, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(Options::new()),
        Some(v) => read_options(v, &format!("args.{name}")),
    }
}

fn float_list(args: &Value, name: &str) -> Result<Vec<f64>, EvalError> {
    list(args, name)?
        .iter()
        .enumerate()
        .map(|(i, v)| read_float(v, &format!("args.{name}[{i}]")))
        .collect()
}

fn read_ruleset(spec: &Value) -> Result<Ruleset, EvalError> {
    let mut r = Ruleset::new(
        &str_field(spec, "ruleset_id")?,
        &str_field(spec, "family")?,
        &str_field(spec, "label")?,
        &str_field(spec, "source")?,
        f64_arg(spec, "r_mult_convoy")?,
        f64_arg(spec, "r_mult_motor")?,
    );
    r.width_mult = f64_or(spec, "width_mult", 0.0)?;
    r.c_c = f64_or(spec, "c_c", 0.0)?;
    r.depth_mult = f64_or(spec, "depth_mult", 0.0)?;
    r.bank_mult_convoy = f64_or(spec, "bank_mult_convoy", 0.0)?;
    r.bank_mult_motor = f64_or(spec, "bank_mult_motor", 0.0)?;
    r.depth_add_m = f64_or(spec, "depth_add_m", 0.0)?;
    if let Some(n) = spec.get("notes").and_then(Value::as_str) {
        r.notes = n.to_string();
    }
    Ok(r)
}

/// `ChannelRuleInputs(**d)`. `default_water_level` stands in when `min_water_level_m` is absent
/// (the width-profile bridge defaults it to 0.0).
fn read_rule_inputs(
    d: &Value,
    default_water_level: Option<f64>,
) -> Result<ChannelRuleInputs, EvalError> {
    let obj = d
        .as_object()
        .ok_or_else(|| bad("inputs must be an object".into()))?;
    let need = |name: &str| f64_arg(d, name);
    let water = match (d.get("min_water_level_m"), default_water_level) {
        (Some(v), _) if !v.is_null() => read_float(v, "min_water_level_m")?,
        (_, Some(w)) => w,
        _ => return Err(bad("`min_water_level_m` is required".into())),
    };
    let mut inp = ChannelRuleInputs::new(need("beam_m")?, need("loa_m")?, need("draft_m")?, water);
    for (k, v) in obj {
        let what = k.as_str();
        match what {
            "beam_m" | "loa_m" | "draft_m" | "min_water_level_m" => {}
            "rule_set" => inp.rule_set = read_str(v, what)?,
            "side_slope_h_per_v" => inp.side_slope_h_per_v = read_float(v, what)?,
            "ukc_fraction" => inp.ukc_fraction = read_float(v, what)?,
            "ukc_min_m" => inp.ukc_min_m = read_float(v, what)?,
            "reference_bend_radius_m" => {
                inp.reference_bend_radius_m = if v.is_null() {
                    None
                } else {
                    Some(read_float(v, what)?)
                }
            }
            "pianc_width_multiple" => inp.pianc_width_multiple = read_float(v, what)?,
            "pianc_use_buildup" => inp.pianc_use_buildup = read_bool(v, what)?,
            "pianc_maneuverability" => inp.pianc_maneuverability = read_str(v, what)?,
            "pianc_radius_multiple" => inp.pianc_radius_multiple = read_float(v, what)?,
            "pianc_extra_widths" => {
                inp.pianc_extra_widths = v
                    .as_object()
                    .ok_or_else(|| bad("pianc_extra_widths must be an object".into()))?
                    .iter()
                    .map(|(k, x)| Ok((k.clone(), read_float(x, k)?)))
                    .collect::<Result<_, EvalError>>()?
            }
            "pianc_bank_clearance_outer" => inp.pianc_bank_clearance_outer = read_float(v, what)?,
            "pianc_bank_clearance_inner" => inp.pianc_bank_clearance_inner = read_float(v, what)?,
            "options" => inp.options = read_options(v, what)?,
            other => return Err(bad(format!("ChannelRuleInputs has no field `{other}`"))),
        }
    }
    Ok(inp)
}

// --- writing --------------------------------------------------------------------------------

fn opt_value_json(v: &OptValue) -> Value {
    match v {
        OptValue::Float(x) => float_json(*x),
        OptValue::Bool(b) => json!(b),
        OptValue::Str(s) => json!(s),
        OptValue::Map(m) => options_json(m),
    }
}

fn options_json(o: &Options) -> Value {
    let mut m = Map::new();
    for (k, v) in o.iter() {
        m.insert(k.to_string(), opt_value_json(v));
    }
    Value::Object(m)
}

fn opt_float_json(x: Option<f64>) -> Value {
    match x {
        Some(x) => float_json(x),
        None => Value::Null,
    }
}

fn check_json(c: &Check) -> Value {
    json!({ "name": c.name, "ok": c.ok, "message": c.message, "severity": c.severity })
}

fn checks_json(cs: &[Check]) -> Value {
    Value::Array(cs.iter().map(check_json).collect())
}

fn ruleset_json(r: &Ruleset) -> Value {
    json!({
        "ruleset_id": r.ruleset_id,
        "family": r.family,
        "label": r.label,
        "source": r.source,
        "r_mult_convoy": float_json(r.r_mult_convoy),
        "r_mult_motor": float_json(r.r_mult_motor),
        "width_mult": float_json(r.width_mult),
        "c_c": float_json(r.c_c),
        "depth_mult": float_json(r.depth_mult),
        "bank_mult_convoy": float_json(r.bank_mult_convoy),
        "bank_mult_motor": float_json(r.bank_mult_motor),
        "depth_add_m": float_json(r.depth_add_m),
        "notes": r.notes,
    })
}

fn shape_json(s: &rs::ChannelShape) -> Value {
    json!({
        "ruleset_id": s.ruleset_id,
        "vessel_standard_id": s.vessel_standard_id,
        "loa_m": float_json(s.loa_m),
        "beam_m": float_json(s.beam_m),
        "draught_m": float_json(s.draught_m),
        "formation": s.formation,
        "bottom_width_straight_m": float_json(s.bottom_width_straight_m),
        "min_bend_radius_m": float_json(s.min_bend_radius_m),
        "design_depth_m": float_json(s.design_depth_m),
        "bend_widening_at_rmin_m": float_json(s.bend_widening_at_rmin_m),
        "bottom_width_at_rmin_m": float_json(s.bottom_width_at_rmin_m),
        "width_basis": s.width_basis,
        "source": s.source,
        "warnings": s.warnings,
    })
}

fn rule_inputs_json(i: &ChannelRuleInputs) -> Value {
    let mut extra = Map::new();
    for (k, v) in &i.pianc_extra_widths {
        extra.insert(k.clone(), float_json(*v));
    }
    json!({
        "beam_m": float_json(i.beam_m),
        "loa_m": float_json(i.loa_m),
        "draft_m": float_json(i.draft_m),
        "min_water_level_m": float_json(i.min_water_level_m),
        "rule_set": i.rule_set,
        "side_slope_h_per_v": float_json(i.side_slope_h_per_v),
        "ukc_fraction": float_json(i.ukc_fraction),
        "ukc_min_m": float_json(i.ukc_min_m),
        "reference_bend_radius_m": opt_float_json(i.reference_bend_radius_m),
        "pianc_width_multiple": float_json(i.pianc_width_multiple),
        "pianc_use_buildup": i.pianc_use_buildup,
        "pianc_maneuverability": i.pianc_maneuverability,
        "pianc_radius_multiple": float_json(i.pianc_radius_multiple),
        "pianc_extra_widths": Value::Object(extra),
        "pianc_bank_clearance_outer": float_json(i.pianc_bank_clearance_outer),
        "pianc_bank_clearance_inner": float_json(i.pianc_bank_clearance_inner),
        "options": options_json(&i.options),
    })
}

fn rule_result_json(r: &rs::ChannelRuleResult) -> Value {
    json!({
        "rule_set": r.rule_set,
        "straight_bottom_width_m": float_json(r.straight_bottom_width_m),
        "min_turn_radius_m": float_json(r.min_turn_radius_m),
        "reference_bend_radius_m": float_json(r.reference_bend_radius_m),
        "bend_bottom_width_m": float_json(r.bend_bottom_width_m),
        "bend_widening_m": float_json(r.bend_widening_m),
        "required_depth_m": float_json(r.required_depth_m),
        "under_keel_clearance_m": float_json(r.under_keel_clearance_m),
        "min_water_level_m": float_json(r.min_water_level_m),
        "bed_level_m": float_json(r.bed_level_m),
        "top_width_straight_m": float_json(r.top_width_straight_m),
        "width_breakdown": options_json(&r.width_breakdown),
        "checks": checks_json(&r.checks),
        "warnings": r.warnings,
    })
}

// --- rulesets -------------------------------------------------------------------------------

fn rulesets(methods: &MethodRegistry, function: &str, args: &Value) -> Result<Value, EvalError> {
    let reg = ruleset_registry(args)?;
    match function {
        "rule_sets" => ok(json!(methods.names())),
        "rulesets_table" => {
            let table: Vec<Value> = rs::CN_TABLE_A2
                .iter()
                .map(|(l, b, w)| json!([float_json(*l), float_json(*b), float_json(*w)]))
                .collect();
            let mut clearance = Map::new();
            for (class, (a, b)) in rs::CN_DEPTH_CLEARANCE_M {
                clearance.insert(class.into(), json!([float_json(a), float_json(b)]));
            }
            let mut sets = Map::new();
            for r in reg.iter() {
                sets.insert(r.ruleset_id.clone(), ruleset_json(r));
            }
            ok(json!({
                "rulesets": Value::Object(sets),
                "cn_table_a2": table,
                "cn_table_5_1_width_mult": float_json(rs::CN_TABLE_5_1_WIDTH_MULT),
                "cn_depth_clearance_m": Value::Object(clearance),
                "cn_depth_clearance_end": rs::CN_DEPTH_CLEARANCE_END.as_str(),
                "bank_cut_fraction": float_json(rs::BANK_CUT_FRACTION),
                "reference_deflection_deg": float_json(rs::REFERENCE_DEFLECTION_DEG),
            }))
        }
        "lateral_room_m" => ok(float_json(rs::lateral_room_m(
            f64_arg(args, "wetted_width_m")?,
            f64_arg(args, "top_width_m")?,
            f64_arg(args, "bottom_width_m")?,
            f64_or(args, "bank_cut_fraction", rs::BANK_CUT_FRACTION)?,
        ))),
        "classify_bend" => ok(json!(rs::classify_bend(
            f64_arg(args, "e_m")?,
            f64_arg(args, "room_m")?,
            f64_or(args, "tol_m", 1e-9)?,
        ))),
        "get_ruleset" => rule(reg.get(&str_field(args, "ruleset_id")?), ruleset_json),
        "channel_shape" => rule(
            rs::channel_shape(
                &reg,
                &str_field(args, "ruleset_id")?,
                &str_field(args, "vessel_standard_id")?,
                opt_f64(args, "draught_m")?,
            ),
            |s| shape_json(&s),
        ),
        "channel_shape_widths" => {
            let shape = tri!(rs::channel_shape(
                &reg,
                &str_field(args, "ruleset_id")?,
                &str_field(args, "vessel_standard_id")?,
                None,
            ));
            let rows: Vec<Value> = float_list(args, "radii")?
                .into_iter()
                .map(|r| {
                    json!({
                        "radius": float_json(r),
                        "bend_widening_m": float_json(shape.bend_widening_m(r)),
                        "bottom_width_at_m": float_json(shape.bottom_width_at_m(r)),
                    })
                })
                .collect();
            ok(Value::Array(rows))
        }
        "register_ruleset_then_channel_shape" => {
            let new = read_ruleset(arg(args, "ruleset")?)?;
            let replace = match args.get("replace") {
                None | Some(Value::Null) => false,
                Some(v) => read_bool(v, "args.replace")?,
            };
            let id = new.ruleset_id.clone();
            let mut local = reg.clone();
            tri!(local.register(new, replace));
            rule(
                rs::channel_shape(&local, &id, &str_field(args, "vessel_standard_id")?, None),
                |s| shape_json(&s),
            )
        }
        "size_channel" => {
            let inp = read_rule_inputs(arg(args, "inputs")?, None)?;
            rule(
                rs::size_channel(methods, &inp),
                |res| json!({ "result": rule_result_json(&res), "ok": res.ok() }),
            )
        }
        "width_at_radius" => {
            let inp = read_rule_inputs(arg(args, "inputs")?, None)?;
            tri!(inp.validate(methods));
            let mut out = Vec::new();
            for r in float_list(args, "radii")? {
                out.push(float_json(tri!(rs::width_at_radius(methods, &inp, r))));
            }
            ok(Value::Array(out))
        }
        "channel_rule_inputs" => {
            let inp = read_rule_inputs(arg(args, "inputs")?, None)?;
            tri!(inp.validate(methods));
            ok(json!({
                "inputs": rule_inputs_json(&inp),
                "method_options": options_json(&inp.method_options()),
            }))
        }
        other => Err(bad(format!("rulesets has no parity function `{other}`"))),
    }
}

// --- methods --------------------------------------------------------------------------------

fn methods_ep(
    mut methods: MethodRegistry,
    function: &str,
    args: &Value,
) -> Result<Value, EvalError> {
    match function {
        "width_methods" => ok(json!(methods.names())),
        "evaluate" => {
            let m = tri!(methods.get(&str_field(args, "name")?));
            let opts = opt_options(args, "options")?;
            let (loa, beam) = (f64_arg(args, "loa")?, f64_arg(args, "beam")?);
            let deflection = f64_or(args, "deflection_deg", 45.0)?;
            let straight = tri!((m.straight_width)(loa, beam, &opts));
            let min_radius = tri!((m.min_radius)(loa, &opts));
            let mut bends = Vec::new();
            for r in float_list(args, "radii")? {
                bends.push(tri!((m.bend_width)(loa, beam, &opts, r, r, deflection)));
            }
            let breakdown = match &m.breakdown {
                Some(f) => options_json(&tri!(f(loa, beam, &opts))),
                None => Value::Null,
            };
            ok(json!({
                "name": m.name,
                "description": m.description,
                "straight": float_json(straight),
                "min_radius": float_json(min_radius),
                "bends": floats_json(&bends),
                "breakdown": breakdown,
            }))
        }
        "get_width_method" => rule(
            methods.get(&str_field(args, "name")?),
            |m| json!({ "name": m.name, "description": m.description }),
        ),
        "register_duplicate" => {
            let m = tri!(methods.get(&str_field(args, "name")?));
            let name = m.name.clone();
            rule(methods.register((*m).clone(), false), |()| json!(name))
        }
        "unregister" => {
            tri!(methods.unregister(&str_field(args, "name")?));
            ok(json!(methods.names()))
        }
        other => Err(bad(format!("methods has no parity function `{other}`"))),
    }
}

// --- design ---------------------------------------------------------------------------------

/// `ChannelDesignParams(**d)`, checked against the registry as Python checks it on construction.
/// The outer error is a malformed case; the inner one is a Python refusal.
fn read_params(
    methods: &MethodRegistry,
    v: Option<&Value>,
) -> Result<Result<ChannelDesignParams, RuleError>, EvalError> {
    let mut p = ChannelDesignParams::default();
    if let Some(d) = v.filter(|v| !v.is_null()) {
        let obj = d
            .as_object()
            .ok_or_else(|| bad("params must be an object".into()))?;
        for (k, x) in obj {
            let what = k.as_str();
            match what {
                "width_multiple" => p.width_multiple = read_float(x, what)?,
                "radius_multiple" => p.radius_multiple = read_float(x, what)?,
                "side_slope" => p.side_slope = read_float(x, what)?,
                "width_method" => {
                    p.width_method = if x.is_null() {
                        None
                    } else {
                        Some(read_str(x, what)?)
                    }
                }
                "width_options" => p.width_options = read_options(x, what)?,
                "draft_m" => p.draft_m = read_float(x, what)?,
                "design_depth_m" => {
                    p.design_depth_m = if x.is_null() {
                        None
                    } else {
                        Some(read_float(x, what)?)
                    }
                }
                "ukc_fraction" => p.ukc_fraction = read_float(x, what)?,
                "ukc_min_m" => p.ukc_min_m = read_float(x, what)?,
                "extra_width_coefficient" => p.extra_width_coefficient = read_float(x, what)?,
                "max_extra_width_radius" => p.max_extra_width_radius = read_float(x, what)?,
                "n_bands" => {
                    p.n_bands = x
                        .as_i64()
                        .ok_or_else(|| bad("n_bands must be an integer".into()))?
                }
                "radius_sample_length" => p.radius_sample_length = read_float(x, what)?,
                "rolling_window_m" => p.rolling_window_m = read_float(x, what)?,
                "edge_margin_m" => p.edge_margin_m = read_float(x, what)?,
                "reference_level_m" => p.reference_level_m = read_float(x, what)?,
                "passing_place_spacing_m" => p.passing_place_spacing_m = read_float(x, what)?,
                "passing_place_length_multiple" => {
                    p.passing_place_length_multiple = read_float(x, what)?
                }
                "passing_place_width_multiple" => {
                    p.passing_place_width_multiple = read_float(x, what)?
                }
                "extra_width_reasonable_ratio" => {
                    p.extra_width_reasonable_ratio = read_float(x, what)?
                }
                other => return Err(bad(format!("ChannelDesignParams has no field `{other}`"))),
            }
        }
    }
    Ok(p.validate(methods).map(|()| p))
}

fn read_centreline(v: Option<&Value>) -> Result<Option<Centreline>, EvalError> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let rows = v
        .as_array()
        .ok_or_else(|| bad("centreline_points must be a list".into()))?;
    let mut xy = Vec::with_capacity(rows.len());
    let mut z: Vec<f64> = Vec::new();
    let mut width = None;
    for row in rows {
        let r = row
            .as_array()
            .filter(|r| r.len() == 2 || r.len() == 3)
            .ok_or_else(|| bad("centreline point must be [x, y] or [x, y, z]".into()))?;
        if *width.get_or_insert(r.len()) != r.len() {
            return Err(bad("centreline points differ in length".into()));
        }
        xy.push([read_float(&r[0], "x")?, read_float(&r[1], "y")?]);
        if r.len() == 3 {
            z.push(read_float(&r[2], "z")?);
        }
    }
    Ok(Some(Centreline {
        xy,
        z: if width == Some(3) { Some(z) } else { None },
    }))
}

fn summary_json(s: &ds::DesignSummary) -> Value {
    json!({
        "total_loa": float_json(s.total_loa),
        "beam": float_json(s.beam),
        "design_depth_m": float_json(s.design_depth_m),
        "base_width_m": float_json(s.base_width_m),
        "required_min_radius_m": float_json(s.required_min_radius_m),
        "total_length_m": float_json(s.total_length_m),
        "min_bend_radius_m": opt_float_json(s.min_bend_radius_m),
        "tight_bend_count": s.tight_bend_count,
        "mean_width_m": float_json(s.mean_width_m),
        "max_extra_width_m": float_json(s.max_extra_width_m),
        "max_top_width_m": float_json(s.max_top_width_m),
        "passing_place_spacing_m": float_json(s.passing_place_spacing_m),
        "passing_place_length_m": float_json(s.passing_place_length_m),
        "passing_place_width_m": float_json(s.passing_place_width_m),
        "ok": s.ok,
    })
}

fn dict_json(d: &ds::DesignDict) -> Value {
    let mut out = match summary_json(&d.summary) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let s = &d.sections;
    let radii = Value::Array(
        s.smoothed_radius_m
            .iter()
            .map(|&x| opt_float_json(x))
            .collect(),
    );
    out.insert(
        "sections".into(),
        json!({
            "chainage_m": floats_json(&s.chainage_m),
            "smoothed_radius_m": radii,
            "extra_width_m": floats_json(&s.extra_width_m),
            "bottom_width_m": floats_json(&s.bottom_width_m),
            "top_width_m": floats_json(&s.top_width_m),
            "bed_level_m": floats_json(&s.bed_level_m),
            "half_offset_m": floats_json(&s.half_offset_m),
            "radius_ok": s.radius_ok,
        }),
    );
    // `to_dict` lists name, ok and message only: no severity.
    let checks: Vec<Value> = d
        .checks
        .iter()
        .map(|c| json!({ "name": c.name, "ok": c.ok, "message": c.message }))
        .collect();
    out.insert("checks".into(), Value::Array(checks));
    out.insert("warnings".into(), json!(d.warnings));
    Value::Object(out)
}

fn design_result_json(r: &ds::ChannelDesignResult) -> Value {
    json!({
        "total_loa": float_json(r.total_loa),
        "beam": float_json(r.beam),
        "design_depth_m": float_json(r.design_depth_m),
        "base_width_m": float_json(r.base_width_m),
        "required_min_radius_m": float_json(r.required_min_radius_m),
        "chainage_m": floats_json(&r.chainage_m),
        "radius_m": floats_json(&r.radius_m),
        "smoothed_radius_m": floats_json(&r.smoothed_radius_m),
        "extra_width_m": floats_json(&r.extra_width_m),
        "bottom_width_m": floats_json(&r.bottom_width_m),
        "top_width_m": floats_json(&r.top_width_m),
        "bed_level_m": floats_json(&r.bed_level_m),
        "half_offset_m": floats_json(&r.half_offset_m),
        "radius_ok": r.radius_ok,
        "total_length_m": float_json(r.total_length_m),
        "min_bend_radius_m": float_json(r.min_bend_radius_m),
        "tight_bend_count": r.tight_bend_count,
        "mean_width_m": float_json(r.mean_width_m),
        "max_extra_width_m": float_json(r.max_extra_width_m),
        "max_top_width_m": float_json(r.max_top_width_m),
        "passing_place_spacing_m": float_json(r.passing_place_spacing_m),
        "passing_place_length_m": float_json(r.passing_place_length_m),
        "passing_place_width_m": float_json(r.passing_place_width_m),
        "checks": checks_json(&r.checks),
        "warnings": r.warnings,
    })
}

/// `n_samples`: absent means 160, `null` means no downsampling.
fn n_samples(args: &Value) -> Result<Option<usize>, EvalError> {
    match args.get("n_samples") {
        None => Ok(Some(160)),
        Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| bad("args.n_samples must be a whole number".into())),
    }
}

fn design(methods: &MethodRegistry, function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "under_keel_clearance" => ok(float_json(ds::under_keel_clearance(
            f64_arg(args, "draft_m")?,
            f64_or(args, "ukc_fraction", 0.15)?,
            f64_or(args, "ukc_min_m", 0.5)?,
        ))),
        "required_depth" => ok(float_json(ds::required_depth(
            f64_arg(args, "draft_m")?,
            f64_or(args, "ukc_fraction", 0.15)?,
            f64_or(args, "ukc_min_m", 0.5)?,
        ))),
        "max_draft_for_depth" => ok(float_json(ds::max_draft_for_depth(
            f64_arg(args, "depth_m")?,
            f64_or(args, "ukc_fraction", 0.15)?,
            f64_or(args, "ukc_min_m", 0.5)?,
        ))),
        "resolve_design_depth" => {
            let params = tri!(read_params(methods, args.get("params"))?);
            ok(float_json(ds::resolve_design_depth(&params)))
        }
        "extra_width" => {
            let params = tri!(read_params(methods, args.get("params"))?);
            let loa = f64_arg(args, "total_loa")?;
            let out: Vec<f64> = float_list(args, "radii")?
                .into_iter()
                .map(|r| ds::extra_width(r, loa, &params))
                .collect();
            ok(floats_json(&out))
        }
        "min_bend_radius" => {
            let params = tri!(read_params(methods, args.get("params"))?);
            ok(float_json(ds::min_bend_radius(
                f64_arg(args, "total_loa")?,
                &params,
            )))
        }
        "compute" => {
            let params = tri!(read_params(methods, args.get("params"))?);
            let inp = ChannelDesignInputs {
                total_loa: f64_arg(args, "total_loa")?,
                beam: f64_arg(args, "beam")?,
                centreline: read_centreline(args.get("centreline_points"))?,
                params,
            };
            rule(ds::compute(methods, &inp), |res| {
                json!({
                    "result": design_result_json(&res),
                    "summary": summary_json(&res.summary()),
                    "dict": dict_json(&res.to_dict(Some(40))),
                    "ok": res.ok(),
                })
            })
        }
        "compute_scenario_grid" => {
            let params = match args.get("params").filter(|v| !v.is_null()) {
                Some(p) => Some(tri!(read_params(methods, Some(p))?)),
                None => None,
            };
            let centreline = read_centreline(args.get("centreline_points"))?;
            let loas = float_list(args, "loa_values")?;
            let beams = float_list(args, "beam_values")?;
            rule(
                ds::compute_scenario_grid(
                    methods,
                    centreline.as_ref(),
                    &loas,
                    &beams,
                    params.as_ref(),
                    n_samples(args)?,
                ),
                |g| {
                    let mut scenarios = Map::new();
                    for (k, d) in &g.scenarios {
                        scenarios.insert(k.clone(), dict_json(d));
                    }
                    json!({
                        "design_depth_m": float_json(g.design_depth_m),
                        "total_length_m": float_json(g.total_length_m),
                        "loa_values": floats_json(&g.loa_values),
                        "beam_values": floats_json(&g.beam_values),
                        "scenarios": Value::Object(scenarios),
                    })
                },
            )
        }
        "stage_profiles" => {
            let params = tri!(read_params(methods, args.get("params"))?);
            let centreline = read_centreline(args.get("centreline_points"))?;
            let curv = ds::compute_curvature(centreline.as_ref(), &params);
            let loa = ds::precompute_loa(&curv, f64_arg(args, "total_loa")?, &params);
            let points: Vec<Value> = curv
                .points
                .iter()
                .map(|p| json!([float_json(p[0]), float_json(p[1])]))
                .collect();
            ok(json!({
                "curvature": {
                    "points": points,
                    "chainages": floats_json(&curv.chainages),
                    "radius": floats_json(&curv.radius),
                    "smoothed_radius": floats_json(&curv.smoothed_radius),
                    "total_length": float_json(curv.total_length),
                    "z": curv.z.as_ref().map_or(Value::Null, |z| floats_json(z)),
                    "heading": floats_json(&curv.heading),
                },
                "loa": {
                    "total_loa": float_json(loa.total_loa),
                    "required_min_radius": float_json(loa.required_min_radius),
                    "radius_ok": loa.radius_ok,
                    "tight_mask": loa.tight_mask,
                    "bend_phi_deg": floats_json(&loa.bend_phi_deg),
                    "bend_r1_m": floats_json(&loa.bend_r1_m),
                },
            }))
        }
        other => Err(bad(format!("design has no parity function `{other}`"))),
    }
}

// --- simple ---------------------------------------------------------------------------------

fn read_pianc_inputs(d: &Value) -> Result<PiancInputs, EvalError> {
    let obj = d
        .as_object()
        .ok_or_else(|| bad("inputs must be an object".into()))?;
    let mut inp = PiancInputs::new(f64_arg(d, "beam_m")?, f64_arg(d, "loa_m")?);
    for (k, v) in obj {
        let what = k.as_str();
        match what {
            "beam_m" | "loa_m" => {}
            "draft_m" => inp.draft_m = read_float(v, what)?,
            "width_multiple_straight" => inp.width_multiple_straight = read_float(v, what)?,
            "radius_multiple" => inp.radius_multiple = read_float(v, what)?,
            "ukc_fraction" => inp.ukc_fraction = read_float(v, what)?,
            "ukc_min_m" => inp.ukc_min_m = read_float(v, what)?,
            "side_slope_h_per_v" => inp.side_slope_h_per_v = read_float(v, what)?,
            "passing_place_spacing_m" => inp.passing_place_spacing_m = read_float(v, what)?,
            "passing_place_length_multiple" => {
                inp.passing_place_length_multiple = read_float(v, what)?
            }
            "width_method" => inp.width_method = read_str(v, what)?,
            "width_options" => inp.width_options = read_options(v, what)?,
            other => return Err(bad(format!("PIANCInputs has no field `{other}`"))),
        }
    }
    Ok(inp)
}

fn simple_ep(methods: &MethodRegistry, function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "size_channel" => {
            let inp = read_pianc_inputs(arg(args, "inputs")?)?;
            rule(
                simple::size_channel(methods, &inp, opt_f64(args, "depth_m")?),
                |c| {
                    json!({
                        "bottom_width_straight_m": float_json(c.bottom_width_straight_m),
                        "bottom_width_bend_m": float_json(c.bottom_width_bend_m),
                        "bend_widening_m": float_json(c.bend_widening_m),
                        "min_bend_radius_m": float_json(c.min_bend_radius_m),
                        "required_depth_m": float_json(c.required_depth_m),
                        "max_draft_for_depth_m": float_json(c.max_draft_for_depth_m),
                        "top_width_straight_m": float_json(c.top_width_straight_m),
                        "design_depth_m": float_json(c.design_depth_m),
                        "passing_place_spacing_m": float_json(c.passing_place_spacing_m),
                        "passing_place_length_m": float_json(c.passing_place_length_m),
                    })
                },
            )
        }
        other => Err(bad(format!("simple has no parity function `{other}`"))),
    }
}

// --- width_profile --------------------------------------------------------------------------

/// The bridge's `make_width_fn(loa_m, beam_m, draft_m, rule_set, **overrides)` as inputs.
fn bridge_inputs(args: &Value) -> Result<ChannelRuleInputs, EvalError> {
    let mut d = Map::new();
    d.insert("loa_m".into(), arg(args, "loa_m")?.clone());
    d.insert("beam_m".into(), arg(args, "beam_m")?.clone());
    d.insert("draft_m".into(), arg(args, "draft_m")?.clone());
    d.insert(
        "rule_set".into(),
        args.get("rule_set")
            .cloned()
            .unwrap_or_else(|| json!("pianc")),
    );
    if let Some(Value::Object(o)) = args.get("overrides") {
        for (k, v) in o {
            d.insert(k.clone(), v.clone());
        }
    }
    read_rule_inputs(&Value::Object(d), Some(0.0))
}

fn width_profile(
    methods: &MethodRegistry,
    function: &str,
    args: &Value,
) -> Result<Value, EvalError> {
    match function {
        "make_width_fn" => {
            let f: WidthFn = tri!(WidthFn::new(methods, bridge_inputs(args)?));
            let mut rows = Vec::new();
            for r in float_list(args, "radii")? {
                let w = tri!(f.width(r));
                let h = tri!(f.half_width(r));
                rows.push(json!({
                    "radius": float_json(r),
                    "width": float_json(w),
                    "half_width": float_json(h),
                }));
            }
            ok(Value::Array(rows))
        }
        "width_params_dict" => rule(wp::width_params(methods, bridge_inputs(args)?), |p| {
            json!({
                "rule_set": p.rule_set,
                "straight_bottom_m": float_json(p.straight_bottom_m),
                "widening_coeff_note": p.widening_coeff_note,
                "loa_m": float_json(p.loa_m),
                "beam_m": float_json(p.beam_m),
                "draft_m": float_json(p.draft_m),
            })
        }),
        other => Err(bad(format!(
            "width_profile has no parity function `{other}`"
        ))),
    }
}
