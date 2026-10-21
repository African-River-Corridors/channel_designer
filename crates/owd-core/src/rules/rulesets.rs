//! Channel sizing: the channel a selection of Rule Sets requires ([`channel_shape_for`], spec 0007),
//! and the width methods (`size_channel`). Mirrors `rules/rulesets.py`.
//!
//! - [`channel_shape_for`]: the selected Rule Sets plus a vessel class give the channel shape. The
//!   rules engine (`arc-rules`, [`super::engine`]) sizes it: the governing limit of each Requirement.
//! - [`size_channel`]: one-lane channel geometry (straight width, min turn radius, bend width,
//!   required depth, bed level against the pool's minimum water level) from a width method. The
//!   crate ships `"pianc"`; another crate can register more, see [`super::methods`]. The width
//!   methods, the `pianc` method with its PIANC Report 121 build-up, and `ChannelDesignParams` are
//!   design assumptions, not verified rules: no document stage uses them, and Report 121 is a
//!   maritime approach-channel guideline.
//!
//! `"pianc"` is the PIANC concept-design path: straight width = `k_w x beam` (or the Report 121
//! build-up W = W_BM + sum(W_i) + W_Br + W_Bg), min radius = `k_r x LOA`, bend widening
//! LOA^2/(8R). The coefficients are typical concept-design values for a slow-speed inland reach;
//! confirm them against the PIANC report and your own design basis before use.
//!
//! Water level and bed: `min_water_level_m` is the pool's minimum water level (masl). The design
//! bed sits at `bed_level_m = min_water_level_m - required_depth`. A pool at sea level (min WL
//! = 0 masl) puts the bed below datum (a marine or tidal dredge reach) and is flagged.

use super::design::{required_depth, under_keel_clearance};
use super::engine::{self, Selection, Vessel};
use super::methods::{MethodRegistry, OptValue, Options, PIANC_EXTRA_WIDTHS, WidthMethod};
use super::{RuleError, value_err};

use crate::py_max;
use arc_rules::eval::{MeasureValue, Outcome, Trace, TraceRow};
use arc_rules::model::Limit;
use ovcd_core::checks::Check;
use ovcd_core::pyfmt::{fixed, py_str};
use ovcd_core::vessel_standards::get_standard;
use std::sync::Arc;

// =====================================================================================
// BEND TIGHTNESS: does the channel still fit the river through this bend?
// =====================================================================================
// A channel rounding a bend at its minimum radius cannot trace the river's centreline; it cuts
// the corner, so its centreline sits a lateral distance e from the river's. The bend is TIGHT
// when e exceeds the lateral ROOM available, and a tight bend is what the alignment optimiser
// has to solve rather than follow.
//
// WHAT "ROOM" MEANS IS A JUDGEMENT, NOT A GEOMETRIC FACT, and on a narrow river it decides most
// of the answer. Where the channel is about as wide as the river, the genuine spare room is
// close to zero.
//
//   room = w/2                       -> lets the channel CENTRELINE sit at the bank, i.e. half
//                                       the channel outside the river, while still calling the
//                                       bend "gentle". That is a large bank cut hidden in geometry.
//   room = (w - W_top)/2 + b_bank    -> the honest form: the spare room the river actually has,
//                                       plus an EXPLICIT permitted bank cut.
//
// BANK_CUT_FRACTION is a USER CHOICE, not a standard: b_bank = BANK_CUT_FRACTION x channel
// BOTTOM width, i.e. the design channel bottom must still substantially overlap the natural bed.
// It is a fraction rather than metres so it scales with the vessel. 0.40 is an example default,
// not a recommendation; pass your own to `lateral_room_m`.
pub const BANK_CUT_FRACTION: f64 = 0.40;

