//! Line primitives: length, interpolate, project, substring and distance.
//!
//! Follows GEOS `Length::ofLine`, `LengthIndexedLine` (`LengthLocationMap`, `LengthIndexOfPoint`),
//! `Distance::pointToSegment` and shapely's `shapely.ops.substring`.

use super::{Geometry, Pt, SegHit, dist, segment_intersection};

/// `Length::ofLine`: the sum of segment lengths.
pub fn length(line: &[Pt]) -> f64 {
    line.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// `Distance::pointToSegment`.
pub fn point_segment_distance(p: Pt, a: Pt, b: Pt) -> f64 {
    if a == b {
        return dist(p, a);
    }
    let len2 = (b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1]);
    let r = ((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / len2;
    if r <= 0.0 {
        return dist(p, a);
    }
    if r >= 1.0 {
        return dist(p, b);
    }
    let s = ((a[1] - p[1]) * (b[0] - a[0]) - (a[0] - p[0]) * (b[1] - a[1])) / len2;
    s.abs() * len2.sqrt()
}

/// `LineSegment::projectionFactor`.
pub(crate) fn projection_factor(p0: Pt, p1: Pt, p: Pt) -> f64 {
    if p == p0 {
        return 0.0;
    }
    if p == p1 {
        return 1.0;
    }
    let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let len2 = dx * dx + dy * dy;
    ((p[0] - p0[0]) * dx + (p[1] - p0[1]) * dy) / len2
}

/// Where a length falls: segment index and fraction along it (`LinearLocation`).
#[derive(Debug, Clone, Copy)]
struct Location {
    seg: usize,
    frac: f64,
}

/// `LengthLocationMap::getLocationForward`.
fn location_forward(line: &[Pt], len: f64) -> Location {
    let end = Location {
        seg: line.len().saturating_sub(2),
        frac: 1.0,
    };
    if len <= 0.0 {
        return Location { seg: 0, frac: 0.0 };
    }
    let mut total = 0.0;
    for (i, w) in line.windows(2).enumerate() {
        let seg_len = dist(w[0], w[1]);
        if total + seg_len > len {
            return Location {
                seg: i,
                frac: (len - total) / seg_len,
            };
        }
        total += seg_len;
    }
    end
}

/// `LinearLocation::pointAlongSegmentByFraction`.
fn point_at(p0: Pt, p1: Pt, frac: f64) -> Pt {
    if frac <= 0.0 {
        return p0;
    }
    if frac >= 1.0 {
        return p1;
    }
    [
        (p1[0] - p0[0]) * frac + p0[0],
        (p1[1] - p0[1]) * frac + p0[1],
    ]
}

/// shapely `line.interpolate(distance)`: a negative distance counts from the end; a distance past
/// either end gives that end. An empty line gives `None`.
pub fn interpolate(line: &[Pt], distance: f64) -> Option<Pt> {
    if line.is_empty() {
        return None;
    }
    if line.len() == 1 {
        return Some(line[0]);
    }
    let forward = if distance < 0.0 {
        length(line) + distance
    } else {
        distance
    };
    let loc = location_forward(line, forward);
    Some(point_at(line[loc.seg], line[loc.seg + 1], loc.frac))
}

/// shapely `line.interpolate(fraction, normalized=True)`.
pub fn interpolate_normalized(line: &[Pt], fraction: f64) -> Option<Pt> {
    interpolate(line, fraction * length(line))
}

/// [`project`] for many points on one long line: the same answer to the last bit, found through a
/// grid of the line's segments instead of a scan of all of them. A point's nearest segment is
/// searched in rings of cells around it, until no unread cell can hold a closer segment; among
/// segments at the same distance the first wins, as in [`project`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectIndex {
    line: Vec<Pt>,
    /// The chainage where each segment starts, summed in order as [`project`] sums it.
    start: Vec<f64>,
    origin: Pt,
    cell: f64,
    cols: usize,
    rows: usize,
    /// For each cell, row-major, the segments whose box meets it, in order.
    cells: Vec<Vec<usize>>,
    /// Every coordinate of the line is finite (else `segments_meeting` reads every segment).
    finite: bool,
}

impl ProjectIndex {
    pub fn new(line: &[Pt]) -> ProjectIndex {
        let mut start = Vec::with_capacity(line.len().saturating_sub(1));
        let mut at = 0.0;
        for w in line.windows(2) {
            start.push(at);
            at += dist(w[0], w[1]);
        }
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in line {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        // About as many cells as segments, at least 1 m across.
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let n = start.len().max(1) as f64;
        let cell = (span / n.sqrt()).max(1.0);
        let cols = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let rows = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut cells = vec![Vec::new(); if line.len() < 2 { 0 } else { cols * rows }];
        for (i, w) in line.windows(2).enumerate() {
            let c0 = ((w[0][0].min(w[1][0]) - lo[0]) / cell).floor() as usize;
            let c1 = ((w[0][0].max(w[1][0]) - lo[0]) / cell).floor() as usize;
            let r0 = ((w[0][1].min(w[1][1]) - lo[1]) / cell).floor() as usize;
            let r1 = ((w[0][1].max(w[1][1]) - lo[1]) / cell).floor() as usize;
            for r in r0..=r1.min(rows - 1) {
                for c in c0..=c1.min(cols - 1) {
                    cells[r * cols + c].push(i);
                }
            }
        }
        ProjectIndex {
            line: line.to_vec(),
            start,
            origin: lo,
            cell,
            cols,
            rows,
            cells,
            finite: line.iter().flatten().all(|v| v.is_finite()),
        }
    }

    /// The chainage of `p` on the line, as [`project`] gives it.
    pub fn project(&self, p: Pt) -> f64 {
        if self.line.len() < 2 || !(p[0].is_finite() && p[1].is_finite()) {
            return project(&self.line, p);
        }
        let fc = ((p[0] - self.origin[0]) / self.cell).floor();
        let fr = ((p[1] - self.origin[1]) / self.cell).floor();
        // The distance from `p` to the nearest edge of its own cell, so that every cell outside
        // ring `k` lies farther than `k * cell - edge`.
        let (pc, pr) = (fc as i64, fr as i64);
        let mut best: Option<(f64, usize)> = None;
        let far = self.cols.max(self.rows) as i64 + (pc.abs()).max(pr.abs()) + 1;
        let mut k: i64 = 0;
        loop {
            for r in pr - k..=pr + k {
                for c in pc - k..=pc + k {
                    if (r - pr).abs() != k && (c - pc).abs() != k {
                        continue;
                    }
                    if r < 0 || c < 0 || r >= self.rows as i64 || c >= self.cols as i64 {
                        continue;
                    }
                    for &i in &self.cells[r as usize * self.cols + c as usize] {
                        let d = point_segment_distance(p, self.line[i], self.line[i + 1]);
                        if best.is_none_or(|(bd, bi)| d < bd || (d == bd && i < bi)) {
                            best = Some((d, i));
                        }
                    }
                }
            }
            // Every unread cell lies at least `k * cell` from `p`'s own cell; a segment there is
            // at least that far from `p`. Stop only when the best is strictly closer, so a tie with
            // an earlier segment farther out is still found.
            if best.is_some_and(|(bd, _)| bd < k as f64 * self.cell) {
                break;
            }
            if k > far {
                break;
            }
            k += 1;
        }
        let Some((_, i)) = best else {
            return project(&self.line, p);
        };
        let (a, b) = (self.line[i], self.line[i + 1]);
        let seg_len = dist(a, b);
        let proj = projection_factor(a, b, p);
        if proj <= 0.0 {
            self.start[i]
        } else if proj <= 1.0 {
            self.start[i] + proj * seg_len
        } else {
            self.start[i] + seg_len
        }
    }

    /// The segments of the line whose box meets the box `lo..hi`, in line order: the segments
    /// [`line_intersection`] reads of its second line when its first has that box.
    pub fn segments_meeting(&self, lo: Pt, hi: Pt) -> Vec<usize> {
        let meets = |i: usize| {
            let t = [self.line[i], self.line[i + 1]];
            (0..2).all(|k| t[0][k].min(t[1][k]) <= hi[k] && t[0][k].max(t[1][k]) >= lo[k])
        };
        let n = self.line.len().saturating_sub(1);
        if self.cells.is_empty() || !self.finite || !(lo.iter().chain(&hi).all(|v| v.is_finite())) {
            return (0..n).filter(|&i| meets(i)).collect();
        }
        // A cell range of the box; a segment is in every cell its own box meets, and the map from a
        // coordinate to a cell is monotone, so each segment that meets the box is in one of these.
        let cell = |v: f64, o: f64, count: usize| -> usize {
            let k = ((v - o) / self.cell).floor();
            if k < 0.0 {
                0
            } else {
                (k as usize).min(count - 1)
            }
        };
        let (c0, c1) = (
            cell(lo[0], self.origin[0], self.cols),
            cell(hi[0], self.origin[0], self.cols),
        );
        let (r0, r1) = (
            cell(lo[1], self.origin[1], self.rows),
            cell(hi[1], self.origin[1], self.rows),
        );
        let mut out: Vec<usize> = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                out.extend(&self.cells[r * self.cols + c]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out.retain(|&i| meets(i));
        out
    }
}

/// shapely `line.project(point)` (`LengthIndexOfPoint::indexOfFromStart`): the length along the
/// line of the point nearest `p`. The first nearest segment wins a tie.
pub fn project(line: &[Pt], p: Pt) -> f64 {
    let mut min_distance = f64::MAX;
    let mut measure = -1.0;
    let mut start_measure = 0.0;
    for w in line.windows(2) {
        let seg_distance = point_segment_distance(p, w[0], w[1]);
        let seg_len = dist(w[0], w[1]);
        let proj = projection_factor(w[0], w[1], p);
        let to_pt = if proj <= 0.0 {
            start_measure
        } else if proj <= 1.0 {
            start_measure + proj * seg_len
        } else {
            start_measure + seg_len
        };
        if seg_distance < min_distance && to_pt > -1.0 {
            measure = to_pt;
            min_distance = seg_distance;
        }
        start_measure += seg_len;
    }
    measure
}

/// shapely `substring(line, start, end)` (the distance form). Returns a point when the two ends
/// meet, and a reversed line when `start` is past `end`.
pub fn substring(line: &[Pt], start: f64, end: f64) -> Geometry {
    let len = length(line);
    let pt = |d: f64| Geometry::Point(interpolate(line, d).unwrap_or([f64::NAN, f64::NAN]));
    if start == end {
        return pt(start);
    }
    if start >= len && end >= len {
        return pt(len);
    }
    if -start >= len && -end >= len {
        return pt(0.0);
    }
    if start < 0.0 && 0.0 < end && start.abs() + end == len {
        return pt(end);
    }
    if end < 0.0 && 0.0 < start && end.abs() + start == len {
        return pt(start);
    }
    let start_point = interpolate(line, start).unwrap_or([f64::NAN, f64::NAN]);
    let end_point = interpolate(line, end).unwrap_or([f64::NAN, f64::NAN]);
    let mut start_dist = if start < 0.0 { len + start } else { start };
    let mut end_dist = if end < 0.0 { len + end } else { end };
    let reverse = start_dist > end_dist;
    if reverse {
        core::mem::swap(&mut start_dist, &mut end_dist);
    }
    start_dist = start_dist.max(0.0);
    let mut vertices = vec![if reverse { end_point } else { start_point }];
    let mut current = 0.0;
    for w in line.windows(2) {
        let (p1, p2) = (w[0], w[1]);
        if start_dist < current && current < end_dist {
            vertices.push(p1);
        } else if current >= end_dist {
            break;
        }
        let (dx, dy) = (p2[0] - p1[0], p2[1] - p1[1]);
        current += (dx * dx + dy * dy).sqrt();
    }
    if reverse {
        vertices.push(start_point);
        vertices.reverse();
    } else {
        vertices.push(end_point);
    }
    Geometry::LineString(vertices)
}

/// Distance from a point to a line (the minimum over its segments), as shapely `distance`.
pub fn point_line_distance(p: Pt, line: &[Pt]) -> f64 {
    match line.len() {
        0 => f64::NAN,
        1 => dist(p, line[0]),
        _ => line
            .windows(2)
            .map(|w| point_segment_distance(p, w[0], w[1]))
            .fold(f64::INFINITY, f64::min),
    }
}

/// `Distance::segmentToSegment`: zero if the segments meet, else the least end-to-segment distance.
pub fn segment_segment_distance(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    if a == b {
        return point_segment_distance(a, c, d);
    }
    if c == d {
        return point_segment_distance(d, a, b);
    }
    let envelopes_meet = a[0].min(b[0]) <= c[0].max(d[0])
        && a[0].max(b[0]) >= c[0].min(d[0])
        && a[1].min(b[1]) <= c[1].max(d[1])
        && a[1].max(b[1]) >= c[1].min(d[1]);
    let mut no_intersection = !envelopes_meet;
    if !no_intersection {
        let denom = (b[0] - a[0]) * (d[1] - c[1]) - (b[1] - a[1]) * (d[0] - c[0]);
        if denom == 0.0 {
            no_intersection = true;
        } else {
            let r_num = (a[1] - c[1]) * (d[0] - c[0]) - (a[0] - c[0]) * (d[1] - c[1]);
            let s_num = (a[1] - c[1]) * (b[0] - a[0]) - (a[0] - c[0]) * (b[1] - a[1]);
            let (s, r) = (s_num / denom, r_num / denom);
            if !(0.0..=1.0).contains(&r) || !(0.0..=1.0).contains(&s) {
                no_intersection = true;
            }
        }
    }
    if no_intersection {
        return point_segment_distance(a, c, d)
            .min(point_segment_distance(b, c, d))
            .min(point_segment_distance(c, a, b))
            .min(point_segment_distance(d, a, b));
    }
    0.0
}

/// Distance between two lines.
pub fn line_line_distance(a: &[Pt], b: &[Pt]) -> f64 {
    if a.len() < 2 || b.len() < 2 {
        return f64::NAN;
    }
    let mut best = f64::INFINITY;
    for s in a.windows(2) {
        for t in b.windows(2) {
            best = best.min(segment_segment_distance(s[0], s[1], t[0], t[1]));
            if best == 0.0 {
                return 0.0;
            }
        }
    }
    best
}

/// The points and collinear overlaps where two lines meet (shapely `a.intersection(b)` for two
/// lines, as the package reads it: points, and line parts by their ends). Points are distinct and
/// none lies on an overlap; overlaps are merged into maximal lines. Neither list is in GEOS's
/// order: callers sort or pick by a measure.
pub fn line_intersection(a: &[Pt], b: &[Pt]) -> (Vec<Pt>, Vec<Vec<Pt>>) {
    // A segment of `b` whose box misses the box of all of `a` meets nothing: skip it up front, so
    // a small `a` (a circle) against a long `b` costs one pass over `b`, not one per segment of `a`.
    let (lo, hi) = bounds(a);
    let near: Vec<&[Pt]> = b
        .windows(2)
        .filter(|t| (0..2).all(|k| t[0][k].min(t[1][k]) <= hi[k] && t[0][k].max(t[1][k]) >= lo[k]))
        .collect();
    intersect_near(a, &near)
}

/// [`line_intersection`] of `a` with the line `index` holds, its segments found through the index's
/// grid: the same answer, without a pass over the whole line.
pub fn line_intersection_indexed(a: &[Pt], index: &ProjectIndex) -> (Vec<Pt>, Vec<Vec<Pt>>) {
    let (lo, hi) = bounds(a);
    let near: Vec<&[Pt]> = index
        .segments_meeting(lo, hi)
        .into_iter()
        .map(|i| &index.line[i..i + 2])
        .collect();
    intersect_near(a, &near)
}

/// The box of `a`'s points (`min`/`max`, so a NaN coordinate is passed over).
fn bounds(a: &[Pt]) -> (Pt, Pt) {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in a {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// The segments of `near` by cells of a grid over `a`'s box, for [`intersect_near`]. A segment is
/// listed in every cell its box meets (cells clamped to the grid), and the map from a coordinate to
/// a cell is monotone, so two segments whose boxes meet share a cell. Without a grid (few segments,
/// or a coordinate that is not finite) every segment is a candidate.
struct NearGrid {
    origin: Pt,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
}

impl NearGrid {
    fn new(a: &[Pt], near: &[&[Pt]]) -> Option<NearGrid> {
        let finite = |p: &Pt| p[0].is_finite() && p[1].is_finite();
        if a.len() * near.len() < 4096
            || !a.iter().all(finite)
            || !near.iter().all(|t| t.iter().all(finite))
        {
            return None;
        }
        let (lo, hi) = bounds(a);
        let n = (a.len() as f64).sqrt().ceil().max(1.0);
        let cell = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / n).max(1e-9);
        let cols = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let rows = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut g = NearGrid {
            origin: lo,
            cell,
            cols,
            rows,
            cells: vec![Vec::new(); cols * rows],
        };
        for (j, t) in near.iter().enumerate() {
            let (c0, c1, r0, r1) = g.range(t[0], t[1]);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    g.cells[r * cols + c].push(j);
                }
            }
        }
        Some(g)
    }

    /// The cells the box of `p..q` meets, clamped to the grid: columns then rows, inclusive.
    fn range(&self, p: Pt, q: Pt) -> (usize, usize, usize, usize) {
        let at = |v: f64, o: f64, count: usize| -> usize {
            let k = ((v - o) / self.cell).floor();
            if k < 0.0 {
                0
            } else {
                (k as usize).min(count - 1)
            }
        };
        (
            at(p[0].min(q[0]), self.origin[0], self.cols),
            at(p[0].max(q[0]), self.origin[0], self.cols),
            at(p[1].min(q[1]), self.origin[1], self.rows),
            at(p[1].max(q[1]), self.origin[1], self.rows),
        )
    }
}

trait Candidates {
    fn candidates(&self, p: Pt, q: Pt, count: usize, out: &mut Vec<usize>);
}

impl Candidates for Option<NearGrid> {
    /// The segments that may meet `p..q`, in order, into `out`.
    fn candidates(&self, p: Pt, q: Pt, count: usize, out: &mut Vec<usize>) {
        out.clear();
        let Some(g) = self else {
            out.extend(0..count);
            return;
        };
        let (c0, c1, r0, r1) = g.range(p, q);
        for r in r0..=r1 {
            for c in c0..=c1 {
                out.extend(&g.cells[r * g.cols + c]);
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// The meeting of `a` with the segments `near` (each a pair of points), in the order given.
fn intersect_near(a: &[Pt], near: &[&[Pt]]) -> (Vec<Pt>, Vec<Vec<Pt>>) {
    let mut points: Vec<Pt> = Vec::new();
    let mut parts: Vec<Vec<Pt>> = Vec::new();
    let grid = NearGrid::new(a, near);
    let mut candidates: Vec<usize> = Vec::new();
    for s in a.windows(2) {
        // Only the segments of `near` whose box may meet `s`'s, in `near`'s order: for any other,
        // `segment_intersection` gives `SegHit::None`, so skipping it changes nothing.
        grid.candidates(s[0], s[1], near.len(), &mut candidates);
        for &j in &candidates {
            let t = near[j];
            match segment_intersection(s[0], s[1], t[0], t[1]) {
                SegHit::None => {}
                SegHit::Point { p, .. } => {
                    if !points.contains(&p) {
                        points.push(p);
                    }
                }
                SegHit::Overlap { a: p, b: q } => parts.push(vec![p, q]),
            }
        }
    }
    let lines = super::merge::line_merge(&parts);
    points.retain(|p| !lines.iter().any(|l| point_line_distance(*p, l) == 0.0));
    (points, lines)
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn interpolate_and_project_agree_on_a_bent_line() {
        let line = [[0.0, 0.0], [100.0, 0.0], [100.0, 60.0]];
        assert_eq!(length(&line), 160.0);
        assert_eq!(interpolate(&line, 130.0), Some([100.0, 30.0]));
        assert_eq!(interpolate(&line, -10.0), Some([100.0, 50.0]));
        assert_eq!(interpolate(&line, 1e9), Some([100.0, 60.0]));
        assert_eq!(project(&line, [130.0, 30.0]), 130.0);
    }

    #[test]
    fn crossing_lines_meet_at_points_and_overlaps() {
        let (pts, lines) =
            line_intersection(&[[0.0, 0.0], [20.0, 0.0]], &[[0.0, 10.0], [20.0, -10.0]]);
        assert_eq!((pts, lines.len()), (vec![[10.0, 0.0]], 0));
        let (pts, lines) =
            line_intersection(&[[0.0, 0.0], [20.0, 0.0]], &[[10.0, 0.0], [30.0, 0.0]]);
        assert!(pts.is_empty());
        assert_eq!(lines, vec![vec![[10.0, 0.0], [20.0, 0.0]]]);
    }
}

#[cfg(test)]
mod project_index_tests {
    //! Label: property (the index gives what the scan gives, to the last bit)
    use super::*;

    #[test]
    fn the_indexed_meeting_of_a_circle_and_a_line_is_the_scan_s() {
        // Wandering lines (with repeated points, sharp turns, and stretches that run along a
        // circle's edge), met by 512-sided circles placed on and near them: the points and the
        // shared stretches, in order, as `line_intersection` gives them.
        let mut x: u64 = 29;
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut meetings = 0;
        for case in 0..40 {
            let n = 2 + (next() * 1500.0) as usize;
            let mut line = vec![[500_000.0, 600_000.0]];
            let mut heading: f64 = next() * std::f64::consts::TAU;
            for i in 1..n {
                heading += (next() - 0.5) * if case % 5 == 0 { 3.0 } else { 0.4 };
                let step = if i % 41 == 0 {
                    0.0
                } else {
                    1.0 + next() * 20.0
                };
                let last = line[line.len() - 1];
                line.push([
                    last[0] + step * libm::cos(heading),
                    last[1] + step * libm::sin(heading),
                ]);
            }
            let index = ProjectIndex::new(&line);
            for k in 0..25 {
                let at = line[(next() * (line.len() - 1) as f64) as usize];
                let r = 20.0 + next() * 300.0;
                let centre = [at[0] + (next() - 0.5) * r, at[1] + (next() - 0.5) * r];
                let mut ring: Vec<Pt> = (0..512)
                    .map(|j| {
                        let t = j as f64 * std::f64::consts::TAU / 512.0;
                        [centre[0] + r * libm::cos(t), centre[1] + r * libm::sin(t)]
                    })
                    .collect();
                ring.push(ring[0]);
                if k == 0 && line.len() > 30 {
                    // a ring that runs along the line itself, for shared stretches
                    ring = line[5..30].to_vec();
                }
                let want = line_intersection(&ring, &line);
                let got = line_intersection_indexed(&ring, &index);
                assert_eq!(
                    format!("{got:?}"),
                    format!("{want:?}"),
                    "case {case}, ring {k}"
                );
                meetings += want.0.len() + want.1.len();
            }
        }
        assert!(meetings > 1000, "{meetings}");
    }

    #[test]
    fn the_index_projects_every_point_to_the_chainage_the_scan_gives() {
        let mut x: u64 = 11;
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        for case in 0..60 {
            // A wandering river line, sometimes with repeated points and a sharp turn back.
            let n = 2 + (next() * 400.0) as usize;
            let mut line = vec![[500_000.0, 600_000.0]];
            let mut heading: f64 = next() * std::f64::consts::TAU;
            for i in 1..n {
                heading += (next() - 0.5) * if case % 5 == 0 { 3.0 } else { 0.6 };
                let step = if i % 37 == 0 {
                    0.0
                } else {
                    5.0 + next() * 200.0
                };
                let last = line[line.len() - 1];
                line.push([
                    last[0] + step * libm::cos(heading),
                    last[1] + step * libm::sin(heading),
                ]);
            }
            let index = ProjectIndex::new(&line);
            for _ in 0..300 {
                let a = line[(next() * (n as f64)) as usize % n];
                let spread = if next() < 0.2 { 20_000.0 } else { 600.0 };
                let p = [
                    a[0] + (next() - 0.5) * spread,
                    a[1] + (next() - 0.5) * spread,
                ];
                assert_eq!(
                    index.project(p).to_bits(),
                    project(&line, p).to_bits(),
                    "case {case}, {p:?}"
                );
            }
            // A vertex itself, where two segments tie.
            for v in line.iter().take(20) {
                assert_eq!(index.project(*v).to_bits(), project(&line, *v).to_bits());
            }
        }
        assert_eq!(
            ProjectIndex::new(&[[0.0, 0.0]]).project([1.0, 1.0]),
            project(&[[0.0, 0.0]], [1.0, 1.0])
        );
        let l = [[0.0, 0.0], [10.0, 0.0]];
        assert_eq!(
            ProjectIndex::new(&l).project([f64::NAN, 1.0]).to_bits(),
            project(&l, [f64::NAN, 1.0]).to_bits()
        );
    }
}
