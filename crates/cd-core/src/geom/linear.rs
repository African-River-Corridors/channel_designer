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
    let mut points: Vec<Pt> = Vec::new();
    let mut parts: Vec<Vec<Pt>> = Vec::new();
    // A segment of `b` whose box misses the box of all of `a` meets nothing: skip it up front, so
    // a small `a` (a circle) against a long `b` costs one pass over `b`, not one per segment of `a`.
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in a {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let near: Vec<&[Pt]> = b
        .windows(2)
        .filter(|t| (0..2).all(|k| t[0][k].min(t[1][k]) <= hi[k] && t[0][k].max(t[1][k]) >= lo[k]))
        .collect();
    for s in a.windows(2) {
        for t in &near {
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
