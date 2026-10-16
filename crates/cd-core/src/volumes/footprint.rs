//! Excavation footprint in plan: the design channel polygon minus the present river polygon.
//! Mirrors `volumes/footprint.py`. Areas only (m²).

use crate::alignment::limits::{self, LimitsDiagnostics, Widening};
use crate::geom::interior::representative_point;
use crate::geom::offset::buffer_polygons;
use crate::geom::overlay::{self, Op, polygon_area};
use crate::geom::{Geometry, Polygon, Pt, dist, linear, valid};

/// `orient_like`: flip `candidate` to run the same way as `reference`.
pub fn orient_like(reference: &[Pt], candidate: &[Pt]) -> Vec<Pt> {
    let (rs, re) = (reference[0], reference[reference.len() - 1]);
    let (cs, ce) = (candidate[0], candidate[candidate.len() - 1]);
    let same = dist(rs, cs) + dist(re, ce);
    let opposite = dist(rs, ce) + dist(re, cs);
    let mut c = candidate.to_vec();
    if opposite < same {
        c.reverse();
    }
    c
}

/// The polygons of a geometry (`iter_polygons`).
fn polygons_of(g: Geometry) -> Vec<Polygon> {
    match g {
        Geometry::Polygon(p) if !p.exterior.is_empty() => vec![p],
        Geometry::MultiPolygon(ps) => ps.into_iter().filter(|p| !p.exterior.is_empty()).collect(),
        Geometry::Collection(gs) => gs.into_iter().flat_map(polygons_of).collect(),
        _ => vec![],
    }
}

/// `polygon_from_bank_pair`: the closed polygon between two lines (the second reversed),
/// repaired by `buffer(0)` when invalid.
pub fn polygon_from_bank_pair(left: &[Pt], right: &[Pt]) -> Result<Vec<Polygon>, String> {
    let mut ring: Vec<Pt> = left.to_vec();
    ring.extend(right.iter().rev());
    if ring.first() != ring.last() {
        ring.push(ring[0]);
    }
    let poly = Polygon {
        exterior: ring,
        holes: vec![],
    };
    let polys = if valid::is_valid(core::slice::from_ref(&poly)) {
        vec![poly]
    } else {
        polygons_of(buffer_polygons(&[poly], 0.0).map_err(|e| e.to_string())?)
    };
    if polys.is_empty() {
        return Err("Constructed polygon is empty; check the input bank geometries.".into());
    }
    Ok(polys)
}

/// `subtract_polygons`: channel minus river (repaired by `buffer(0)`), slivers under `min_area`
/// dropped, in GEOS's order.
pub fn subtract_polygons(
    channel: &[Polygon],
    river: &[Polygon],
    min_area: f64,
) -> Result<Vec<Polygon>, String> {
    let river = polygons_of(buffer_polygons(river, 0.0).map_err(|e| e.to_string())?);
    let cut = overlay::overlay(channel, &river, Op::Difference);
    let floor = crate::py_max(min_area, 0.0);
    Ok(cut
        .into_iter()
        .filter(|p| polygon_area(p) >= floor)
        .collect())
}

/// `classify_sides`: "left" or "right" by the nearer channel line to each polygon's
/// representative point ("left" on a tie).
pub fn classify_sides(polys: &[Polygon], left: &[Pt], right: &[Pt]) -> Vec<&'static str> {
    polys
        .iter()
        .map(|p| {
            let rep = representative_point(core::slice::from_ref(p)).unwrap_or([f64::NAN; 2]);
            if linear::point_line_distance(rep, left) <= linear::point_line_distance(rep, right) {
                "left"
            } else {
                "right"
            }
        })
        .collect()
}

/// One excavation polygon (`compute_excavation` record).
#[derive(Debug, Clone, PartialEq)]
pub struct Excavation {
    pub polygon: Polygon,
    pub bank_side: &'static str,
    pub area_m2: f64,
}

