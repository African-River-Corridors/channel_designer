//! Channel rules on the shared rules engine, `arc-rules` (spec 0007, ADR 0008).
//!
//! OWD declares its domain to the engine: the `channel` object, its Measures, the inputs a Rule may
//! apply on, and its formula shapes. The Rule Sets are data (`rules/<id>/<version>.json`), built into
//! every binary and loaded through [`arc_rules::load::load`], the same path a user's file takes.
//!
//! OWD adds no comparison, strictness or trace logic of its own. To size a channel it evaluates the
//! selected sets with [`arc_rules::eval::evaluate`] and reads, for each sized Measure, the limit of the
//! row the engine marks governing.

use super::{RuleError, value_err};
use crate::{py_max, py_min};
use arc_rules::domain::{
    Domain, FormulaShape, InputDef, InputScope, MeasureInfo, MethodStatus, ValueKind, coefficient,
};
use arc_rules::eval::{MeasureSource, MeasureValue, Trace, applies, evaluate};
use arc_rules::load::{Library, load};
use arc_rules::model::{Limit, MeasureRef, Operand, RuleSet};
use std::sync::{Arc, OnceLock};

/// The object type of the design itself: one channel.
pub const CHANNEL: &str = "channel";
/// A reach of the river, by chainage on its centreline: it carries `waterway` and
/// `reduced_radius_claim` (spec 0007 T4).
pub const REACH: &str = "reach";
/// A station of a design line option. It inherits the inputs of the reach its projection onto the
/// river centreline falls in.
pub const STATION: &str = "station";

/// Bottom width on a straight (m). Sized.
pub const BOTTOM_WIDTH: &str = "channel.bottom-width";
/// The radius of the design line at a station (m). The radius Rules size the channel's minimum
/// bend radius from it, reach by reach.
pub const MIN_BEND_RADIUS: &str = "station.radius";
/// Design depth (m). Sized.
pub const DESIGN_DEPTH: &str = "channel.design-depth";
/// Bend widening at the radius `channel.bend-radius` (m). Sized.
pub const BEND_WIDENING: &str = "channel.bend-widening";
/// The radius a bend widening is read at (m).
pub const BEND_RADIUS: &str = "channel.bend-radius";
/// The vessel's length overall, the long end of its class (m).
pub const LOA: &str = "vessel.loa";
/// The vessel's beam (m).
pub const BEAM: &str = "vessel.beam";
/// The design draught (m).
pub const DRAUGHT: &str = "vessel.draught";
/// The GB 50139 channel class and how it was found (text), such as `II, by tonnage: CEMT IV up to
/// 1500 t`. A `note` Rule shows it in the trace.
pub const GB_CLASS_FOUND: &str = "vessel.gb-class";
/// The vessel as GB 50139-2014 Table 3.0.2-1 lists a typical vessel: its channel class, LOA and beam
/// to the 0.1 m the table prints, such as `IV 67.5x10.8` (text). The table rows are keyed by it.
pub const GB_TYPICAL: &str = "vessel.gb-typical";

/// The Measures, in listing order: id, what it is, and whether a Rule sizes it.
pub const MEASURES: &[(&str, &str)] = &[
    (BOTTOM_WIDTH, "bottom width of the channel on a straight"),
    (
        MIN_BEND_RADIUS,
        "radius of the design line at a station (sizes the minimum bend radius)",
    ),
    (DESIGN_DEPTH, "design depth below the design water level"),
    (
        BEND_WIDENING,
        "bend widening at the radius channel.bend-radius",
    ),
    (BEND_RADIUS, "the radius a bend widening is read at"),
    (LOA, "vessel length overall, long end of its class"),
    (BEAM, "vessel beam"),
    (DRAUGHT, "design draught"),
    (
        GB_TYPICAL,
        "the vessel as a GB 50139 Table 3.0.2-1 key: channel class, LOA x beam (text)",
    ),
    (
        GB_CLASS_FOUND,
        "the GB 50139 channel class and how it was found (text)",
    ),
];

