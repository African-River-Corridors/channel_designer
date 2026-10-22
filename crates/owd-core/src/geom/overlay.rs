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

/// One ring, its segments grouped by horizontal strips: only a segment whose y range holds the
/// point's y can change [`ring_contains`], so those, in their order, give its answer exactly.
#[derive(Debug, Clone)]
struct RingStrips {
    ring: Vec<Pt>,
    y0: f64,
    h: f64,
    strips: Vec<Vec<u32>>,
}

impl RingStrips {
    fn new(ring: &[Pt]) -> RingStrips {
        let n = ring.len().saturating_sub(1).max(1);
        let (y0, y1) = ring
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
        let count = ((n as f64).sqrt().ceil() as usize).max(1);
        let h = ((y1 - y0) / count as f64).max(1e-9);
        let mut strips = vec![Vec::new(); count];
        let at = |y: f64| (((y - y0) / h).floor().max(0.0) as usize).min(count - 1);
        for (i, w) in ring.windows(2).enumerate() {
            let (a, b) = (at(w[0][1].min(w[1][1])), at(w[0][1].max(w[1][1])));
            for strip in &mut strips[a..=b] {
                strip.push(i as u32);
            }
        }
        RingStrips {
            ring: ring.to_vec(),
            y0,
            h,
            strips,
        }
    }

    /// [`ring_contains`], over the segments of the point's strip.
    fn contains(&self, p: Pt) -> Option<bool> {
        if !(p[1].is_finite() && p[0].is_finite()) || self.ring.len() < 2 {
            return ring_contains(&self.ring, p);
        }
        let k = (p[1] - self.y0) / self.h;
        if k < 0.0 || k >= self.strips.len() as f64 + 1.0 {
            // Outside every segment's y range: no segment can count.
            return ring_contains(&[], p);
        }
        let k = (k.floor() as usize).min(self.strips.len() - 1);
        let mut seg: Vec<Pt> = Vec::new();
        let mut crossings = 0u32;
        for &i in &self.strips[k] {
            let i = i as usize;
            seg.clear();
            seg.extend_from_slice(&self.ring[i..i + 2]);
            match ring_contains(&seg, p) {
                None => return None,
                Some(true) => crossings += 1,
                Some(false) => {}
            }
        }
        Some(crossings % 2 == 1)
    }
}

/// [`locate_all`] and [`contains_xy`] for many points in one set of polygons: the same answers,
/// with each ring's segments grouped by horizontal strips (see `RingStrips`).
#[derive(Debug, Clone)]
pub struct PolygonIndex {
    polys: Vec<(RingStrips, Vec<RingStrips>)>,
}

impl PolygonIndex {
    pub fn new(polys: &[Polygon]) -> PolygonIndex {
        PolygonIndex {
            polys: polys
                .iter()
                .map(|p| {
                    (
                        RingStrips::new(&p.exterior),
                        p.holes.iter().map(|h| RingStrips::new(h)).collect(),
                    )
                })
                .collect(),
        }
    }

    /// [`locate_all`].
    pub fn locate_all(&self, p: Pt) -> Loc {
        let mut out = Loc::Exterior;
        for (ext, holes) in &self.polys {
            let here = match ext.contains(p) {
                None => Loc::Boundary,
                Some(false) => Loc::Exterior,
                Some(true) => holes
                    .iter()
                    .map(|h| h.contains(p))
                    .find_map(|c| match c {
                        None => Some(Loc::Boundary),
                        Some(true) => Some(Loc::Exterior),
                        Some(false) => None,
                    })
                    .unwrap_or(Loc::Interior),
            };
            match here {
                Loc::Interior => return Loc::Interior,
                Loc::Boundary => out = Loc::Boundary,
                Loc::Exterior => {}
            }
        }
        out
    }

    /// [`contains_xy`].
    pub fn contains_xy(&self, p: Pt) -> bool {
        self.locate_all(p) == Loc::Interior
    }
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
    oriented_edges_with_origins(polys).0
}

/// `oriented_edges`, with each edge's start in the ring's own direction (where GEOS's noder
/// starts walking it).
fn oriented_edges_with_origins(polys: &[Polygon]) -> (Vec<[Pt; 2]>, Vec<Pt>) {
    let mut origins = Vec::new();
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
                    origins.push(w[0]);
                }
            }
        }
    }
    (edges, origins)
}

