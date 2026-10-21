//! `LineString.is_simple` (GEOS `IsSimpleOp`, Mod-2 boundary rule) for one open or closed line.
//!
//! A line is not simple when two of its segments meet anywhere but at the shared vertex of two
//! consecutive segments, or when it ends on itself anywhere but at its own start (a closed line
//! is simple). A collinear overlap is never simple.

use super::{Pt, SegHit, segment_intersection};

/// Whether the line is simple. Consecutive repeated points are dropped first, as GEOS does when
/// it builds the segments. A line of fewer than two distinct points is simple.
pub fn line_is_simple(line: &[Pt]) -> bool {
    let mut pts: Vec<Pt> = Vec::with_capacity(line.len());
    for &p in line {
        if pts.last() != Some(&p) {
            pts.push(p);
        }
    }
    let n = pts.len().saturating_sub(1); // segment count
    if n < 2 {
        return true;
    }
    let minx = |i: usize| pts[i][0].min(pts[i + 1][0]);
    let maxx = |i: usize| pts[i][0].max(pts[i + 1][0]);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| minx(a).total_cmp(&minx(b)));
    // `p` is the start of the whole line at its first segment, or its end at its last.
    let is_line_end =
        |seg: usize, p: Pt| (seg == 0 && p == pts[0]) || (seg == n - 1 && p == pts[n]);
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            if minx(j) > maxx(i) {
                break;
            }
            let (lo, hi) = (i.min(j), i.max(j));
            let hit = segment_intersection(pts[lo], pts[lo + 1], pts[hi], pts[hi + 1]);
            let p = match hit {
                SegHit::None => continue,
                SegHit::Overlap { .. } => return false,
                SegHit::Point { p, .. } => p,
            };
            let ends = |a: Pt, b: Pt| p == a || p == b;
            // Interior to either segment: a crossing, or an end resting on the other's interior.
            if !(ends(pts[lo], pts[lo + 1]) && ends(pts[hi], pts[hi + 1])) {
                return false;
            }
            if hi - lo <= 1 {
                continue; // consecutive segments share their vertex
            }
            // A vertex shared by two apart segments is fine only where the line meets itself end to end.
            if !(is_line_end(lo, p) && is_line_end(hi, p)) {
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

    #[test]
    fn a_bow_tie_is_not_simple_and_a_closed_ring_is() {
        assert!(!line_is_simple(&[
            [0.0, 0.0],
            [10.0, 10.0],
            [10.0, 0.0],
            [0.0, 10.0]
        ]));
        assert!(line_is_simple(&[
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 0.0]
        ]));
    }

    #[test]
    fn an_end_resting_on_the_line_and_a_doubled_back_line_are_not_simple() {
        assert!(!line_is_simple(&[
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [5.0, 0.0]
        ]));
        assert!(!line_is_simple(&[[0.0, 0.0], [10.0, 0.0], [5.0, 0.0]]));
    }

    #[test]
    fn a_line_with_repeated_points_is_judged_on_its_distinct_points() {
        assert!(line_is_simple(&[
            [0.0, 0.0],
            [0.0, 0.0],
            [5.0, 0.0],
            [5.0, 0.0],
            [5.0, 5.0]
        ]));
        assert!(line_is_simple(&[[1.0, 1.0], [1.0, 1.0]]));
    }
}