/// Lateral room per side before a bend counts as TIGHT.
///
/// room = (w - W_top)/2 + bank_cut_fraction * W_bottom
///
/// The first term is the river's genuine spare room (often ~0, and negative where the channel is
/// wider than the river, clamped at 0, since a channel wider than the river has no spare room, it
/// does not have negative room). The second is the permitted bank cut.
pub fn lateral_room_m(
    wetted_width_m: f64,
    top_width_m: f64,
    bottom_width_m: f64,
    bank_cut_fraction: f64,
) -> f64 {
    let spare = py_max(0.0, (wetted_width_m - top_width_m) / 2.0);
    spare + bank_cut_fraction * bottom_width_m
}

/// Bend class from the offset a minimum-radius curve needs against the room available.
///
/// - `'S'` straight: the bend needs no offset at all
/// - `'G'` gentle: it fits inside the room; the channel can follow the river
/// - `'T'` tight: it does not; this bend is the optimiser's problem
pub fn classify_bend(e_m: f64, room_m: f64, tol_m: f64) -> &'static str {
    if e_m <= tol_m {
        return "S";
    }
    if e_m <= room_m + tol_m { "G" } else { "T" }
}

/// The channel a selection of Rule Sets and a vessel class require. Everything here is derived.
///
/// The shape keeps the selection that sized it, narrowed to the bend-widening Rules, so the water
/// checks read the governing widening at any radius.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelShape {
    /// The selected sets, as `id@version` joined with ` + `.
    pub rule_sets: String,
    pub vessel_standard_id: String,
    pub loa_m: f64,
    pub beam_m: f64,
    pub draught_m: f64,
    pub formation: String,
    pub bottom_width_straight_m: f64,
    pub min_bend_radius_m: f64,
    pub design_depth_m: f64,
    pub bend_widening_at_rmin_m: f64,
    pub bottom_width_at_rmin_m: f64,
    /// `"table"`, `"table-5.1-ratio"` or `"rule"`.
    pub width_basis: String,
    pub source: String,
    pub warnings: Vec<String>,
    /// The selection that sized the shape, narrowed to the bend-widening Rules, and its vessel.
    engine: (Arc<Selection>, Vessel),
}

impl ChannelShape {
    /// The selection that sized the shape, narrowed to the bend-widening Rules.
    pub fn selection(&self) -> &Selection {
        &self.engine.0
    }

    pub fn bend_widening_m(&self, radius_m: f64) -> f64 {
        let (selection, vessel) = &self.engine;
        engine::widening(selection, vessel, radius_m)
            .expect("the selection sized the bend widening at R_min, so it sizes it here")
    }

    pub fn bottom_width_at_m(&self, radius_m: f64) -> f64 {
        self.bottom_width_straight_m + self.bend_widening_m(radius_m)
    }
}

/// The vessel the engine sizes for a vessel class: the long end of its LOA, its beam, the draught
/// (the class's nominal one unless given), its formation and class.
pub fn vessel_for(vessel_standard_id: &str, draught_m: Option<f64>) -> Result<Vessel, RuleError> {
    let v = get_standard(vessel_standard_id).map_err(RuleError::Key)?;
    Ok(Vessel {
        loa_m: v.loa_m.1,
        beam_m: v.beam_m,
        draught_m: draught_m.unwrap_or_else(|| v.draught_nominal_m()),
        formation: v.formation.clone(),
        class_name: v.class_name.clone(),
    })
}

