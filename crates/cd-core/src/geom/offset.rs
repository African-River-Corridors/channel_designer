//! Offset curves and buffers of points and lines, built as GEOS 3.14 builds them.
//!
//! Follows `BufferInputLineSimplifier`, `OffsetSegmentGenerator`, `OffsetSegmentString`,
//! `OffsetCurveBuilder`, `BufferCurveSetBuilder::addPoint` / `addLineString` and `OffsetCurve`.
//! Round joins only (the package uses no other).
//!
//! For a line whose buffer curve does not cross itself, GEOS's noding finds nothing to split and
//! its polygon builder returns the raw curve as the shell, with the same start and direction, so
//! the shell is the raw curve as it stands. A curve that crosses itself (an offset wider than the
//! bend, a closing segment at a sharp inside turn) is noded and resolved by depth (`depth`), and so
//! is the `buffer(0)` repair of a polygon.

use super::{GeomError, Geometry, Polygon, Pt, depth, dist, linear, orientation};
use super::{SegHit, segment_intersection};
use core::f64::consts::{FRAC_PI_2, PI};

/// GEOS `BufferParameters` end cap style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Round,
    Flat,
    Square,
}

impl Cap {
    /// shapely's names and codes: `"round"`/1, `"flat"`/2, `"square"`/3.
    pub fn from_shapely(name: &str) -> Option<Cap> {
        match name {
            "round" | "1" => Some(Cap::Round),
            "flat" | "2" => Some(Cap::Flat),
            "square" | "3" => Some(Cap::Square),
            _ => None,
        }
    }
}

// --- BufferInputLineSimplifier ----------------------------------------------------------------

/// GEOS `BufferInputLineSimplifier::simplify(line, distanceTol)`. A positive tolerance removes
/// shallow counter-clockwise (left-side) concavities, a negative one clockwise ones.
///
/// GEOS quirk, kept: `isShallowSampled` checks the distance of the middle vertex to the segments
/// from the first vertex to each sampled vertex, starting with the first vertex itself. So a
/// vertex is removed only when it lies within the tolerance of the vertex before it.
fn simplify(line: &[Pt], distance_tol: f64) -> Vec<Pt> {
    let tol = distance_tol.abs();
    let angle_orientation = if distance_tol < 0.0 { -1 } else { 1 };
    let n = line.len();
    let mut deleted = vec![false; n];
    let next = |deleted: &[bool], i: usize| {
        let mut j = i + 1;
        while j < n && deleted[j] {
            j += 1;
        }
        j
    };
    let is_shallow = |p0: Pt, p1: Pt, p2: Pt| linear::point_segment_distance(p1, p0, p2) < tol;
    loop {
        let mut changed = false;
        let mut index = 1;
        let mut mid = next(&deleted, index);
        let mut last = next(&deleted, mid);
        while last < n {
            let (p0, p1, p2) = (line[index], line[mid], line[last]);
            let deletable =
                orientation(p0, p1, p2) == angle_orientation && is_shallow(p0, p1, p2) && {
                    let inc = ((last - index) / 10).max(1);
                    (index..last)
                        .step_by(inc)
                        .all(|i| is_shallow(p0, p1, line[i]))
                };
            if deletable {
                deleted[mid] = true;
                changed = true;
                index = last;
            } else {
                index = mid;
            }
            mid = next(&deleted, index);
            last = next(&deleted, mid);
        }
        if !changed {
            break;
        }
    }
    line.iter()
        .zip(&deleted)
        .filter(|(_, d)| !**d)
        .map(|(p, _)| *p)
        .collect()
}

// --- OffsetSegmentGenerator -------------------------------------------------------------------

const CURVE_VERTEX_SNAP_DISTANCE_FACTOR: f64 = 1.0e-4;
const OFFSET_SEGMENT_SEPARATION_FACTOR: f64 = 1.0e-3;
const INSIDE_TURN_VERTEX_SNAP_DISTANCE_FACTOR: f64 = 1.0e-3;
const MAX_CLOSING_SEG_LEN_FACTOR: f64 = 80.0;
const LEFT: i32 = 1;
const RIGHT: i32 = 2;

type Seg = [Pt; 2];

/// GEOS `Angle::sinCosSnap`: values below 5e-16 in size snap to zero.
fn sin_cos_snap(a: f64) -> (f64, f64) {
    let (mut s, mut c) = (libm::sin(a), libm::cos(a));
    if s.abs() < 5e-16 {
        s = 0.0;
    }
    if c.abs() < 5e-16 {
        c = 0.0;
    }
    (s, c)
}

/// GEOS `OffsetSegmentGenerator::computeOffsetSegment`.
fn offset_segment(seg: Seg, side: i32, d: f64) -> Seg {
    let sign = if side == LEFT { 1.0 } else { -1.0 };
    let (dx, dy) = (seg[1][0] - seg[0][0], seg[1][1] - seg[0][1]);
    let len = (dx * dx + dy * dy).sqrt();
    let ux = sign * d * dx / len;
    let uy = sign * d * dy / len;
    [
        [seg[0][0] - uy, seg[0][1] + ux],
        [seg[1][0] - uy, seg[1][1] + ux],
    ]
}

/// GEOS `OffsetSegmentGenerator` with its `OffsetSegmentString`, round joins.
struct SegGen {
    cap: Cap,
    distance: f64,
    fillet_angle_quantum: f64,
    closing_seg_length_factor: f64,
    min_vertex_distance: f64,
    pts: Vec<Pt>,
    s0: Pt,
    s1: Pt,
    s2: Pt,
    offset0: Seg,
    offset1: Seg,
    side: i32,
}

