//! Plane geometry, matched to shapely (GEOS 3.14) where the match changes a number (ADR 0005).
//!
//! Each function names the GEOS routine it follows. Line vertices come out equal to GEOS's within
//! 1e-9: the offset curve, the line buffer and the point circle are built vertex for vertex.
//! A buffer whose raw curve folds back on itself, and `buffer(0)` repair, are resolved by GEOS's
//! depth rule (`depth`). Those polygons, and polygon overlay, may start a ring at another vertex
//! than GEOS's, so they are compared normalised (`polycmp`).

pub mod dd;
pub mod depth;
pub mod interior;
pub mod linear;
pub mod merge;
pub mod offset;
pub mod overlay;
pub mod polycmp;
pub mod simple;
pub mod valid;

/// A point in the plane.
pub type Pt = [f64; 2];

/// A polygon: one exterior ring and any holes. Rings are closed (first point repeated at the end).
#[derive(Debug, Clone, PartialEq)]
pub struct Polygon {
    pub exterior: Vec<Pt>,
    pub holes: Vec<Vec<Pt>>,
}

/// A geometry, as far as the package needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Point(Pt),
    LineString(Vec<Pt>),
    Polygon(Polygon),
    MultiPoint(Vec<Pt>),
    MultiLineString(Vec<Vec<Pt>>),
    MultiPolygon(Vec<Polygon>),
    Collection(Vec<Geometry>),
    /// An empty geometry of the named kind (`"Point"`, `"LineString"`, `"Polygon"`, ...).
    Empty(&'static str),
}

/// Why a geometry function could not give a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeomError {
    /// The input is of a kind this function does not handle.
    Unsupported(String),
}

impl core::fmt::Display for GeomError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GeomError::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for GeomError {}

/// GEOS `Coordinate::distance`: `sqrt(dx*dx + dy*dy)` (not `hypot`).
pub(crate) fn dist(a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    (dx * dx + dy * dy).sqrt()
}

/// GEOS `Orientation::index` (`CGAlgorithmsDD::orientationIndex`): +1 if `q` is left of
/// `p1 -> p2` (counter-clockwise), -1 if right, 0 if collinear. A fast filter, then the
/// determinant in double-double, as GEOS does.
pub(crate) fn orientation(p1: Pt, p2: Pt, q: Pt) -> i32 {
    let detleft = (p1[0] - q[0]) * (p2[1] - q[1]);
    let detright = (p1[1] - q[1]) * (p2[0] - q[0]);
    let det = detleft - detright;
    let error = (detleft + detright).abs() * 3.330_669_062_177_372_4e-16;
    if det.abs() >= error {
        return (det > 0.0) as i32 - (det < 0.0) as i32;
    }
    use dd::Dd;
    let dx1 = Dd::new(p2[0]).plus(Dd::new(-p1[0]));
    let dy1 = Dd::new(p2[1]).plus(Dd::new(-p1[1]));
    let dx2 = Dd::new(q[0]).plus(Dd::new(-p2[0]));
    let dy2 = Dd::new(q[1]).plus(Dd::new(-p2[1]));
    dx1.times(dy2).minus(dy1.times(dx2)).signum()
}

/// GEOS `Envelope::intersects(p1, p2, q)`: `q` inside the box of `p1`, `p2` (edges included).
fn env_contains(p1: Pt, p2: Pt, q: Pt) -> bool {
    q[0] >= p1[0].min(p2[0])
        && q[0] <= p1[0].max(p2[0])
        && q[1] >= p1[1].min(p2[1])
        && q[1] <= p1[1].max(p2[1])
}

/// GEOS `Envelope::intersects(p1, p2, q1, q2)`: the boxes of two segments meet.
fn envs_meet(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> bool {
    let (minq, maxq) = (q1[0].min(q2[0]), q1[0].max(q2[0]));
    let (minp, maxp) = (p1[0].min(p2[0]), p1[0].max(p2[0]));
    if minp > maxq || maxp < minq {
        return false;
    }
    let (minq, maxq) = (q1[1].min(q2[1]), q1[1].max(q2[1]));
    let (minp, maxp) = (p1[1].min(p2[1]), p1[1].max(p2[1]));
    !(minp > maxq || maxp < minq)
}

/// How two segments meet (GEOS `LineIntersector::computeIntersection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SegHit {
    None,
    /// One point; `proper` when it is interior to both segments.
    Point {
        p: Pt,
        proper: bool,
    },
    /// A collinear overlap from `a` to `b`.
    Overlap {
        a: Pt,
        b: Pt,
    },
}