/// The Safety and Ease quality of PIANC WG 141 (A, B or C), one value for the design.
pub const SE_QUALITY: &str = "se_quality";
/// The end of each range GB 50139 states: `permissive` or `strict`.
pub const RANGE_END: &str = "range_end";
/// The vessel's formation: `motor_vessel` or `pushed_convoy`. From the vessel.
pub const FORMATION: &str = "formation";
/// The vessel's class (`I` to `VII`, as the vessel classes name it). From the vessel.
pub const VESSEL_CLASS: &str = "vessel_class";
/// The GB 50139-2014 channel class (Table 3.0.1, by tonnage). Typed, it overrides the class found
/// from the vessel (see [`gb_class_found`]). CEMT and Chinese classes share names, so a CEMT class
/// never stands in for it by name.
pub const GB_CLASS: &str = "gb_class";
/// Whether the vessel is a typical vessel of GB 50139-2014 Table 3.0.2-1 (`typical`: its key has
/// a row in a selected set's table) or a special design vessel (`special`, 3.0.7). From the vessel
/// and the selected sets; never typed.
pub const GB_VESSEL: &str = "gb_vessel";
/// The `gb_class` of a vessel that has none.
pub const GB_CLASS_UNKNOWN: &str = "unknown";

/// The kind of waterway a reach is: `canal`, `impounded-river` or `free-flowing-river`. One value
/// per station, from its reach.
pub const WATERWAY: &str = "waterway";
/// Whether the user claims a reduced radius on a reach (`yes` or `no`). One value per station, from
/// its reach. GB 50139-2014 removed the 2004 edition's reduced radius (3.0.5 and its explanation), so
/// no built-in Rule reads it; a user's Rule Set may.
pub const REDUCED_RADIUS_CLAIM: &str = "reduced_radius_claim";

/// The inputs a Rule may apply on, with their values. `waterway` and `reduced_radius_claim` have one
/// value per station; the others one for the design.
pub const INPUTS: &[(&str, &[&str])] = &[
    (SE_QUALITY, &["A", "B", "C"]),
    (RANGE_END, &["permissive", "strict"]),
    (FORMATION, &["motor_vessel", "pushed_convoy"]),
    (
        VESSEL_CLASS,
        &[
            "I", "II", "III", "IV", "Va", "Vb", "VIa", "VIb", "VIc", "VII",
        ],
    ),
    (
        GB_CLASS,
        &["I", "II", "III", "IV", "V", "VI", "VII", GB_CLASS_UNKNOWN],
    ),
    (GB_VESSEL, &["typical", "special"]),
    (
        WATERWAY,
        &["canal", "impounded-river", "free-flowing-river"],
    ),
    (REDUCED_RADIUS_CLAIM, &["yes", "no"]),
];

/// `k × x`.
fn multiple(
    shape: &str,
    c: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    Ok(coefficient(shape, c, "k")? * op("x")?)
}

/// `max(k × x, floor × x)`, Python's `max` (the first unless the second is larger).
fn ratio_with_floor(
    shape: &str,
    c: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    let x = op("x")?;
    Ok(py_max(
        coefficient(shape, c, "k")? * x,
        coefficient(shape, c, "floor")? * x,
    ))
}

/// `min(c × L² / R, L)` (WG 141 5.3.3, the widening capped at L); none on a straight.
fn capped_widening(
    shape: &str,
    c: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    let (k, l, r) = (coefficient(shape, c, "c")?, op("length")?, op("radius")?);
    if r <= 0.0 || r.is_infinite() {
        return Ok(0.0);
    }
    Ok(py_min(k * l * l / r, l))
}

/// `x + clearance` (GB 50139 depth: draught plus a fixed clearance).
fn plus_clearance(
    shape: &str,
    c: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    Ok(op("x")? + coefficient(shape, c, "clearance")?)
}

/// `L² / (2R + B)` (WG 141 A.3.3, the GB 50139 bend widening); none on a straight.
fn gb_widening(
    _: &str,
    _: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    let (l, r, b) = (op("length")?, op("radius")?, op("beam")?);
    if r <= 0.0 || r.is_infinite() {
        return Ok(0.0);
    }
    Ok(l * l / (2.0 * r + b))
}

/// The GB 50139-2014 single-line width of a special design vessel (A.0.2): the swept path
/// `BF = B + L sin beta` plus a clearance `d = s x BF` on each side, `BF + 2d`.
fn gb_swept_width(
    shape: &str,
    c: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    let (loa, beam) = (op("loa")?, op("beam")?);
    let beta = coefficient(shape, c, "beta_deg")? * (core::f64::consts::PI / 180.0);
    let swept = beam + loa * libm::sin(beta);
    Ok(swept + 2.0 * (coefficient(shape, c, "s")? * swept))
}

