//! Polygon area, point location and polygon overlay (intersection, difference, union).
//!
//! Area and location follow GEOS (`Area::ofRingSigned`, `RayCrossingCounter`). The overlay is
//! not GEOS's: it nodes the two inputs against each other in floating point, keeps the edges each
//! operation needs, and links them into rings. Its result has the same shape as GEOS's but may
//! start a ring elsewhere or keep a collinear vertex GEOS drops, so it is compared normalised
//! (ADR 0005, `polycmp`). Inputs must be valid polygons.

use super::{Geometry, Polygon, Pt, SegHit, orientation, segment_intersection};
use std::collections::BTreeMap;

/// Where a point lies relative to a polygon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loc {
    Interior,
    Boundary,
    Exterior,
}

/// GEOS `Area::ofRingSigned`: positive for a clockwise ring (GEOS's sign).
pub fn ring_signed_area(ring: &[Pt]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let x0 = ring[0][0];
    let mut sum = 0.0;
    for i in 1..n - 1 {
        let x = ring[i][0] - x0;
        sum += x * (ring[i - 1][1] - ring[i + 1][1]);
    }
    sum / 2.0
}

/// GEOS `Polygon::getArea`: the shell's area less the holes'.
pub fn polygon_area(p: &Polygon) -> f64 {
    let mut a = ring_signed_area(&p.exterior).abs();
    for h in &p.holes {
        a -= ring_signed_area(h).abs();
    }
    a
}

/// GEOS `RayCrossingCounter` over one ring: `None` if `p` is on the ring, else whether it is inside.
fn ring_contains(ring: &[Pt], p: Pt) -> Option<bool> {
    let mut crossings = 0u32;
    for w in ring.windows(2) {
        let (p1, p2) = (w[0], w[1]);
        if p1[0] < p[0] && p2[0] < p[0] {
            continue;
        }
        if p == p2 {
            return None;
        }
        if p1[1] == p[1] && p2[1] == p[1] {
            let (lo, hi) = (p1[0].min(p2[0]), p1[0].max(p2[0]));
            if p[0] >= lo && p[0] <= hi {
                return None;
            }
            continue;
        }
        if (p1[1] > p[1] && p2[1] <= p[1]) || (p2[1] > p[1] && p1[1] <= p[1]) {
            let mut orient = orientation(p1, p2, p);
            if orient == 0 {
                return None;
            }
            if p2[1] < p1[1] {
                orient = -orient;
            }
            if orient == 1 {
                crossings += 1;
            }
        }
    }
    Some(crossings % 2 == 1)
}

/// GEOS `IndexedPointInAreaLocator::locate` for one polygon.
pub fn locate(p: Pt, poly: &Polygon) -> Loc {
    match ring_contains(&poly.exterior, p) {
        None => return Loc::Boundary,
        Some(false) => return Loc::Exterior,
        Some(true) => {}
    }
    for h in &poly.holes {
        match ring_contains(h, p) {
            None => return Loc::Boundary,
            Some(true) => return Loc::Exterior,
            Some(false) => {}
        }
    }
    Loc::Interior
}

/// Location relative to a set of polygons that do not overlap (a valid multipolygon).
pub fn locate_all(p: Pt, polys: &[Polygon]) -> Loc {
    let mut out = Loc::Exterior;
    for poly in polys {
        match locate(p, poly) {
            Loc::Interior => return Loc::Interior,
            Loc::Boundary => out = Loc::Boundary,
            Loc::Exterior => {}
        }
    }
    out
}

/// shapely `contains_xy(polygon, x, y)`: strictly inside (the boundary is not contained).
pub fn contains_xy(polys: &[Polygon], p: Pt) -> bool {
    locate_all(p, polys) == Loc::Interior
}

/// shapely `geom.distance(polygon)` for a point: zero inside or on it, else the distance to the
/// nearest ring.
pub fn point_polygon_distance(p: Pt, polys: &[Polygon]) -> f64 {
    if locate_all(p, polys) != Loc::Exterior {
        return 0.0;
    }
    let mut best = f64::INFINITY;
    for poly in polys {
        for ring in core::iter::once(&poly.exterior).chain(&poly.holes) {
            best = best.min(super::linear::point_line_distance(p, ring));
        }
    }
    best
}

/// A polygon overlay operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Intersection,
    Difference,
    Union,
}

