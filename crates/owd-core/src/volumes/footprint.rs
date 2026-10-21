//! Excavation footprint in plan: the design channel polygon minus the present river polygon.
//! Mirrors `volumes/footprint.py`. Areas only (m²).

use crate::alignment::limits::{self, LimitsDiagnostics, Widening};
use crate::geom::interior::representative_point;
use crate::geom::offset::buffer_polygons;
use crate::geom::overlay::{self, Op, polygon_area};
use crate::geom::{Geometry, Polygon, Pt, SegHit, dist, linear, segment_intersection, valid};

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

/// The polygon between two bank lines, whichever way the second one runs (spec 0005 Q12).
#[derive(Debug, Clone, PartialEq)]
pub struct BankPairPolygon {
    pub polygons: Vec<Polygon>,
    /// The second bank was reversed to run like the first.
    pub reoriented: bool,
}

/// The first point where two lines meet, other than an end they share (the closed-pair case).
fn bank_crossing(left: &[Pt], right: &[Pt]) -> Option<Pt> {
    let ends = [left[0], left[left.len() - 1]];
    let shared = |p: Pt| ends.contains(&p) && (p == right[0] || p == right[right.len() - 1]);
    for a in left.windows(2) {
        for b in right.windows(2) {
            match segment_intersection(a[0], a[1], b[0], b[1]) {
                SegHit::None => {}
                SegHit::Point { p, .. } if shared(p) => {}
                SegHit::Point { p, .. } => return Some(p),
                SegHit::Overlap { a: p, .. } => return Some(p),
            }
        }
    }
    None
}

/// `polygon_from_bank_pair` with the second bank oriented like the first (`orient_like`), so a
/// reversed second bank gives the same polygon as a correctly ordered pair. Banks that cross are
/// refused, with the crossing point in the message.
///
/// `polygon_from_bank_pair` itself stays the frozen-Python behaviour until the port closes (#37):
/// its parity cases hold a reversed pair and a crossing pair.
pub fn bank_pair_polygon(left: &[Pt], right: &[Pt]) -> Result<BankPairPolygon, String> {
    if left.len() < 2 || right.len() < 2 {
        return Err("each bank needs at least two points".into());
    }
    let oriented = orient_like(left, right);
    let reoriented = oriented.as_slice() != right;
    if let Some(p) = bank_crossing(left, &oriented) {
        return Err(format!(
            "the banks cross at ({:.3}, {:.3}); check the bank lines",
            p[0], p[1]
        ));
    }
    Ok(BankPairPolygon {
        polygons: polygon_from_bank_pair(left, &oriented)?,
        reoriented,
    })
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

#[cfg(test)]
mod bank_pair_tests {
    //! Label: sourced - hand calculation (a 500 m by 40 m strip, crossing diagonals)
    use super::*;

    const LEFT: [Pt; 3] = [[0.0, 20.0], [250.0, 25.0], [500.0, 20.0]];
    const RIGHT: [Pt; 3] = [[0.0, -20.0], [250.0, -15.0], [500.0, -20.0]];

    #[test]
    fn a_reversed_second_bank_gives_the_same_polygon_and_says_so() {
        let ordered = bank_pair_polygon(&LEFT, &RIGHT).unwrap();
        let mut rev = RIGHT.to_vec();
        rev.reverse();
        let reversed = bank_pair_polygon(&LEFT, &rev).unwrap();
        assert!(!ordered.reoriented);
        assert!(reversed.reoriented);
        assert_eq!(ordered.polygons, reversed.polygons);
        let area: f64 = reversed.polygons.iter().map(polygon_area).sum();
        assert!((area - 500.0 * 40.0).abs() < 1e-6, "{area}");
        // Without the orientation the reversed pair is a bow-tie that repairs to a part.
        let bow: f64 = polygon_from_bank_pair(&LEFT, &rev)
            .unwrap()
            .iter()
            .map(polygon_area)
            .sum();
        assert!(bow < area / 2.0 + 1.0);
    }

    #[test]
    fn a_correctly_ordered_pair_gives_todays_polygon() {
        for (l, r) in [
            (LEFT.to_vec(), RIGHT.to_vec()),
            (
                vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0]],
            ),
        ] {
            let new = bank_pair_polygon(&l, &r).unwrap();
            assert!(!new.reoriented);
            assert_eq!(new.polygons, polygon_from_bank_pair(&l, &r).unwrap());
        }
    }

    #[test]
    fn banks_that_cross_are_refused_naming_the_crossing_point() {
        let e = bank_pair_polygon(&[[0.0, 0.0], [100.0, 100.0]], &[[0.0, 100.0], [100.0, 0.0]])
            .unwrap_err();
        assert!(e.contains("(50.000, 50.000)"), "{e}");
        // A bank that dips across the other one.
        let e =
            bank_pair_polygon(&LEFT, &[[0.0, -20.0], [250.0, 30.0], [500.0, -20.0]]).unwrap_err();
        assert!(e.contains("cross"), "{e}");
    }
}