/// The channel a selection of Rule Sets requires for a vessel class (spec 0007): the governing limit
/// of each sized Measure, and warnings from the Rule Trace of the sized channel.
///
/// `draught_m` overrides the vessel class's nominal draught.
pub fn channel_shape_for(
    selection: &Selection,
    vessel_standard_id: &str,
    draught_m: Option<f64>,
) -> Result<ChannelShape, RuleError> {
    let v = get_standard(vessel_standard_id).map_err(RuleError::Key)?;
    let vessel = Vessel {
        loa_m: v.loa_m.1, // size on the LONG end of the class
        beam_m: v.beam_m,
        draught_m: draught_m.unwrap_or_else(|| v.draught_nominal_m()),
        formation: v.formation.clone(),
        class_name: v.class_name.clone(),
    };
    let sized = engine::size(selection, &vessel)?;
    let check = selection.check(&vessel, &sized);
    let keys: Vec<String> = selection.sets.iter().map(|s| s.key()).collect();
    Ok(ChannelShape {
        rule_sets: keys.join(" + "),
        vessel_standard_id: v.standard_id.clone(),
        loa_m: vessel.loa_m,
        beam_m: vessel.beam_m,
        draught_m: vessel.draught_m,
        formation: v.formation.clone(),
        bottom_width_straight_m: sized.bottom_width_straight_m,
        min_bend_radius_m: sized.min_bend_radius_m,
        design_depth_m: sized.design_depth_m,
        bend_widening_at_rmin_m: sized.bend_widening_at_rmin_m,
        bottom_width_at_rmin_m: sized.bottom_width_straight_m + sized.bend_widening_at_rmin_m,
        width_basis: width_basis(selection, &vessel, &check).into(),
        source: format!("Rule Sets {}; vessel {}", keys.join(", "), v.source),
        warnings: engine_warnings(selection, &vessel, &sized, &check),
        engine: (
            Arc::new(selection.narrowed_to(engine::BEND_WIDENING, &vessel)),
            vessel,
        ),
    })
}

/// The `gb-straight-width` limit of a Rule: its shape and coefficients.
fn gb_width_limit(rule: &arc_rules::model::Rule) -> Option<(&str, &[(String, f64)])> {
    match &rule.limit {
        Some(Limit::Formula {
            shape,
            coefficients,
            ..
        }) if shape == "gb-straight-width" => Some((shape.as_str(), coefficients.as_slice())),
        _ => None,
    }
}

/// The rows of a trace that tested a Rule on the channel (not `not applicable`).
fn applied(check: &Trace) -> impl Iterator<Item = &TraceRow> {
    check
        .rows
        .iter()
        .filter(|r| r.object.is_some() && r.outcome != Outcome::NotApplicable)
}

/// `"table"` or `"table-5.1-ratio"` when a GB 50139 width governs, by whether the vessel matches a
/// Table A.2 entry; `"rule"` otherwise.
fn width_basis(selection: &Selection, vessel: &Vessel, check: &Trace) -> &'static str {
    let governing = check.rows.iter().find(|r| {
        r.governing
            && r.measure
                .as_ref()
                .is_some_and(|m| m.id == engine::BOTTOM_WIDTH)
    });
    let rule = governing.and_then(|r| selection.rule(r.rule.as_deref()?));
    match rule.and_then(gb_width_limit) {
        Some((shape, c)) => match engine::gb_table_match(shape, c, vessel.loa_m, vessel.beam_m) {
            Ok(Some(_)) => "table",
            _ => "table-5.1-ratio",
        },
        None => "rule",
    }
}

/// GB 50139's depth clearance ranges by class (WG 141 A.3.2, p162), for the warning text only.
const GB_DEPTH_RANGE_M: [(&str, (f64, f64)); 3] =
    [("I", (0.4, 0.5)), ("II", (0.3, 0.4)), ("III", (0.3, 0.4))];

