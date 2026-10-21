//! The screening outputs of spec 0007 (T5): the design basis of a sized channel, and the constraint
//! stretches of a design line option, where the river as found falls short of a governing limit.
//!
//! The governing limits come from the rules engine ([`super::engine`]): the governing bottom width
//! and depth, the governing bend widening at each station's radius, and the governing minimum radius
//! of each reach. This module only finds where the river's own width, depth and radius fall below
//! them, groups those stations into stretches, and adds up the dredge and dry cut inside each.

use super::RuleError;
use super::engine::{self, Selection, Sized, Vessel};
use arc_rules::eval::{Outcome, Trace, TraceRow};
use arc_rules::model::Compare;

/// The kind of a constraint stretch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchKind {
    /// The navigable water is narrower than the channel's governing width.
    Width,
    /// The water over the bed at the design line is shallower than the governing depth.
    Depth,
    /// The river centreline bends more tightly than its reach's governing minimum radius.
    Radius,
}

impl StretchKind {
    pub const ALL: [StretchKind; 3] = [StretchKind::Width, StretchKind::Depth, StretchKind::Radius];

    pub fn key(self) -> &'static str {
        match self {
            StretchKind::Width => "width",
            StretchKind::Depth => "depth",
            StretchKind::Radius => "radius",
        }
    }

    /// The works that follow: a bank cut, a dredge or a cut-through.
    pub fn works(self) -> &'static str {
        match self {
            StretchKind::Width => "bank-cut",
            StretchKind::Depth => "dredge",
            StretchKind::Radius => "cut-through",
        }
    }
}

/// One station of a design line option, as the river is found there.
#[derive(Debug, Clone, PartialEq)]
pub struct StationIn {
    /// Chainage along the design line (m).
    pub chainage_m: f64,
    /// Position, project CRS.
    pub xy: [f64; 2],
    /// Width of the navigable water across the design line (m); NaN where unknown.
    pub river_width_m: f64,
    /// Water level less the bed at the design line (m); NaN where unknown.
    pub river_depth_m: f64,
    /// Radius of the river centreline where the station projects onto it (m); infinity on a
    /// straight.
    pub river_radius_m: f64,
    /// Radius of the design line at the station (m), for the bend widening it needs.
    pub line_radius_m: f64,
    /// The reach the station falls in: an index into the selection's reaches.
    pub reach: usize,
    /// Dredge and dry-cut areas of the station's cross-section (m²).
    pub dredge_area_m2: f64,
    pub dry_cut_area_m2: f64,
}

/// A run of stations where the river falls short of one governing limit.
#[derive(Debug, Clone, PartialEq)]
pub struct Stretch {
    pub kind: StretchKind,
    /// The first and last failing station: chainage along the design line and position.
    pub start_chainage_m: f64,
    pub end_chainage_m: f64,
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// The Rule that sets the limit, at the worst station.
    pub governing_rule: String,
    /// The required value and the river's own value at the worst station, and that station.
    pub required: f64,
    pub river: f64,
    pub worst_chainage_m: f64,
    /// `required - river` at the worst station: the largest shortfall in the stretch.
    pub deficit: f64,
    pub works: &'static str,
    /// Dredge and dry cut inside the stretch: the station areas times the station spacing.
    pub dredge_m3: f64,
    pub dry_cut_m3: f64,
    /// The indices of the stations in the stretch, first to last.
    pub stations: Vec<usize>,
}

/// The governing row of the lower-bound Requirement on `measure` at `object` (`None`: any object).
fn governing_row<'a>(
    trace: &'a Trace,
    measure: &str,
    object: Option<&str>,
) -> Option<&'a TraceRow> {
    trace.rows.iter().find(|r| {
        r.governing
            && r.measure.as_ref().is_some_and(|m| m.id == measure)
            && r.compare.is_some_and(Compare::is_lower_bound)
            && object.is_none_or(|o| r.object.as_deref() == Some(o))
    })
}

/// What a station needs: (kind, required value, governing rule) for each kind, from the engine.
struct Limits {
    /// The selection narrowed to the bend-widening Rules, read at every station.
    widening: Selection,
    width_rule: String,
    depth: f64,
    depth_rule: String,
    /// Per reach: governing minimum radius and its Rule.
    radius: Vec<(f64, String)>,
}

