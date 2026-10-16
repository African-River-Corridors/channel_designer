//! Width methods: the pluggable registry behind `size_channel`, `width_at_radius` and
//! `design::compute`. Mirrors `rules/methods.py`.
//!
//! A *width method* turns a vessel (length overall and beam) into a straight channel width, a
//! width through a bend, and a design minimum bend radius. The crate ships one method, `"pianc"`.
//! Another crate adds its own, for example a consultant's method it may not publish, without
//! editing this one: build a [`WidthMethod`] from closures and call
//! [`MethodRegistry::register`] (or [`register_width_method`] for the process-wide registry).
//!
//! Python also finds methods through the `channel_designer.width_methods` entry-point group. Rust
//! has no such mechanism; a plugin registers itself explicitly.
//!
//! Every callable takes an [`Options`] value: the method's own parameters, passed through
//! untouched from `ChannelRuleInputs.options` or `ChannelDesignParams.width_options`.
//!
//! Contract (the tests hold every registered method to it):
//!
//! - `straight_width(loa_m, beam_m, options) -> m`: bottom width on a straight, > 0.
//! - `bend_width(loa_m, beam_m, options, bend_radius_m, design_radius_m, deflection_deg) -> m`:
//!   bottom width through a bend of radius `bend_radius_m` when the design radius is
//!   `design_radius_m` and the bend turns `deflection_deg`. Callers take `max(straight, bend)`,
//!   so a method need not clamp.
//! - `min_radius(loa_m, options) -> m`: the design minimum bend radius.
//! - `breakdown(loa_m, beam_m, options)`: optional; the width build-up, for reports.

use super::{RuleError, tuple_repr, value_err};
use crate::pyrepr::repr_str;
use ovcd_core::pyfmt::round_to;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

/// One option value. Mirrors the JSON-like values a Python options mapping holds.
#[derive(Debug, Clone, PartialEq)]
pub enum OptValue {
    Float(f64),
    Bool(bool),
    Str(String),
    /// A mapping that keeps its insertion order (a Python `dict`).
    Map(Options),
}

impl OptValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            OptValue::Float(x) => Some(*x),
            _ => None,
        }
    }
}

/// An insertion-ordered mapping of option name to value, as a Python `dict`: setting a key that
/// exists keeps its position.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options(Vec<(String, OptValue)>);

impl Options {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&OptValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `d[key] = value`.
    pub fn set(&mut self, key: &str, value: OptValue) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// Builder form of [`Options::set`].
    pub fn with(mut self, key: &str, value: OptValue) -> Self {
        self.set(key, value);
        self
    }

