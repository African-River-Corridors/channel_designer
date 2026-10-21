//! Simple one-call channel sizing: straight width and a single design bend. Mirrors
//! `rules/simple.py`.
//!
//! A thin layer over a width method ([`super::methods`], default `"pianc"`) and [`super::design`]
//! (depth/UKC). For curvature-driven widening along a real centreline use
//! [`super::design::compute`]; for a named standard and vessel class use
//! [`super::rulesets::channel_shape`].

use super::RuleError;
use super::design::{max_draft_for_depth, required_depth};
use super::methods::{MethodRegistry, OptValue, Options};

pub use super::design::{max_draft_for_depth as max_draft_for_depth_m, under_keel_clearance};

#[derive(Debug, Clone, PartialEq)]
pub struct PiancInputs {
    pub beam_m: f64,
    /// Barge length (drives bends; excludes pusher).
    pub loa_m: f64,
    /// Laden draft (0 means: size from a given depth instead).
    pub draft_m: f64,
    /// One-lane bottom width = k_w x beam.
    pub width_multiple_straight: f64,
    /// Min bend radius = k_r x LOA.
    pub radius_multiple: f64,
    /// Under-keel clearance as a fraction of draft.
    pub ukc_fraction: f64,
    /// Minimum absolute UKC.
    pub ukc_min_m: f64,
    pub side_slope_h_per_v: f64,
    /// One passing place every ~2 km.
    pub passing_place_spacing_m: f64,
    /// Passing-place length = k x LOA.
    pub passing_place_length_multiple: f64,
    /// A registered width method (see [`super::methods`]).
    pub width_method: String,
    /// The method's own parameters.
    pub width_options: Options,
}

impl PiancInputs {
    /// The two required fields; everything else takes the Python default.
    pub fn new(beam_m: f64, loa_m: f64) -> Self {
        Self {
            beam_m,
            loa_m,
            draft_m: 0.0,
            width_multiple_straight: 2.5,
            radius_multiple: 4.0,
            ukc_fraction: 0.15,
            ukc_min_m: 0.5,
            side_slope_h_per_v: 3.0,
            passing_place_spacing_m: 2000.0,
            passing_place_length_multiple: 5.0,
            width_method: "pianc".into(),
            width_options: Options::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiancChannel {
    pub bottom_width_straight_m: f64,
    pub bottom_width_bend_m: f64,
    pub bend_widening_m: f64,
    pub min_bend_radius_m: f64,
    /// Depth implied by the given draft (0 if draft not given).
    pub required_depth_m: f64,
    /// Laden draft a given channel depth allows.
    pub max_draft_for_depth_m: f64,
    /// At the design depth (bottom + 2 x slope x depth).
    pub top_width_straight_m: f64,
    pub design_depth_m: f64,
    pub passing_place_spacing_m: f64,
    pub passing_place_length_m: f64,
}

/// Compute one-lane channel geometry for a barge with a width method.
///
/// Default (`"pianc"`): straight bottom width = `width_multiple_straight x beam`; bend width =
/// straight + LOA^2/(8R) at the design radius R = radius_multiple x LOA. For curvature-driven
/// widening along a real surveyed centreline, use [`super::design::compute`].
pub fn size_channel(
    methods: &MethodRegistry,
    inp: &PiancInputs,
    depth_m: Option<f64>,
) -> Result<PiancChannel, RuleError> {
    let method = methods.get(&inp.width_method)?;
    // `{"width_multiple": k, **options}`: the field first, an option of the same name wins.
    let mut opts = Options::new();
    if inp.width_method == "pianc" {
        opts.set(
            "width_multiple",
            OptValue::Float(inp.width_multiple_straight),
        );
    }
    opts.update(&inp.width_options);
    let bottom_straight = (method.straight_width)(inp.loa_m, inp.beam_m, &opts)?;
    let radius = inp.radius_multiple * inp.loa_m;
    let bend = (method.bend_width)(inp.loa_m, inp.beam_m, &opts, radius, radius, 45.0)?;
    let bottom_bend = crate::py_max(bottom_straight, bend);
    let bend_widening = bottom_bend - bottom_straight;

    // `if inp.draft_m`: zero is falsy.
    let req_depth = if inp.draft_m != 0.0 {
        required_depth(inp.draft_m, inp.ukc_fraction, inp.ukc_min_m)
    } else {
        0.0
    };
    let design_depth = depth_m.unwrap_or(req_depth);
    let max_draft = max_draft_for_depth(design_depth, inp.ukc_fraction, inp.ukc_min_m);
    let top_straight = bottom_straight + 2.0 * inp.side_slope_h_per_v * design_depth;

    Ok(PiancChannel {
        bottom_width_straight_m: bottom_straight,
        bottom_width_bend_m: bottom_bend,
        bend_widening_m: bend_widening,
        min_bend_radius_m: radius,
        required_depth_m: req_depth,
        max_draft_for_depth_m: max_draft,
        top_width_straight_m: top_straight,
        design_depth_m: design_depth,
        passing_place_spacing_m: inp.passing_place_spacing_m,
        passing_place_length_m: inp.passing_place_length_multiple * inp.loa_m,
    })
}