fn limits(selection: &Selection, vessel: &Vessel, sized: &Sized) -> Result<Limits, RuleError> {
    let check = selection.check(vessel, sized);
    let rule = |r: Option<&TraceRow>| r.and_then(|r| r.rule.clone()).unwrap_or_default();
    let stations = selection.reach_stations(sized.min_bend_radius_m);
    let mut radius = Vec::new();
    for s in &stations {
        let row = governing_row(&check, engine::MIN_BEND_RADIUS, Some(&s.label));
        let limit = row.and_then(|r| r.limit).ok_or_else(|| {
            RuleError::Value(format!("no Rule sizes the radius on reach `{}`", s.label))
        })?;
        radius.push((limit, rule(row)));
    }
    Ok(Limits {
        widening: selection.narrowed_to(engine::BEND_WIDENING, vessel),
        width_rule: rule(governing_row(&check, engine::BOTTOM_WIDTH, None)),
        depth: sized.design_depth_m,
        depth_rule: rule(governing_row(&check, engine::DESIGN_DEPTH, None)),
        radius,
    })
}

/// The required value at a station, its Rule, and the river's value, for one kind. `None` where
/// the river's value is unknown.
fn need(
    kind: StretchKind,
    st: &StationIn,
    lim: &Limits,
    vessel: &Vessel,
    sized: &Sized,
) -> Result<Option<(f64, String, f64)>, RuleError> {
    Ok(match kind {
        StretchKind::Width => {
            if st.river_width_m.is_nan() {
                return Ok(None);
            }
            let widening = engine::widening(&lim.widening, vessel, st.line_radius_m)?;
            Some((
                sized.bottom_width_straight_m + widening,
                lim.width_rule.clone(),
                st.river_width_m,
            ))
        }
        StretchKind::Depth => {
            if st.river_depth_m.is_nan() {
                return Ok(None);
            }
            Some((lim.depth, lim.depth_rule.clone(), st.river_depth_m))
        }
        StretchKind::Radius => {
            if st.river_radius_m.is_nan() {
                return Ok(None);
            }
            let (r, rule) = lim.radius.get(st.reach).cloned().ok_or_else(|| {
                RuleError::Value(format!("station at {} m has no reach", st.chainage_m))
            })?;
            Some((r, rule, st.river_radius_m))
        }
    })
}

/// Whether the river falls short of a limit at a station.
pub fn falls_short(required: f64, river: f64) -> bool {
    river < required
}

/// The constraint stretches of one design line option: each run of consecutive stations where the
/// river falls short of a governing limit, by kind, in chainage order. `station_m` is the station
/// spacing, for the volumes inside each stretch.
pub fn constraint_stretches(
    selection: &Selection,
    vessel: &Vessel,
    sized: &Sized,
    stations: &[StationIn],
    station_m: f64,
) -> Result<Vec<Stretch>, RuleError> {
    let lim = limits(selection, vessel, sized)?;
    let mut out = Vec::new();
    for kind in StretchKind::ALL {
        let mut run: Option<Stretch> = None;
        for (i, st) in stations.iter().enumerate() {
            let short = match need(kind, st, &lim, vessel, sized)? {
                Some((required, rule, river)) if falls_short(required, river) => {
                    Some((required, rule, river))
                }
                _ => None,
            };
            match (short, run.as_mut()) {
                (Some((required, rule, river)), Some(s)) => {
                    s.end_chainage_m = st.chainage_m;
                    s.end = st.xy;
                    s.dredge_m3 += st.dredge_area_m2 * station_m;
                    s.dry_cut_m3 += st.dry_cut_area_m2 * station_m;
                    s.stations.push(i);
                    if required - river > s.deficit {
                        s.deficit = required - river;
                        s.required = required;
                        s.river = river;
                        s.worst_chainage_m = st.chainage_m;
                        s.governing_rule = rule;
                    }
                }
                (Some((required, rule, river)), None) => {
                    run = Some(Stretch {
                        kind,
                        start_chainage_m: st.chainage_m,
                        end_chainage_m: st.chainage_m,
                        start: st.xy,
                        end: st.xy,
                        governing_rule: rule,
                        required,
                        river,
                        worst_chainage_m: st.chainage_m,
                        deficit: required - river,
                        works: kind.works(),
                        dredge_m3: st.dredge_area_m2 * station_m,
                        dry_cut_m3: st.dry_cut_area_m2 * station_m,
                        stations: vec![i],
                    });
                }
                (None, _) => {
                    if let Some(s) = run.take() {
                        out.push(s);
                    }
                }
            }
        }
        if let Some(s) = run.take() {
            out.push(s);
        }
    }
    out.sort_by(|a, b| {
        a.start_chainage_m
            .total_cmp(&b.start_chainage_m)
            .then(a.kind.key().cmp(b.kind.key()))
    });
    Ok(out)
}

