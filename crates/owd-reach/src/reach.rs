//! From a reach to a route to the document (spec 0012): plan a route from clicks or a trace over
//! Overpass, and record it. The document keeps the user's own clicks and way ids, never OSM geometry.

use crate::error::ReachError;
use crate::geo::{LonLat, bbox};
use crate::overpass::{Candidate, Overpass};
use crate::provenance::provenance_record;
use crate::route::{Graph, Part, Route, TRACE_TOLERANCE_M};
use owd_format::design::Session;
use owd_format::design::provenance::Mark;
use serde_json::{Value, json};

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct ReachSpec {
    /// The river name searched, if any.
    pub river: Option<String>,
    pub start: LonLat,
    pub end: LonLat,
    pub via: Vec<LonLat>,
    /// The rough line the user traced, if the reach came from a trace.
    pub trace: Option<Vec<LonLat>>,
}

/// The result of planning.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub spec: ReachSpec,
    pub route: Route,
    /// What the name search found; empty with no river name.
    pub candidates: Vec<Candidate>,
}

/// Pad a bounding box so a click near the edge still finds its waterway: 10 % of the larger span,
/// and at least 0.03 degrees (about 3 km).
fn padded(points: &[LonLat]) -> Result<[f64; 4], ReachError> {
    let b = bbox(points).ok_or_else(|| ReachError::Input("no points".into()))?;
    let m = (0.1 * (b[2] - b[0]).max(b[3] - b[1])).max(0.03);
    Ok([b[0] - m, b[1] - m, b[2] + m, b[3] + m])
}

fn check_point(p: LonLat) -> Result<(), ReachError> {
    if p[0].is_finite() && p[1].is_finite() && p[0].abs() <= 180.0 && p[1].abs() <= 90.0 {
        Ok(())
    } else {
        Err(ReachError::Input(format!(
            "({}, {}) is not a longitude and latitude",
            p[0], p[1]
        )))
    }
}

/// Plan a reach from a start, an end and via points: search the river by name (if given), read the
/// waterways around the points, and route through them.
pub fn plan_points(
    ov: &Overpass,
    river: Option<&str>,
    start: LonLat,
    end: LonLat,
    via: &[LonLat],
) -> Result<Planned, ReachError> {
    let mut points = vec![start];
    points.extend_from_slice(via);
    points.push(end);
    points.iter().try_for_each(|p| check_point(*p))?;
    let candidates = match river {
        Some(n) => ov.search_rivers(n)?,
        None => Vec::new(),
    };
    let graph = Graph::new(ov.waterways(padded(&points)?)?);
    let route = graph.route(&points)?;
    Ok(Planned {
        spec: ReachSpec {
            river: river.map(str::to_string),
            start,
            end,
            via: via.to_vec(),
            trace: None,
        },
        route,
        candidates,
    })
}

/// Plan a reach from a rough trace of the river.
pub fn plan_trace(
    ov: &Overpass,
    river: Option<&str>,
    trace: &[LonLat],
) -> Result<Planned, ReachError> {
    if trace.len() < 2 {
        return Err(ReachError::Input(
            "a trace needs at least two points".into(),
        ));
    }
    trace.iter().try_for_each(|p| check_point(*p))?;
    let candidates = match river {
        Some(n) => ov.search_rivers(n)?,
        None => Vec::new(),
    };
    let graph = Graph::new(ov.waterways(padded(trace)?)?);
    let route = graph.match_trace(trace, TRACE_TOLERANCE_M)?;
    Ok(Planned {
        spec: ReachSpec {
            river: river.map(str::to_string),
            start: trace[0],
            end: *trace.last().expect("two or more points"),
            via: Vec::new(),
            trace: Some(trace.to_vec()),
        },
        route,
        candidates,
    })
}

fn pos(p: LonLat) -> Value {
    json!([p[0], p[1]])
}

/// The `reach` value of the document: the user's points, and the route as way ids and as-drawn
/// lines. No OSM coordinates.
pub fn reach_value(spec: &ReachSpec, route: &Route) -> Value {
    let parts: Vec<Value> = route
        .parts
        .iter()
        .map(|p| match p {
            Part::Waterway { way_ids, .. } => json!({"kind": "waterway", "way_ids": way_ids}),
            Part::AsDrawn { line } => {
                json!({"kind": "as-drawn", "line": line.iter().map(|p| pos(*p)).collect::<Vec<_>>()})
            }
        })
        .collect();
    json!({
        "river": spec.river,
        "start": pos(spec.start),
        "end": pos(spec.end),
        "via": spec.via.iter().map(|p| pos(*p)).collect::<Vec<_>>(),
        "trace": spec.trace.as_ref().map(|t| t.iter().map(|p| pos(*p)).collect::<Vec<_>>()),
        "length_m": route.length_m,
        "route": parts,
    })
}

/// Record a planned reach in a document, with the OSM notice in its provenance (`date` is
/// `YYYY-MM-DD`, given by the caller: this crate has no clock).
pub fn record_reach(
    session: &mut Session,
    planned: &Planned,
    date: &str,
) -> Result<(), ReachError> {
    let rec =
        provenance_record("osm-overpass", date, Mark::Default, false).map_err(ReachError::Input)?;
    session
        .set_reach(reach_value(&planned.spec, &planned.route))
        .map_err(|e| ReachError::Input(e.to_string()))?;
    session
        .set_provenance("route", rec)
        .map_err(|e| ReachError::Input(e.to_string()))
}