/// Split every edge of `a` at the points where it meets an edge of `b`, and the other way round.
#[allow(clippy::type_complexity)]
fn node(
    a: &[[Pt; 2]],
    b: &[[Pt; 2]],
    snap: Option<f64>,
) -> (Vec<([Pt; 2], usize)>, Vec<([Pt; 2], usize)>) {
    // Snapped: the vertices of both inputs first merge into clusters `tol` wide, and every edge is
    // redrawn between the cluster points. An edge that shrinks to a point stays in the list (so
    // edge numbers do not move) and splits into nothing.
    let mut reps = Snap::new(snap.unwrap_or(0.0));
    let (a, b): (Vec<[Pt; 2]>, Vec<[Pt; 2]>) = if snap.is_some() {
        let redraw = |es: &[[Pt; 2]], reps: &mut Snap| -> Vec<[Pt; 2]> {
            es.iter()
                .map(|e| [reps.add(e[0]), reps.add(e[1])])
                .collect()
        };
        let a2 = redraw(a, &mut reps);
        let b2 = redraw(b, &mut reps);
        (a2, b2)
    } else {
        (a.to_vec(), b.to_vec())
    };
    let (a, b) = (&a[..], &b[..]);
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
    if let Some(tol) = snap {
        snap_vertices(a, b, tol, &mut cuts_b);
        snap_vertices(b, a, tol, &mut cuts_a);
        // Each cut point joins the vertex cluster it falls in, or starts one: both edges that
        // share a node must name it by the same coordinates.
        for c in cuts_a.iter_mut().chain(cuts_b.iter_mut()) {
            for p in c.iter_mut() {
                *p = reps.add(*p);
            }
        }
    }
    (split_ranked(a, cuts_a), split_ranked(b, cuts_b))
}

/// Cluster points: each new point is merged into the first earlier point within `tol`, or becomes
/// a cluster point itself. Deterministic: it depends on the order points arrive in.
struct Snap {
    tol: f64,
    cells: BTreeMap<(i64, i64), Vec<Pt>>,
    seen: BTreeMap<Key, Pt>,
}

impl Snap {
    fn new(tol: f64) -> Self {
        Self {
            tol,
            cells: BTreeMap::new(),
            seen: BTreeMap::new(),
        }
    }

    fn cell(&self, p: Pt) -> (i64, i64) {
        (
            (p[0] / self.tol).floor() as i64,
            (p[1] / self.tol).floor() as i64,
        )
    }

    fn add(&mut self, p: Pt) -> Pt {
        if let Some(&q) = self.seen.get(&Key(p)) {
            return q;
        }
        let (cx, cy) = self.cell(p);
        let mut found: Option<Pt> = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(v) = self.cells.get(&(cx + dx, cy + dy)) {
                    for &q in v {
                        if (p[0] - q[0]).abs() <= self.tol && (p[1] - q[1]).abs() <= self.tol {
                            found = Some(q);
                            break 'search;
                        }
                    }
                }
            }
        }
        let rep = found.unwrap_or(p);
        if found.is_none() {
            self.cells.entry((cx, cy)).or_default().push(p);
        }
        self.seen.insert(Key(p), rep);
        rep
    }
}

/// Distance from `p` to the segment `e`.
fn point_segment_distance(p: Pt, e: &[Pt; 2]) -> f64 {
    let (dx, dy) = (e[1][0] - e[0][0], e[1][1] - e[0][1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - e[0][0]) * dx + (p[1] - e[0][1]) * dy) / len2).clamp(0.0, 1.0)
    };
    let (qx, qy) = (e[0][0] + t * dx, e[0][1] + t * dy);
    ((p[0] - qx) * (p[0] - qx) + (p[1] - qy) * (p[1] - qy)).sqrt()
}

/// Make every vertex of `src` that lies within `tol` of an edge of `dst` (but is not one of its
/// ends) a node of that edge. Floating-point noding finds a touch only when the orientation test
/// is exactly zero, so a vertex a hair off an edge is a crossing with no node: the two boundaries
/// then disagree about which side of each other it is on and the rings do not close. GEOS meets
/// this with a snapping noder (`OverlayNGRobust`), which this stands in for.
fn snap_vertices(src: &[[Pt; 2]], dst: &[[Pt; 2]], tol: f64, cuts_dst: &mut [Vec<Pt>]) {
    let minx = |e: &[Pt; 2]| e[0][0].min(e[1][0]);
    let maxx = |e: &[Pt; 2]| e[0][0].max(e[1][0]);
    let mut order: Vec<usize> = (0..dst.len()).collect();
    order.sort_by(|&i, &j| minx(&dst[i]).total_cmp(&minx(&dst[j])));
    let mins: Vec<f64> = order.iter().map(|&j| minx(&dst[j])).collect();
    for e in src {
        let p = e[0];
        let hi = mins.partition_point(|&m| m <= p[0] + tol);
        for &j in &order[..hi] {
            let d = &dst[j];
            if maxx(d) < p[0] - tol || p == d[0] || p == d[1] {
                continue;
            }
            if point_segment_distance(p, d) <= tol {
                cuts_dst[j].push(p);
            }
        }
    }
}