/// Rings oriented with the interior on the left: shells counter-clockwise, holes clockwise.
fn oriented_edges(polys: &[Polygon]) -> Vec<[Pt; 2]> {
    let mut edges = Vec::new();
    for poly in polys {
        for (k, ring) in core::iter::once(&poly.exterior)
            .chain(&poly.holes)
            .enumerate()
        {
            // GEOS sign: positive is clockwise.
            let cw = ring_signed_area(ring) > 0.0;
            let want_cw = k > 0;
            let flip = cw != want_cw;
            for w in ring.windows(2) {
                if w[0] != w[1] {
                    edges.push(if flip { [w[1], w[0]] } else { [w[0], w[1]] });
                }
            }
        }
    }
    edges
}

/// Split every edge of `a` at the points where it meets an edge of `b`, and the other way round.
fn node(a: &[[Pt; 2]], b: &[[Pt; 2]]) -> (Vec<[Pt; 2]>, Vec<[Pt; 2]>) {
    let mut cuts_a: Vec<Vec<Pt>> = vec![Vec::new(); a.len()];
    let mut cuts_b: Vec<Vec<Pt>> = vec![Vec::new(); b.len()];
    let minx = |e: &[Pt; 2]| e[0][0].min(e[1][0]);
    let maxx = |e: &[Pt; 2]| e[0][0].max(e[1][0]);
    let mut order_b: Vec<usize> = (0..b.len()).collect();
    order_b.sort_by(|&i, &j| minx(&b[i]).total_cmp(&minx(&b[j])));
    let bmins: Vec<f64> = order_b.iter().map(|&j| minx(&b[j])).collect();
    for (i, ea) in a.iter().enumerate() {
        let hi = bmins.partition_point(|&m| m <= maxx(ea));
        for &j in &order_b[..hi] {
            let eb = &b[j];
            if maxx(eb) < minx(ea) {
                continue;
            }
            match segment_intersection(ea[0], ea[1], eb[0], eb[1]) {
                SegHit::None => {}
                SegHit::Point { p, .. } => {
                    cuts_a[i].push(p);
                    cuts_b[j].push(p);
                }
                SegHit::Overlap { a: p, b: q } => {
                    cuts_a[i].extend([p, q]);
                    cuts_b[j].extend([p, q]);
                }
            }
        }
    }
    (split(a, cuts_a), split(b, cuts_b))
}

fn split(edges: &[[Pt; 2]], cuts: Vec<Vec<Pt>>) -> Vec<[Pt; 2]> {
    let mut out = Vec::new();
    for (e, mut c) in edges.iter().zip(cuts) {
        let d2 = |p: &Pt| (p[0] - e[0][0]) * (p[0] - e[0][0]) + (p[1] - e[0][1]) * (p[1] - e[0][1]);
        c.retain(|p| p != &e[0] && p != &e[1]);
        c.sort_by(|p, q| d2(p).total_cmp(&d2(q)));
        c.dedup();
        let mut prev = e[0];
        for p in c.into_iter().chain(core::iter::once(e[1])) {
            if p != prev {
                out.push([prev, p]);
                prev = p;
            }
        }
    }
    out
}

/// A coordinate as an exact map key.
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

fn mid(e: &[Pt; 2]) -> Pt {
    [(e[0][0] + e[1][0]) / 2.0, (e[0][1] + e[1][1]) / 2.0]
}

/// shapely `a.intersection(b)`, `a.difference(b)` or `a.union(b)` for valid (multi)polygons, as a
/// geometry. As GEOS does, an intersection with no area is the shared edges, merged into lines, or
/// failing those the points where the boundaries touch.
pub fn overlay_geometry(a: &[Polygon], b: &[Polygon], op: Op) -> Geometry {
    let mut polys = overlay(a, b, op);
    match polys.len() {
        1 => return Geometry::Polygon(polys.remove(0)),
        n if n > 1 => return Geometry::MultiPolygon(polys),
        _ => {}
    }
    if op != Op::Intersection {
        return Geometry::Empty("Polygon");
    }
    let (ea, eb) = node(&oriented_edges(a), &oriented_edges(b));
    let rev: BTreeMap<(Key, Key), ()> = eb.iter().map(|e| ((Key(e[1]), Key(e[0])), ())).collect();
    let shared: Vec<Vec<Pt>> = ea
        .iter()
        .filter(|e| rev.contains_key(&(Key(e[0]), Key(e[1]))))
        .map(|e| e.to_vec())
        .collect();
    let mut lines = super::merge::line_merge(&shared);
    match lines.len() {
        1 => return Geometry::LineString(lines.remove(0)),
        n if n > 1 => return Geometry::MultiLineString(lines),
        _ => {}
    }
    let vb: BTreeMap<Key, ()> = eb.iter().map(|e| (Key(e[0]), ())).collect();
    let mut touch: Vec<Pt> = Vec::new();
    for e in &ea {
        if vb.contains_key(&Key(e[0])) && !touch.contains(&e[0]) {
            touch.push(e[0]);
        }
    }
    match touch.len() {
        0 => Geometry::Empty("Polygon"),
        1 => Geometry::Point(touch[0]),
        _ => Geometry::MultiPoint(touch),
    }
}

