//! Channel-sizing rules: named rulesets (`channel_shape`) and width methods (`size_channel`).
//! Mirrors `rules/rulesets.py`.
//!
//! Two front doors:
//!
//! - [`channel_shape`]: a named standard (PIANC WG 141 qualities A/B/C, the Chinese GB 50139
//!   family) plus a vessel class gives the channel shape. Rulesets are coefficients as data; add
//!   your own with [`RulesetRegistry::register`].
//! - [`size_channel`]: one-lane channel geometry (straight width, min turn radius, bend width,
//!   required depth, bed level against the pool's minimum water level) from a width method. The
//!   crate ships `"pianc"`; another crate can register more, see [`super::methods`].
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
use crate::pyrepr::repr_str;
use crate::{py_max, py_min};
use ovcd_core::checks::Check;
use ovcd_core::pyfmt::{fixed, py_str};
use ovcd_core::vessel_standards::get_standard;
use std::sync::{Arc, Mutex, OnceLock};

// =====================================================================================
// RULESETS: a channel design standard, named. Ruleset x vessel class -> channel shape.
// =====================================================================================
// ONE axis, not two: PIANC's Safety & Ease qualities are part of the PIANC ruleset, because the
// concept does not travel; the Chinese standard has no A/B/C tiers at all. Collapsing them into
// named rulesets avoids inventing a mapping the Chinese standard does not contain.
//
// The VESSEL CLASS (ovcd-core's vessel standards) supplies LOA, beam, draught and formation, so
// a ruleset never needs to ask whether it is sizing for a convoy or a motor vessel: it looks
// that up. Together the two choices fully determine the channel shape.
//
// WHERE THE TWO FAMILIES DISAGREE, which is why both are carried:
//   width  PIANC charges for BEAM only (k_w x B). China charges for beam AND LENGTH, via a
//          drift angle: swept = B + L*sin(beta), plus a bank increment. Long convoys cost more
//          under China; short wide ones cost less.
//   bend   PIANC dF_C = c_C*L^2/R, capped <= L. China dB = L^2/(2R + B) which for R >> B is
//          ~0.5*L^2/R, i.e. PIANC's c_C = 0.5. Two independent traditions on the same
//          coefficient: strong evidence c_C ~ 0.5 is the floor, and that the LOA^2/(8R) the
//          "pianc" width method uses (c_C = 0.125) never was.
//   depth  PIANC sets a RATIO (1.3 x T). China sets draught PLUS A FIXED CLEARANCE. They
//          diverge with draught: at T = 4.5 m China is ~16% shallower.

/// One named channel-design standard.
#[derive(Debug, Clone, PartialEq)]
pub struct Ruleset {
    pub ruleset_id: String,
    /// `"pianc"` or `"china"`.
    pub family: String,
    pub label: String,
    pub source: String,
    /// R_min = mult x LOA, pushed convoy.
    pub r_mult_convoy: f64,
    /// R_min = mult x LOA, self-propelled.
    pub r_mult_motor: f64,
    /// pianc: bottom width = width_mult x beam.
    pub width_mult: f64,
    /// pianc: bend coefficient.
    pub c_c: f64,
    /// pianc: depth = depth_mult x draught.
    pub depth_mult: f64,
    /// china: bank increment, fraction of swept path.
    pub bank_mult_convoy: f64,
    pub bank_mult_motor: f64,
    /// china: depth = draught + this.
    pub depth_add_m: f64,
    pub notes: String,
}

impl Ruleset {
    /// A ruleset with the Python defaults: every coefficient 0, no notes.
    pub fn new(
        ruleset_id: &str,
        family: &str,
        label: &str,
        source: &str,
        r_mult_convoy: f64,
        r_mult_motor: f64,
    ) -> Self {
        Self {
            ruleset_id: ruleset_id.into(),
            family: family.into(),
            label: label.into(),
            source: source.into(),
            r_mult_convoy,
            r_mult_motor,
            width_mult: 0.0,
            c_c: 0.0,
            depth_mult: 0.0,
            bank_mult_convoy: 0.0,
            bank_mult_motor: 0.0,
            depth_add_m: 0.0,
            notes: String::new(),
        }
    }
}

