//! shapely `is_valid` for polygons, as far as the checks GEOS `IsValidOp` makes that a buffer or
//! an overlay could break: closed rings of four or more points with area, no ring that crosses or
//! touches itself, rings of a polygon that meet at most at single points, holes inside their shell
//! and outside each other, and polygons whose interiors do not overlap. (Not checked: a polygon
//! interior cut in two by holes touching at several points, and two rings that cross exactly at a
//! shared vertex.)

use super::overlay::{Loc, locate, ring_signed_area};
use super::{Polygon, Pt, SegHit, offset, segment_intersection};

/// Do two different rings cross or overlap along a segment? Touching at points is allowed.
fn rings_cross(a: &[Pt], b: &[Pt]) -> bool {
    for s in a.windows(2) {
        for t in b.windows(2) {
            match segment_intersection(s[0], s[1], t[0], t[1]) {
                SegHit::None => {}
                SegHit::Overlap { .. } => return true,
                SegHit::Point { proper, .. } => {
                    if proper {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// A vertex of `ring` that is not on `other`, if any, and where it lies relative to `other` as a
/// polygon without holes.
fn inside(ring: &[Pt], other: &[Pt]) -> Option<bool> {
    let shell = Polygon {
        exterior: other.to_vec(),
        holes: vec![],
    };
    ring.iter().find_map(|&v| match locate(v, &shell) {
        Loc::Boundary => None,
        l => Some(l == Loc::Interior),
    })
}

fn ring_ok(r: &[Pt]) -> bool {
    r.len() >= 4 && r.first() == r.last() && ring_signed_area(r) != 0.0 && offset::ring_is_simple(r)
}

/// Whether the polygons form a valid (multi)polygon.
pub fn is_valid(polys: &[Polygon]) -> bool {
    for p in polys {
        if !ring_ok(&p.exterior) || !p.holes.iter().all(|h| ring_ok(h)) {
            return false;
        }
        for (i, h) in p.holes.iter().enumerate() {
            if rings_cross(&p.exterior, h) || inside(h, &p.exterior) == Some(false) {
                return false;
            }
            for g in &p.holes[i + 1..] {
                if rings_cross(h, g) || inside(h, g) == Some(true) || inside(g, h) == Some(true) {
                    return false;
                }
            }
        }
    }
    for (i, p) in polys.iter().enumerate() {
        for q in &polys[i + 1..] {
            if rings_cross(&p.exterior, &q.exterior) {
                return false;
            }
            let p_in_q = p.exterior.iter().any(|&v| locate(v, q) == Loc::Interior);
            let q_in_p = q.exterior.iter().any(|&v| locate(v, p) == Loc::Interior);
            if p_in_q || q_in_p {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn ring(pts: &[[f64; 2]]) -> Vec<Pt> {
        let mut r = pts.to_vec();
        r.push(pts[0]);
        r
    }

    #[test]
    fn a_square_with_a_hole_is_valid_and_a_bow_tie_is_not() {
        let sq = ring(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        let hole = ring(&[[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0]]);
        assert!(is_valid(&[Polygon {
            exterior: sq.clone(),
            holes: vec![hole]
        }]));
        let bow = ring(&[[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]]);
        assert!(!is_valid(&[Polygon {
            exterior: bow,
            holes: vec![]
        }]));
        let outside = ring(&[[20.0, 2.0], [24.0, 2.0], [24.0, 4.0]]);
        assert!(!is_valid(&[Polygon {
            exterior: sq,
            holes: vec![outside]
        }]));
    }

    #[test]
    fn a_ring_that_touches_itself_is_not_valid() {
        let touch = ring(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [2.0, 0.0], [0.0, 4.0]]);
        assert!(!is_valid(&[Polygon {
            exterior: touch,
            holes: vec![]
        }]));
    }
}
