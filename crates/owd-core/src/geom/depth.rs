//! The polygon a set of closed curves encloses, by GEOS's buffer depth rule (`BufferBuilder`,
//! `BufferSubgraph::computeDepths` and `findResultEdges`).
//!
//! GEOS labels each raw curve so that crossing it towards its interior side adds one to the depth,
//! sums the labels of coincident edges, and keeps the faces of depth one or more. That depth is the
//! signed winding number of the curves: `depth(p) = sum of c * winding(p, curve)`, with the
//! winding counted counter-clockwise and `c` = +1 for a curve whose interior is on its left
//! (counter-clockwise) and -1 for one whose interior is on its right (clockwise).
//!
//! The steps: node every segment against every other, merge coincident pieces (their coefficients
//! add), find the depth on each side of each piece by an exact ray count, keep the pieces that
//! separate depth one or more from depth zero or less, and link them into rings. Faces match GEOS's;
//! a ring may start at another vertex, so results are compared normalised (ADR 0005).

use super::overlay::{build_polygons, split};
use super::{Polygon, Pt, SegHit, orientation, segment_intersection};
use std::collections::BTreeMap;

/// A closed curve and its coefficient: +1 if its interior is on its left, -1 if on its right.
pub type Curve = (Vec<Pt>, i32);

/// An exact coordinate key.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key(Pt);
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> core::cmp::Ordering {
        self.0[0]
            .total_cmp(&o.0[0])
            .then(self.0[1].total_cmp(&o.0[1]))
    }
}

/// Split every segment where it meets any other (GEOS's buffer noding, in floating point).
fn node_all(edges: &[[Pt; 2]]) -> Vec<Vec<Pt>> {
    let mut cuts: Vec<Vec<Pt>> = vec![Vec::new(); edges.len()];
    let minx = |e: &[Pt; 2]| e[0][0].min(e[1][0]);
    let maxx = |e: &[Pt; 2]| e[0][0].max(e[1][0]);
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by(|&i, &j| minx(&edges[i]).total_cmp(&minx(&edges[j])));
    for (k, &i) in order.iter().enumerate() {
        let ei = &edges[i];
        for &j in &order[k + 1..] {
            let ej = &edges[j];
            if minx(ej) > maxx(ei) {
                break;
            }
            match segment_intersection(ei[0], ei[1], ej[0], ej[1]) {
                SegHit::None => {}
                SegHit::Point { p, .. } => {
                    cuts[i].push(p);
                    cuts[j].push(p);
                }
                SegHit::Overlap { a, b } => {
                    cuts[i].extend([a, b]);
                    cuts[j].extend([a, b]);
                }
            }
        }
    }
    cuts
}

/// The contribution of edge `a -> b` (coefficient `c`) to the winding number at `m`, counted with a
/// ray towards +x and a half-open rule on y.
fn crossing(a: Pt, b: Pt, c: i32, m: Pt) -> i32 {
    if a[1] <= m[1] && b[1] > m[1] && orientation(a, b, m) > 0 {
        c
    } else if b[1] <= m[1] && a[1] > m[1] && orientation(a, b, m) < 0 {
        -c
    } else {
        0
    }
}

/// The pieces grouped by horizontal strips: each piece is listed in every strip its y range meets.
/// [`crossing`] is zero for a piece unless `min y <= m[1] < max y`, and such a piece is listed in the
/// strip of `m[1]` (the strip of a y is monotone in y), so summing over that strip's pieces gives the
/// same integer as summing over all of them.
struct Strips {
    y0: f64,
    h: f64,
    strips: Vec<Vec<usize>>,
}

impl Strips {
    fn new(set: &[(Pt, Pt, i32)]) -> Strips {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(a, b, _) in set {
            lo = lo.min(a[1].min(b[1]));
            hi = hi.max(a[1].max(b[1]));
        }
        // About one strip a piece; fewer if long pieces would list each in many strips.
        let mut n = set.len().max(1);
        loop {
            let h = if hi > lo { (hi - lo) / n as f64 } else { 1.0 };
            let mut out = Strips {
                y0: lo,
                h,
                strips: vec![Vec::new(); n],
            };
            let spans: Vec<(usize, usize)> = set
                .iter()
                .map(|&(a, b, _)| (out.strip(a[1].min(b[1])), out.strip(a[1].max(b[1]))))
                .collect();
            let entries: usize = spans.iter().map(|(k0, k1)| k1 - k0 + 1).sum();
            if n > 1 && entries > 16 * set.len() {
                n /= 2;
                continue;
            }
            for (j, &(k0, k1)) in spans.iter().enumerate() {
                for k in k0..=k1 {
                    out.strips[k].push(j);
                }
            }
            return out;
        }
    }

    /// The strip of `y`, clamped to the strips there are.
    fn strip(&self, y: f64) -> usize {
        let k = ((y - self.y0) / self.h).floor();
        if k.is_nan() || k < 0.0 {
            0
        } else {
            (k as usize).min(self.strips.len().max(1) - 1)
        }
    }

    /// The winding number at `m` of every piece of `set` but piece `skip`, as the full sum gives it.
    fn winding(&self, set: &[(Pt, Pt, i32)], m: Pt, skip: usize) -> i32 {
        self.strips[self.strip(m[1])]
            .iter()
            .filter(|&&j| j != skip)
            .map(|&j| {
                let (p, q, k) = set[j];
                crossing(p, q, k, m)
            })
            .sum()
    }
}

/// A quarter turn clockwise, `(x, y) -> (y, -x)`: exact, and it keeps orientation.
fn turn(p: Pt) -> Pt {
    [p[1], -p[0]]
}