impl Ruleset {
    /// Refuse a ruleset that gives no bend-widened width (spec 0005 R2-Q3): the water-aware
    /// checks test each station at its measured local radius, so the ruleset must widen a bend.
    /// A `"china"` ruleset always does (dB = L^2/(2R + B)); a `"pianc"` one needs `c_c > 0`.
    /// `register` does not call this, so a ruleset registered before spec 0005 still loads.
    pub fn validate(&self) -> Result<(), RuleError> {
        if self.family == "pianc" && !(self.c_c.is_finite() && self.c_c > 0.0) {
            return value_err(format!(
                "ruleset {} gives no bend-widened width (c_c = {}); the water-aware width \
                 check needs one",
                repr_str(&self.ruleset_id),
                self.c_c
            ));
        }
        Ok(())
    }
}

// PIANC WG 141 (2019), free-flowing river, single-lane.
//   width  Table 5.7 (p109). Table 5.7 prints 2.8*B for quality C but carries a 3.0*B minimum
//          "for security reasons" spanning C and B, so C is 3.0, not 2.8.
//   c_c    Table 5.7 fn.3 + Table E.5 (p261). A is read from Table E.5 by VESSEL CLASS; 0.6 is
//          Class VIa loaded downstream (short, wide convoys). B follows from A by the
//          bow-thruster reduction in the Table E.5 remarks. C is the field-data floor and the
//          Figure 5.5 single-lane gradient.
//   depth  5.3.4.3 (p108): 1.3 x draught, never below 1.2.
//   R_min  Table 5.7 (p109): 2L / 3L / 4L.
// Chinese GB 50139, as summarised by WG 141 Appendix A.3 (p162-164).
//   width  A.3.2/A.3.3: swept = B + L*sin(beta); bank increment 0.25-0.30 of swept for convoys,
//          0.34-0.40 for motor vessels. CN uses the permissive end, CN-conservative the strict
//          end: the standard states a range and does not tier it.
//   depth  A.3.2: draught + 0.4-0.5 m (class I) or 0.3-0.4 m (classes II-III).
//   R_min  A.3.2: 3L pushed fleet / 4L cargo vessel, reducible to 2L / 3L where the extra width
//          and driving visibility allow. CN-reduced is that reduction.

/// The built-in rulesets, in the Python dict's order.
pub fn builtin_rulesets() -> Vec<Ruleset> {
    let mut a = Ruleset::new(
        "PIANC-A",
        "pianc",
        "PIANC WG 141 \u{2014} Safety & Ease quality A",
        "WG 141 Table 5.7 (p109), Table E.5 (p261)",
        4.0,
        4.0,
    );
    a.width_mult = 3.4;
    a.c_c = 0.6;
    a.depth_mult = 1.3;
    a.notes = "Highest ease. c_C from Table E.5 for Class VIa laden downstream.".into();

    let mut b = Ruleset::new(
        "PIANC-B",
        "pianc",
        "PIANC WG 141 \u{2014} Safety & Ease quality B",
        "WG 141 Table 5.7 (p109)",
        3.0,
        3.0,
    );
    b.width_mult = 3.2;
    b.c_c = 0.5;
    b.depth_mult = 1.3;
    b.notes = "c_C reduced from A by the Table E.5 bow-thruster allowance.".into();

    let mut c = Ruleset::new(
        "PIANC-C",
        "pianc",
        "PIANC WG 141 \u{2014} Safety & Ease quality C",
        "WG 141 Table 5.7 (p109) incl. the 3.0*B security minimum",
        2.0,
        2.0,
    );
    c.width_mult = 3.0;
    c.c_c = 0.5;
    c.depth_mult = 1.2;
    c.notes = "Lowest ease. Restricted driving; skilled pilots assumed.".into();

    let mut cn = Ruleset::new(
        "CN",
        "china",
        "Chinese GB 50139 \u{2014} standard",
        "WG 141 Appendix A.3 (p162-164)",
        3.0,
        4.0,
    );
    cn.bank_mult_convoy = 0.25;
    cn.bank_mult_motor = 0.34;
    cn.depth_add_m = 0.3;
    cn.notes = "Permissive end of every stated range.".into();

    let mut cons = Ruleset::new(
        "CN-conservative",
        "china",
        "Chinese GB 50139 \u{2014} conservative",
        "WG 141 Appendix A.3 (p162-164)",
        3.0,
        4.0,
    );
    cons.bank_mult_convoy = 0.30;
    cons.bank_mult_motor = 0.40;
    cons.depth_add_m = 0.5;
    cons.notes = "Strict end of every stated range.".into();

    let mut red = Ruleset::new(
        "CN-reduced",
        "china",
        "Chinese GB 50139 \u{2014} reduced radius",
        "WG 141 Appendix A.3 (p162-164)",
        2.0,
        3.0,
    );
    red.bank_mult_convoy = 0.30;
    red.bank_mult_motor = 0.40;
    red.depth_add_m = 0.5;
    red.notes = "A USER CHOICE built on GB 50139, not a named Chinese ruleset. \
A.3.2 (p164) permits the radius to drop to 2L convoy / 3L \
motor where the extra width and driving visibility allow; this \
ruleset pairs that with the STRICT end of the width and depth \
ranges. The clause is written for CANAL sections. On a \
canalised river only the cut-throughs are canal, so this \
ruleset may apply to PART of an alignment only. Do not quote a \
whole-river total under it without saying so."
        .into();
    vec![a, b, c, cn, cons, red]
}