/// The formula shapes OWD registers. Each keeps the order of operations of the code it replaced,
/// so no bit of a result moves.
pub const SHAPES: &[FormulaShape] = &[
    FormulaShape {
        id: "multiple",
        operands: &["x"],
        coefficients: &["k"],
        free_unit_operands: &[],
        value: multiple,
    },
    FormulaShape {
        id: "ratio-with-floor",
        operands: &["x"],
        coefficients: &["floor", "k"],
        free_unit_operands: &[],
        value: ratio_with_floor,
    },
    FormulaShape {
        id: "capped-widening",
        operands: &["length", "radius"],
        coefficients: &["c"],
        free_unit_operands: &[],
        value: capped_widening,
    },
    FormulaShape {
        id: "plus-clearance",
        operands: &["x"],
        coefficients: &["clearance"],
        free_unit_operands: &[],
        value: plus_clearance,
    },
    FormulaShape {
        id: "gb-widening",
        operands: &["beam", "length", "radius"],
        coefficients: &[],
        free_unit_operands: &[],
        value: gb_widening,
    },
    FormulaShape {
        id: "gb-swept-width",
        operands: &["beam", "loa"],
        coefficients: &["beta_deg", "s"],
        free_unit_operands: &[],
        value: gb_swept_width,
    },
];

/// OWD's vocabulary, for the loader.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChannelDomain;

impl Domain for ChannelDomain {
    fn measure(&self, id: &str) -> Option<MeasureInfo> {
        MEASURES.iter().any(|(m, _)| *m == id).then(|| MeasureInfo {
            unit: "m".into(),
            object: if id.starts_with("station.") {
                STATION
            } else {
                CHANNEL
            }
            .into(),
            value: if id == GB_TYPICAL || id == GB_CLASS_FOUND {
                ValueKind::Text
            } else {
                ValueKind::Number
            },
            args: Vec::new(),
            method_status: MethodStatus::Unverified,
        })
    }

    fn input(&self, name: &str) -> Option<InputDef> {
        INPUTS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(n, values)| InputDef {
                values: values.iter().map(|v| (*v).into()).collect(),
                scope: if [WATERWAY, REDUCED_RADIUS_CLAIM].contains(n) {
                    InputScope::Object
                } else {
                    InputScope::Design
                },
            })
    }

    fn shapes(&self) -> &[FormulaShape] {
        SHAPES
    }

    fn design_object(&self) -> &str {
        CHANNEL
    }

    fn measure_list_hint(&self) -> Option<&str> {
        Some("see owd_core::rules::engine::MEASURES")
    }
}

/// The Rule Set files built into every binary: (path, text).
pub const BUILT_IN: &[(&str, &str)] = &[
    (
        "rules/pianc-wg141/0.3.0.json",
        include_str!("../../rules/pianc-wg141/0.3.0.json"),
    ),
    (
        "rules/gb50139/0.3.0.json",
        include_str!("../../rules/gb50139/0.3.0.json"),
    ),
    (
        "rules/owd/0.3.0.json",
        include_str!("../../rules/owd/0.3.0.json"),
    ),
];

/// The set every selection holds: OWD's own checks.
pub const OWD_SET: &str = "owd";

/// The built-in sets, loaded once through the same loader a user's file takes.
pub fn built_in() -> &'static Library {
    static LIB: OnceLock<Library> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut lib = Library::default();
        for (path, text) in BUILT_IN {
            let loaded = load(text, &ChannelDomain).unwrap_or_else(|e| panic!("{path}: {e}"));
            lib.add(loaded).unwrap_or_else(|e| panic!("{path}: {e}"));
        }
        lib
    })
}

/// The vessel a channel is sized for.
#[derive(Debug, Clone, PartialEq)]
pub struct Vessel {
    pub loa_m: f64,
    pub beam_m: f64,
    pub draught_m: f64,
    /// `motor_vessel` or `pushed_convoy`.
    pub formation: String,
    /// The class: its catalogue id, such as `CEMT-IV` (`IV` in older terms).
    pub class_name: String,
    /// The family (scheme) of vessel classes the class belongs to, such as `CEMT-1992` or
    /// `CN-GB50139` (`CEMT` or `CN` in older terms).
    pub family: String,
}

/// The family of the Chinese vessel classes, whose class is the GB 50139 channel class.
pub const CN_FAMILY: &str = "CN";

/// The GB 50139-2014 Table 3.0.2-1 key of a vessel of channel class `class`.
pub fn gb_typical_key(class: &str, vessel: &Vessel) -> String {
    format!("{class} {:.1}x{:.1}", vessel.loa_m, vessel.beam_m)
}