/// The region of depth one or more, as polygons with the interior on the left of every ring
/// (shells counter-clockwise, holes clockwise).
pub fn region(curves: &[Curve]) -> Vec<Polygon> {
    let mut edges: Vec<[Pt; 2]> = Vec::new();
    let mut coef: Vec<i32> = Vec::new();
    for (pts, c) in curves {
        for w in pts.windows(2) {
            if w[0] != w[1] {
                edges.push([w[0], w[1]]);
                coef.push(*c);
            }
        }
    }
    let cuts = node_all(&edges);
    // Pieces, merged by their (unordered) ends; each runs from its lower to its higher end.
    let mut merged: BTreeMap<(Key, Key), i32> = BTreeMap::new();
    for ((e, cut), c) in edges.iter().zip(cuts).zip(&coef) {
        for piece in split(core::slice::from_ref(e), vec![cut]) {
            let (a, b) = (Key(piece[0]), Key(piece[1]));
            let (key, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
            *merged.entry(key).or_insert(0) += sign * c;
        }
    }
    let pieces: Vec<(Pt, Pt, i32)> = merged
        .into_iter()
        .filter(|(_, c)| *c != 0)
        .map(|((a, b), c)| (a.0, b.0, c))
        .collect();
    let turned: Vec<(Pt, Pt, i32)> = pieces
        .iter()
        .map(|&(a, b, c)| (turn(a), turn(b), c))
        .collect();
    // The ray counts read only the pieces whose y range holds the ray (see `Strips`); a scan of
    // every piece for every piece was quadratic, most of an alignment's time on a long river.
    let (strips, turned_strips) = (Strips::new(&pieces), Strips::new(&turned));
    let mut boundary: Vec<[Pt; 2]> = Vec::new();
    for (i, &(a, b, c)) in pieces.iter().enumerate() {
        // A horizontal piece is counted in the turned frame, where it is vertical.
        let (set, index, a, b) = if a[1] == b[1] {
            (&turned, &turned_strips, turn(a), turn(b))
        } else {
            (&pieces, &strips, a, b)
        };
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let w: i32 = index.winding(set, m, i);
        // `w` is the depth just to the +x side of the piece. Going up, that is its right side.
        let (left, right) = if a[1] < b[1] { (w + c, w) } else { (w, w - c) };
        let (lo, hi) = (pieces[i].0, pieces[i].1);
        match (left > 0, right > 0) {
            (true, false) => boundary.push([lo, hi]),
            (false, true) => boundary.push([hi, lo]),
            _ => {}
        }
    }
    build_polygons(&boundary)
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (areas of unit squares and triangles)
    use super::*;

    #[test]
    fn the_strips_count_the_ray_as_the_full_scan_does() {
        // Random pieces on a coarse grid (so ends share y values and some pieces are horizontal),
        // queried at their own midpoints and at grid points; the sums must be equal.
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 41) as f64 * 0.5 - 10.0
        };
        let mut queries = 0;
        for round in 0..40 {
            let n = 5 + round * 7;
            let set: Vec<(Pt, Pt, i32)> = (0..n)
                .map(|k| {
                    (
                        [next(), next()],
                        [next(), next()],
                        if k % 3 == 0 { -1 } else { 1 },
                    )
                })
                .collect();
            let strips = Strips::new(&set);
            let full = |m: Pt, skip: usize| -> i32 {
                set.iter()
                    .enumerate()
                    .filter(|(j, _)| *j != skip)
                    .map(|(_, &(p, q, k))| crossing(p, q, k, m))
                    .sum()
            };
            for (i, &(a, b, _)) in set.iter().enumerate() {
                let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                assert_eq!(strips.winding(&set, m, i), full(m, i), "{m:?}");
                let g = [next(), next()];
                assert_eq!(strips.winding(&set, g, usize::MAX), full(g, usize::MAX));
                queries += 2;
            }
        }
        assert!(queries > 5000, "{queries}");
    }
    use crate::geom::overlay::polygon_area;

    fn area(ps: &[Polygon]) -> f64 {
        ps.iter().map(polygon_area).sum()
    }

    #[test]
    fn a_clockwise_square_with_coefficient_minus_one_is_itself() {
        let sq = vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let r = region(&[(sq, -1)]);
        assert_eq!(r.len(), 1);
        assert_eq!(area(&r), 1.0);
    }

    #[test]
    fn a_bow_tie_keeps_only_the_lobe_of_positive_depth() {
        // Lobes: the left one is counter-clockwise, the right one clockwise.
        let bow = vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0], [0.0, 0.0]];
        let ccw = region(&[(bow.clone(), 1)]);
        let cw = region(&[(bow, -1)]);
        assert_eq!((ccw.len(), area(&ccw)), (1, 1.0));
        assert_eq!((cw.len(), area(&cw)), (1, 1.0));
        assert_ne!(ccw, cw);
    }

    #[test]
    fn overlapping_squares_of_the_same_sense_merge_and_a_hole_cuts_out() {
        let a = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]];
        let b = vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0], [1.0, 1.0]];
        assert_eq!(area(&region(&[(a.clone(), 1), (b, 1)])), 7.0);
        let hole = vec![[0.5, 0.5], [1.5, 0.5], [1.5, 1.5], [0.5, 1.5], [0.5, 0.5]];
        let r = region(&[(a, 1), (hole, -1)]);
        assert_eq!((r.len(), r[0].holes.len(), area(&r)), (1, 1, 3.0));
    }
}