/// PIANC (2019) Report No. 141, Design Guidelines for Inland Waterways, Table A.2 (p163): the
/// Chinese standard's own single-lane river widths, which include bank clearance and wind.
/// ONLY the entries the code reads are kept: those the vessel classes in ovcd-core match
/// (`cn_table_width`). Key (LOA m, beam m) -> width m. For any other vessel the width falls back
/// to the Table 5.1 ratio below.
pub const CN_TABLE_A2: [(f64, f64, f64); 3] = [
    (223.0, 32.4, 70.0),
    (186.0, 32.4, 70.0),
    (167.0, 21.6, 45.0),
];

// The A.3.2 swept-path formula under-predicts the Table A.2 widths (by roughly a third, measured
// against the published table). A.3.3 says the formula carries bank and passing increments only,
// so wind is not in it. That gap is why the width basis is a published wind-included ratio, not
// the formula.
//
// The width basis is WG 141 Table 5.1 (p88), row "China River", single-lane W_F/B = 2.3.
// Why this and not the calibrated formula:
//   * it is the STANDARD'S OWN published ratio, not a factor we derived;
//   * Table 5.1's footnote states in WG 141's words that the "Chinese, German and US profile:
//     wind included", which is exactly the gap A.3.3 leaves open (it lists curvature, bank
//     interaction, cross-flow and passing distance, not wind);
//   * it agrees closely with a formula-times-calibration estimate, so it is a change of BASIS,
//     not of number.
// CAUTIONS, both real, both to be quoted with the number:
//   * Table 5.1 is captioned "Canal fairway dimension in existing guidelines"; the China River
//     row is a river row inside a canal comparison. WG 141 prints it there; we did not move it.
//   * W_F is defined (5.2.1, p87) as the net bank-to-bank distance measured AT DESIGN DRAUGHT
//     DEPTH, not at the bed. We apply it as a BOTTOM width, which on a sloped section is the
//     conservative reading (more excavation).
pub const CN_TABLE_5_1_WIDTH_MULT: f64 = 2.3;

/// Chinese depth clearance, WG 141 A.3.2 (p162): depth = draught + a fixed clearance, given as a
/// RANGE per class (class I 0.4-0.5 m; classes II-III 0.3-0.4 m). Classes IV and up are NOT
/// covered by the clause; this module does not invent a clearance for them. Class ->
/// (permissive end, strict end).
pub const CN_DEPTH_CLEARANCE_M: [(&str, (f64, f64)); 3] =
    [("I", (0.4, 0.5)), ("II", (0.3, 0.4)), ("III", (0.3, 0.4))];

/// Which end of the clearance range to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearanceEnd {
    Permissive,
    Strict,
}

impl ClearanceEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            ClearanceEnd::Permissive => "permissive",
            ClearanceEnd::Strict => "strict",
        }
    }
}

/// Default: the STRICT end. GB 50139's clearance is an ABSOLUTE add-on calibrated on a class II
/// design draught of 2.6 m (Table A.2); at deeper draughts a fixed add-on is a shrinking ratio,
/// so the strict end keeps the design inside the standard while acknowledging that. A user
/// choice: set `Permissive` to take the low end.
pub const CN_DEPTH_CLEARANCE_END: ClearanceEnd = ClearanceEnd::Strict;

/// A.3.2 drift angle building the Chinese swept path: 3 deg classes I-V, 2 deg VI-VII.
const CN_DRIFT_DEG_LARGE_BEAM: f64 = 2.0;
const CN_DRIFT_DEG_SMALL_BEAM: f64 = 3.0;
const CN_LARGE_BEAM_M: f64 = 22.8;

/// `math.radians`.
fn radians(deg: f64) -> f64 {
    deg * (core::f64::consts::PI / 180.0)
}

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