pub(crate) fn split(edges: &[[Pt; 2]], cuts: Vec<Vec<Pt>>) -> Vec<[Pt; 2]> {
    split_ranked(edges, cuts)
        .into_iter()
        .map(|(e, _)| e)
        .collect()
}

/// `split`, keeping the index of the edge each piece came from.
fn split_ranked(edges: &[[Pt; 2]], cuts: Vec<Vec<Pt>>) -> Vec<([Pt; 2], usize)> {
    let mut out = Vec::new();
    for (k, (e, mut c)) in edges.iter().zip(cuts).enumerate() {
        let d2 = |p: &Pt| (p[0] - e[0][0]) * (p[0] - e[0][0]) + (p[1] - e[0][1]) * (p[1] - e[0][1]);
        c.retain(|p| p != &e[0] && p != &e[1]);
        c.sort_by(|p, q| d2(p).total_cmp(&d2(q)));
        c.dedup();
        let mut prev = e[0];
        for p in c.into_iter().chain(core::iter::once(e[1])) {
            if p != prev {
                out.push(([prev, p], k));
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
    let (ea, eb) = node(&oriented_edges(a), &oriented_edges(b), None);
    let ea: Vec<[Pt; 2]> = ea.into_iter().map(|(e, _)| e).collect();
    let eb: Vec<[Pt; 2]> = eb.into_iter().map(|(e, _)| e).collect();
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
    // As GEOS `OverlayNGRobust`: plain floating-point noding first; if its edges do not close
    // into rings, node again with vertices snapped onto the edges they nearly touch, at a
    // tolerance of 1e-12 of the coordinates' size, then a thousand times that.
    let magnitude = a
        .iter()
        .chain(b)
        .flat_map(|p| p.exterior.iter().chain(p.holes.iter().flatten()))
        .fold(1.0_f64, |m, q| m.max(q[0].abs()).max(q[1].abs()));
    for snap in [None, Some(magnitude * 1e-12), Some(magnitude * 1e-9)] {
        if let Some(polys) = overlay_with(a, b, op, snap, false) {
            return polys;
        }
    }
    // Last resort: link what closes and drop the loose edges, rather than fail.
    overlay_with(a, b, op, Some(magnitude * 1e-9), true).unwrap_or_default()
}

/// One try of [`overlay`]: `None` when the kept edges do not close into rings and `lenient` is off.
fn overlay_with(
    a: &[Polygon],
    b: &[Polygon],
    op: Op,
    snap: Option<f64>,
    lenient: bool,
) -> Option<Vec<Polygon>> {
    let (oa, origin_a) = oriented_edges_with_origins(a);
    let (ob, origin_b) = oriented_edges_with_origins(b);
    let (ea, eb) = node(&oa, &ob, snap);
    let set_b: BTreeMap<(Key, Key), ()> = eb
        .iter()
        .map(|(e, _)| ((Key(e[0]), Key(e[1])), ()))
        .collect();
    let set_a: BTreeMap<(Key, Key), ()> = ea
        .iter()
        .map(|(e, _)| ((Key(e[0]), Key(e[1])), ()))
        .collect();
    let has = |s: &BTreeMap<(Key, Key), ()>, p: Pt, q: Pt| s.contains_key(&(Key(p), Key(q)));
    // Each kept edge's place in GEOS's edge list: input (A first), the ring segment it came from,
    // and how far along that segment, in the ring's own direction, it starts.
    let rank = |src: u8, e: &[Pt; 2], parent: usize, origin: &[Pt]| -> Rank {
        let o = origin[parent];
        let d2 = |p: Pt| (p[0] - o[0]) * (p[0] - o[0]) + (p[1] - o[1]) * (p[1] - o[1]);
        (src, parent, d2(e[0]).min(d2(e[1])))
    };
    let mut result: Vec<[Pt; 2]> = Vec::new();
    let mut ranks: Vec<Rank> = Vec::new();
    for (e, parent) in &ea {
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
            ranks.push(rank(0, e, *parent, &origin_a));
        }
    }
    for (e, parent) in &eb {
        if has(&set_a, e[0], e[1]) || has(&set_a, e[1], e[0]) {
            continue;
        }
        let inside_a = locate_all(mid(e), a) == Loc::Interior;
        let kept = match op {
            Op::Intersection if inside_a => Some(*e),
            Op::Union if !inside_a => Some(*e),
            Op::Difference if inside_a => Some([e[1], e[0]]),
            _ => None,
        };
        if let Some(k) = kept {
            result.push(k);
            ranks.push(rank(1, e, *parent, &origin_b));
        }
    }
    build_polygons_checked(&result, &ranks, lenient)
}

/// An edge's place in GEOS's overlay edge list (see `overlay`).
type Rank = (u8, usize, f64);

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
pub(crate) fn build_polygons(edges: &[[Pt; 2]]) -> Vec<Polygon> {
    let ranks: Vec<Rank> = (0..edges.len()).map(|i| (0, i, 0.0)).collect();
    build_polygons_checked(edges, &ranks, true).unwrap_or_default()
}

/// `build_polygons_ranked` on edges that must close: an edge that ends where no kept edge starts
/// is a loose end. Strict, that is `None`; lenient, loose edges (and those that only lead to
/// them) are dropped first.
fn build_polygons_checked(
    edges: &[[Pt; 2]],
    ranks: &[Rank],
    lenient: bool,
) -> Option<Vec<Polygon>> {
    let mut keep: Vec<bool> = vec![true; edges.len()];
    loop {
        let mut starts: BTreeMap<Key, usize> = BTreeMap::new();
        for (i, e) in edges.iter().enumerate() {
            if keep[i] {
                *starts.entry(Key(e[0])).or_default() += 1;
            }
        }
        let mut changed = false;
        for (i, e) in edges.iter().enumerate() {
            if keep[i] && !starts.contains_key(&Key(e[1])) {
                if !lenient {
                    return None;
                }
                keep[i] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let (edges, ranks): (Vec<[Pt; 2]>, Vec<Rank>) = edges
        .iter()
        .zip(ranks)
        .zip(&keep)
        .filter(|&(_, &k)| k)
        .map(|((e, r), _)| (*e, *r))
        .unzip();
    Some(build_polygons_ranked(&edges, &ranks))
}

/// `build_polygons`, with the polygons in the order of the lowest-ranked edge of each shell: GEOS
/// OverlayNG builds its shells in the order of its edge list, so this is the order it returns.
fn build_polygons_ranked(edges: &[[Pt; 2]], ranks: &[Rank]) -> Vec<Polygon> {
    let mut out_of: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for (i, e) in edges.iter().enumerate() {
        out_of.entry(Key(e[0])).or_default().push(i);
    }
    for (k, outs) in out_of.iter_mut() {
        outs.sort_by(|&x, &y| compare_dir(k.0, edges[x][1], edges[y][1]));
    }
    let mut used = vec![false; edges.len()];
    let mut rings: Vec<(Vec<Pt>, Rank)> = Vec::new();
    for s in 0..edges.len() {
        if used[s] {
            continue;
        }
        let mut ring = vec![edges[s][0]];
        let mut cur = s;
        let mut low = ranks[s];
        loop {
            used[cur] = true;
            if ranks[cur].partial_cmp(&low) == Some(core::cmp::Ordering::Less) {
                low = ranks[cur];
            }
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
            rings.push((ring, low));
        }
    }
    let mut shells: Vec<(Vec<Pt>, Rank)> = Vec::new();
    let mut holes: Vec<Vec<Pt>> = Vec::new();
    for (r, low) in rings {
        if ring_signed_area(&r) < 0.0 {
            shells.push((r, low))
        } else {
            holes.push(r)
        }
    }
    shells.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(core::cmp::Ordering::Equal));
    let shells: Vec<Vec<Pt>> = shells.into_iter().map(|(r, _)| r).collect();
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

    #[test]
    fn vertices_a_hair_off_an_edge_still_close_into_rings() {
        // Label: property. Found on a real channel footprint: two vertices of B lie 1.4e-14 from
        // A's end edge (and B's edge between them runs along it), so floating-point noding finds no
        // touch there and the kept edges did not close. B lies inside A, so A \ B must lose B's
        // whole area and A & B must be B.
        let a = Polygon {
            exterior: vec![
                [5900.0, 40.0],
                [5985.122858308741, 13.664210738209482],
                [6014.877141691259, -13.66421073820999],
                [5900.0, -40.0],
                [5900.0, 40.0],
            ],
            holes: vec![],
        };
        let b = Polygon {
            exterior: vec![
                [5985.270156741328, 13.528921522979894],
                [5975.276074558899, 2.6476946778540116],
                [6000.0, -1.7845575639194763e-13],
                [5985.270156741328, 13.528921522979894],
            ],
            holes: vec![],
        };
        let (a, b) = ([a], [b]);
        let inter = area(&overlay(&a, &b, Op::Intersection));
        let diff = area(&overlay(&a, &b, Op::Difference));
        assert!((inter - area(&b)).abs() < 1e-6, "{inter}");
        assert!((diff - (area(&a) - area(&b))).abs() < 1e-6, "{diff}");
    }
}