/// The warnings of a sized channel, from its Rule Trace, in the old order: the GB width basis, the
/// GB depth clearance, and the WG 141 depth floor.
fn engine_warnings(
    selection: &Selection,
    vessel: &Vessel,
    sized: &engine::Sized,
    check: &Trace,
) -> Vec<String> {
    let mut warns = Vec::new();
    let number = |r: &TraceRow| match r.value {
        Some(MeasureValue::Number(x)) => Some(x),
        _ => None,
    };
    let swept = applied(check)
        .find(|r| {
            r.measure
                .as_ref()
                .is_some_and(|m| m.id == engine::GB_SWEPT_PATH)
        })
        .and_then(number);
    let (loa_m, beam_m, t) = (vessel.loa_m, vessel.beam_m, vessel.draught_m);
    for row in applied(check) {
        let Some(rule) = row.rule.as_deref().and_then(|id| selection.rule(id)) else {
            continue;
        };
        let (Some((shape, c)), Some(w_formula)) = (gb_width_limit(rule), swept) else {
            continue;
        };
        match engine::gb_table_match(shape, c, loa_m, beam_m) {
            Ok(Some(w_tab)) => {
                if (w_tab - w_formula).abs() / w_tab > 0.05 {
                    warns.push(format!(
                        "Table A.2 gives {} m where the A.3.2 formula gives {} m; the table \
                         includes wind, the formula does not. Using the table.",
                        fixed(w_tab, 0),
                        fixed(w_formula, 1)
                    ));
                }
            }
            Ok(None) => {
                let ratio = c
                    .iter()
                    .find(|(k, _)| k == "ratio")
                    .map_or(0.0, |(_, x)| *x);
                let w = ratio * beam_m;
                warns.push(format!(
                    "no Table A.2 entry near LOA {} m / beam {} m, so the width is WG 141 Table 5.1's \
                     China-River single-lane ratio W_F/B = {} ({} m), which the table's own footnote \
                     records as WIND-INCLUDED. The A.3.2 formula gives {} m and omits wind (A.3.3 lists \
                     curvature, bank interaction, cross-flow and passing distance only); the implied \
                     ratio between them is {}.",
                    fixed(loa_m, 0),
                    fixed(beam_m, 1),
                    py_str(ratio),
                    fixed(w, 1),
                    fixed(w_formula, 1),
                    fixed(w / w_formula, 2)
                ));
            }
            Err(_) => {}
        }
    }
    for row in applied(check) {
        let Some(rule) = row.rule.as_deref().and_then(|id| selection.rule(id)) else {
            continue;
        };
        let by_class = rule
            .applies_when
            .conditions
            .iter()
            .any(|(k, _)| k == engine::VESSEL_CLASS);
        let clearance = match &rule.limit {
            Some(Limit::Formula {
                shape,
                coefficients,
                ..
            }) if shape == "plus-clearance" && by_class => coefficients
                .iter()
                .find(|(k, _)| k == "clearance")
                .map(|(_, x)| *x),
            _ => None,
        };
        let Some(c) = clearance else {
            continue;
        };
        let class = &vessel.class_name;
        match GB_DEPTH_RANGE_M.iter().find(|(k, _)| k == class) {
            None => warns.push(format!(
                "GB 50139's depth clause (A.3.2, p162) covers classes I-III only; class {} \
                 is outside it, so the depth falls back to the ruleset's {} m add-on, which \
                 is a user choice, not the standard's.",
                class,
                py_str(c)
            )),
            Some((_, rng)) => warns.push(format!(
                "depth = draught + {} m, the {} end of GB 50139's class-{} range {}-{} m \
                 (WG 141 A.3.2, p162, as summarised \u{2014} we do not hold GB 50139 itself).",
                py_str(c),
                if c == rng.1 { "strict" } else { "permissive" },
                class,
                py_str(rng.0),
                py_str(rng.1)
            )),
        }
    }
    let depth = sized.design_depth_m;
    if applied(check).any(|r| {
        r.rule.as_deref() == Some("owd/5.3.4.3/depth-ratio-floor") && r.outcome == Outcome::Warn
    }) {
        warns.push(format!(
            "design depth {} m is {} m BELOW PIANC's river minimum depth ratio of 1.2 x draught \
             ({} m at T {} m; WG 141 5.3.4.3, p108, which recommends 1.3 x T and says never \
             below 1.2).",
            fixed(depth, 2),
            fixed(1.2 * t - depth, 2),
            fixed(1.2 * t, 2),
            fixed(t, 2)
        ));
    }
    warns
}