/// The named rulesets for `channel_shape`, in insertion order (a Python dict). A new registry
/// holds the built-in six.
#[derive(Debug, Clone)]
pub struct RulesetRegistry {
    rulesets: Vec<Ruleset>,
}

impl Default for RulesetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RulesetRegistry {
    pub fn new() -> Self {
        Self {
            rulesets: builtin_rulesets(),
        }
    }

    /// Add a named ruleset for `channel_shape`.
    ///
    /// A ruleset is coefficients as data. `family` must be `"pianc"` (width = width_mult x beam,
    /// bend dF_C = c_c x L^2/R, depth = depth_mult x draught) or `"china"` (swept path plus bank
    /// increment, dB = L^2/(2R + B), depth = draught + depth_add_m). A method that is not
    /// coefficients belongs in [`super::methods`] instead.
    pub fn register(&mut self, ruleset: Ruleset, replace: bool) -> Result<(), RuleError> {
        if ruleset.family != "pianc" && ruleset.family != "china" {
            return value_err(format!(
                "ruleset family must be 'pianc' or 'china'; got {}",
                repr_str(&ruleset.family)
            ));
        }
        let existing = self
            .rulesets
            .iter()
            .position(|r| r.ruleset_id == ruleset.ruleset_id);
        match existing {
            Some(_) if !replace => value_err(format!(
                "ruleset {} is already registered; pass replace=True to override it",
                repr_str(&ruleset.ruleset_id)
            )),
            Some(i) => {
                self.rulesets[i] = ruleset;
                Ok(())
            }
            None => {
                self.rulesets.push(ruleset);
                Ok(())
            }
        }
    }

    pub fn get(&self, ruleset_id: &str) -> Result<&Ruleset, RuleError> {
        match self.rulesets.iter().find(|r| r.ruleset_id == ruleset_id) {
            Some(r) => Ok(r),
            None => {
                let mut known: Vec<&str> = self
                    .rulesets
                    .iter()
                    .map(|r| r.ruleset_id.as_str())
                    .collect();
                known.sort_unstable();
                Err(RuleError::Key(format!(
                    "unknown ruleset {}; known: {}",
                    repr_str(ruleset_id),
                    known.join(", ")
                )))
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Ruleset> {
        self.rulesets.iter()
    }
}

static GLOBAL: OnceLock<Mutex<RulesetRegistry>> = OnceLock::new();

/// The process-wide registry: Python's module-level `RULESETS`.
pub fn global_rulesets() -> &'static Mutex<RulesetRegistry> {
    GLOBAL.get_or_init(|| Mutex::new(RulesetRegistry::new()))
}

/// The channel a (ruleset, vessel class) pair requires. Everything here is derived.
///
/// Unlike the Python, which looks the ruleset up again by id on every `bend_widening_m` call,
/// the shape keeps the coefficients it was built from, so a later `replace` of the ruleset does
/// not change a shape that already exists.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelShape {
    pub ruleset_id: String,
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
    ruleset: Ruleset,
    /// The engine selection that sized the shape, and its vessel (spec 0007). `None` for a ruleset
    /// that does not go through the engine yet.
    engine: Option<(Arc<Selection>, Vessel)>,
}

impl ChannelShape {
    /// The ruleset the shape was built from.
    pub fn ruleset(&self) -> &Ruleset {
        &self.ruleset
    }

    pub fn bend_widening_m(&self, radius_m: f64) -> f64 {
        match &self.engine {
            Some((selection, vessel)) => engine::widening(selection, vessel, radius_m)
                .expect("the selection sized the bend widening at R_min, so it sizes it here"),
            None => bend_widening(&self.ruleset, self.loa_m, self.beam_m, radius_m),
        }
    }

    pub fn bottom_width_at_m(&self, radius_m: f64) -> f64 {
        self.bottom_width_straight_m + self.bend_widening_m(radius_m)
    }
}

fn bend_widening(rs: &Ruleset, loa_m: f64, beam_m: f64, radius_m: f64) -> f64 {
    if radius_m <= 0.0 || radius_m.is_infinite() {
        return 0.0;
    }
    if rs.family == "china" {
        return loa_m * loa_m / (2.0 * radius_m + beam_m); // A.3.3 (p164)
    }
    py_min(rs.c_c * loa_m * loa_m / radius_m, loa_m) // 5.3.3 (p101), cap dF_C <= L
}

/// Nearest Table A.2 entry, if the vessel plausibly matches one. Else `None`.
fn cn_table_width(loa_m: f64, beam_m: f64) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None; // (width, distance)
    for (l, b, w) in CN_TABLE_A2 {
        let d = (l - loa_m).abs() / py_max(l, 1.0) + (b - beam_m).abs() / py_max(b, 1.0);
        match best {
            Some((_, bd)) if !matches!(d.partial_cmp(&bd), Some(core::cmp::Ordering::Less)) => {}
            _ => best = Some((w, d)),
        }
    }
    // only trust it as a direct read when both dimensions are within 10%
    match best {
        Some((w, d)) if d <= 0.20 => Some(w),
        _ => None,
    }
}