/// A reach of the river as the rules read it: its waterway kind and the reduced-radius claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterwayReach {
    /// How the trace names the reach, such as `0 to 2000 m`.
    pub label: String,
    pub waterway: String,
    /// `yes` or `no`.
    pub claim: String,
    /// Whether the user typed the claim (`false`: the default `no`).
    pub claim_typed: bool,
}

impl WaterwayReach {
    /// The whole river as one free-flowing reach with no claim: a river with no reach marked.
    pub fn whole_river() -> Self {
        Self {
            label: "the whole river".into(),
            waterway: "free-flowing-river".into(),
            claim: "no".into(),
            claim_typed: false,
        }
    }
}

/// A station of a design line, for a check: the reach it falls in (an index into the selection's
/// reaches) and the line's radius there.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub label: String,
    pub reach: usize,
    pub radius_m: f64,
}

/// One channel as the engine reads it: the vessel, the set inputs, the stations with their reaches,
/// and the value of each sized Measure (0 while sizing: the evaluator needs a number to report a
/// limit).
struct Channel<'a> {
    vessel: &'a Vessel,
    /// The selected sets: their GB 50139 tables tell a typical vessel from a special one.
    sets: &'a [Arc<RuleSet>],
    inputs: &'a [(String, String)],
    reaches: &'a [WaterwayReach],
    /// The stations. While sizing there is one per reach, as no line exists yet.
    stations: &'a [Station],
    radius_m: f64,
    /// The channel as sized, for a check of the result; `None` while sizing.
    sized: Option<&'a Sized>,
}

impl MeasureSource for Channel<'_> {
    fn def(&self, id: &str) -> Option<MeasureInfo> {
        ChannelDomain.measure(id)
    }

    fn objects(&self, object: &str) -> Vec<String> {
        match object {
            CHANNEL => vec![CHANNEL.into()],
            STATION => self.stations.iter().map(|s| s.label.clone()).collect(),
            _ => Vec::new(),
        }
    }

    fn object_input(&self, object: &str, index: usize, name: &str) -> Option<String> {
        let reach = match object {
            STATION => self.reaches.get(self.stations.get(index)?.reach)?,
            _ => return self.input(name),
        };
        match name {
            WATERWAY => Some(reach.waterway.clone()),
            REDUCED_RADIUS_CLAIM => Some(reach.claim.clone()),
            _ => self.input(name),
        }
    }

    fn value(&self, m: &MeasureRef, index: usize) -> Result<MeasureValue, String> {
        let v = match m.id.as_str() {
            LOA => self.vessel.loa_m,
            BEAM => self.vessel.beam_m,
            DRAUGHT => self.vessel.draught_m,
            BEND_RADIUS => self.radius_m,
            GB_CLASS_FOUND => {
                let found = gb_class_found(self.vessel, self.inputs);
                return Ok(MeasureValue::Text(format!(
                    "{}, {}",
                    found.class, found.how
                )));
            }
            GB_TYPICAL => {
                let class = self.gb_class();
                return Ok(MeasureValue::Text(gb_typical_key(&class, self.vessel)));
            }
            BOTTOM_WIDTH => self.sized.map_or(0.0, |s| s.bottom_width_straight_m),
            MIN_BEND_RADIUS => match self.sized {
                None => 0.0,
                Some(_) => self.stations.get(index).ok_or("no such station")?.radius_m,
            },
            DESIGN_DEPTH => self.sized.map_or(0.0, |s| s.design_depth_m),
            BEND_WIDENING => self.sized.map_or(0.0, |s| s.bend_widening_at_rmin_m),
            other => return Err(format!("unknown Measure `{other}`")),
        };
        Ok(MeasureValue::Number(v))
    }

    fn input(&self, name: &str) -> Option<String> {
        match name {
            FORMATION => Some(self.vessel.formation.clone()),
            VESSEL_CLASS => Some(self.vessel.class_name.clone()),
            GB_CLASS => Some(self.gb_class()),
            GB_VESSEL => Some(self.gb_vessel().into()),
            _ => self
                .inputs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone()),
        }
    }
}

/// The GB 50139-2014 design tonnage of each channel class (Table 3.0.1, p3), smallest first. A
/// waterway for ships above 3000 t is class I (note 2).
pub const GB_CLASS_TONNAGE_T: [(&str, f64); 7] = [
    ("VII", 50.0),
    ("VI", 100.0),
    ("V", 300.0),
    ("IV", 500.0),
    ("III", 1000.0),
    ("II", 2000.0),
    ("I", 3000.0),
];

