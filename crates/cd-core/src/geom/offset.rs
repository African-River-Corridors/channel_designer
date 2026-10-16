//! Offset curves and buffers of points and lines, built as GEOS 3.14 builds them.
//!
//! Follows `BufferInputLineSimplifier`, `OffsetSegmentGenerator`, `OffsetSegmentString`,
//! `OffsetCurveBuilder`, `BufferCurveSetBuilder::addPoint` / `addLineString` and `OffsetCurve`.
//! Round joins only (the package uses no other).
//!
//! For a line whose buffer curve does not cross itself, GEOS's noding finds nothing to split and
//! its polygon builder returns the raw curve as the shell, with the same start and direction. That
//! is the case built here. A curve that crosses itself (an offset wider than the bend, a closing
//! segment at a sharp inside turn) needs noding and overlay, and returns
//! [`GeomError::NeedsGeneralBuffer`] (ticket 43).

use super::{GeomError, Geometry, Polygon, Pt, dist, linear, orientation};
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
    if !ring_is_simple(&ring) {
        return Err(GeomError::NeedsGeneralBuffer(
            "the buffer curve of the line crosses itself".into(),
        ));
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
    Ok(match line_buffer_ring(line, distance, quad_segs, cap)? {
        Some(ring) => Geometry::Polygon(Polygon {
            exterior: ring,
            holes: vec![],
        }),
        None => Geometry::Empty("Polygon"),
    })
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
    let Some(mut ring) = line_buffer_ring(line, distance.abs(), quad, Cap::Round)? else {
        return Ok(Geometry::Empty("LineString"));
    };
    if distance < 0.0 {
        ring.reverse();
    }
    let match_distance = distance.abs() / MATCH_DISTANCE_FACTOR;
    let mut sections = curve_sections(&ring, &raw, match_distance);
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
    fn a_curve_that_folds_needs_the_general_buffer() {
        // A hairpin far tighter than the buffer distance.
        let line = [[0.0, 0.0], [100.0, 0.0], [100.0, 5.0], [0.0, 5.0]];
        assert!(matches!(
            buffer_line(&line, 20.0, 8, Cap::Flat),
            Err(GeomError::NeedsGeneralBuffer(_))
        ));
    }

    #[test]
    fn simple_and_crossing_rings() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        assert!(ring_is_simple(&square));
        let bow_tie = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0], [0.0, 0.0]];
        assert!(!ring_is_simple(&bow_tie));
    }
}