/// One row of the design basis: a quantity with the Rule that governs it.
#[derive(Debug, Clone, PartialEq)]
pub struct BasisRow {
    /// `bottom-width`, `min-bend-radius`, `design-depth` or `widening-at-rmin`.
    pub quantity: &'static str,
    /// The waterway kind of the reach, for the minimum radius; `None` for a quantity of the whole
    /// channel.
    pub waterway: Option<String>,
    pub value: f64,
    pub unit: String,
    pub rule: String,
    /// The sets that hold the Rule, as `id@version`.
    pub rule_sets: Vec<String>,
    pub citation: String,
    pub origin: String,
    /// `verified` or `unverified`.
    pub status: &'static str,
    /// The other Rules of the Requirement that have a limit, with their limits.
    pub others: Vec<(String, f64)>,
}

/// The design basis of a sized channel: one row per quantity, and the minimum radius once per
/// waterway kind present, each with its governing Rule, from the Rule Trace of the sized channel.
pub fn design_basis(
    selection: &Selection,
    vessel: &Vessel,
    sized: &Sized,
) -> Result<Vec<BasisRow>, RuleError> {
    let check = selection.check(vessel, sized);
    let stations = selection.reach_stations(sized.min_bend_radius_m);
    let mut out = Vec::new();
    let row_of = |quantity: &'static str,
                  measure: &str,
                  object: Option<&str>,
                  waterway: Option<String>|
     -> Result<BasisRow, RuleError> {
        let g = governing_row(&check, measure, object)
            .ok_or_else(|| RuleError::Value(format!("no Rule governs `{measure}`")))?;
        let others = check
            .rows
            .iter()
            .filter(|r| {
                !r.governing
                    && r.limit.is_some()
                    && r.measure == g.measure
                    && r.object == g.object
                    && r.outcome != Outcome::NotApplicable
            })
            .map(|r| {
                (
                    r.rule.clone().unwrap_or_default(),
                    r.limit.unwrap_or(f64::NAN),
                )
            })
            .collect();
        Ok(BasisRow {
            quantity,
            waterway,
            value: g.limit.unwrap_or(f64::NAN),
            unit: g.unit.clone(),
            rule: g.rule.clone().unwrap_or_default(),
            rule_sets: g.rule_sets.clone(),
            citation: g.citation.clone(),
            origin: g.origin.map_or("", |o| o.key()).to_string(),
            status: if g.verified { "verified" } else { "unverified" },
            others,
        })
    };
    out.push(row_of("bottom-width", engine::BOTTOM_WIDTH, None, None)?);
    let mut kinds: Vec<&str> = Vec::new();
    for (s, reach) in stations.iter().zip(&selection.reaches) {
        if kinds.contains(&reach.waterway.as_str()) {
            continue;
        }
        kinds.push(&reach.waterway);
        out.push(row_of(
            "min-bend-radius",
            engine::MIN_BEND_RADIUS,
            Some(&s.label),
            Some(reach.waterway.clone()),
        )?);
    }
    out.push(row_of("design-depth", engine::DESIGN_DEPTH, None, None)?);
    out.push(row_of(
        "widening-at-rmin",
        engine::BEND_WIDENING,
        None,
        None,
    )?);
    Ok(out)
}

/// The blocking Rules of a sized channel's trace that are not evaluated, each once, by id.
pub fn blocking_not_evaluated(
    selection: &Selection,
    vessel: &Vessel,
    sized: &Sized,
) -> Vec<String> {
    selection
        .check(vessel, sized)
        .blocking_not_evaluated()
        .into_iter()
        .filter_map(|r| r.rule.clone())
        .collect()
}