/// The top of each CEMT class's tonnage range (t), over its motor vessel, barge and pushed convoy
/// rows: ECMT Resolution No. 92/2, *New classification of inland waterways* (1992), the table AGN
/// Annex III repeats. Secondary copies print class Va as 1500-2400 t or up to 3000 t; the top,
/// 3000 t, is used, and either gives GB class I.
pub const CEMT_TONNAGE_TOP_T: [(&str, f64); 10] = [
    ("I", 400.0),
    ("II", 650.0),
    ("III", 1200.0),
    ("IV", 1500.0),
    ("Va", 3000.0),
    ("Vb", 6000.0),
    ("VIa", 6000.0),
    ("VIb", 12000.0),
    ("VIc", 18000.0),
    ("VII", 27000.0),
];

/// A vessel's GB 50139 channel class, and how it was found.
#[derive(Debug, Clone, PartialEq)]
pub struct GbClass {
    /// `I` to `VII`, or `unknown`.
    pub class: String,
    /// In words: from the vessel, by tonnage, typed, or why there is none.
    pub how: String,
    /// The vessel is above 3000 t: class I, sized as a special design vessel (3.0.7).
    pub above_3000_t: bool,
}

/// A vessel's GB 50139 channel class. In order: the typed `gb_class`; a Chinese vessel's own class;
/// for a CEMT vessel, the smallest class of Table 3.0.1 whose design tonnage covers the top of its
/// CEMT class's range; for any other catalogue class, the top of its deadweight range in the vessel
/// catalogue (above 3000 t: class I, a special design vessel); else `unknown`.
pub fn gb_class_found(vessel: &Vessel, inputs: &[(String, String)]) -> GbClass {
    let found = |class: &str, how: String, above_3000_t| GbClass {
        class: class.into(),
        how,
        above_3000_t,
    };
    if let Some((_, typed)) = inputs.iter().find(|(k, _)| k == GB_CLASS) {
        return found(typed, "typed".into(), false);
    }
    // A custom vessel (spec 0015 T1): GB 50139 cannot derive a class for a vessel it does not know.
    if vessel.family == super::custom::CUSTOM_FAMILY {
        return found(
            GB_CLASS_UNKNOWN,
            "a custom vessel has no GB 50139 class unless one is given (its `gb_class`)".into(),
            false,
        );
    }
    // The Chinese classes: `CN-III-MOTOR` in the catalogue (`III` in older studies' terms).
    if vessel.family == CN_FAMILY || vessel.family.starts_with("CN-") {
        let class = vessel
            .class_name
            .strip_prefix("CN-")
            .map_or(vessel.class_name.as_str(), |r| {
                r.split('-').next().unwrap_or(r)
            });
        return found(class, "from the vessel (a Chinese class)".into(), false);
    }
    // A CEMT class letter: `CEMT-Va` in the catalogue (`Va` in older studies' terms).
    let cemt = vessel.family.starts_with("CEMT");
    let letter = vessel
        .class_name
        .strip_prefix("CEMT-")
        .unwrap_or(&vessel.class_name);
    let by_cemt = CEMT_TONNAGE_TOP_T
        .iter()
        .find(|(c, _)| cemt && *c == letter)
        .map(|(_, t)| {
            (
                *t,
                format!("by tonnage: CEMT {letter} up to {t} t (ECMT Resolution 92/2)"),
            )
        });
    let by_catalogue = || {
        super::classes::deadweight_top_t(&vessel.class_name).map(|t| {
            (
                t,
                format!(
                    "by deadweight: {} up to {t} t (the vessel catalogue)",
                    vessel.class_name
                ),
            )
        })
    };
    let Some((t, by)) = by_cemt.or_else(by_catalogue) else {
        return found(
            GB_CLASS_UNKNOWN,
            format!(
                "no GB 50139 class and no tonnage for a {} {} vessel",
                vessel.family, vessel.class_name
            ),
            false,
        );
    };
    match GB_CLASS_TONNAGE_T.iter().find(|(_, cap)| *cap >= t) {
        Some((class, cap)) => found(
            class,
            format!("{by}, within class {class}'s {cap} t"),
            false,
        ),
        None => found(
            "I",
            format!("{by}, above 3000 t: class I, a special design vessel (3.0.7)"),
            true,
        ),
    }
}

/// A vessel's GB 50139 channel class (see [`gb_class_found`]).
pub fn gb_class(vessel: &Vessel, inputs: &[(String, String)]) -> String {
    gb_class_found(vessel, inputs).class
}

