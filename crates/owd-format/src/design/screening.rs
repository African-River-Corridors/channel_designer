//! The screening outputs of a channel design (spec 0007 T5): A, the verdict of each design line
//! option; B, the design basis (stored with the sizing); C, the constraint stretches of each option;
//! and E, the full Rule Trace, rebuilt on demand by [`Session::rule_trace`] and never stored. D, the
//! quantities, is the volumes stage as before.

use super::geojson::all_positions;
use super::rule_sets::reach_ranges;
use super::session::{DesignError, Session};
use owd_core::alignment::navigable::{RadiusOn, WaterWidth, local_radius_on};
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
        Self::reach_at(ranges, linear::project(centre, p))
    }

    /// The reach of the river chainage `s`.
    fn reach_at(ranges: &[(f64, f64, engine::WaterwayReach)], s: f64) -> usize {
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
        self.screening_value_on(option, walk, river, centre, None)
    }

    /// [`Session::screening_value`], with the river's own radius read on `radius_line` (a plan line
    /// such as a fairway fit) instead of the river centreline. The reaches and everything else still
    /// read the river centreline.
    pub(super) fn screening_value_on(
        &self,
        option: &Value,
        walk: &SectionVolumes,
        river: &[Polygon],
        centre: &[Pt],
        radius_line: Option<&[Pt]>,
    ) -> Result<Value, DesignError> {
        let (selection, vessel, sized) = self.engine_inputs()?;
        let line: Vec<Pt> = all_positions(&option["line"]);
        let chord = option["checks"]["radius_check_chord_m"]
            .as_f64()
            .unwrap_or(DEFAULT_CHORD_M);
        let ranges = reach_ranges(&self.doc);
        // Many stations on one long line and one water: index them once (the same answers as the
        // plain functions, to the last bit).
        let widths = (!river.is_empty()).then(|| WaterWidth::new(&line, river));
        let on_line = RadiusOn::new(&line);
        let on_centre =
            (centre.len() >= 2).then(|| (RadiusOn::new(centre), linear::ProjectIndex::new(centre)));
        let on_plan = radius_line
            .filter(|l| l.len() >= 2)
            .map(|l| (RadiusOn::new(l), linear::ProjectIndex::new(l)));
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
                let river_s = on_centre.as_ref().map_or(f64::NAN, |(_, ix)| ix.project(p));
                StationIn {
                    chainage_m: cs.chainage_m,
                    xy: p,
                    river_width_m: widths.as_ref().map_or(f64::NAN, |w| w.at(cs.chainage_m)),
                    river_depth_m: cs.wl - bed,
                    river_radius_m: match (&on_plan, &on_centre) {
                        (Some((r, ix)), _) => r.at(ix.project(p), chord),
                        (None, Some((r, _))) if !river_s.is_nan() => r.at(river_s, chord),
                        _ => f64::NAN,
                    },
                    line_radius_m: on_line.at(cs.chainage_m, chord),
                    reach: if on_centre.is_some() {
                        Session::reach_at(&ranges, river_s)
                    } else {
                        0
                    },
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
        // A failing row: an infeasible stretch, a line tighter than R_min, or no feasible line.
        let failing = !infeasible.is_empty()
            || option["checks"]["radius_ok"] == false
            || super::export::no_feasible_line(option);
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
                    .map(|w| {
                        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                        (dx * dx + dy * dy).sqrt()
                    })
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

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::design::Role;
    use owd_core::volumes::sections::CrossSection;

    /// A 50 km line: the screening outputs hold stretches, not stations, so the document stays of
    /// the same order of size as without them. A straight 50 km river 200 m wide with a shallow bar
    /// in every 5 km, walked every 10 m (the volumes stage's walk, built here so the test needs no
    /// DEM and no 50 km alignment run).
    #[test]
    fn a_50_km_line_saves_a_document_of_the_same_order_of_size() {
        let utm = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
        let (x0, y0, len) = (500_000.0, 600_000.0, 50_000.0);
        let centre = json!([[x0, y0], [x0 + len, y0]]);
        let ring = json!([[
            [x0 - 100.0, y0 - 100.0],
            [x0 + len + 100.0, y0 - 100.0],
            [x0 + len + 100.0, y0 + 100.0],
            [x0 - 100.0, y0 + 100.0],
            [x0 - 100.0, y0 - 100.0]
        ]]);
        let mut s = Session::new("Long river", "2026-10-21");
        s.import(&[
            (
                Role::Centreline,
                &format!(r#"{{"type":"LineString"{utm},"coordinates":{centre}}}"#),
            ),
            (
                Role::RiverPolygon,
                &format!(r#"{{"type":"Polygon"{utm},"coordinates":{ring}}}"#),
            ),
        ])
        .unwrap();
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
        s.run_stage(crate::design::Stage::Sizing).unwrap();
        // The option: the river's own centreline, every 10 m as the alignment writes a line.
        let line: Vec<Pt> = (0..=5000).map(|i| [x0 + i as f64 * 10.0, y0]).collect();
        let option = json!({
            "id": "o", "name": "o", "method": "rolling_circle", "settings": {},
            "line": {"type": "LineString", "coordinates": line},
            "checks": {"radius_ok": true, "r_min_m": 330.0, "radius_check_chord_m": 100.0},
            "details": {},
        });
        s.doc["options"] = json!([option.clone()]);
        let stations = line
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let bar = (100..120).contains(&(i % 500));
                CrossSection {
                    chainage_m: i as f64 * 10.0,
                    x: p[0],
                    y: p[1],
                    heading_rad: 0.0,
                    offsets: vec![-10.0, 0.0, 10.0],
                    terrain: vec![if bar { -1.0 } else { -6.0 }; 3],
                    wet: vec![true; 3],
                    hb: 15.0,
                    wl: 0.0,
                    grade: -3.25,
                    depth: 3.25,
                    model: "b".into(),
                    cutface: vec![f64::NAN; 3],
                    dredge_area_m2: if bar { 50.0 } else { 0.0 },
                    excav_area_m2: 0.0,
                    max_face_h_m: 0.0,
                    bench_left: false,
                    bench_right: false,
                    tag: String::new(),
                }
            })
            .collect();
        let walk = SectionVolumes {
            stations,
            station_m: 10.0,
        };
        let river = vec![Polygon {
            exterior: ring[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
                .collect(),
            holes: vec![],
        }];
        let centre_pts = vec![[x0, y0], [x0 + len, y0]];
        let screening = s
            .screening_value(&option, &walk, &river, &centre_pts)
            .unwrap();
        let stretches = screening["constraint_stretches"].as_array().unwrap();
        // Ten bars, ten dredge stretches, each 20 stations of 50 m² at 10 m: 10,000 m³.
        let kinds: Vec<String> = stretches
            .iter()
            .map(|x| {
                format!(
                    "{} {} {}",
                    x["kind"], x["start_chainage_m"], x["end_chainage_m"]
                )
            })
            .collect();
        assert_eq!(stretches.len(), 10, "{kinds:?}");
        assert!(stretches.iter().all(|x| x["kind"] == "depth"));
        assert_eq!(stretches[0]["dredge_m3"], 20.0 * 50.0 * 10.0);
        assert_eq!(screening["verdict"], "navigable-with-works");
        let without = s.to_text().len();
        s.doc["stages"]["volumes"]["results"] =
            json!({"options": [{"id": "o", "screening": screening}]});
        let with = s.to_text().len();
        assert!(
            (with as f64) < 1.05 * without as f64,
            "with the screening {with} bytes, without {without}"
        );
    }
}