    /// `d.update(other)`.
    pub fn update(&mut self, other: &Options) {
        for (k, v) in &other.0 {
            self.set(k, v.clone());
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &OptValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `straight_width(loa_m, beam_m, options)`.
pub type StraightFn = Arc<dyn Fn(f64, f64, &Options) -> Result<f64, RuleError> + Send + Sync>;
/// `bend_width(loa_m, beam_m, options, bend_radius_m, design_radius_m, deflection_deg)`.
pub type BendFn =
    Arc<dyn Fn(f64, f64, &Options, f64, f64, f64) -> Result<f64, RuleError> + Send + Sync>;
/// `min_radius(loa_m, options)`.
pub type MinRadiusFn = Arc<dyn Fn(f64, &Options) -> Result<f64, RuleError> + Send + Sync>;
/// `breakdown(loa_m, beam_m, options)`.
pub type BreakdownFn = Arc<dyn Fn(f64, f64, &Options) -> Result<Options, RuleError> + Send + Sync>;

/// One channel-width method. See the module docs for the contract.
#[derive(Clone)]
pub struct WidthMethod {
    pub name: String,
    pub straight_width: StraightFn,
    pub bend_width: BendFn,
    pub min_radius: MinRadiusFn,
    pub breakdown: Option<BreakdownFn>,
    pub description: String,
}

impl core::fmt::Debug for WidthMethod {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WidthMethod")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

/// The registered width methods, by name. A new registry holds the built-in `"pianc"`.
#[derive(Debug, Clone)]
pub struct MethodRegistry {
    methods: BTreeMap<String, Arc<WidthMethod>>,
}

impl Default for MethodRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MethodRegistry {
    pub fn new() -> Self {
        let mut methods = BTreeMap::new();
        let pianc = pianc_method();
        methods.insert(pianc.name.clone(), Arc::new(pianc));
        Self { methods }
    }

    /// Add `method` under `method.name`. A name that is taken is refused unless `replace`.
    pub fn register(&mut self, method: WidthMethod, replace: bool) -> Result<(), RuleError> {
        if method.name.is_empty() {
            return value_err("a width method needs a name".into());
        }
        if self.methods.contains_key(&method.name) && !replace {
            return value_err(format!(
                "width method {} is already registered; pass replace=True to override it",
                repr_str(&method.name)
            ));
        }
        self.methods.insert(method.name.clone(), Arc::new(method));
        Ok(())
    }

    /// Remove a method. The built-in `"pianc"` cannot be removed; an unknown name is ignored.
    pub fn unregister(&mut self, name: &str) -> Result<(), RuleError> {
        if name == "pianc" {
            return value_err("the built-in 'pianc' method cannot be unregistered".into());
        }
        self.methods.remove(name);
        Ok(())
    }

    /// The method called `name`.
    pub fn get(&self, name: &str) -> Result<Arc<WidthMethod>, RuleError> {
        match self.methods.get(name) {
            Some(m) => Ok(Arc::clone(m)),
            None => value_err(format!(
                "unknown width method {}; registered: {}",
                repr_str(name),
                self.names().join(", ")
            )),
        }
    }

    /// Names of every registered method, sorted.
    pub fn names(&self) -> Vec<String> {
        self.methods.keys().cloned().collect()
    }

    /// `repr(tuple(names))`, as the Python error messages quote it.
    pub(crate) fn names_repr(&self) -> String {
        tuple_repr(&self.names())
    }
}

static GLOBAL: OnceLock<Mutex<MethodRegistry>> = OnceLock::new();

/// The process-wide registry: Python's module-level `_REGISTRY`.
pub fn global_methods() -> &'static Mutex<MethodRegistry> {
    GLOBAL.get_or_init(|| Mutex::new(MethodRegistry::new()))
}

/// Register on the process-wide registry.
pub fn register_width_method(method: WidthMethod, replace: bool) -> Result<(), RuleError> {
    global_methods()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .register(method, replace)
}

// =====================================================================================
// The built-in method: "pianc"
// =====================================================================================
// Default path: one-lane fairway = width_multiple x beam, bend widening LOA^2/(8R),
// R_min = radius_multiple x LOA. NOTE the LOA^2/(8R) widening (c_C = 0.125) is BELOW WG 141's
// own floor of c_C ~ 0.5: see the rulesets in `rulesets`. Prefer `channel_shape` with a named
// ruleset for new work.
//
// `use_buildup` switches the straight width to the PIANC Report 121 concept-design build-up
// W = W_BM + sum(W_i) + W_Br + W_Bg, coefficients as fractions of beam. Those are typical
// concept-design values for a slow-speed inland reach; confirm them against the report before
// use.

/// W_BM = n x B, by maneuverability.
pub const PIANC_MANEUVER: [(&str, f64); 3] = [("good", 1.3), ("moderate", 1.5), ("poor", 1.8)];

/// The additional widths sum(W_i), each a fraction of beam (slow-speed inland defaults).
pub const PIANC_EXTRA_WIDTHS: [(&str, f64); 8] = [
    ("prevailing_wind", 0.1),
    ("cross_current", 0.4),
    ("longitudinal_current", 0.0),
    ("wave", 0.0),
    ("aids_to_navigation", 0.1),
    ("bottom_surface", 0.1),
    ("depth_to_draft", 0.2),
    ("cargo_hazard", 0.0),
];

fn extra_widths_default() -> Options {
    let mut o = Options::new();
    for (k, v) in PIANC_EXTRA_WIDTHS {
        o.set(k, OptValue::Float(v));
    }
    o
}

/// `PIANC_DEFAULTS`: one-lane fairway = k_w x beam (2.8), min bend radius = k_r x LOA (3.0),
/// no build-up, "poor" maneuverability, bank clearances 0.5 (steep / hard) and 0.3 (sloping).
pub fn pianc_defaults() -> Options {
    Options::new()
        .with("width_multiple", OptValue::Float(2.8))
        .with("radius_multiple", OptValue::Float(3.0))
        .with("use_buildup", OptValue::Bool(false))
        .with("maneuverability", OptValue::Str("poor".into()))
        .with("extra_widths", OptValue::Map(extra_widths_default()))
        .with("bank_clearance_outer", OptValue::Float(0.5))
        .with("bank_clearance_inner", OptValue::Float(0.3))
}

/// The built-in method's options after defaults and checks.
struct PiancOpts {
    width_multiple: f64,
    radius_multiple: f64,
    use_buildup: bool,
    maneuverability: String,
    extra_widths: Vec<f64>,
    bank_outer: f64,
    bank_inner: f64,
}

fn maneuver_factor(name: &str) -> Option<f64> {
    PIANC_MANEUVER
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
}

fn maneuver_names() -> String {
    let names: Vec<String> = PIANC_MANEUVER.iter().map(|(k, _)| k.to_string()).collect();
    tuple_repr(&names)
}

fn float_opt(o: &Options, key: &str) -> Result<f64, RuleError> {
    o.get(key)
        .and_then(OptValue::as_f64)
        .ok_or_else(|| RuleError::Value(format!("option {key} must be a number")))
}

fn pianc_opts(options: &Options) -> Result<PiancOpts, RuleError> {
    let mut o = pianc_defaults();
    o.update(options);
    let maneuverability = match o.get("maneuverability") {
        Some(OptValue::Str(s)) if maneuver_factor(s).is_some() => s.clone(),
        _ => {
            return value_err(format!(
                "maneuverability must be one of {}",
                maneuver_names()
            ));
        }
    };
    let use_buildup = match o.get("use_buildup") {
        Some(OptValue::Bool(b)) => *b,
        Some(OptValue::Float(x)) => *x != 0.0,
        _ => return value_err("option use_buildup must be true or false".into()),
    };
    let extra_widths = match o.get("extra_widths") {
        Some(OptValue::Map(m)) => m
            .iter()
            .map(|(k, v)| {
                v.as_f64()
                    .ok_or_else(|| RuleError::Value(format!("extra width {k} must be a number")))
            })
            .collect::<Result<Vec<f64>, _>>()?,
        _ => return value_err("option extra_widths must be a mapping".into()),
    };
    Ok(PiancOpts {
        width_multiple: float_opt(&o, "width_multiple")?,
        radius_multiple: float_opt(&o, "radius_multiple")?,
        use_buildup,
        maneuverability,
        extra_widths,
        bank_outer: float_opt(&o, "bank_clearance_outer")?,
        bank_inner: float_opt(&o, "bank_clearance_inner")?,
    })
}

/// Python's `sum()` of floats, starting from the int 0. CPython 3.12 sums floats with Neumaier
/// compensation, so a plain left-to-right fold differs in the last bit (and `round(..., 2)` of
/// the result can then differ). The reference is printed by 3.12.
fn py_sum(xs: &[f64]) -> f64 {
    let mut sum = 0.0_f64;
    let mut comp = 0.0_f64;
    for &x in xs {
        if !x.is_finite() {
            sum += x;
            continue;
        }
        let t = sum + x;
        if sum.abs() >= x.abs() {
            comp += (sum - t) + x;
        } else {
            comp += (x - t) + sum;
        }
        sum = t;
    }
    if comp != 0.0 && comp.is_finite() {
        sum += comp;
    }
    sum
}

fn pianc_straight(_loa_m: f64, beam_m: f64, options: &Options) -> Result<f64, RuleError> {
    let o = pianc_opts(options)?;
    if !o.use_buildup {
        return Ok(o.width_multiple * beam_m);
    }
    let w_bm = maneuver_factor(&o.maneuverability).unwrap_or(0.0) * beam_m;
    let extra = py_sum(&o.extra_widths) * beam_m;
    let banks = (o.bank_outer + o.bank_inner) * beam_m;
    Ok(w_bm + extra + banks)
}

fn pianc_bend(
    loa_m: f64,
    beam_m: f64,
    options: &Options,
    bend_radius_m: f64,
    _design_radius_m: f64,
    _deflection_deg: f64,
) -> Result<f64, RuleError> {
    let straight = pianc_straight(loa_m, beam_m, options)?;
    if bend_radius_m <= 0.0 || bend_radius_m.is_infinite() {
        return Ok(straight);
    }
    Ok(straight + loa_m * loa_m / (8.0 * bend_radius_m))
}

fn pianc_min_radius(loa_m: f64, options: &Options) -> Result<f64, RuleError> {
    Ok(pianc_opts(options)?.radius_multiple * loa_m)
}

fn pianc_breakdown(_loa_m: f64, beam_m: f64, options: &Options) -> Result<Options, RuleError> {
    let o = pianc_opts(options)?;
    let b = beam_m;
    if !o.use_buildup {
        return Ok(Options::new()
            .with(
                "fairway_kw_x_beam",
                OptValue::Float(round_to(o.width_multiple * b, 2)),
            )
            .with("width_multiple", OptValue::Float(o.width_multiple)));
    }
    let n = maneuver_factor(&o.maneuverability).unwrap_or(0.0);
    Ok(Options::new()
        .with("W_BM_manoeuvring_lane", OptValue::Float(round_to(n * b, 2)))
        .with(
            "sum_Wi_additional",
            OptValue::Float(round_to(py_sum(&o.extra_widths) * b, 2)),
        )
        .with(
            "bank_clearances_Br_Bg",
            OptValue::Float(round_to((o.bank_outer + o.bank_inner) * b, 2)),
        )
        .with("maneuverability", OptValue::Str(o.maneuverability)))
}

/// The built-in `"pianc"` method.
pub fn pianc_method() -> WidthMethod {
    WidthMethod {
        name: "pianc".into(),
        straight_width: Arc::new(pianc_straight),
        bend_width: Arc::new(pianc_bend),
        min_radius: Arc::new(pianc_min_radius),
        breakdown: Some(Arc::new(pianc_breakdown)),
        description: "PIANC: straight = k_w x beam (or the Report 121 build-up); \
                      bend = straight + LOA^2 / (8R); R_min = k_r x LOA"
            .into(),
    }
}