/// `compute_excavation`: bank excavation polygons from the channel limits and the bank lines.
pub fn compute_excavation(
    channel_left: &[Pt],
    channel_right: &[Pt],
    bank_left: &[Pt],
    bank_right: &[Pt],
    min_area: f64,
) -> Result<Vec<Excavation>, String> {
    let channel_right = orient_like(channel_left, channel_right);
    let bank_right = orient_like(bank_left, bank_right);
    let channel = polygon_from_bank_pair(channel_left, &channel_right)?;
    let river = polygon_from_bank_pair(bank_left, &bank_right)?;
    let polys = subtract_polygons(&channel, &river, min_area)?;
    let sides = classify_sides(&polys, channel_left, &channel_right);
    Ok(polys
        .into_iter()
        .zip(sides)
        .map(|(p, s)| Excavation {
            area_m2: polygon_area(&p),
            polygon: p,
            bank_side: s,
        })
        .collect())
}

/// The inputs of `channel_footprint` beyond the lines, with the Python defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootprintParams {
    pub loa: f64,
    pub beam: f64,
    pub depth: f64,
    pub side_slope: f64,
    pub width_beam_multiple: f64,
    pub min_radius_length_multiple: f64,
    pub extra_width_coefficient: f64,
    pub radius_sample_length: f64,
    pub max_extra_width_radius: f64,
    pub min_area: f64,
}

impl FootprintParams {
    /// The defaults, for a vessel of `loa`, `beam` and channel `depth`.
    pub fn new(loa: f64, beam: f64, depth: f64) -> Self {
        FootprintParams {
            loa,
            beam,
            depth,
            side_slope: 3.0,
            width_beam_multiple: 2.8,
            min_radius_length_multiple: 3.0,
            extra_width_coefficient: 0.6,
            radius_sample_length: 10.0,
            max_extra_width_radius: 2000.0,
            min_area: 0.5,
        }
    }
}

/// The result of `channel_footprint`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelFootprint {
    pub centreline: Vec<Vec<Pt>>,
    pub channel_left: Vec<Vec<Pt>>,
    pub channel_right: Vec<Vec<Pt>>,
    pub excavation: Vec<Excavation>,
    pub diagnostics: LimitsDiagnostics,
}

impl ChannelFootprint {
    /// `total_excavation_area_m2`: Python's `sum` of the areas (the integer 0 when there are none).
    pub fn total_excavation_area_m2(&self) -> f64 {
        crate::alignment::bends::py_sum(self.excavation.iter().map(|e| e.area_m2))
    }
}

/// `channel_footprint`: channel limits at the waterline and the plan excavation outside the
/// present banks. `lines` are the centreline's lines (`extract_lines`).
pub fn channel_footprint(
    lines: Vec<Vec<Pt>>,
    bank_left: &[Pt],
    bank_right: &[Pt],
    p: &FootprintParams,
) -> Result<ChannelFootprint, String> {
    if lines.is_empty() {
        return Err("centreline has no line geometry".into());
    }
    let base_offset = (p.width_beam_multiple * p.beam) / 2.0 + p.side_slope * p.depth;
    let w = Widening {
        loa: p.loa,
        extra_width_coefficient: p.extra_width_coefficient,
        min_radius_length_multiple: p.min_radius_length_multiple,
        max_extra_width_radius: Some(p.max_extra_width_radius),
        radius_threshold: p.max_extra_width_radius,
    };
    let limits::ChannelLimits {
        left,
        right,
        diagnostics,
    } = limits::build_channel_limits(&lines, base_offset, &w, p.radius_sample_length)?;
    let longest = |ls: &[Vec<Pt>]| -> Vec<Pt> {
        let mut best: Option<(&Vec<Pt>, f64)> = None;
        for l in ls {
            let len = linear::length(l);
            if best.is_none_or(|(_, b)| len > b) {
                best = Some((l, len));
            }
        }
        best.map(|(l, _)| l.clone()).unwrap_or_default()
    };
    let excavation = compute_excavation(
        &longest(&left),
        &longest(&right),
        bank_left,
        bank_right,
        p.min_area,
    )?;
    Ok(ChannelFootprint {
        centreline: lines,
        channel_left: left,
        channel_right: right,
        excavation,
        diagnostics,
    })
}
