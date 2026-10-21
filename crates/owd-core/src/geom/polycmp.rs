//! The normalised polygon check (ADR 0005): two (multi)polygons are the same shape when their
//! areas agree within a relative tolerance and every vertex of each lies within a distance of the
//! other's boundary.
//!
//! So a ring may start at any vertex, run either way, and keep or drop a vertex on a straight
//! edge, and the polygons of a multipolygon may come in any order. A shape moved by more than the
//! vertex distance fails.

use super::overlay::polygon_area;
use super::{Polygon, Pt, linear};

fn rings(ps: &[Polygon]) -> impl Iterator<Item = &Vec<Pt>> {
    ps.iter()
        .flat_map(|p| core::iter::once(&p.exterior).chain(&p.holes))
}

/// The distance from `p` to the nearest ring of `ps`.
fn boundary_distance(p: Pt, ps: &[Polygon]) -> f64 {
    rings(ps)
        .map(|r| linear::point_line_distance(p, r))
        .fold(f64::INFINITY, f64::min)
}

/// Compare `actual` with `expected`. `Err` says what differs.
pub fn same_shape(
    expected: &[Polygon],
    actual: &[Polygon],
    area_rel: f64,
    vertex_m: f64,
) -> Result<(), String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "expected {} polygon(s), got {}",
            expected.len(),
            actual.len()
        ));
    }
    let holes = |ps: &[Polygon]| ps.iter().map(|p| p.holes.len()).sum::<usize>();
    if holes(expected) != holes(actual) {
        return Err(format!(
            "expected {} hole(s), got {}",
            holes(expected),
            holes(actual)
        ));
    }
    let (ae, aa) = (
        expected.iter().map(polygon_area).sum::<f64>(),
        actual.iter().map(polygon_area).sum::<f64>(),
    );
    if (ae - aa).abs() > area_rel * ae.abs().max(aa.abs()) {
        return Err(format!("area: expected {ae}, got {aa}"));
    }
    for (what, from, to) in [("actual", actual, expected), ("expected", expected, actual)] {
        for r in rings(from) {
            for &v in r {
                let d = boundary_distance(v, to);
                if d.is_nan() || d > vertex_m {
                    return Err(format!("{what} vertex {v:?} is {d} m from the other shape"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn square(off: f64) -> Polygon {
        Polygon {
            exterior: vec![
                [off, 0.0],
                [100.0 + off, 0.0],
                [100.0 + off, 100.0],
                [off, 100.0],
                [off, 0.0],
            ],
            holes: vec![],
        }
    }

    #[test]
    fn a_ring_that_starts_at_another_vertex_passes() {
        let a = square(0.0);
        let mut ring = a.exterior[2..4].to_vec();
        ring.extend_from_slice(&a.exterior[0..3]);
        let b = Polygon {
            exterior: ring,
            holes: vec![],
        };
        assert_eq!(b.exterior[0], [100.0, 100.0]);
        assert_eq!(same_shape(&[a], &[b], 1e-9, 1e-6), Ok(()));
    }

    #[test]
    fn a_shape_moved_by_2e_6_m_fails() {
        let r = same_shape(&[square(0.0)], &[square(2e-6)], 1e-9, 1e-6);
        assert!(r.is_err(), "{r:?}");
    }

    #[test]
    fn a_reversed_ring_with_an_extra_collinear_vertex_passes() {
        let a = square(0.0);
        let b = Polygon {
            exterior: vec![
                [0.0, 0.0],
                [0.0, 100.0],
                [100.0, 100.0],
                [100.0, 0.0],
                [50.0, 0.0],
                [0.0, 0.0],
            ],
            holes: vec![],
        };
        assert_eq!(same_shape(&[a], &[b], 1e-9, 1e-6), Ok(()));
    }
}
