//! Thin bridge: channel bottom width as a function of bend radius, sourced from the single width
//! authority ([`super::rulesets::width_at_radius`]). Mirrors `rules/width_profile.py`.
//!
//! No width math is duplicated here: this module only wires inputs through and returns a value
//! the alignment solver can use as a width-aware edge cost.

use super::RuleError;
use super::methods::{MethodRegistry, WidthMethod};
use super::rulesets::{ChannelRuleInputs, width_at_radius_for};
use std::sync::Arc;

/// `width(radius_m) -> bottom width (m)`, delegating to `width_at_radius`. Also gives
/// `half_width` (= width/2, the `hb` the FaithfulScorer template uses).
#[derive(Debug, Clone)]
pub struct WidthFn {
    inputs: ChannelRuleInputs,
    method: Arc<WidthMethod>,
}

impl WidthFn {
    /// Check `inputs` against `methods` and look the width method up once. Python checks the
    /// inputs when it builds the callable, so the same refusals come here.
    pub fn new(methods: &MethodRegistry, inputs: ChannelRuleInputs) -> Result<Self, RuleError> {
        inputs.validate(methods)?;
        let method = methods.get(&inputs.rule_set)?;
        Ok(Self { inputs, method })
    }

    pub fn width(&self, radius_m: f64) -> Result<f64, RuleError> {
        width_at_radius_for(&self.method, &self.inputs, radius_m)
    }

    pub fn half_width(&self, radius_m: f64) -> Result<f64, RuleError> {
        Ok(self.width(radius_m)? / 2.0)
    }
}

/// `make_width_fn`. Callers who want to set more than the vessel (`pianc_use_buildup`,
/// `options`, ...) build a [`ChannelRuleInputs`] themselves and use [`WidthFn::new`]. As in
/// Python, the pool level is not used by the width and is left at 0.0 unless the caller sets it.
pub fn make_width_fn(
    methods: &MethodRegistry,
    loa_m: f64,
    beam_m: f64,
    draft_m: f64,
    rule_set: &str,
) -> Result<WidthFn, RuleError> {
    let mut inputs = ChannelRuleInputs::new(beam_m, loa_m, draft_m, 0.0);
    inputs.rule_set = rule_set.into();
    WidthFn::new(methods, inputs)
}

/// Provenance for artifact metadata (`alignment_key.width_params`).
#[derive(Debug, Clone, PartialEq)]
pub struct WidthParams {
    pub rule_set: String,
    pub straight_bottom_m: f64,
    pub widening_coeff_note: String,
    pub loa_m: f64,
    pub beam_m: f64,
    pub draft_m: f64,
}

/// `width_params_dict`.
pub fn width_params(
    methods: &MethodRegistry,
    inputs: ChannelRuleInputs,
) -> Result<WidthParams, RuleError> {
    let f = WidthFn::new(methods, inputs)?;
    let straight_bottom_m = f.width(f64::INFINITY)?;
    Ok(WidthParams {
        rule_set: f.inputs.rule_set.clone(),
        straight_bottom_m,
        widening_coeff_note: f.method.description.clone(),
        loa_m: f.inputs.loa_m,
        beam_m: f.inputs.beam_m,
        draft_m: f.inputs.draft_m,
    })
}