impl SegGen {
    fn new(quad_segs: i32, cap: Cap, distance: f64) -> Self {
        let q = quad_segs.max(1);
        SegGen {
            cap,
            distance,
            fillet_angle_quantum: FRAC_PI_2 / f64::from(q),
            closing_seg_length_factor: if quad_segs >= 8 {
                MAX_CLOSING_SEG_LEN_FACTOR
            } else {
                1.0
            },
            min_vertex_distance: distance * CURVE_VERTEX_SNAP_DISTANCE_FACTOR,
            pts: Vec::new(),
            s0: [0.0; 2],
            s1: [0.0; 2],
            s2: [0.0; 2],
            offset0: [[0.0; 2]; 2],
            offset1: [[0.0; 2]; 2],
            side: 0,
        }
    }

    /// `OffsetSegmentString::addPt`: a point within the minimum vertex distance of the last one is
    /// dropped.
    fn add_pt(&mut self, p: Pt) {
        if let Some(&last) = self.pts.last()
            && dist(p, last) < self.min_vertex_distance
        {
            return;
        }
        self.pts.push(p);
    }

    fn close_ring(&mut self) {
        if let (Some(&first), Some(&last)) = (self.pts.first(), self.pts.last())
            && first != last
        {
            self.pts.push(first);
        }
    }

    /// `getCoordinates`, which closes the ring first (GEOS does this for every curve, the
    /// single-sided offset curve included).
    fn into_coordinates(mut self) -> Vec<Pt> {
        self.close_ring();
        self.pts
    }

    fn init_side_segments(&mut self, s1: Pt, s2: Pt, side: i32) {
        self.s1 = s1;
        self.s2 = s2;
        self.side = side;
        self.offset1 = offset_segment([s1, s2], side, self.distance);
    }

    fn add_first_segment(&mut self) {
        self.add_pt(self.offset1[0]);
    }

    fn add_last_segment(&mut self) {
        self.add_pt(self.offset1[1]);
    }

    fn add_next_segment(&mut self, p: Pt, add_start_point: bool) {
        if self.s2 == p {
            return;
        }
        self.s0 = self.s1;
        self.s1 = self.s2;
        self.s2 = p;
        self.offset0 = offset_segment([self.s0, self.s1], self.side, self.distance);
        self.offset1 = offset_segment([self.s1, self.s2], self.side, self.distance);
        let orient = orientation(self.s0, self.s1, self.s2);
        let outside_turn =
            (orient == -1 && self.side == LEFT) || (orient == 1 && self.side == RIGHT);
        if orient == 0 {
            self.add_collinear();
        } else if outside_turn {
            self.add_outside_turn(orient, add_start_point);
        } else {
            self.add_inside_turn();
        }
    }

    fn add_collinear(&mut self) {
        // A reversal: GEOS's LineIntersector reports two intersection points (an overlap).
        if let SegHit::Overlap { .. } = segment_intersection(self.s0, self.s1, self.s1, self.s2) {
            let (s1, a, b) = (self.s1, self.offset0[1], self.offset1[0]);
            self.add_directed_fillet_pts(s1, a, b, -1, self.distance);
        }
    }

    fn add_outside_turn(&mut self, orient: i32, add_start_point: bool) {
        if dist(self.offset0[1], self.offset1[0]) < self.distance * OFFSET_SEGMENT_SEPARATION_FACTOR
        {
            self.add_pt(self.offset0[1]);
            return;
        }
        if add_start_point {
            self.add_pt(self.offset0[1]);
        }
        let (s1, a, b) = (self.s1, self.offset0[1], self.offset1[0]);
        self.add_directed_fillet_pts(s1, a, b, orient, self.distance);
        self.add_pt(self.offset1[0]);
    }

    fn add_inside_turn(&mut self) {
        let (o0, o1) = (self.offset0, self.offset1);
        match segment_intersection(o0[0], o0[1], o1[0], o1[1]) {
            SegHit::Point { p, .. } => self.add_pt(p),
            SegHit::Overlap { a, .. } => self.add_pt(a),
            SegHit::None => {
                // A closing segment: the curve crosses itself here, so the buffer needs noding.
                // It is still built, so that `is_simple` reports the crossing.
                if dist(o0[1], o1[0]) < self.distance * INSIDE_TURN_VERTEX_SNAP_DISTANCE_FACTOR {
                    self.add_pt(o0[1]);
                } else {
                    self.add_pt(o0[1]);
                    let f = self.closing_seg_length_factor;
                    let s1 = self.s1;
                    self.add_pt([
                        (f * o0[1][0] + s1[0]) / (f + 1.0),
                        (f * o0[1][1] + s1[1]) / (f + 1.0),
                    ]);
                    self.add_pt([
                        (f * o1[0][0] + s1[0]) / (f + 1.0),
                        (f * o1[0][1] + s1[1]) / (f + 1.0),
                    ]);
                    self.add_pt(o1[0]);
                }
            }
        }
    }

    /// `addDirectedFillet(p, p0, p1, direction, radius)`: from `p0` to `p1` around `p`.
    fn add_directed_fillet_pts(&mut self, p: Pt, p0: Pt, p1: Pt, direction: i32, radius: f64) {
        let mut start = libm::atan2(p0[1] - p[1], p0[0] - p[0]);
        let end = libm::atan2(p1[1] - p[1], p1[0] - p[0]);
        if direction == -1 {
            if start <= end {
                start += 2.0 * PI;
            }
        } else if start >= end {
            start -= 2.0 * PI;
        }
        self.add_pt(p0);
        self.add_directed_fillet(p, start, end, direction, radius);
        self.add_pt(p1);
    }