impl Channel<'_> {
    fn gb_class(&self) -> String {
        gb_class(self.vessel, self.inputs)
    }

    /// `typical` when a table of a selected set keyed by [`GB_TYPICAL`] has a row for the vessel,
    /// else `special`; always `special` above 3000 t, and for a custom vessel (spec 0015 T1:
    /// Appendix A).
    fn gb_vessel(&self) -> &'static str {
        if self.vessel.family == super::custom::CUSTOM_FAMILY
            || gb_class_found(self.vessel, self.inputs).above_3000_t
        {
            return "special";
        }
        let key = gb_typical_key(&self.gb_class(), self.vessel);
        let typical = self.sets.iter().flat_map(|s| s.tables.iter()).any(|t| {
            t.input == GB_TYPICAL
                && t.rows.iter().any(
                    |r| matches!(&r.when, arc_rules::model::RowWhen::OneOf(v) if v.contains(&key)),
                )
        });
        if typical { "typical" } else { "special" }
    }
}

/// The selected sets and their inputs: what sizes a channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub sets: Vec<Arc<RuleSet>>,
    /// Set inputs by name, such as `se_quality` = `C`.
    pub inputs: Vec<(String, String)>,
    /// The reaches of the river, in chainage order. One free-flowing reach with no claim when the
    /// user marked none.
    pub reaches: Vec<WaterwayReach>,
}

impl Selection {
    /// Built-in sets by `id` or `id@version`, with their inputs.
    pub fn built_in(ids: &[&str], inputs: &[(&str, &str)]) -> Result<Self, RuleError> {
        let lib = built_in();
        let mut sets = Vec::new();
        for id in ids {
            let set = lib
                .lookup(id)
                .ok_or_else(|| RuleError::Key(format!("unknown Rule Set `{id}`")))?;
            sets.push(Arc::new(set.clone()));
        }
        Ok(Self {
            sets,
            inputs: inputs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            reaches: vec![WaterwayReach::whole_river()],
        })
    }

    /// Sets of a library by `(id, version)`, closed under `includes` and checked as the engine
    /// checks a selection (no two versions of one set, no rule id with two meanings), with their
    /// inputs. A set chosen twice is selected once.
    pub fn from_library(
        library: &Library,
        chosen: &[(&str, &str)],
        inputs: Vec<(String, String)>,
    ) -> Result<Self, RuleError> {
        let picked = library.select(chosen).map_err(RuleError::Value)?;
        Ok(Self {
            sets: picked
                .sets
                .into_iter()
                .map(|s| Arc::new(s.clone()))
                .collect(),
            inputs,
            reaches: vec![WaterwayReach::whole_river()],
        })
    }

    /// The same selection on these reaches (at least one).
    pub fn with_reaches(mut self, reaches: Vec<WaterwayReach>) -> Self {
        if !reaches.is_empty() {
            self.reaches = reaches;
        }
        self
    }

    /// The same selection with only the Rules on `measure` that apply to these inputs. The trace of
    /// the narrowed selection gives the same rows on that Measure, at a fraction of the cost: the
    /// water checks read the bend widening at every station.
    pub fn narrowed_to(&self, measure: &str, vessel: &Vessel) -> Self {
        let source = Channel {
            vessel,
            sets: &self.sets,
            inputs: &self.inputs,
            reaches: &self.reaches,
            stations: &[],
            radius_m: 0.0,
            sized: None,
        };
        let sets = self
            .sets
            .iter()
            .map(|set| {
                let mut set = RuleSet::clone(set);
                set.rules
                    .retain(|r| r.measure.id == measure && applies(&r.applies_when, &source));
                Arc::new(set)
            })
            .collect();
        Self {
            sets,
            inputs: self.inputs.clone(),
            reaches: self.reaches.clone(),
        }
    }

    /// The Rule Trace of a channel for this vessel, with the bend widening read at `radius_m`.
    pub fn trace(&self, vessel: &Vessel, radius_m: f64) -> Trace {
        let stations = self.reach_stations(0.0);
        self.evaluate(vessel, radius_m, None, &stations)
    }

    /// The Rule Trace of a sized channel, read at R_min: every Rule tested on the sized values, with
    /// one station per reach at the sized R_min.
    pub fn check(&self, vessel: &Vessel, sized: &Sized) -> Trace {
        let stations = self.reach_stations(sized.min_bend_radius_m);
        self.evaluate(vessel, sized.min_bend_radius_m, Some(sized), &stations)
    }

    /// The Rule Trace of a sized channel at the stations of a design line, each at its own radius.
    pub fn check_stations(&self, vessel: &Vessel, sized: &Sized, stations: &[Station]) -> Trace {
        self.evaluate(vessel, sized.min_bend_radius_m, Some(sized), stations)
    }