/// The polygon part of `overlay_geometry`: a list of polygons (empty when there is no area).
pub fn overlay(a: &[Polygon], b: &[Polygon], op: Op) -> Vec<Polygon> {
    let (ea, eb) = node(&oriented_edges(a), &oriented_edges(b));
    let set_b: BTreeMap<(Key, Key), ()> = eb.iter().map(|e| ((Key(e[0]), Key(e[1])), ())).collect();
    let set_a: BTreeMap<(Key, Key), ()> = ea.iter().map(|e| ((Key(e[0]), Key(e[1])), ())).collect();
    let has = |s: &BTreeMap<(Key, Key), ()>, p: Pt, q: Pt| s.contains_key(&(Key(p), Key(q)));
    let mut result: Vec<[Pt; 2]> = Vec::new();
    for e in &ea {
        let same = has(&set_b, e[0], e[1]);
        let opposite = has(&set_b, e[1], e[0]);
        let keep = if same {
            op != Op::Difference
        } else if opposite {
            op == Op::Difference
        } else {
            let inside_b = locate_all(mid(e), b) == Loc::Interior;
            match op {
                Op::Intersection => inside_b,
                Op::Difference | Op::Union => !inside_b,
            }
        };
        if keep {
            result.push(*e);
        }
    }
    for e in &eb {
        if has(&set_a, e[0], e[1]) || has(&set_a, e[1], e[0]) {
            continue;
        }
        let inside_a = locate_all(mid(e), a) == Loc::Interior;
        match op {
            Op::Intersection if inside_a => result.push(*e),
            Op::Union if !inside_a => result.push(*e),
            Op::Difference if inside_a => result.push([e[1], e[0]]),
            _ => {}
        }
    }
    build_polygons(&result)
}

/// GEOS `Quadrant::quadrant`.
fn quadrant(dx: f64, dy: f64) -> i32 {
    if dx >= 0.0 {
        if dy >= 0.0 { 0 } else { 3 }
    } else if dy >= 0.0 {
        1
    } else {
        2
    }
}

/// Order of two directions from the same point, counter-clockwise from east (GEOS
/// `DirectedEdge::compareDirection`).
fn compare_dir(o: Pt, a: Pt, b: Pt) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    let (qa, qb) = (
        quadrant(a[0] - o[0], a[1] - o[1]),
        quadrant(b[0] - o[0], b[1] - o[1]),
    );
    if qa != qb {
        return qa.cmp(&qb);
    }
    match orientation(o, b, a) {
        1 => Greater,
        -1 => Less,
        _ => Equal,
    }
}