/// Vessel + reach inputs for [`size_channel`].
///
/// `rule_set` names a width method (see [`MethodRegistry::names`]). The `pianc_*` fields are the
/// built-in method's parameters; `options` carries a registered method's own parameters and is
/// passed to it untouched (for `"pianc"` it overrides the `pianc_*` fields).
///
/// Python checks the inputs when it builds them; here [`ChannelRuleInputs::validate`] does, and
/// [`size_channel`] and [`width_at_radius`] call it first.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRuleInputs {
    pub beam_m: f64,
    /// Convoy length overall (barge + pusher): drives bends.
    pub loa_m: f64,
    pub draft_m: f64,
    /// Pool minimum water level (masl); pool 0 = 0.0.
    pub min_water_level_m: f64,
    pub rule_set: String,

    pub side_slope_h_per_v: f64,
    pub ukc_fraction: f64,
    pub ukc_min_m: f64,
    /// Evaluate bend width here (default 3 x LOA).
    pub reference_bend_radius_m: Option<f64>,

    // --- built-in "pianc" method parameters ---
    // Default path: one-lane fairway = 2.8 x beam + bend widening LOA^2/(8R), R_min = 3 x LOA.
    // NOTE this LOA^2/(8R) widening (c_C = 0.125) is BELOW WG 141's own floor of c_C ~ 0.5: see
    // the Rule Sets (spec 0007). Prefer `channel_shape_for` with a Rule Set for new work. The Report-121
    // concept-design build-up (W_BM + sum(W_i) + bank clearances) is retained behind
    // `pianc_use_buildup` for later coefficient confirmation against the source PDFs.
    pub pianc_width_multiple: f64,
    pub pianc_use_buildup: bool,
    pub pianc_maneuverability: String,
    pub pianc_radius_multiple: f64,
    /// Additional widths sum(W_i), each a fraction of beam, in order.
    pub pianc_extra_widths: Vec<(String, f64)>,
    pub pianc_bank_clearance_outer: f64,
    pub pianc_bank_clearance_inner: f64,

    /// A registered width method's own parameters.
    pub options: Options,
}

impl ChannelRuleInputs {
    /// The four required fields; everything else takes the Python default.
    pub fn new(beam_m: f64, loa_m: f64, draft_m: f64, min_water_level_m: f64) -> Self {
        Self {
            beam_m,
            loa_m,
            draft_m,
            min_water_level_m,
            rule_set: "pianc".into(),
            side_slope_h_per_v: 3.0,
            ukc_fraction: 0.15,
            ukc_min_m: 0.5,
            reference_bend_radius_m: None,
            pianc_width_multiple: 2.8,
            pianc_use_buildup: false,
            pianc_maneuverability: "poor".into(),
            pianc_radius_multiple: 3.0,
            pianc_extra_widths: PIANC_EXTRA_WIDTHS
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            pianc_bank_clearance_outer: 0.5,
            pianc_bank_clearance_inner: 0.3,
            options: Options::new(),
        }
    }

    /// The Python `__post_init__` checks, in its order.
    pub fn validate(&self, methods: &MethodRegistry) -> Result<(), RuleError> {
        if self.rule_set == "china" {
            return value_err(
                "rule_set='china' is not implemented on the size_channel path (it used to run \
                 the PIANC path silently). Use channel_shape(vessel_standard_id=..., \
                 rule_sets=['gb50139'], set_inputs={'range_end': ...}) instead."
                    .into(),
            );
        }
        if !methods.names().contains(&self.rule_set) {
            return value_err(format!(
                "rule_set must be one of {} (register more with \
                 owd.rules.methods.register_width_method)",
                methods.names_repr()
            ));
        }
        for (name, v) in [
            ("beam_m", self.beam_m),
            ("loa_m", self.loa_m),
            ("draft_m", self.draft_m),
        ] {
            if v <= 0.0 {
                return value_err(format!("{name} must be > 0"));
            }
        }
        if !matches!(
            self.pianc_maneuverability.as_str(),
            "good" | "moderate" | "poor"
        ) {
            return value_err(
                "pianc_maneuverability must be one of ('good', 'moderate', 'poor')".into(),
            );
        }
        Ok(())
    }