    /// One station per reach, at `radius_m`: what sizing evaluates before a line exists. Its label
    /// names the reach and its inputs, each marked typed or default.
    pub fn reach_stations(&self, radius_m: f64) -> Vec<Station> {
        self.reaches
            .iter()
            .enumerate()
            .map(|(i, r)| Station {
                label: format!(
                    "{} ({}, reduced_radius_claim {} {})",
                    r.label,
                    r.waterway,
                    r.claim,
                    if r.claim_typed { "typed" } else { "default" }
                ),
                reach: i,
                radius_m,
            })
            .collect()
    }

    fn evaluate(
        &self,
        vessel: &Vessel,
        radius_m: f64,
        sized: Option<&Sized>,
        stations: &[Station],
    ) -> Trace {
        let sets: Vec<&RuleSet> = self.sets.iter().map(|s| s.as_ref()).collect();
        let source = Channel {
            vessel,
            sets: &self.sets,
            inputs: &self.inputs,
            reaches: &self.reaches,
            stations,
            radius_m,
            sized,
        };
        evaluate(&sets, SHAPES, &source)
    }

    /// A Rule of a selected set by id.
    pub fn rule(&self, id: &str) -> Option<&arc_rules::model::Rule> {
        self.sets.iter().find_map(|s| s.rule(id))
    }
}

/// The governing limit of the lower-bound Requirement on `measure`, as the engine marks it.
pub fn governing_limit(trace: &Trace, measure: &str) -> Result<f64, RuleError> {
    let row = trace.rows.iter().find(|r| {
        r.governing
            && r.measure.as_ref().is_some_and(|m| m.id == measure)
            && r.compare.is_some_and(|c| c.is_lower_bound())
    });
    match row.and_then(|r| r.limit) {
        Some(limit) => Ok(limit),
        None => {
            let why = trace
                .rows
                .iter()
                .filter(|r| r.measure.as_ref().is_some_and(|m| m.id == measure))
                .find_map(|r| r.gap.clone());
            value_err(match why {
                Some(gap) => format!("no Rule sizes `{measure}`: {gap}"),
                None => format!("no selected Rule sizes `{measure}`"),
            })
        }
    }
}

/// A channel sized by the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Sized {
    pub bottom_width_straight_m: f64,
    pub min_bend_radius_m: f64,
    pub design_depth_m: f64,
    pub bend_widening_at_rmin_m: f64,
}

/// The governing limit of the radius Requirement at each station of a trace, in station order.
pub fn governing_radii(trace: &Trace) -> Result<Vec<(String, f64)>, RuleError> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for r in &trace.rows {
        if r.governing
            && r.measure.as_ref().is_some_and(|m| m.id == MIN_BEND_RADIUS)
            && r.compare.is_some_and(|c| c.is_lower_bound())
            && let (Some(object), Some(limit)) = (&r.object, r.limit)
        {
            out.push((object.clone(), limit));
        }
    }
    if out.is_empty() {
        governing_limit(trace, MIN_BEND_RADIUS)?;
    }
    Ok(out)
}

/// Size a channel: the governing limit of each sized Measure. The alignment takes one minimum
/// radius: the largest governing radius of any reach (spec 0007), so the line meets every reach's
/// Rule. The bend widening is read at that radius.
pub fn size(selection: &Selection, vessel: &Vessel) -> Result<Sized, RuleError> {
    let first = selection.trace(vessel, 0.0);
    let radii = governing_radii(&first)?;
    if radii.len() != selection.reaches.len() {
        return value_err(format!(
            "no selected Rule sizes `{MIN_BEND_RADIUS}` on every reach"
        ));
    }
    let r_min = radii
        .iter()
        .map(|(_, r)| *r)
        .fold(f64::NEG_INFINITY, py_max);
    let at_rmin = selection.trace(vessel, r_min);
    Ok(Sized {
        bottom_width_straight_m: governing_limit(&at_rmin, BOTTOM_WIDTH)?,
        min_bend_radius_m: r_min,
        design_depth_m: governing_limit(&at_rmin, DESIGN_DEPTH)?,
        bend_widening_at_rmin_m: widening(selection, vessel, r_min)?,
    })
}