/// Returns (width_m, basis, warnings).
fn straight_width(
    rs: &Ruleset,
    loa_m: f64,
    beam_m: f64,
    formation: &str,
) -> (f64, &'static str, Vec<String>) {
    let mut warns = Vec::new();
    if rs.family == "china" {
        let w_tab = cn_table_width(loa_m, beam_m);
        let beta = radians(if beam_m >= CN_LARGE_BEAM_M {
            CN_DRIFT_DEG_LARGE_BEAM
        } else {
            CN_DRIFT_DEG_SMALL_BEAM
        });
        let swept = beam_m + loa_m * libm::sin(beta);
        let bank = if formation == "motor_vessel" {
            rs.bank_mult_motor
        } else {
            rs.bank_mult_convoy
        };
        let w_formula = swept * (1.0 + bank);
        if let Some(w_tab) = w_tab {
            if (w_tab - w_formula).abs() / w_tab > 0.05 {
                warns.push(format!(
                    "Table A.2 gives {} m where the A.3.2 formula gives {} m; the table \
                     includes wind, the formula does not. Using the table.",
                    fixed(w_tab, 0),
                    fixed(w_formula, 1)
                ));
            }
            return (w_tab, "table", warns);
        }
        let w = CN_TABLE_5_1_WIDTH_MULT * beam_m;
        warns.push(format!(
            "no Table A.2 entry near LOA {} m / beam {} m, so the width is WG 141 Table 5.1's \
             China-River single-lane ratio W_F/B = {} ({} m), which the table's own footnote \
             records as WIND-INCLUDED. The A.3.2 formula gives {} m and omits wind (A.3.3 lists \
             curvature, bank interaction, cross-flow and passing distance only); the implied \
             ratio between them is {}.",
            fixed(loa_m, 0),
            fixed(beam_m, 1),
            py_str(CN_TABLE_5_1_WIDTH_MULT),
            fixed(w, 1),
            fixed(w_formula, 1),
            fixed(w / w_formula, 2)
        ));
        return (w, "table-5.1-ratio", warns);
    }
    (rs.width_mult * beam_m, "rule", warns)
}

/// THE FRONT DOOR: a ruleset plus a vessel class fully determine the channel.
///
/// `draught_m` overrides the vessel class's nominal draught: a project's design draught is its
/// own decision, so it stays injectable.
pub fn channel_shape(
    rulesets: &RulesetRegistry,
    ruleset_id: &str,
    vessel_standard_id: &str,
    draught_m: Option<f64>,
) -> Result<ChannelShape, RuleError> {
    let mut shape = channel_shape_legacy(rulesets, ruleset_id, vessel_standard_id, draught_m)?;
    let Some(selection) = engine_selection(&shape.ruleset)? else {
        return Ok(shape);
    };
    let class = get_standard(vessel_standard_id).map_err(RuleError::Key)?;
    let vessel = Vessel {
        loa_m: shape.loa_m,
        beam_m: shape.beam_m,
        draught_m: shape.draught_m,
        formation: shape.formation.clone(),
        class_name: class.class_name,
    };
    let sized = engine::size(&selection, &vessel)?;
    shape.bottom_width_straight_m = sized.bottom_width_straight_m;
    shape.min_bend_radius_m = sized.min_bend_radius_m;
    shape.design_depth_m = sized.design_depth_m;
    shape.bend_widening_at_rmin_m = sized.bend_widening_at_rmin_m;
    shape.bottom_width_at_rmin_m = sized.bottom_width_straight_m + sized.bend_widening_at_rmin_m;
    shape.engine = Some((
        Arc::new(selection.narrowed_to(engine::BEND_WIDENING, &vessel)),
        vessel,
    ));
    Ok(shape)
}