/// GEOS `LineIntersector::computeIntersect` for segments `p1-p2` and `q1-q2`.
pub(crate) fn segment_intersection(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> SegHit {
    if !envs_meet(p1, p2, q1, q2) {
        return SegHit::None;
    }
    let pq1 = orientation(p1, p2, q1);
    let pq2 = orientation(p1, p2, q2);
    if (pq1 > 0 && pq2 > 0) || (pq1 < 0 && pq2 < 0) {
        return SegHit::None;
    }
    let qp1 = orientation(q1, q2, p1);
    let qp2 = orientation(q1, q2, p2);
    if (qp1 > 0 && qp2 > 0) || (qp1 < 0 && qp2 < 0) {
        return SegHit::None;
    }
    if pq1 == 0 && pq2 == 0 && qp1 == 0 && qp2 == 0 {
        return collinear_intersection(p1, p2, q1, q2);
    }
    if pq1 == 0 || pq2 == 0 || qp1 == 0 || qp2 == 0 {
        let p = if p1 == q1 || p1 == q2 {
            p1
        } else if p2 == q1 || p2 == q2 {
            p2
        } else if pq1 == 0 {
            q1
        } else if pq2 == 0 {
            q2
        } else if qp1 == 0 {
            p1
        } else {
            p2
        };
        return SegHit::Point { p, proper: false };
    }
    SegHit::Point {
        p: proper_point(p1, p2, q1, q2),
        proper: true,
    }
}

/// GEOS `LineIntersector::computeCollinearIntersection`.
fn collinear_intersection(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> SegHit {
    let q1in_p = env_contains(p1, p2, q1);
    let q2in_p = env_contains(p1, p2, q2);
    let p1in_q = env_contains(q1, q2, p1);
    let p2in_q = env_contains(q1, q2, p2);
    let pair = |a: Pt, b: Pt, single: bool| {
        if single {
            SegHit::Point {
                p: a,
                proper: false,
            }
        } else {
            SegHit::Overlap { a, b }
        }
    };
    if q1in_p && q2in_p {
        return pair(q1, q2, false);
    }
    if p1in_q && p2in_q {
        return pair(p1, p2, false);
    }
    if q1in_p && p1in_q {
        return pair(q1, p1, q1 == p1 && !q2in_p && !p2in_q);
    }
    if q1in_p && p2in_q {
        return pair(q1, p2, q1 == p2 && !q2in_p && !p1in_q);
    }
    if q2in_p && p1in_q {
        return pair(q2, p1, q2 == p1 && !q1in_p && !p2in_q);
    }
    if q2in_p && p2in_q {
        return pair(q2, p2, q2 == p2 && !q1in_p && !p1in_q);
    }
    SegHit::None
}

/// GEOS `LineIntersector::intersection`: the double-double crossing point, or the nearest
/// endpoint when that point falls outside either segment's box.
fn proper_point(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> Pt {
    let p =
        dd::line_intersection(p1, p2, q1, q2).unwrap_or_else(|| nearest_endpoint(p1, p2, q1, q2));
    if env_contains(p1, p2, p) && env_contains(q1, q2, p) {
        p
    } else {
        nearest_endpoint(p1, p2, q1, q2)
    }
}

/// GEOS `LineIntersector::nearestEndpoint`.
fn nearest_endpoint(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> Pt {
    use linear::point_segment_distance as psd;
    let mut nearest = p1;
    let mut min = psd(p1, q1, q2);
    for (p, d) in [
        (p2, psd(p2, q1, q2)),
        (q1, psd(q1, p1, p2)),
        (q2, psd(q2, p1, p2)),
    ] {
        if d < min {
            min = d;
            nearest = p;
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn orientation_is_exact_on_nearly_collinear_points() {
        assert_eq!(orientation([0.0, 0.0], [1.0, 1.0], [2.0, 2.0]), 0);
        assert_eq!(orientation([0.0, 0.0], [1.0, 0.0], [0.5, 1e-300]), 1);
        assert_eq!(orientation([0.0, 0.0], [1.0, 0.0], [0.5, -1e-300]), -1);
        // A point a hair off the diagonal, where the filter hands over to double-double.
        let (a, b) = ([0.5, 0.5], [12.0, 12.0]);
        let q = [24.0, 24.000_000_000_000_004];
        assert_eq!(orientation(a, b, q), 1);
    }

    #[test]
    fn crossing_segments_meet_at_one_proper_point() {
        let hit = segment_intersection([0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]);
        assert_eq!(
            hit,
            SegHit::Point {
                p: [1.0, 1.0],
                proper: true
            }
        );
    }

    #[test]
    fn collinear_segments_overlap() {
        let hit = segment_intersection([0.0, 0.0], [20.0, 0.0], [10.0, 0.0], [30.0, 0.0]);
        assert_eq!(
            hit,
            SegHit::Overlap {
                a: [10.0, 0.0],
                b: [20.0, 0.0]
            }
        );
        let touch = segment_intersection([0.0, 0.0], [10.0, 0.0], [10.0, 0.0], [30.0, 0.0]);
        assert_eq!(
            touch,
            SegHit::Point {
                p: [10.0, 0.0],
                proper: false
            }
        );
    }
}
