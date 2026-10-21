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
/// The GB 50139 swept path with its bank increment, WG 141 A.3.2 (m): shown beside the width used.
pub const GB_SWEPT_PATH: &str = "channel.gb-swept-path";

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
        GB_SWEPT_PATH,
        "GB 50139 swept path (B + L sin beta) x (1 + bank increment), WG 141 A.3.2",
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

/// The kind of waterway a reach is: `canal`, `impounded-river` or `free-flowing-river`. One value
/// per station, from its reach.
pub const WATERWAY: &str = "waterway";
/// Whether the user claims the reduced GB 50139 radius on a reach (`yes` or `no`). One value per
/// station, from its reach.
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

/// The GB 50139 straight width: the width of the nearest WG 141 Table A.2 entry when the vessel
/// matches it (the relative LOA and beam differences add up to at most `match`; the first of two
/// equal entries wins), else `ratio × B` (WG 141 Table 5.1, China River).
fn gb_straight_width(
    shape: &str,
    c: &[(String, f64)],
    op: &mut dyn FnMut(&str) -> Result<f64, String>,
) -> Result<f64, String> {
    let (loa, beam) = (op("loa")?, op("beam")?);
    match gb_table_match(shape, c, loa, beam)? {
        Some(w) => Ok(w),
        None => Ok(coefficient(shape, c, "ratio")? * beam),
    }
}

/// The width of the Table A.2 entry a vessel matches, by the coefficients of a `gb-straight-width`
/// limit; `None` when it matches none.
pub fn gb_table_match(
    shape: &str,
    c: &[(String, f64)],
    loa: f64,
    beam: f64,
) -> Result<Option<f64>, String> {
    let k = |name: &str| coefficient(shape, c, name);
    let mut best: Option<(f64, f64)> = None; // (width, distance)
    for n in 1..=3 {
        let (l, b, w) = (
            k(&format!("a{n}_loa"))?,
            k(&format!("a{n}_beam"))?,
            k(&format!("a{n}_width"))?,
        );
        let d = (l - loa).abs() / py_max(l, 1.0) + (b - beam).abs() / py_max(b, 1.0);
        match best {
            Some((_, bd)) if !matches!(d.partial_cmp(&bd), Some(core::cmp::Ordering::Less)) => {}
            _ => best = Some((w, d)),
        }
    }
    match best {
        Some((w, d)) if d <= k("match")? => Ok(Some(w)),
        _ => Ok(None),
    }
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
        id: "gb-straight-width",
        operands: &["beam", "loa"],
        coefficients: &[
            "a1_beam", "a1_loa", "a1_width", "a2_beam", "a2_loa", "a2_width", "a3_beam", "a3_loa",
            "a3_width", "match", "ratio",
        ],
        free_unit_operands: &[],
        value: gb_straight_width,
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
            value: ValueKind::Number,
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
    /// The class, such as `IV` or `VIa`.
    pub class_name: String,
}

/// The drift angle of the GB 50139 swept path (WG 141 A.3.2): 3 degrees for classes I to V, 2 degrees
/// for VI and VII, told apart by a beam of 22.8 m.
const GB_DRIFT_DEG_LARGE_BEAM: f64 = 2.0;
const GB_DRIFT_DEG_SMALL_BEAM: f64 = 3.0;
const GB_LARGE_BEAM_M: f64 = 22.8;

/// The GB 50139 bank increment, a fraction of the swept path (WG 141 A.3.2): 0.25 to 0.30 for a
/// convoy, 0.34 to 0.40 for a motor vessel; `range_end` picks the end.
fn gb_bank_increment(motor: bool, strict: bool) -> f64 {
    match (motor, strict) {
        (false, false) => 0.25,
        (false, true) => 0.30,
        (true, false) => 0.34,
        (true, true) => 0.40,
    }
}

/// The GB 50139 swept path with its bank increment, `(B + L sin beta) × (1 + bank)`.
fn gb_swept_path(vessel: &Vessel, range_end: Option<&str>) -> Result<f64, String> {
    let strict = match range_end {
        Some("strict") => true,
        Some("permissive") => false,
        _ => return Err(format!("the swept path needs the `{RANGE_END}` input")),
    };
    let deg = if vessel.beam_m >= GB_LARGE_BEAM_M {
        GB_DRIFT_DEG_LARGE_BEAM
    } else {
        GB_DRIFT_DEG_SMALL_BEAM
    };
    let beta = deg * (core::f64::consts::PI / 180.0);
    let swept = vessel.beam_m + vessel.loa_m * libm::sin(beta);
    let bank = gb_bank_increment(vessel.formation == "motor_vessel", strict);
    Ok(swept * (1.0 + bank))
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
            GB_SWEPT_PATH => gb_swept_path(self.vessel, self.input(RANGE_END).as_deref())?,
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
            _ => self
                .inputs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone()),
        }
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
/// `gb50139` at the permissive or strict end. CN-reduced is `gb50139` strict too, and needs a canal
/// reach with the reduced-radius claim besides (see [`selection_for_old_ruleset`]).
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