    /// `addDirectedFillet(p, startAngle, endAngle, direction, radius)`.
    fn add_directed_fillet(&mut self, p: Pt, start: f64, end: f64, direction: i32, radius: f64) {
        let factor = if direction == -1 { -1.0 } else { 1.0 };
        let total = (start - end).abs();
        let n_segs = (total / self.fillet_angle_quantum + 0.5) as i32;
        if n_segs < 1 {
            return;
        }
        let inc = total / f64::from(n_segs);
        for i in 0..n_segs {
            let (s, c) = sin_cos_snap(start + factor * f64::from(i) * inc);
            self.add_pt([p[0] + radius * c, p[1] + radius * s]);
        }
    }

    /// `addLineEndCap(p0, p1)`: the cap at `p1` of a segment coming from `p0`.
    fn add_line_end_cap(&mut self, p0: Pt, p1: Pt) {
        let ol = offset_segment([p0, p1], LEFT, self.distance);
        let or = offset_segment([p0, p1], RIGHT, self.distance);
        let angle = libm::atan2(p1[1] - p0[1], p1[0] - p0[0]);
        match self.cap {
            Cap::Round => {
                self.add_pt(ol[1]);
                self.add_directed_fillet(
                    p1,
                    angle + FRAC_PI_2,
                    angle - FRAC_PI_2,
                    -1,
                    self.distance,
                );
                self.add_pt(or[1]);
            }
            Cap::Flat => {
                self.add_pt(ol[1]);
                self.add_pt(or[1]);
            }
            Cap::Square => {
                let (s, c) = sin_cos_snap(angle);
                let (ox, oy) = (self.distance.abs() * c, self.distance.abs() * s);
                self.add_pt([ol[1][0] + ox, ol[1][1] + oy]);
                self.add_pt([or[1][0] + ox, or[1][1] + oy]);
            }
        }
    }

    /// `createCircle(p, distance)`: clockwise from `(x + r, y)`.
    fn create_circle(&mut self, p: Pt, r: f64) {
        self.add_pt([p[0] + r, p[1]]);
        self.add_directed_fillet(p, 0.0, 2.0 * PI, -1, r);
        self.close_ring();
    }

    /// `createSquare(p, distance)`.
    fn create_square(&mut self, p: Pt, r: f64) {
        self.add_pt([p[0] + r, p[1] + r]);
        self.add_pt([p[0] + r, p[1] - r]);
        self.add_pt([p[0] - r, p[1] - r]);
        self.add_pt([p[0] - r, p[1] + r]);
        self.close_ring();
    }

    /// `OffsetCurveBuilder::computePointCurve`.
    fn point_curve(&mut self, p: Pt) {
        match self.cap {
            Cap::Round => self.create_circle(p, self.distance),
            Cap::Square => self.create_square(p, self.distance),
            Cap::Flat => {}
        }
    }
}

/// `OffsetCurveBuilder::simplifyTolerance`.
fn simplify_tolerance(d: f64) -> f64 {
    d / 100.0
}

/// `OffsetCurveBuilder::computeLineBufferCurve`: the closed, clockwise buffer curve of a line
/// (at least two distinct points).
fn line_buffer_curve(pts: &[Pt], d: f64, quad_segs: i32, cap: Cap) -> Vec<Pt> {
    let mut g = SegGen::new(quad_segs, cap, d);
    let tol = simplify_tolerance(d);
    let simp1 = simplify(pts, tol);
    let n1 = simp1.len() - 1;
    g.init_side_segments(simp1[0], simp1[1], LEFT);
    for p in &simp1[2..] {
        g.add_next_segment(*p, true);
    }
    g.add_last_segment();
    g.add_line_end_cap(simp1[n1 - 1], simp1[n1]);
    let simp2 = simplify(pts, -tol);
    let n2 = simp2.len() - 1;
    g.init_side_segments(simp2[n2], simp2[n2 - 1], LEFT);
    for i in (1..n2).rev() {
        g.add_next_segment(simp2[i - 1], true);
    }
    g.add_last_segment();
    g.add_line_end_cap(simp2[1], simp2[0]);
    g.close_ring();
    g.into_coordinates()
}

/// `OffsetCurveBuilder::getOffsetCurve` (`computeOffsetCurve`): the raw one-sided offset, closed
/// as GEOS closes it, and reversed for the right side.
fn raw_offset_curve(pts: &[Pt], d: f64, quad_segs: i32) -> Vec<Pt> {
    let is_right = d < 0.0;
    let pos = d.abs();
    let mut g = SegGen::new(quad_segs, Cap::Round, pos);
    let tol = simplify_tolerance(pos);
    if is_right {
        let simp2 = simplify(pts, -tol);
        let n2 = simp2.len() - 1;
        g.init_side_segments(simp2[n2], simp2[n2 - 1], LEFT);
        g.add_first_segment();
        for i in (1..n2).rev() {
            g.add_next_segment(simp2[i - 1], true);
        }
    } else {
        let simp1 = simplify(pts, tol);
        g.init_side_segments(simp1[0], simp1[1], LEFT);
        g.add_first_segment();
        for p in &simp1[2..] {
            g.add_next_segment(*p, true);
        }
    }
    g.add_last_segment();
    let mut out = g.into_coordinates();
    if is_right {
        out.reverse();
    }
    out
}

