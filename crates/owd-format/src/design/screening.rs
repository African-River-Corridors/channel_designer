//! The screening outputs of a channel design (spec 0007 T5): A, the verdict of each design line
//! option; B, the design basis (stored with the sizing); C, the constraint stretches of each option;
//! and E, the full Rule Trace, rebuilt on demand by [`Session::rule_trace`] and never stored. D, the
//! quantities, is the volumes stage as before.

use super::geojson::all_positions;
use super::rule_sets::reach_ranges;
use super::session::{DesignError, Session};
use owd_core::alignment::navigable::{local_radius_on, water_width_on};
use owd_core::geom::linear;
use owd_core::geom::{Polygon, Pt};
use owd_core::rules::engine::{self, Selection, Sized, Station, Vessel};
use owd_core::rules::rulesets::vessel_for;
use owd_core::rules::screening::{self, StationIn};
use owd_core::volumes::sections::SectionVolumes;
use serde_json::{Value, json};

/// The label every screening output carries.
pub const SCREENING_NOTICE: &str = "Screening \u{2014} not for construction";
/// The notice every output carries while the Rules are unverified.
pub const UNVERIFIED_NOTICE: &str =
    "Every Rule is unverified until a second reader checks it against its source.";

/// The chord the radii are read on when an option does not record one (m).
const DEFAULT_CHORD_M: f64 = 100.0;

fn err(e: owd_core::rules::RuleError) -> DesignError {
    DesignError::Input(e.raw_message().to_string())
}

fn round_trip(x: f64) -> Value {
    if x.is_finite() { json!(x) } else { Value::Null }
}

impl Session {
    /// The selection (with the reaches), the vessel and the sized channel of the sizing settings.
    pub(super) fn engine_inputs(&self) -> Result<(Selection, Vessel, Sized), DesignError> {
        let settings = self.sizing_settings();
        let vessel_id = settings["vessel_standard_id"].as_str().ok_or_else(|| {
            DesignError::Input("the sizing stage has no vessel class: choose one first".into())
        })?;
        let selection = self.selection()?;
        let vessel = vessel_for(vessel_id, settings["draught_m"].as_f64()).map_err(err)?;
        let sized = engine::size(&selection, &vessel).map_err(err)?;
        Ok((selection, vessel, sized))
    }