    /// The options passed to the width method.
    pub fn method_options(&self) -> Options {
        if self.rule_set != "pianc" {
            return self.options.clone();
        }
        let mut extra = Options::new();
        for (k, v) in &self.pianc_extra_widths {
            extra.set(k, OptValue::Float(*v));
        }
        let mut o = Options::new()
            .with("width_multiple", OptValue::Float(self.pianc_width_multiple))
            .with("use_buildup", OptValue::Bool(self.pianc_use_buildup))
            .with(
                "maneuverability",
                OptValue::Str(self.pianc_maneuverability.clone()),
            )
            .with(
                "radius_multiple",
                OptValue::Float(self.pianc_radius_multiple),
            )
            .with("extra_widths", OptValue::Map(extra))
            .with(
                "bank_clearance_outer",
                OptValue::Float(self.pianc_bank_clearance_outer),
            )
            .with(
                "bank_clearance_inner",
                OptValue::Float(self.pianc_bank_clearance_inner),
            );
        o.update(&self.options);
        o
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRuleResult {
    pub rule_set: String,
    pub straight_bottom_width_m: f64,
    /// The rule's nominal minimum bend radius.
    pub min_turn_radius_m: f64,
    pub reference_bend_radius_m: f64,
    /// Required bottom width through the reference bend.
    pub bend_bottom_width_m: f64,
    /// Bend width minus straight width.
    pub bend_widening_m: f64,
    pub required_depth_m: f64,
    pub under_keel_clearance_m: f64,
    pub min_water_level_m: f64,
    /// `min_water_level - required_depth`.
    pub bed_level_m: f64,
    /// Bottom + 2 x slope x depth.
    pub top_width_straight_m: f64,
    pub width_breakdown: Options,
    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
}

impl ChannelRuleResult {
    /// `all(c.ok for c in checks if c.severity != "warning")`. Not the same as
    /// `ovcd_core::checks::all_ok`: any severity but `"warning"` gates here.
    pub fn ok(&self) -> bool {
        self.checks
            .iter()
            .filter(|c| c.severity != "warning")
            .all(|c| c.ok)
    }
}

/// The deflection a single reference bend is assumed to turn, for methods whose bend width
/// depends on it. The built-in "pianc" method ignores it.
pub const REFERENCE_DEFLECTION_DEG: f64 = 45.0;

/// `(straight, r_min, bend, breakdown)` for a method at a reference radius.
fn size(
    m: &WidthMethod,
    inp: &ChannelRuleInputs,
    ref_radius: f64,
) -> Result<(f64, f64, f64, Options), RuleError> {
    let opts = inp.method_options();
    let straight = (m.straight_width)(inp.loa_m, inp.beam_m, &opts)?;
    let r_min = (m.min_radius)(inp.loa_m, &opts)?;
    let bend = py_max(
        straight,
        (m.bend_width)(
            inp.loa_m,
            inp.beam_m,
            &opts,
            ref_radius,
            ref_radius,
            REFERENCE_DEFLECTION_DEG,
        )?,
    );
    let mut breakdown = match &m.breakdown {
        Some(f) => f(inp.loa_m, inp.beam_m, &opts)?,
        None => Options::new(),
    };
    breakdown.set(
        "bend_widening_at_reference_radius",
        OptValue::Float(ovcd_core::pyfmt::round_to(bend - straight, 2)),
    );
    Ok((straight, r_min, bend, breakdown))
}

/// Required bottom width (m) at a bend of the given radius: the single width authority for the
/// alignment solver's width-aware edge cost.
///
/// Straight (radius = inf or <= 0 treated as straight) gives the method's straight width.
/// Otherwise `max(straight, bend_width(R1 = R2 = R, 45 deg))`; for `"pianc"` that is
/// straight + LOA^2/(8R). Monotonic non-increasing in R for every shipped method.
pub fn width_at_radius(
    methods: &MethodRegistry,
    inp: &ChannelRuleInputs,
    radius_m: f64,
) -> Result<f64, RuleError> {
    inp.validate(methods)?;
    let m = methods.get(&inp.rule_set)?;
    width_at_radius_for(&m, inp, radius_m)
}

/// [`width_at_radius`] with the method already looked up.
pub(crate) fn width_at_radius_for(
    m: &WidthMethod,
    inp: &ChannelRuleInputs,
    radius_m: f64,
) -> Result<f64, RuleError> {
    let opts = inp.method_options();
    let straight = (m.straight_width)(inp.loa_m, inp.beam_m, &opts)?;
    if radius_m <= 0.0 || radius_m.is_infinite() {
        return Ok(straight);
    }
    Ok(py_max(
        straight,
        (m.bend_width)(
            inp.loa_m,
            inp.beam_m,
            &opts,
            radius_m,
            radius_m,
            REFERENCE_DEFLECTION_DEG,
        )?,
    ))
}

/// One-lane channel geometry under the selected width method (`inp.rule_set`).
pub fn size_channel(
    methods: &MethodRegistry,
    inp: &ChannelRuleInputs,
) -> Result<ChannelRuleResult, RuleError> {
    inp.validate(methods)?;
    // Python: `inp.reference_bend_radius_m or (3.0 * inp.loa_m)`; None and 0.0 are both falsy.
    let ref_radius = match inp.reference_bend_radius_m {
        Some(r) if r != 0.0 => r,
        _ => 3.0 * inp.loa_m,
    };
    let m = methods.get(&inp.rule_set)?;
    let (straight, r_min, bend, breakdown) = size(&m, inp, ref_radius)?;

    let ukc = under_keel_clearance(inp.draft_m, inp.ukc_fraction, inp.ukc_min_m);
    let depth = required_depth(inp.draft_m, inp.ukc_fraction, inp.ukc_min_m);
    let bed = inp.min_water_level_m - depth;
    let top = straight + 2.0 * inp.side_slope_h_per_v * depth;

    let checks = vec![
        Check::new(
            "straight_width_positive",
            straight > 0.0,
            format!("straight width {} m > 0", fixed(straight, 1)),
        ),
        Check::new(
            "depth_covers_draft",
            depth >= inp.draft_m,
            format!(
                "required depth {} m \u{2265} draft {} m",
                fixed(depth, 2),
                fixed(inp.draft_m, 2)
            ),
        ),
        Check::with_severity(
            "ref_radius_at_or_above_rule_min",
            ref_radius >= r_min - 1e-6,
            format!(
                "reference bend radius {} m vs rule minimum {} m \u{2014} bends below the rule \
                 minimum need widening / a cut-through",
                fixed(ref_radius, 0),
                fixed(r_min, 0)
            ),
            "warning",
        ),
    ];
    let mut warnings = Vec::new();
    if bed < 0.0 {
        warnings.push(format!(
            "bed level {} masl is below datum \u{2014} marine/tidal dredge reach \
             (pool at min WL {} masl)",
            fixed(bed, 2),
            fixed(inp.min_water_level_m, 1)
        ));
    }

    Ok(ChannelRuleResult {
        rule_set: inp.rule_set.clone(),
        straight_bottom_width_m: straight,
        min_turn_radius_m: r_min,
        reference_bend_radius_m: ref_radius,
        bend_bottom_width_m: bend,
        bend_widening_m: bend - straight,
        required_depth_m: depth,
        under_keel_clearance_m: ukc,
        min_water_level_m: inp.min_water_level_m,
        bed_level_m: bed,
        top_width_straight_m: top,
        width_breakdown: breakdown,
        checks,
        warnings,
    })
}