/// Link directed edges (interior on the left) into rings, taking the sharpest left turn at each
/// node, and group them into shells (counter-clockwise) with their holes.
fn build_polygons(edges: &[[Pt; 2]]) -> Vec<Polygon> {
    let mut out_of: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for (i, e) in edges.iter().enumerate() {
        out_of.entry(Key(e[0])).or_default().push(i);
    }
    for (k, outs) in out_of.iter_mut() {
        outs.sort_by(|&x, &y| compare_dir(k.0, edges[x][1], edges[y][1]));
    }
    let mut used = vec![false; edges.len()];
    let mut rings: Vec<Vec<Pt>> = Vec::new();
    for s in 0..edges.len() {
        if used[s] {
            continue;
        }
        let mut ring = vec![edges[s][0]];
        let mut cur = s;
        loop {
            used[cur] = true;
            let [u, v] = edges[cur];
            ring.push(v);
            if v == edges[s][0] {
                break;
            }
            let outs = &out_of[&Key(v)];
            // The next edge clockwise from the way back to `u`: the last one counter-clockwise
            // below that direction, wrapping round.
            let below: Vec<usize> = outs
                .iter()
                .copied()
                .filter(|&x| {
                    !used[x] && compare_dir(v, edges[x][1], u) == core::cmp::Ordering::Less
                })
                .collect();
            let next = below
                .last()
                .copied()
                .or_else(|| outs.iter().copied().rfind(|&x| !used[x]));
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        if ring.len() >= 4 && ring.first() == ring.last() && ring_signed_area(&ring) != 0.0 {
            rings.push(ring);
        }
    }
    let (mut shells, mut holes): (Vec<Vec<Pt>>, Vec<Vec<Pt>>) = (Vec::new(), Vec::new());
    for r in rings {
        if ring_signed_area(&r) < 0.0 {
            shells.push(r)
        } else {
            holes.push(r)
        }
    }
    let mut polys: Vec<Polygon> = shells
        .into_iter()
        .map(|s| Polygon {
            exterior: s,
            holes: vec![],
        })
        .collect();
    for h in holes {
        let mut best: Option<(usize, f64)> = None;
        for (i, p) in polys.iter().enumerate() {
            let inside = h.iter().find_map(|&v| ring_contains(&p.exterior, v));
            if inside == Some(true) {
                let a = ring_signed_area(&p.exterior).abs();
                if best.is_none_or(|(_, ba)| a < ba) {
                    best = Some((i, a));
                }
            }
        }
        if let Some((i, _)) = best {
            polys[i].holes.push(h);
        }
    }
    polys
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (areas of axis-aligned squares and their overlaps)
    use super::*;

    fn sq(x0: f64, y0: f64, s: f64) -> Polygon {
        Polygon {
            exterior: vec![
                [x0, y0],
                [x0 + s, y0],
                [x0 + s, y0 + s],
                [x0, y0 + s],
                [x0, y0],
            ],
            holes: vec![],
        }
    }

    fn area(ps: &[Polygon]) -> f64 {
        ps.iter().map(polygon_area).sum()
    }

    #[test]
    fn overlapping_squares() {
        let (a, b) = ([sq(0.0, 0.0, 100.0)], [sq(50.0, 50.0, 100.0)]);
        assert_eq!(area(&overlay(&a, &b, Op::Intersection)), 2500.0);
        assert_eq!(area(&overlay(&a, &b, Op::Difference)), 7500.0);
        assert_eq!(area(&overlay(&a, &b, Op::Union)), 17500.0);
    }

    #[test]
    fn a_strip_cuts_a_square_in_two_and_a_contained_square_makes_a_hole() {
        let strip = Polygon {
            exterior: vec![
                [-10.0, 40.0],
                [110.0, 40.0],
                [110.0, 60.0],
                [-10.0, 60.0],
                [-10.0, 40.0],
            ],
            holes: vec![],
        };
        let parts = overlay(&[sq(0.0, 0.0, 100.0)], &[strip], Op::Difference);
        assert_eq!(parts.len(), 2);
        assert_eq!(area(&parts), 8000.0);
        let holed = overlay(
            &[sq(0.0, 0.0, 100.0)],
            &[sq(10.0, 10.0, 20.0)],
            Op::Difference,
        );
        assert_eq!(holed.len(), 1);
        assert_eq!(holed[0].holes.len(), 1);
        assert_eq!(area(&holed), 9600.0);
    }

    #[test]
    fn squares_sharing_an_edge() {
        let (a, b) = ([sq(0.0, 0.0, 100.0)], [sq(100.0, 0.0, 60.0)]);
        assert!(overlay(&a, &b, Op::Intersection).is_empty());
        assert_eq!(area(&overlay(&a, &b, Op::Union)), 13600.0);
        assert_eq!(overlay(&a, &b, Op::Union).len(), 1);
        assert_eq!(area(&overlay(&a, &b, Op::Difference)), 10000.0);
    }

    #[test]
    fn locate_tells_the_boundary_apart() {
        let s = sq(0.0, 0.0, 10.0);
        assert_eq!(locate([5.0, 5.0], &s), Loc::Interior);
        assert_eq!(locate([10.0, 5.0], &s), Loc::Boundary);
        assert_eq!(locate([0.0, 0.0], &s), Loc::Boundary);
        assert_eq!(locate([11.0, 5.0], &s), Loc::Exterior);
    }
}