/// GEOS `RepeatedPointRemover::removeRepeatedPoints`: consecutive equal points become one.
fn remove_repeated(line: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(line.len());
    for &p in line {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

/// A closed line of at least four points (GEOS `CoordinateSequence::isRing`).
fn is_ring(pts: &[Pt]) -> bool {
    pts.len() >= 4 && pts.first() == pts.last()
}

/// Whether a closed curve crosses or touches itself anywhere but at consecutive segments' shared
/// vertex. A sweep over segment boxes sorted by their least x.
pub(crate) fn ring_is_simple(ring: &[Pt]) -> bool {
    let n = ring.len().saturating_sub(1);
    if n < 3 {
        return true;
    }
    let seg = |i: usize| (ring[i], ring[i + 1]);
    let minx = |i: usize| ring[i][0].min(ring[i + 1][0]);
    let maxx = |i: usize| ring[i][0].max(ring[i + 1][0]);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| minx(a).total_cmp(&minx(b)));
    for (k, &i) in order.iter().enumerate() {
        let (a0, a1) = seg(i);
        for &j in &order[k + 1..] {
            if minx(j) > maxx(i) {
                break;
            }
            let (b0, b1) = seg(j);
            let adjacent =
                j == i + 1 || i == j + 1 || (i == 0 && j == n - 1) || (j == 0 && i == n - 1);
            match segment_intersection(a0, a1, b0, b1) {
                SegHit::None => {}
                SegHit::Point { p, .. } if adjacent => {
                    let shared = if j == i + 1 || (i == n - 1 && j == 0) {
                        a1
                    } else {
                        a0
                    };
                    if p != shared {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

/// shapely `Point(x, y).buffer(distance, quad_segs=q)` (round cap): `None` when the distance is
/// not positive (the empty polygon).
pub fn buffer_point(p: Pt, distance: f64, quad_segs: i32) -> Option<Polygon> {
    if distance <= 0.0 {
        return None;
    }
    let mut g = SegGen::new(quad_segs, Cap::Round, distance);
    g.point_curve(p);
    Some(Polygon {
        exterior: g.into_coordinates(),
        holes: vec![],
    })
}

/// The buffer curve of a line as GEOS builds it before noding, or why it cannot be used as the
/// result. `Ok(None)` is the empty polygon.
fn line_buffer_ring(
    line: &[Pt],
    distance: f64,
    quad_segs: i32,
    cap: Cap,
) -> Result<Option<Vec<Pt>>, GeomError> {
    if distance <= 0.0 || line.is_empty() {
        return Ok(None);
    }
    let pts = remove_repeated(line);
    if is_ring(&pts) {
        return Err(GeomError::Unsupported(
            "buffer of a closed line (two-sided ring buffer)".into(),
        ));
    }
    let ring = if pts.len() == 1 {
        let mut g = SegGen::new(quad_segs, cap, distance);
        g.point_curve(pts[0]);
        g.into_coordinates()
    } else {
        line_buffer_curve(&pts, distance, quad_segs, cap)
    };
    if ring.len() < 4 {
        return Ok(None);
    }
    Ok(Some(ring))
}

/// shapely `line.buffer(distance, quad_segs=q, cap_style=cap)` (round joins).
pub fn buffer_line(
    line: &[Pt],
    distance: f64,
    quad_segs: i32,
    cap: Cap,
) -> Result<Geometry, GeomError> {
    Ok(geometry_of(line_buffer_polygons(
        line, distance, quad_segs, cap,
    )?))
}

/// One polygon, several, or the empty polygon.
fn geometry_of(mut polys: Vec<Polygon>) -> Geometry {
    match polys.len() {
        0 => Geometry::Empty("Polygon"),
        1 => Geometry::Polygon(polys.remove(0)),
        _ => Geometry::MultiPolygon(polys),
    }
}

/// Rings as GEOS's buffer gives them: shells clockwise, holes counter-clockwise (the reverse of
/// `depth::region`'s interior-on-the-left rings).
fn geos_oriented(polys: Vec<Polygon>) -> Vec<Polygon> {
    polys
        .into_iter()
        .map(|mut p| {
            p.exterior.reverse();
            p.holes.iter_mut().for_each(|h| h.reverse());
            p
        })
        .collect()
}

/// The buffer of a line as polygons in GEOS's orientation. A raw curve that does not cross itself
/// is the shell as it stands; one that does (an offset wider than the bend, a closing segment at a
/// sharp inside turn) is noded and resolved by depth (`depth::region`).
fn line_buffer_polygons(
    line: &[Pt],
    distance: f64,
    quad_segs: i32,
    cap: Cap,
) -> Result<Vec<Polygon>, GeomError> {
    let Some(ring) = line_buffer_ring(line, distance, quad_segs, cap)? else {
        return Ok(vec![]);
    };
    if ring_is_simple(&ring) {
        return Ok(vec![Polygon {
            exterior: ring,
            holes: vec![],
        }]);
    }
    // The raw curve runs clockwise: its interior is on its right.
    Ok(geos_oriented(depth::region(&[(ring, -1)])))
}

/// shapely's default `quad_segs` (16; GEOS's own default is 8).
pub const SHAPELY_QUAD_SEGS: i32 = 16;

/// `OffsetCurveBuilder::computeRingBufferCurve`: the closed offset curve of a ring on one side,
/// round joins, the input simplified first as GEOS does.
fn ring_buffer_curve(ring: &[Pt], side: i32, d: f64, quad_segs: i32) -> Vec<Pt> {
    let mut g = SegGen::new(quad_segs, Cap::Round, d);
    let tol = if side == RIGHT {
        -simplify_tolerance(d)
    } else {
        simplify_tolerance(d)
    };
    let simp = simplify(ring, tol);
    let n = simp.len() - 1;
    if n < 3 {
        return Vec::new();
    }
    g.init_side_segments(simp[n - 1], simp[0], side);
    for (i, p) in simp.iter().enumerate().skip(1) {
        g.add_next_segment(*p, i != 1);
    }
    g.close_ring();
    g.into_coordinates()
}

/// GEOS `BufferCurveSetBuilder::isErodedCompletely`: a quick test that a ring vanishes when
/// buffered by `d` (negative): its envelope is narrower than `2|d|`, or, for a triangle, its
/// incircle is smaller than `|d|`.
fn is_eroded_completely(ring: &[Pt], d: f64) -> bool {
    if ring.len() < 4 {
        return d < 0.0;
    }
    if ring.len() == 4 {
        let (a, b, c) = (ring[0], ring[1], ring[2]);
        let (la, lb, lc) = (dist(b, c), dist(a, c), dist(a, b));
        let p = la + lb + lc;
        let centre = [
            (la * a[0] + lb * b[0] + lc * c[0]) / p,
            (la * a[1] + lb * b[1] + lc * c[1]) / p,
        ];
        return linear::point_segment_distance(centre, a, b) < d.abs();
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in ring {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let min_dim = (hi[0] - lo[0]).min(hi[1] - lo[1]);
    d < 0.0 && 2.0 * d.abs() > min_dim
}

/// shapely `polygon.buffer(distance)` (round joins, shapely's default 16 segments a quarter
/// circle). See [`buffer_polygons_q`].
pub fn buffer_polygons(polys: &[Polygon], distance: f64) -> Result<Geometry, GeomError> {
    Ok(geometry_of(buffer_polygons_q(
        polys,
        distance,
        SHAPELY_QUAD_SEGS,
    )))
}

/// The buffer of polygons at any distance, as polygons in GEOS's orientation (GEOS
/// `BufferCurveSetBuilder::addPolygon` / `addRingSide`).
///
/// - At zero this is `buffer(0)`: the rings as raw curves, each labelled by its signed area,
///   resolved by depth. This repairs a bow-tie or a self-touching ring as GEOS does.
/// - A positive distance grows the polygons, with round joins of radius `distance` at convex
///   corners; a negative one shrinks them (round joins at reflex corners). Each ring is offset to
///   one side, keeps its label, and the curves are resolved by depth (the signed winding number),
///   so a part narrower than `2|distance|` vanishes and a narrow waist splits.
pub fn buffer_polygons_q(polys: &[Polygon], distance: f64, quad_segs: i32) -> Vec<Polygon> {
    // `isCCWArea`: GEOS's signed area is negative for a counter-clockwise ring.
    let ccw = |r: &[Pt]| super::overlay::ring_signed_area(r) < 0.0;
    let mut curves: Vec<depth::Curve> = Vec::new();
    if distance == 0.0 {
        for p in polys {
            let shell = remove_repeated(&p.exterior);
            if shell.len() < 3 {
                continue;
            }
            if shell.len() >= 4 {
                curves.push((shell.clone(), if ccw(&shell) { 1 } else { -1 }));
            }
            for h in &p.holes {
                let hole = remove_repeated(h);
                if hole.len() >= 4 {
                    curves.push((hole.clone(), if ccw(&hole) { -1 } else { 1 }));
                }
            }
        }
        return geos_oriented(depth::region(&curves));
    }
    let (off, side) = if distance < 0.0 {
        (-distance, RIGHT)
    } else {
        (distance, LEFT)
    };
    let opposite = |s: i32| if s == LEFT { RIGHT } else { LEFT };
    // `addRingSide`: a clockwise ring has the exterior on its left; a counter-clockwise one is
    // offset to the other side. The label (interior side) does not change with the offset.
    let mut add_ring = |ring: &[Pt], side: i32, is_shell: bool| {
        if ring.len() < 4 {
            return;
        }
        let is_ccw = ccw(ring);
        let s = if is_ccw { opposite(side) } else { side };
        let curve = ring_buffer_curve(ring, s, off, quad_segs);
        if curve.len() < 4 {
            return;
        }
        let interior_left = is_ccw == is_shell;
        curves.push((curve, if interior_left { 1 } else { -1 }));
    };
    for p in polys {
        let shell = remove_repeated(&p.exterior);
        if shell.len() < 3 || (distance < 0.0 && is_eroded_completely(&shell, distance)) {
            continue;
        }
        add_ring(&shell, side, true);
        for h in &p.holes {
            let hole = remove_repeated(h);
            if distance > 0.0 && is_eroded_completely(&hole, -distance) {
                continue;
            }
            add_ring(&hole, opposite(side), false);
        }
    }
    geos_oriented(depth::region(&curves))
}

/// Morphological opening: shrink by `d` then grow by `d` (`d` > 0). Removes every part and sliver
/// narrower than `2d` and rounds convex corners to radius `d`.
pub fn open(polys: &[Polygon], d: f64, quad_segs: i32) -> Vec<Polygon> {
    let shrunk = buffer_polygons_q(polys, -d, quad_segs);
    buffer_polygons_q(&shrunk, d, quad_segs)
}

/// Morphological closing: grow by `d` then shrink by `d` (`d` > 0). Fills every notch and gap
/// narrower than `2d` and rounds reflex corners to radius `d`.
pub fn close(polys: &[Polygon], d: f64, quad_segs: i32) -> Vec<Polygon> {
    let grown = buffer_polygons_q(polys, d, quad_segs);
    buffer_polygons_q(&grown, -d, quad_segs)
}

/// GEOS `OffsetCurve` match distance factor.
const MATCH_DISTANCE_FACTOR: f64 = 10_000.0;
/// GEOS `OffsetCurve::MIN_QUADRANT_SEGMENTS`.
const MIN_QUADRANT_SEGMENTS: i32 = 8;
const NOT_IN_CURVE: f64 = -1.0;

/// shapely `line.offset_curve(distance, quad_segs=q)` (round joins): GEOS `OffsetCurve`. Left for
/// a positive distance, right for a negative one, in the direction of the line.
pub fn offset_curve(line: &[Pt], distance: f64, quad_segs: i32) -> Result<Geometry, GeomError> {
    if !distance.is_finite() {
        return Err(GeomError::Unsupported(
            "offset distance must be finite".into(),
        ));
    }
    if line.len() < 2 || linear::length(line) == 0.0 {
        return Ok(Geometry::Empty("LineString"));
    }
    if distance == 0.0 {
        return Ok(Geometry::LineString(line.to_vec()));
    }
    if line.len() == 2 {
        // `offsetSegment`: `LineSegment::offset`.
        let (p0, p1) = (line[0], line[1]);
        let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (distance * dx / len, distance * dy / len);
        let at = |f: f64| [p0[0] + f * dx - uy, p0[1] + f * dy + ux];
        return Ok(Geometry::LineString(vec![at(0.0), at(1.0)]));
    }
    let quad = quad_segs.max(MIN_QUADRANT_SEGMENTS);
    let clean = remove_repeated(line);
    let raw = raw_offset_curve(&clean, distance, quad);
    if raw.is_empty() {
        return Ok(Geometry::Empty("LineString"));
    }
    // `getBufferOriented`: the largest polygon of the buffer, reversed for a right offset.
    let polys = line_buffer_polygons(line, distance.abs(), quad, Cap::Round)?;
    let mut best: Option<&Polygon> = None;
    for p in &polys {
        if best.is_none_or(|b| super::overlay::polygon_area(p) > super::overlay::polygon_area(b)) {
            best = Some(p);
        }
    }
    let Some(poly) = best else {
        return Ok(Geometry::Empty("LineString"));
    };
    let mut rings: Vec<Vec<Pt>> = core::iter::once(&poly.exterior)
        .chain(&poly.holes)
        .cloned()
        .collect();
    if distance < 0.0 {
        rings.iter_mut().for_each(|r| r.reverse());
    }
    let match_distance = distance.abs() / MATCH_DISTANCE_FACTOR;
    let mut sections: Vec<(f64, Vec<Pt>)> = rings
        .iter()
        .flat_map(|r| curve_sections(r, &raw, match_distance))
        .collect();
    Ok(match sections.len() {
        0 => Geometry::Empty("LineString"),
        1 => Geometry::LineString(sections.remove(0).1),
        _ => {
            sections.sort_by(|a, b| a.0.total_cmp(&b.0));
            Geometry::MultiLineString(sections.into_iter().map(|s| s.1).collect())
        }
    })
}

/// `OffsetCurve::computeCurveSections` and `extractSections`: the runs of buffer-ring segments
/// that lie on the raw offset curve, each with its location along the raw curve.
fn curve_sections(ring: &[Pt], raw: &[Pt], match_distance: f64) -> Vec<(f64, Vec<Pt>)> {
    let nseg = ring.len() - 1;
    let mut loc = vec![NOT_IN_CURVE; nseg];
    let mut first: Option<usize> = None;
    let mut min_raw = -1.0;
    for i in 0..raw.len() - 1 {
        let (r0, r1) = (raw[i], raw[i + 1]);
        let (lo_x, hi_x) = (
            r0[0].min(r1[0]) - match_distance,
            r0[0].max(r1[0]) + match_distance,
        );
        let (lo_y, hi_y) = (
            r0[1].min(r1[1]) - match_distance,
            r0[1].max(r1[1]) + match_distance,
        );
        let mut seg_min: Option<(usize, f64)> = None;
        for j in 0..nseg {
            let (b0, b1) = (ring[j], ring[j + 1]);
            if b0[0].max(b1[0]) < lo_x
                || b0[0].min(b1[0]) > hi_x
                || b0[1].max(b1[1]) < lo_y
                || b0[1].min(b1[1]) > hi_y
            {
                continue;
            }
            // `segmentMatchFrac`: both ends of the ring segment within the match distance.
            if match_distance < linear::point_segment_distance(b0, r0, r1)
                || match_distance < linear::point_segment_distance(b1, r0, r1)
            {
                continue;
            }
            let frac = linear::projection_factor(r0, r1, b0).clamp(0.0, 1.0);
            let location = i as f64 + frac;
            loc[j] = location;
            if seg_min.is_none_or(|(_, m)| location < m) {
                seg_min = Some((j, location));
            }
        }
        if let Some((j, _)) = seg_min {
            let pos = loc[j];
            if first.is_none() || pos < min_raw {
                min_raw = pos;
                first = Some(j);
            }
        }
    }
    let Some(start_index) = first else {
        return vec![];
    };
    let size = loc.len();
    let next = |i: usize| if i >= size - 1 { 0 } else { i + 1 };
    let prev = |i: usize| if i == 0 { size - 1 } else { i - 1 };
    let find_end = |start: usize| {
        let mut end = start;
        loop {
            let n = next(end);
            if loc[n] == NOT_IN_CURVE {
                return n;
            }
            end = n;
            if end == start || end == start_index {
                return end;
            }
        }
    };
    let find_start = |end: usize| {
        let mut start = end;
        loop {
            let n = next(start);
            if loc[start] == NOT_IN_CURVE {
                start = n;
            } else if loc[prev(start)] == NOT_IN_CURVE {
                return start;
            } else {
                start = n;
            }
            if start == end {
                return start;
            }
        }
    };
    let mut sections = Vec::new();
    let mut section_start = start_index;
    loop {
        let section_end = find_end(section_start);
        let location = loc[section_start];
        let len = if section_end <= section_start {
            ring.len() - section_start + section_end
        } else {
            section_end - section_start + 1
        };
        let pts = (0..len)
            .map(|k| ring[(section_start + k) % (ring.len() - 1)])
            .collect();
        sections.push((location, pts));
        section_start = find_start(section_end);
        if sections.len() > ring.len() {
            break;
        }
        if section_start == start_index || section_end == start_index {
            break;
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn the_quad_segs_128_circle_has_513_coordinates_clockwise_from_east() {
        let c = buffer_point([3.5, -2.25], 10.0, 128).unwrap().exterior;
        assert_eq!(c.len(), 513);
        assert_eq!(c[0], [13.5, -2.25]);
        assert_eq!(c[512], c[0]);
        assert!(
            c[1][1] < -2.25,
            "clockwise: the second vertex is below the start"
        );
    }

    #[test]
    fn a_non_positive_distance_gives_the_empty_polygon() {
        assert_eq!(buffer_point([0.0, 0.0], 0.0, 8), None);
        assert_eq!(
            buffer_line(&[[0.0, 0.0], [1.0, 0.0]], -1.0, 8, Cap::Flat),
            Ok(Geometry::Empty("Polygon"))
        );
    }

    #[test]
    fn a_hairpin_tighter_than_the_buffer_gives_one_polygon_without_holes() {
        // The flat-capped strips along the two long legs overlap ([0, 100] x [-20, 25]); the bend
        // adds the round outer corner beyond x = 100.
        let line = [[0.0, 0.0], [100.0, 0.0], [100.0, 5.0], [0.0, 5.0]];
        let Ok(Geometry::Polygon(p)) = buffer_line(&line, 20.0, 8, Cap::Flat) else {
            panic!("expected one polygon");
        };
        assert!(p.holes.is_empty());
        let a = crate::geom::overlay::polygon_area(&p);
        assert!(a > 100.0 * 45.0 && a < 120.0 * 45.0, "{a}");
    }

    #[test]
    fn simple_and_crossing_rings() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        assert!(ring_is_simple(&square));
        let bow_tie = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0], [0.0, 0.0]];
        assert!(!ring_is_simple(&bow_tie));
    }
}

#[cfg(test)]
mod polygon_buffer_tests {
    //! Label: sourced - hand calculation (inset squares, offsets of straight edges, widths)
    use super::*;
    use crate::geom::overlay::{contains_xy, polygon_area};
    use crate::geom::valid::is_valid;

    fn poly(ring: &[Pt]) -> Polygon {
        let mut r = ring.to_vec();
        r.push(ring[0]);
        Polygon {
            exterior: r,
            holes: vec![],
        }
    }

    fn square(x0: f64, y0: f64, s: f64) -> Vec<Pt> {
        vec![[x0, y0], [x0 + s, y0], [x0 + s, y0 + s], [x0, y0 + s]]
    }

    fn area(ps: &[Polygon]) -> f64 {
        ps.iter().map(polygon_area).sum()
    }

    fn boundary_distance(p: Pt, ring: &[Pt]) -> f64 {
        ring.windows(2)
            .map(|w| linear::point_segment_distance(p, w[0], w[1]))
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn shrinking_a_square_gives_the_inset_square_with_exact_corners() {
        for ring in [square(0.0, 0.0, 100.0), {
            let mut r = square(0.0, 0.0, 100.0);
            r.reverse();
            r
        }] {
            let out = buffer_polygons_q(&[poly(&ring)], -10.0, 16);
            assert_eq!(out.len(), 1);
            assert!(is_valid(&out));
            let ext = &out[0].exterior;
            assert_eq!(ext.len(), 5, "{ext:?}");
            for c in [[10.0, 10.0], [90.0, 10.0], [90.0, 90.0], [10.0, 90.0]] {
                let near = ext
                    .iter()
                    .map(|p| dist(*p, c))
                    .fold(f64::INFINITY, f64::min);
                assert!(near <= 1e-9, "corner {c:?} off by {near}");
            }
            assert!((area(&out) - 6400.0).abs() < 1e-6);
        }
    }

    #[test]
    fn growing_a_square_gives_round_corners_of_radius_d() {
        let sq = poly(&square(0.0, 0.0, 100.0));
        let out = buffer_polygons_q(std::slice::from_ref(&sq), 10.0, 16);
        assert_eq!(out.len(), 1);
        assert!(is_valid(&out));
        let ext = &out[0].exterior;
        // Every vertex lies at exactly d from the square: on an offset edge or on a corner arc.
        for p in ext {
            let e = (boundary_distance(*p, &sq.exterior) - 10.0).abs();
            assert!(e <= 1e-9, "vertex {p:?} off by {e}");
        }
        // Four arcs of 16 segments each: 17 points an arc, plus the closing point.
        assert_eq!(ext.len(), 4 * 17 + 1);
        let on_arc = ext
            .iter()
            .filter(|p| (p[0] < 0.0 || p[0] > 100.0) && (p[1] < 0.0 || p[1] > 100.0))
            .count();
        assert_eq!(on_arc, 4 * 15);
    }

    #[test]
    fn shrinking_a_part_narrower_than_2d_gives_nothing() {
        // A 15 m wide strip turned 30 degrees, so the envelope test does not decide it.
        let (c, s) = (libm::cos(30f64.to_radians()), libm::sin(30f64.to_radians()));
        let rot = |x: f64, y: f64| [x * c - y * s, x * s + y * c];
        let strip = poly(&[
            rot(0.0, 0.0),
            rot(300.0, 0.0),
            rot(300.0, 15.0),
            rot(0.0, 15.0),
        ]);
        assert!(!is_eroded_completely(&strip.exterior, -8.0));
        let out = buffer_polygons_q(std::slice::from_ref(&strip), -8.0, 16);
        assert!(out.is_empty(), "{out:?}");
        assert!(matches!(
            buffer_polygons(std::slice::from_ref(&strip), -8.0),
            Ok(Geometry::Empty("Polygon"))
        ));
        // Just under half the width leaves a sliver.
        assert_eq!(buffer_polygons_q(&[strip], -7.0, 16).len(), 1);
    }

    fn dumbbell(waist: f64) -> Polygon {
        let h0 = 50.0 - waist / 2.0;
        let h1 = 50.0 + waist / 2.0;
        poly(&[
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, h0],
            [150.0, h0],
            [150.0, 0.0],
            [250.0, 0.0],
            [250.0, 100.0],
            [150.0, 100.0],
            [150.0, h1],
            [100.0, h1],
            [100.0, 100.0],
            [0.0, 100.0],
        ])
    }

    #[test]
    fn shrinking_a_narrow_waist_splits_the_polygon_in_two() {
        let out = buffer_polygons_q(&[dumbbell(20.0)], -15.0, 16);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(is_valid(&out));
        // Each part is the 70 m inset square plus a small bulge into the waist's mouth (round
        // joins at the reflex corners).
        assert!(area(&out) > 2.0 * 70.0 * 70.0);
        assert!(contains_xy(&out, [86.0, 50.0]) && contains_xy(&out, [164.0, 50.0]));
        assert!(!contains_xy(&out, [125.0, 50.0]) && !contains_xy(&out, [90.0, 50.0]));
        // A shrink smaller than half the waist keeps one part.
        assert_eq!(buffer_polygons_q(&[dumbbell(20.0)], -9.0, 16).len(), 1);
    }

    #[test]
    fn open_removes_a_sliver_narrower_than_2d() {
        // A square with a 6 m wide spike 40 m long on its right side.
        let p = poly(&[
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 47.0],
            [140.0, 47.0],
            [140.0, 53.0],
            [100.0, 53.0],
            [100.0, 100.0],
            [0.0, 100.0],
        ]);
        let out = open(&[p], 5.0, 16);
        assert!(is_valid(&out));
        assert!(!contains_xy(&out, [120.0, 50.0]));
        assert!(contains_xy(&out, [50.0, 50.0]));
        assert!(contains_xy(&out, [99.0, 50.0]));
    }

    #[test]
    fn close_fills_a_notch_narrower_than_2d() {
        // A square with a 6 m wide notch 50 m deep cut in from the top.
        let p = poly(&[
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 100.0],
            [53.0, 100.0],
            [53.0, 50.0],
            [47.0, 50.0],
            [47.0, 100.0],
            [0.0, 100.0],
        ]);
        assert!(!contains_xy(std::slice::from_ref(&p), [50.0, 75.0]));
        let out = close(&[p], 5.0, 16);
        assert!(is_valid(&out));
        assert_eq!(out.len(), 1);
        assert!(contains_xy(&out, [50.0, 75.0]));
        // The notch's mouth keeps a dimple 1 m deep: the grown arcs at its corners meet at y = 104.
        assert!(contains_xy(&out, [50.0, 98.5]));
        assert!(!contains_xy(&out, [50.0, 99.5]));
        // The outer corners stay square: closing only rounds reflex corners.
        assert!(contains_xy(&out, [0.5, 0.5]));
    }

    #[test]
    fn a_hole_shrinks_when_growing_and_grows_when_shrinking() {
        let mut hole = square(40.0, 40.0, 20.0);
        hole.reverse();
        hole.push(hole[0]);
        let mut p = poly(&square(0.0, 0.0, 100.0));
        p.holes.push(hole);
        let grown = buffer_polygons_q(std::slice::from_ref(&p), 5.0, 16);
        assert!(is_valid(&grown));
        assert_eq!(grown[0].holes.len(), 1);
        assert!(contains_xy(&grown, [44.0, 44.0]));
        assert!(!contains_xy(&grown, [50.0, 50.0]));
        let filled = buffer_polygons_q(std::slice::from_ref(&p), 11.0, 16);
        assert!(filled[0].holes.is_empty());
        let shrunk = buffer_polygons_q(&[p], -5.0, 16);
        assert!(is_valid(&shrunk));
        assert!(!contains_xy(&shrunk, [37.0, 50.0]));
        assert!(contains_xy(&shrunk, [33.0, 50.0]));
    }
}

#[cfg(test)]
mod river_buffer_tests {
    //! Label: property
    use super::*;
    use crate::geom::overlay::polygon_area;
    use crate::geom::valid::is_valid;
    use crate::synthetic::{RiverParams, synthetic_river};

    #[test]
    fn shrinking_and_opening_the_300_m_synthetic_river_gives_one_valid_reach() {
        let river = synthetic_river(RiverParams {
            width_m: 300.0,
            ..RiverParams::default()
        })
        .unwrap();
        let water = std::slice::from_ref(&river.polygon);
        let shrunk = buffer_polygons_q(water, -15.2, 16);
        assert_eq!(shrunk.len(), 1);
        assert!(is_valid(&shrunk));
        assert!(polygon_area(&shrunk[0]) < polygon_area(&river.polygon));
        for poly in [open(&shrunk, 25.0, 16), close(&shrunk, 25.0, 16)] {
            assert_eq!(poly.len(), 1);
            assert!(is_valid(&poly));
        }
    }

    #[test]
    fn shrinking_the_40_m_synthetic_river_by_15_m_leaves_a_valid_strip_under_10_m_wide() {
        let river = synthetic_river(RiverParams::default()).unwrap();
        let shrunk = buffer_polygons_q(std::slice::from_ref(&river.polygon), -15.2, 16);
        assert!(is_valid(&shrunk));
        // Width 40 - 2 x 15.2 = 9.6 m along the straight parts: area < 9.6 x centreline length.
        let len: f64 = river.centreline.windows(2).map(|w| dist(w[0], w[1])).sum();
        let a: f64 = shrunk.iter().map(polygon_area).sum();
        assert!(a > 0.0 && a < 9.6 * len, "area {a} vs {}", 9.6 * len);
    }
}