/// The engine selection an old ruleset maps to (spec 0007, "Rule Sets at 0.3.0"), or `None` for one
/// that does not go through the engine yet. Only an unchanged built-in ruleset maps: one replaced
/// through [`RulesetRegistry::register`] keeps the old path.
fn engine_selection(rs: &Ruleset) -> Result<Option<Selection>, RuleError> {
    let (set, input, value) = match rs.ruleset_id.as_str() {
        "PIANC-A" => ("pianc-wg141", engine::SE_QUALITY, "A"),
        "PIANC-B" => ("pianc-wg141", engine::SE_QUALITY, "B"),
        "PIANC-C" => ("pianc-wg141", engine::SE_QUALITY, "C"),
        "CN" => ("gb50139", engine::RANGE_END, "permissive"),
        "CN-conservative" => ("gb50139", engine::RANGE_END, "strict"),
        _ => return Ok(None),
    };
    if !builtin_rulesets().contains(rs) {
        return Ok(None);
    }
    Selection::built_in(&[set], &[(input, value)]).map(Some)
}

/// The channel shape by the old `Ruleset` code: the parity reference for the engine path
/// (spec 0007), kept for that check only until the old path goes.
#[doc(hidden)]
pub fn channel_shape_legacy(
    rulesets: &RulesetRegistry,
    ruleset_id: &str,
    vessel_standard_id: &str,
    draught_m: Option<f64>,
) -> Result<ChannelShape, RuleError> {
    let rs = rulesets.get(ruleset_id)?;
    let v = get_standard(vessel_standard_id).map_err(RuleError::Key)?;
    let loa = v.loa_m.1; // size on the LONG end of the class
    let t = draught_m.unwrap_or_else(|| v.draught_nominal_m());
    let motor = v.formation == "motor_vessel";
    let r_mult = if motor {
        rs.r_mult_motor
    } else {
        rs.r_mult_convoy
    };
    let r_min = r_mult * loa;
    let (width, basis, mut warns) = straight_width(rs, loa, v.beam_m, &v.formation);
    let depth = if rs.family == "china" {
        let range = CN_DEPTH_CLEARANCE_M
            .iter()
            .find(|(c, _)| *c == v.class_name)
            .map(|(_, r)| *r);
        let clearance = match range {
            None => {
                // A.3.2 covers classes I-III only. Do not invent a clearance for IV+.
                let c = rs.depth_add_m;
                warns.push(format!(
                    "GB 50139's depth clause (A.3.2, p162) covers classes I-III only; class {} \
                     is outside it, so the depth falls back to the ruleset's {} m add-on, which \
                     is a user choice, not the standard's.",
                    v.class_name,
                    py_str(c)
                ));
                c
            }
            Some(rng) => {
                let c = match CN_DEPTH_CLEARANCE_END {
                    ClearanceEnd::Strict => rng.1,
                    ClearanceEnd::Permissive => rng.0,
                };
                warns.push(format!(
                    "depth = draught + {} m, the {} end of GB 50139's class-{} range {}-{} m \
                     (WG 141 A.3.2, p162, as summarised \u{2014} we do not hold GB 50139 itself).",
                    py_str(c),
                    CN_DEPTH_CLEARANCE_END.as_str(),
                    v.class_name,
                    py_str(rng.0),
                    py_str(rng.1)
                ));
                c
            }
        };
        t + clearance
    } else {
        py_max(rs.depth_mult * t, 1.2 * t)
    };
    // No project floor is applied here: the depth is the ruleset's. Where it falls below PIANC's
    // river minimum ratio of 1.2 x T (WG 141 5.3.4.3, p108) the result says so, but the depth is
    // not changed: a warning, not a hidden correction.
    if depth < 1.2 * t {
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
    let d_w = bend_widening(rs, loa, v.beam_m, r_min);
    Ok(ChannelShape {
        ruleset_id: rs.ruleset_id.clone(),
        vessel_standard_id: v.standard_id.clone(),
        loa_m: loa,
        beam_m: v.beam_m,
        draught_m: t,
        formation: v.formation.clone(),
        bottom_width_straight_m: width,
        min_bend_radius_m: r_min,
        design_depth_m: depth,
        bend_widening_at_rmin_m: d_w,
        bottom_width_at_rmin_m: width + d_w,
        width_basis: basis.into(),
        source: format!("{}; vessel {}", rs.source, v.source),
        warnings: warns,
        ruleset: rs.clone(),
        engine: None,
    })
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
    // the rulesets above. Prefer `channel_shape` with a named ruleset for new work. The Report-121
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
                 the PIANC path silently). Use channel_shape(ruleset_id='CN' | \
                 'CN-conservative' | 'CN-reduced', vessel_standard_id=...) instead."
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