    /// B, the design basis: each quantity with its governing Rule, citation and status.
    pub(super) fn design_basis_value(&self) -> Result<Value, DesignError> {
        let (selection, vessel, sized) = self.engine_inputs()?;
        let rows = screening::design_basis(&selection, &vessel, &sized).map_err(err)?;
        Ok(json!(
            rows.iter()
                .map(|b| json!({
                    "quantity": b.quantity,
                    "waterway": b.waterway,
                    "value": b.value,
                    "unit": b.unit,
                    "rule": b.rule,
                    "rule_sets": b.rule_sets,
                    "citation": b.citation,
                    "origin": b.origin,
                    "status": b.status,
                    "other_rules": b.others.iter()
                        .map(|(r, l)| json!({"rule": r, "limit": l}))
                        .collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>()
        ))
    }

    /// The reach a point falls in, by its chainage on the river centreline.
    fn reach_of(ranges: &[(f64, f64, engine::WaterwayReach)], centre: &[Pt], p: Pt) -> usize {
        if centre.len() < 2 {
            return 0;
        }
        let s = linear::project(centre, p);
        ranges
            .iter()
            .position(|(a, b, _)| s >= *a && s < *b)
            .unwrap_or(ranges.len().saturating_sub(1))
    }

    /// A and C for one design line option, from its volumes station walk: where the river falls
    /// short of a governing limit, the infeasible stretches of spec 0005, and the verdict.
    pub(super) fn screening_value(
        &self,
        option: &Value,
        walk: &SectionVolumes,
        river: &[Polygon],
        centre: &[Pt],
    ) -> Result<Value, DesignError> {
        let (selection, vessel, sized) = self.engine_inputs()?;
        let line: Vec<Pt> = all_positions(&option["line"]);
        let chord = option["checks"]["radius_check_chord_m"]
            .as_f64()
            .unwrap_or(DEFAULT_CHORD_M);
        let ranges = reach_ranges(&self.doc);
        let stations: Vec<StationIn> = walk
            .stations
            .iter()
            .map(|cs| {
                let p = [cs.x, cs.y];
                let bed = cs
                    .offsets
                    .iter()
                    .zip(&cs.terrain)
                    .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
                    .map_or(f64::NAN, |(_, t)| *t);
                let river_s = if centre.len() >= 2 {
                    linear::project(centre, p)
                } else {
                    f64::NAN
                };
                StationIn {
                    chainage_m: cs.chainage_m,
                    xy: p,
                    river_width_m: if river.is_empty() {
                        f64::NAN
                    } else {
                        water_width_on(&line, cs.chainage_m, river)
                    },
                    river_depth_m: cs.wl - bed,
                    river_radius_m: if river_s.is_nan() {
                        f64::NAN
                    } else {
                        local_radius_on(centre, river_s, chord)
                    },
                    line_radius_m: local_radius_on(&line, cs.chainage_m, chord),
                    reach: Session::reach_of(&ranges, centre, p),
                    dredge_area_m2: cs.dredge_area_m2,
                    dry_cut_area_m2: cs.excav_area_m2,
                }
            })
            .collect();
        let found =
            screening::constraint_stretches(&selection, &vessel, &sized, &stations, walk.station_m)
                .map_err(err)?;
        let mut stretches: Vec<Value> = found
            .iter()
            .map(|x| {
                json!({
                    "kind": x.kind.key(),
                    "start_chainage_m": x.start_chainage_m,
                    "end_chainage_m": x.end_chainage_m,
                    "start": x.start,
                    "end": x.end,
                    "governing_rule": x.governing_rule,
                    "required": round_trip(x.required),
                    "river": round_trip(x.river),
                    "worst_chainage_m": x.worst_chainage_m,
                    "deficit": round_trip(x.deficit),
                    "works": x.works,
                    "dredge_m3": x.dredge_m3,
                    "dry_cut_m3": x.dry_cut_m3,
                })
            })
            .collect();
        // Spec 0005's infeasible stretches: no works make the line fit there.
        let infeasible = option["checks"]["fits_in_water"]["stretches"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for f in &infeasible {
            let (a, b) = (
                f["start_chainage_m"].as_f64().unwrap_or(0.0),
                f["end_chainage_m"].as_f64().unwrap_or(0.0),
            );
            let inside = walk
                .stations
                .iter()
                .filter(|cs| cs.chainage_m >= a && cs.chainage_m <= b);
            let (dredge, dry) = inside.fold((0.0, 0.0), |(d, e), cs| {
                (
                    d + cs.dredge_area_m2 * walk.station_m,
                    e + cs.excav_area_m2 * walk.station_m,
                )
            });
            stretches.push(json!({
                "kind": "infeasible",
                "start_chainage_m": a,
                "end_chainage_m": b,
                "start": f["start"],
                "end": f["end"],
                "reason": f["reason"],
                "governing_rule": null,
                "required": null,
                "river": null,
                "worst_chainage_m": null,
                "deficit": null,
                "works": "none-possible",
                "dredge_m3": dredge,
                "dry_cut_m3": dry,
            }));
        }
        stretches.sort_by(|x, y| {
            x["start_chainage_m"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&y["start_chainage_m"].as_f64().unwrap_or(0.0))
        });
        // A failing row: an infeasible stretch, or a line tighter than R_min.
        let failing = !infeasible.is_empty() || option["checks"]["radius_ok"] == false;
        let verdict = if failing {
            "not-navigable"
        } else if !found.is_empty() {
            "navigable-with-works"
        } else {
            "navigable-as-is"
        };
        let blocking = screening::blocking_not_evaluated(&selection, &vessel, &sized);
        Ok(json!({
            "verdict": verdict,
            "notice": SCREENING_NOTICE,
            "rules_notice": UNVERIFIED_NOTICE,
            "blocking_not_evaluated": {"count": blocking.len(), "rules": blocking},
            "constraint_stretches": stretches,
        }))
    }

    /// E, the full Rule Trace: every Rule of every selected set at every object, with its value,
    /// limit, margin, outcome, governing flag and status. Rebuilt from the document and the stored
    /// set hashes, never stored; a stored hash the engine does not hold is refused. With an option,
    /// the station Rules are read at the option's stations every `station_m` (its own radius, and
    /// the reach each falls in); without one, at one station per reach.
    pub fn rule_trace(&self, option: Option<&str>, station_m: f64) -> Result<Value, DesignError> {
        let (selection, vessel, sized) = self.engine_inputs()?;
        let trace = match option {
            None => selection.check(&vessel, &sized),
            Some(id) => {
                let o = self.doc["options"]
                    .as_array()
                    .and_then(|l| l.iter().find(|o| o["id"] == id))
                    .ok_or_else(|| DesignError::Input(format!("no design line option `{id}`")))?;
                let line: Vec<Pt> = all_positions(&o["line"]);
                let chord = o["checks"]["radius_check_chord_m"]
                    .as_f64()
                    .unwrap_or(DEFAULT_CHORD_M);
                let centre = self.river_centreline().unwrap_or_default();
                let ranges = reach_ranges(&self.doc);
                let length: f64 = line
                    .windows(2)
                    .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
                    .sum();
                let n = (length / station_m).ceil().max(1.0) as usize;
                let stations: Vec<Station> = (0..=n)
                    .map(|i| {
                        let s = (i as f64 * station_m).min(length);
                        let p = linear::interpolate(&line, s).unwrap_or(line[0]);
                        Station {
                            label: format!("{s} m"),
                            reach: Session::reach_of(&ranges, &centre, p),
                            radius_m: local_radius_on(&line, s, chord),
                        }
                    })
                    .collect();
                selection.check_stations(&vessel, &sized, &stations)
            }
        };
        let rows: Vec<Value> = trace
            .rows
            .iter()
            .map(|r| {
                json!({
                    "rule": r.rule,
                    "rule_sets": r.rule_sets,
                    "title": r.title,
                    "citation": r.citation,
                    "origin": r.origin.map(|o| o.key()),
                    "status": if r.verified { "verified" } else { "unverified" },
                    "object": r.object,
                    "value": match &r.value {
                        Some(arc_rules::eval::MeasureValue::Number(x)) => round_trip(*x),
                        _ => Value::Null,
                    },
                    "unit": r.unit,
                    "compare": r.compare.map(|c| c.key()),
                    "limit": r.limit.map(round_trip),
                    "margin": r.margin.map(round_trip),
                    "outcome": r.outcome.key(),
                    "governing": r.governing,
                    "consequence": r.consequence.map(|c| c.key()),
                    "gap": r.gap,
                })
            })
            .collect();
        Ok(json!({
            "notice": SCREENING_NOTICE,
            "rules_notice": UNVERIFIED_NOTICE,
            "sets": trace.sets,
            "option": option,
            "rows": rows,
        }))
    }
}