/// The governing bend widening at `radius_m`. None on a straight (a radius that is not finite and
/// positive); NaN in, NaN out, as the code this replaced.
pub fn widening(selection: &Selection, vessel: &Vessel, radius_m: f64) -> Result<f64, RuleError> {
    if radius_m.is_nan() {
        return Ok(f64::NAN);
    }
    if radius_m <= 0.0 || radius_m.is_infinite() {
        return Ok(0.0);
    }
    if let Some(limit) = only_formula(selection, vessel, radius_m, BEND_WIDENING) {
        return limit.map_err(RuleError::Value);
    }
    governing_limit(&selection.trace(vessel, radius_m), BEND_WIDENING)
}

/// The limit of the one formula Rule on `measure`, when the selection holds exactly one Rule on it
/// with no object-scoped condition: with one Rule there is nothing to choose between, so its
/// formula is the governing limit. `None` otherwise; the caller then asks the engine.
fn only_formula(
    selection: &Selection,
    vessel: &Vessel,
    radius_m: f64,
    measure: &str,
) -> Option<Result<f64, String>> {
    let mut rules = selection
        .sets
        .iter()
        .flat_map(|s| s.rules.iter())
        .filter(|r| r.measure.id == measure);
    let rule = rules.next()?;
    if rules.next().is_some() || !rule.applies_when.object_scoped.is_empty() || !rule.evaluated {
        return None;
    }
    let Some(Limit::Formula {
        shape,
        coefficients,
        operands,
        ..
    }) = &rule.limit
    else {
        return None;
    };
    let def = arc_rules::domain::shape(SHAPES, shape)?;
    let source = Channel {
        vessel,
        sets: &selection.sets,
        inputs: &selection.inputs,
        reaches: &selection.reaches,
        stations: &[],
        radius_m,
        sized: None,
    };
    let mut operand = |name: &str| -> Result<f64, String> {
        let id = operands
            .iter()
            .find_map(|(k, op)| match op {
                Operand::Measure(id) if k == name => Some(id),
                _ => None,
            })
            .ok_or_else(|| format!("formula `{shape}` needs operand `{name}`"))?;
        match source.value(
            &MeasureRef {
                id: id.clone(),
                args: Vec::new(),
            },
            0,
        )? {
            MeasureValue::Number(x) => Ok(x),
            _ => Err(format!("`{id}` is not a number")),
        }
    };
    Some((def.value)(shape, coefficients, &mut operand))
}

/// The Rule Set and the one set input an old ruleset name maps to (spec 0007, "Rule Sets at
/// 0.3.0"): PIANC-A, B and C are `pianc-wg141` at that quality; CN and CN-conservative are
/// `gb50139` at the permissive or strict end. CN-reduced is `gb50139` strict too, on a canal reach
/// with the reduced-radius claim (see [`selection_for_old_ruleset`]); GB 50139-2014 has no reduced
/// radius, so it gives the CN-conservative channel.
pub fn old_name_choice(
    name: &str,
) -> Result<(&'static str, &'static str, &'static str), RuleError> {
    Ok(match name {
        "PIANC-A" => ("pianc-wg141", SE_QUALITY, "A"),
        "PIANC-B" => ("pianc-wg141", SE_QUALITY, "B"),
        "PIANC-C" => ("pianc-wg141", SE_QUALITY, "C"),
        "CN" => ("gb50139", RANGE_END, "permissive"),
        "CN-conservative" | "CN-reduced" => ("gb50139", RANGE_END, "strict"),
        other => {
            return Err(RuleError::Key(format!(
                "unknown ruleset `{other}`; the old names are {}",
                OLD_RULESETS.join(", ")
            )));
        }
    })
}

/// The selection an old ruleset name maps to, with the `owd` checks (see [`old_name_choice`]).
/// CN-reduced is `gb50139` strict on one canal reach over the whole line, with the reduced-radius
/// claim. Kept for `owd fits`, whose choices are the old names.
pub fn selection_for_old_ruleset(name: &str) -> Result<Selection, RuleError> {
    let (set, input, value) = old_name_choice(name)?;
    let selection = Selection::built_in(&[set, OWD_SET], &[(input, value)])?;
    Ok(if name == "CN-reduced" {
        selection.with_reaches(vec![WaterwayReach {
            label: "the whole river".into(),
            waterway: "canal".into(),
            claim: "yes".into(),
            claim_typed: true,
        }])
    } else {
        selection
    })
}

/// The old ruleset names [`selection_for_old_ruleset`] maps, in their old order.
pub const OLD_RULESETS: [&str; 6] = [
    "PIANC-A",
    "PIANC-B",
    "PIANC-C",
    "CN",
    "CN-conservative",
    "CN-reduced",
];
