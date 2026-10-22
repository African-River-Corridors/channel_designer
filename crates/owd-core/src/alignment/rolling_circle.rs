//! Rolling-circle meander straightening (`alignment/rolling_circle.py`).
//!
//! Per side and per pass: resample the line every `step`; at each sample place a circle of radius
//! `min_radius` tangent to the line on that side; find the furthest forward point where the circle
//! touches the line again (more than `min_chainage_gap` along it); if there is one, replace the
//! line from the tangent point by the circle arc and a tangent exit (below), and resume from where
//! the exit joins the old line.
//!
//! **Tangent entry (issue #283).** The tangent point T is a vertex of the line: the last vertex at
//! or before the sample. The circle is tangent at T to the segment into T (from the last vertex the
//! splice keeps before T). The splice replaces the line from T, so the line's heading is continuous
//! at T, and the old line's corner at T goes with the replaced part. As for the exit, only vertices
//! are joins. Before #283 the circle was placed at the sample, tangent to the direction read on the
//! resample, which could differ from the line's own heading by most of a degree.
//!
//! **Tangent exit (from 0.3.0, issue #5): circle, then straight, as in a Dubins path.** The arc keeps radius `min_radius` past the
//! touch, and the line leaves the circle along a straight that is tangent to the circle and meets the
//! old line at a join point Q with the old line's heading. The rule:
//!
//! - Walk the old line's vertices forward from the touch. At each vertex V outside the circle, take
//!   the tangent from V to the circle on the circle's side (the one that continues the arc's turn):
//!   its heading h(V) and its tangent point E on the circle.
//! - The heading error at V is h(V) minus the heading of the segment into V, and minus the heading of
//!   the segment out of V. At the touch the error has the sign of the arc's turn (the old line leaves
//!   the circle outward). Q is the first vertex where the error on the segment out of V reaches zero
//!   or changes sign. There h(V) lies between the two segment headings, so the heading step at Q is no
//!   larger than the old line's own turn at that vertex; at E the arc and the straight share a heading.
//! - If the error starts to grow before it reaches zero (the old line curves away from the
//!   straight), or the line ends first, Q is the vertex of least error, but only if that error is
//!   within `EXIT_HEADING_TOL_RAD` (0.25 degree). Otherwise the circle **falls back** to the
//!   crossing exit of before 0.3.0: the arc ends at the touch. The circle's record says which exit
//!   it took (`tangent_exit`). A fallback never fails.
//! - The fallback is for pass 1 only (#283). From pass 2 on, a circle with no tangent exit is not
//!   spliced, and the roll moves on to the next sample: later passes refine the line and never add a
//!   kink. So every circle of pass 2 or later has `tangent_exit: true`.
//! - The arc has a vertex at least every `ARC_MAX_SEGMENT_M` (1 m), so it reads its own radius on a
//!   short chord too. Before 0.3.0 it had 32 segments whatever its length.
//!
//! This is exact on the polyline: within one straight segment the error cannot change sign (the
//! segment's line would have to be tangent to the circle), so only vertices can be joins. Plain
//! arithmetic and `libm` only, so it is the same on every platform.
//!
//! Search class (ADR 0005): the one discrete choice is the circle contact. The circle is the
//! 512-sided polygon GEOS builds for `quad_segs=128`, never an exact circle. The tangent exit uses
//! the exact circle.

pub mod water;
pub use water::{BendRung, Rung, WaterAwareResult, WaterInput, rolling_circle_with_water};

use super::PyError;
use super::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::offset::buffer_point;
use crate::geom::{Geometry, Pt, linear, merge};
use crate::numpy::{
    allclose2, interp, isclose, linspace, np_mod, searchsorted_left, searchsorted_right,
};
use crate::py_max;

/// One circle spliced in.
#[derive(Debug, Clone, PartialEq)]
pub struct Circle {
    pub pass: i64,
    pub side: String,
    pub centre: Pt,
    pub radius_m: f64,
    pub tangent_point: Pt,
    /// Where the splice joins the old line: the tangent exit's join point Q, or the touch on a
    /// fallback.
    pub contact_point: Pt,
    /// Where the arc ends: the straight's tangent point E, or the touch on a fallback.
    pub exit_point: Pt,
    /// True when the circle took the tangent exit; false when it fell back to the crossing exit.
    pub tangent_exit: bool,
    pub start_chainage_m: f64,
    /// The chainage of `contact_point` on the line before the splice.
    pub end_chainage_m: f64,
}

/// `RollingCircleResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct RollingCircleResult {
    pub centreline: Vec<Pt>,
    /// The radius requested (the circle's), not a measurement.
    pub min_radius_m: f64,
    pub circles: Vec<Circle>,
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    pub radius_ok: bool,
}

/// What `find_forward_circle_contact` returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    pub contact_point: Pt,
    pub centre: Pt,
    pub tangent_point: Pt,
    pub contact_chainage: f64,
}

/// The single line to straighten (the longest part of a multi-part line, after merging).
pub fn primary_line(geom: &Geometry) -> Result<Vec<Pt>, PyError> {
    match geom {
        Geometry::LineString(l) => Ok(l.clone()),
        Geometry::MultiLineString(parts) => {
            let merged = merge::line_merge(parts);
            if merged.len() == 1 {
                return Ok(merged.into_iter().next().unwrap_or_default());
            }
            // Python `max(..., key=length)`: the first of equal lengths.
            let mut best: Option<Vec<Pt>> = None;
            for part in merged {
                let better = best
                    .as_ref()
                    .is_none_or(|b| linear::length(&part) > linear::length(b));
                if better {
                    best = Some(part);
                }
            }
            best.ok_or_else(|| PyError::value("max() iterable argument is empty"))
        }
        _ => Err(PyError::value(
            "Centreline geometry must be a LineString or convertible to one.",
        )),
    }
}

fn seg_len(a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (dx * dx + dy * dy).sqrt()
}

/// Resample at approximately `step` metre spacing.
pub fn resample_coordinate_sequence(points: &[Pt], step: f64) -> Result<Vec<Pt>, PyError> {
    if points.len() < 2 {
        return Err(PyError::value(
            "Centreline must contain at least two vertices.",
        ));
    }
    if step <= 0.0 {
        return Err(PyError::value("Resampling step must be positive."));
    }
    let cumulative = cumulative_chainages(points);
    let total = cumulative[cumulative.len() - 1];
    if total <= 0.0 {
        return Err(PyError::value(
            "Centreline has zero length; cannot resample.",
        ));
    }
    let n_samples = ((total / step).ceil() as i64 + 1).max(3) as usize;
    let xs: Vec<f64> = points.iter().map(|p| p[0]).collect();
    let ys: Vec<f64> = points.iter().map(|p| p[1]).collect();
    Ok(linspace(0.0, total, n_samples)
        .into_iter()
        .map(|t| [interp(t, &cumulative, &xs), interp(t, &cumulative, &ys)])
        .collect())
}

/// Running chainage of each point, from 0.
pub fn cumulative_chainages(points: &[Pt]) -> Vec<f64> {
    match points.len() {
        0 => vec![],
        1 => vec![0.0],
        _ => {
            let mut out = Vec::with_capacity(points.len());
            let mut acc = 0.0;
            out.push(acc);
            for w in points.windows(2) {
                acc += seg_len(w[0], w[1]);
                out.push(acc);
            }
            out
        }
    }
}

/// The unit tangent at `index` (a one-sided difference at the ends); zero if there is none.
pub fn compute_unit_tangent(points: &[Pt], index: i64) -> Pt {
    let n = points.len() as i64;
    if n < 2 {
        return [0.0, 0.0];
    }
    let direction = if index <= 0 {
        [points[1][0] - points[0][0], points[1][1] - points[0][1]]
    } else if index >= n - 1 {
        let (a, b) = (points[(n - 2) as usize], points[(n - 1) as usize]);
        [b[0] - a[0], b[1] - a[1]]
    } else {
        let i = index as usize;
        [
            points[i + 1][0] - points[i - 1][0],
            points[i + 1][1] - points[i - 1][1],
        ]
    };
    let norm = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
    if norm == 0.0 {
        return [0.0, 0.0];
    }
    [direction[0] / norm, direction[1] / norm]
}

/// Centre and tangent angle of a circle tangent to the line at `points[index]`; `side` is
/// `"left"` or `"right"` of the direction of increasing chainage.
pub fn place_circle_at_point(
    points: &[Pt],
    index: i64,
    radius: f64,
    side: &str,
) -> Result<(Pt, f64), PyError> {
    if !(0 <= index && index < points.len() as i64) {
        return Err(PyError::index(format!(
            "Point index {index} out of range for {} samples.",
            points.len()
        )));
    }
    if side != "left" && side != "right" {
        return Err(PyError::value(format!(
            "Side must be 'left' or 'right'; received {}.",
            crate::pyrepr::repr_str(side)
        )));
    }
    if radius <= 0.0 {
        return Err(PyError::value("Radius must be positive."));
    }
    let tangent = compute_unit_tangent(points, index);
    // np.allclose(tangent, 0.0): the default absolute tolerance is 1e-8.
    if tangent[0].abs() <= 1e-8 && tangent[1].abs() <= 1e-8 {
        return Err(PyError::value(format!(
            "Cannot compute tangent at index {index}; insufficient variation."
        )));
    }
    let mut normal = [-tangent[1], tangent[0]];
    if side == "right" {
        normal = [-normal[0], -normal[1]];
    }
    let p = points[index as usize];
    let centre = [p[0] + normal[0] * radius, p[1] + normal[1] * radius];
    Ok((centre, libm::atan2(tangent[1], tangent[0])))
}

/// A tangent entry (module doc): the arc starts at `tangent_point` (T, a vertex of the line at
/// `tangent_distance`), on a circle with centre `centre` that is tangent to the segment into T.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TangentEntry {
    pub tangent_point: Pt,
    pub tangent_distance: f64,
    pub centre: Pt,
}

/// The tangent entry for the sample at chainage `sample_distance` on `line` (module doc), or `None`
/// when the vertex T is the line's start (no segment comes into it). `side` is `"left"` or
/// `"right"`, as for `place_circle_at_point`.
pub fn find_tangent_entry(
    line: &[Pt],
    sample_distance: f64,
    radius: f64,
    side: &str,
) -> Option<TangentEntry> {
    if line.len() < 2 || radius.is_nan() || radius <= 0.0 || !sample_distance.is_finite() {
        return None;
    }
    let chainages = cumulative_chainages(line);
    // T: the last vertex at or before the sample.
    let t = searchsorted_right(&chainages, sample_distance + SPLICE_TOL_M).checked_sub(1)?;
    let tangent_point = line[t];
    let tangent_distance = chainages[t];
    // P: the last vertex the splice keeps before T (`replace_segment_with_arc`), so P to T is the
    // segment into T on the spliced line.
    let p = searchsorted_left(&chainages, tangent_distance - SPLICE_TOL_M).checked_sub(1)?;
    let (dx, dy) = (tangent_point[0] - line[p][0], tangent_point[1] - line[p][1]);
    let norm = (dx * dx + dy * dy).sqrt();
    if norm.is_nan() || norm == 0.0 {
        return None;
    }
    // The unit normal towards the circle's side.
    let normal = if side == "right" {
        [dy / norm, -dx / norm]
    } else {
        [-dy / norm, dx / norm]
    };
    Some(TangentEntry {
        tangent_point,
        tangent_distance,
        centre: [
            tangent_point[0] + normal[0] * radius,
            tangent_point[1] + normal[1] * radius,
        ],
    })
}

/// The start angle and the signed sweep (rad) of the arc from `start_point` to `end_point` about
/// `centre`, turning to `side`.
fn arc_sweep(centre: Pt, start_point: Pt, end_point: Pt, side: &str) -> (f64, f64) {
    let angle_start = libm::atan2(start_point[1] - centre[1], start_point[0] - centre[0]);
    let angle_end = libm::atan2(end_point[1] - centre[1], end_point[0] - centre[0]);
    let pi = core::f64::consts::PI;
    let mut delta = angle_end - angle_start;
    delta = np_mod(delta + pi, 2.0 * pi) - pi;
    if side == "left" && delta <= 0.0 {
        delta += 2.0 * pi;
    } else if side == "right" && delta >= 0.0 {
        delta -= 2.0 * pi;
    }
    (angle_start, delta)
}

/// The longest chord (m) between two vertices of a spliced arc. A polygon arc reads tighter than its
/// radius on a short chord when its vertices are far apart (32 vertices on a 400 m arc of 300 m
/// read 239 m on a 10 m resample), so the arc gets at least one vertex a metre.
pub const ARC_MAX_SEGMENT_M: f64 = 1.0;

/// The `num_segments` for a spliced arc: at least 32 (as before 0.3.0), and enough that no chord
/// is longer than `ARC_MAX_SEGMENT_M`.
fn arc_segments(radius: f64, centre: Pt, start_point: Pt, end_point: Pt, side: &str) -> i64 {
    let (_, delta) = arc_sweep(centre, start_point, end_point, side);
    // `num_segments + 2` points make `num_segments + 1` chords.
    let chords = (delta.abs() * radius / ARC_MAX_SEGMENT_M).ceil();
    if chords.is_finite() && chords > 33.0 {
        chords as i64 - 1
    } else {
        32
    }
}

/// Points along the circle arc from `start_point` to `end_point`, turning to `side`.
pub fn create_arc_between_points(
    centre: Pt,
    radius: f64,
    start_point: Pt,
    end_point: Pt,
    side: &str,
    num_segments: i64,
) -> Vec<Pt> {
    let (angle_start, delta) = arc_sweep(centre, start_point, end_point, side);
    linspace(
        angle_start,
        angle_start + delta,
        (num_segments + 2).max(0) as usize,
    )
    .into_iter()
    .map(|a| {
        [
            centre[0] + radius * libm::cos(a),
            centre[1] + radius * libm::sin(a),
        ]
    })
    .collect()
}

/// The coordinate lying `distance` metres along the polyline `coords`.
pub fn interpolate_point_on_line(
    coords: &[Pt],
    chainages: &[f64],
    distance: f64,
) -> Result<Pt, PyError> {
    if coords.is_empty() {
        return Err(PyError::value(
            "Cannot interpolate point on an empty coordinate sequence.",
        ));
    }
    if distance <= 0.0 {
        return Ok(coords[0]);
    }
    if distance >= chainages[chainages.len() - 1] {
        return Ok(coords[coords.len() - 1]);
    }
    let idx = searchsorted_right(chainages, distance);
    if idx == 0 {
        return Ok(coords[0]);
    }
    let prev = idx - 1;
    let (prev_ch, next_ch) = (chainages[prev], chainages[idx]);
    if isclose(next_ch, prev_ch) {
        return Ok(coords[idx]);
    }
    let ratio = (distance - prev_ch) / (next_ch - prev_ch);
    Ok([
        coords[prev][0] + ratio * (coords[idx][0] - coords[prev][0]),
        coords[prev][1] + ratio * (coords[idx][1] - coords[prev][1]),
    ])
}

fn clip(x: f64, lo: f64, hi: f64) -> f64 {
    // np.clip: minimum(maximum(x, lo), hi); NaN stays NaN.
    if x.is_nan() {
        return x;
    }
    x.max(lo).min(hi)
}

/// The distance (m) within which `replace_segment_with_arc` takes two points as one.
const SPLICE_TOL_M: f64 = 1e-9;

/// Splice `arc_points` into `line` between `start_distance` and `end_distance`.
pub fn replace_segment_with_arc(
    line: &[Pt],
    start_distance: f64,
    end_distance: f64,
    arc_points: &[Pt],
) -> Result<Vec<Pt>, PyError> {
    if arc_points.len() < 2 || line.len() < 2 {
        return Ok(line.to_vec());
    }
    let chainages = cumulative_chainages(line);
    let total = chainages[chainages.len() - 1];
    let tol = SPLICE_TOL_M;
    let start_distance = clip(start_distance, 0.0, total);
    let end_distance = clip(end_distance, 0.0, total);
    if end_distance - start_distance <= tol {
        return Ok(line.to_vec());
    }
    let start_point = interpolate_point_on_line(line, &chainages, start_distance)?;
    let end_point = interpolate_point_on_line(line, &chainages, end_distance)?;

    let mut prefix: Vec<Pt> = Vec::new();
    for (idx, &ch) in chainages.iter().enumerate() {
        if ch < start_distance - tol {
            prefix.push(line[idx]);
        } else {
            break;
        }
    }
    if prefix.is_empty() {
        prefix.push(line[0]);
    }
    if !allclose2(prefix[prefix.len() - 1], start_point, tol) {
        prefix.push(start_point);
    }

    let mut suffix: Vec<Pt> = Vec::new();
    for idx in (0..chainages.len()).rev() {
        if chainages[idx] > end_distance + tol {
            suffix.insert(0, line[idx]);
        } else {
            break;
        }
    }
    if suffix.is_empty() {
        suffix.push(line[line.len() - 1]);
    }
    if !allclose2(suffix[0], end_point, tol) {
        suffix.insert(0, end_point);
    }

    let mut arc: Vec<Pt> = arc_points.to_vec();
    if !allclose2(arc[0], start_point, tol) {
        arc.insert(0, start_point);
    }
    if !allclose2(arc[arc.len() - 1], end_point, tol) {
        arc.push(end_point);
    }

    let mut combined: Vec<Pt> = Vec::new();
    for p in prefix.iter().chain(arc.iter()).chain(suffix.iter()) {
        if combined
            .last()
            .is_none_or(|&last| !allclose2(last, *p, tol))
        {
            combined.push(*p);
        }
    }
    if combined.len() < 2 {
        return Ok(line.to_vec());
    }
    Ok(combined)
}

/// Where a ring meets a line: every isolated point, and the two ends of every shared stretch.
fn gather_intersection_points(ring: &[Pt], line: &[Pt]) -> Vec<Pt> {
    let (points, stretches) = linear::line_intersection(ring, line);
    let mut out = points;
    for s in stretches {
        if let (Some(&a), Some(&b)) = (s.first(), s.last()) {
            out.push(a);
            if s.len() > 1 {
                out.push(b);
            }
        }
    }
    out
}

/// The furthest forward touch between the circle and the centreline, or `None`.
pub fn find_forward_circle_contact(
    centreline: &[Pt],
    tangent_point: Pt,
    tangent_chainage: f64,
    centre: Pt,
    radius: f64,
    min_chainage_gap: f64,
) -> Result<Option<Contact>, PyError> {
    if radius <= 0.0 {
        return Err(PyError::value("Radius must be positive."));
    }
    if min_chainage_gap <= 0.0 {
        return Err(PyError::value("min_chainage_gap must be positive."));
    }
    let Some(circle) = buffer_point(centre, radius, 128) else {
        return Ok(None);
    };
    let mut best: Option<(f64, Pt)> = None;
    for p in gather_intersection_points(&circle.exterior, centreline) {
        let chainage = linear::project(centreline, p);
        if chainage - tangent_chainage > min_chainage_gap {
            // Python `max(..., key=chainage)`: the first of equal chainages.
            if best.is_none_or(|(c, _)| chainage > c) {
                best = Some((chainage, p));
            }
        }
    }
    Ok(best.map(|(contact_chainage, contact_point)| Contact {
        contact_point,
        centre,
        tangent_point,
        contact_chainage,
    }))
}

/// A tangent exit: the arc ends at `exit_point` (E, on the circle) and a straight runs from there to
/// `join_point` (Q, a vertex of the old line at `join_chainage`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TangentExit {
    pub exit_point: Pt,
    pub join_point: Pt,
    pub join_chainage: f64,
}

/// `a` wrapped to (-pi, pi].
fn wrap_angle(a: f64) -> f64 {
    let pi = core::f64::consts::PI;
    let w = np_mod(a + pi, 2.0 * pi) - pi;
    if w == -pi { pi } else { w }
}

/// A join whose heading error is no more than this (rad, 0.25 degree) is taken as tangent when the
/// error stops falling before it reaches zero (module doc).
pub const EXIT_HEADING_TOL_RAD: f64 = 0.25 * core::f64::consts::PI / 180.0;

/// The tangent exit of a circle that touches `line` at `contact_chainage` (module doc), or `None` when
/// the circle must fall back to the crossing exit. `side` is the circle's side, as for
/// `place_circle_at_point`. Never fails.
pub fn find_tangent_exit(
    line: &[Pt],
    centre: Pt,
    radius: f64,
    side: &str,
    contact_chainage: f64,
) -> Option<TangentExit> {
    if line.len() < 2 || radius.is_nan() || radius <= 0.0 || !contact_chainage.is_finite() {
        return None;
    }
    // +1: the arc turns left (anticlockwise); -1: right.
    let sigma = if side == "right" { -1.0 } else { 1.0 };
    let half_pi = core::f64::consts::FRAC_PI_2;
    let chainages = cumulative_chainages(line);
    let heading = |a: Pt, b: Pt| libm::atan2(b[1] - a[1], b[0] - a[0]);
    // The error is read signed by the arc's turn, so it is positive at the touch. `best` is the
    // vertex of least error so far, with that error.
    let mut best: Option<(f64, TangentExit)> = None;
    let mut first = true;
    for j in 1..line.len() - 1 {
        if chainages[j] <= contact_chainage {
            continue;
        }
        let v = line[j];
        let q = [v[0] - centre[0], v[1] - centre[1]];
        let d2 = q[0] * q[0] + q[1] * q[1];
        let r2 = radius * radius;
        if d2.is_nan() || d2 <= r2 {
            // On or inside the exact circle (the touch is on the polygon): no tangent from here.
            continue;
        }
        // The segments into and out of V, skipping repeated vertices.
        if line[j - 1] == v {
            continue;
        }
        let Some(&next) = line[j + 1..].iter().find(|&&p| p != v) else {
            break;
        };
        let (theta_in, theta_out) = (heading(line[j - 1], v), heading(v, next));
        let l = (d2 - r2).sqrt();
        let phi = libm::atan2(q[1], q[0]) - sigma * libm::atan2(l, radius);
        let h = phi + sigma * half_pi;
        let err_in = sigma * wrap_angle(h - theta_in);
        let err_out = sigma * wrap_angle(h - theta_out);
        if err_in.abs() >= half_pi || err_out.abs() >= half_pi {
            break;
        }
        if first && err_in <= 0.0 {
            // The old line does not leave the circle outward here: no tangent exit.
            return None;
        }
        first = false;
        let exit = TangentExit {
            exit_point: [
                centre[0] + radius * libm::cos(phi),
                centre[1] + radius * libm::sin(phi),
            ],
            join_point: v,
            join_chainage: chainages[j],
        };
        if err_out <= 0.0 {
            return Some(exit);
        }
        match best {
            Some((least, _)) if err_out > least => break,
            _ => best = Some((err_out, exit)),
        }
    }
    // The error stopped falling (or the line ended) before it reached zero: the vertex of least
    // error, if that error is within the tolerance.
    best.and_then(|(least, exit)| (least <= EXIT_HEADING_TOL_RAD).then_some(exit))
}

/// Straighten `centreline` so a circle of `min_radius` can roll along the given sides.
///
/// The result is measured, not assumed: `radius_ok` is false when the returned line is tighter
/// than `min_radius` on a chord of `check_chord_m` (default `min_radius / 3`).
pub fn rolling_circle(
    centreline: &Geometry,
    min_radius: f64,
    step: f64,
    passes: i64,
    sides: &[String],
    min_chainage_gap: Option<f64>,
    check_chord_m: Option<f64>,
) -> Result<RollingCircleResult, PyError> {
    if min_radius <= 0.0 {
        return Err(PyError::value("min_radius must be positive."));
    }
    if step <= 0.0 {
        return Err(PyError::value("step must be positive."));
    }
    let mut line = primary_line(centreline)?;
    let gap = match min_chainage_gap {
        None => py_max(step, min_radius * 0.25),
        Some(g) => g,
    };
    let mut circles: Vec<Circle> = Vec::new();
    let resampled = |line: &[Pt]| -> Result<(Vec<Pt>, Vec<f64>), PyError> {
        let pts = resample_coordinate_sequence(line, step)?;
        let ch = cumulative_chainages(&pts);
        Ok((pts, ch))
    };
    let start_index = |ch: &[f64]| -> usize {
        let mut idx = searchsorted_left(ch, step).max(1);
        if idx + 1 >= ch.len() {
            idx = ch.len().saturating_sub(2).max(1);
        }
        idx
    };

    for pass_index in 0..passes.max(0) {
        for side in sides {
            let (mut samples, mut chainages) = resampled(&line)?;
            if side != "left" && side != "right" {
                // `place_circle_at_point` refuses any other side, so no circle is placed.
                continue;
            }
            let mut idx = start_index(&chainages);
            while idx + 1 < samples.len() {
                let sample_distance = linear::project(&line, samples[idx]);
                let Some(TangentEntry {
                    tangent_point,
                    tangent_distance,
                    centre,
                }) = find_tangent_entry(&line, sample_distance, min_radius, side)
                else {
                    idx += 1;
                    continue;
                };
                let Some(contact) = find_forward_circle_contact(
                    &line,
                    tangent_point,
                    tangent_distance,
                    centre,
                    min_radius,
                    gap,
                )?
                else {
                    idx += 1;
                    continue;
                };
                let exit = find_tangent_exit(
                    &line,
                    contact.centre,
                    min_radius,
                    side,
                    contact.contact_chainage,
                );
                // Later passes refine the line and never add a kink: no tangent exit, no splice.
                if exit.is_none() && pass_index > 0 {
                    idx += 1;
                    continue;
                }
                // Tangent exit: arc to E, straight to Q. Fallback: arc to the touch, as before 0.3.0.
                let (exit_point, join_point, join_chainage) = match exit {
                    Some(e) => (e.exit_point, e.join_point, e.join_chainage),
                    None => (
                        contact.contact_point,
                        contact.contact_point,
                        contact.contact_chainage,
                    ),
                };
                circles.push(Circle {
                    pass: pass_index + 1,
                    side: side.clone(),
                    centre: contact.centre,
                    radius_m: min_radius,
                    tangent_point: contact.tangent_point,
                    contact_point: join_point,
                    exit_point,
                    tangent_exit: exit.is_some(),
                    start_chainage_m: tangent_distance,
                    end_chainage_m: join_chainage,
                });
                let arc = create_arc_between_points(
                    contact.centre,
                    min_radius,
                    contact.tangent_point,
                    exit_point,
                    side,
                    arc_segments(
                        min_radius,
                        contact.centre,
                        contact.tangent_point,
                        exit_point,
                        side,
                    ),
                );
                // `replace_segment_with_arc` ends the splice at the join point, so a tangent exit
                // gets its straight (E to Q) as the splice's last segment.
                line = replace_segment_with_arc(&line, tangent_distance, join_chainage, &arc)?;
                (samples, chainages) = resampled(&line)?;
                let updated = linear::project(&line, join_point);
                idx = searchsorted_left(&chainages, updated);
                idx = (idx + 1).min(samples.len() - 1);
            }
        }
    }
    let chord = check_chord_m.unwrap_or_else(|| default_check_chord(min_radius));
    let measured = min_radius_on_chord(&line, chord)?;
    Ok(RollingCircleResult {
        centreline: line,
        min_radius_m: min_radius,
        circles,
        measured_min_radius_m: measured,
        radius_check_chord_m: chord,
        radius_ok: radius_ok(measured, min_radius),
    })
}

#[cfg(test)]
mod tests {
    //! Label: property (heading continuity at every exit join; R_min held on short chords; a
    //! fallback that does not fail); sourced - hand calculation (the tangent exit on a 3-4-5
    //! triangle: from V = (400, 0) to the circle of radius 300 about (0, 300), the tangent is 400 m
    //! long and touches at (0, 0), heading 0).
    use super::*;
    use crate::alignment::limits::resample_line;
    use crate::alignment::radius_check::RADIUS_TOL;
    use crate::alignment::route::min_radius_of_line;
    use crate::synthetic::{RiverParams, synthetic_river};

    fn sides() -> Vec<String> {
        vec!["left".into(), "right".into()]
    }

    fn river() -> Geometry {
        Geometry::LineString(synthetic_river(RiverParams::default()).unwrap().centreline)
    }

    fn heading(a: Pt, b: Pt) -> f64 {
        libm::atan2(b[1] - a[1], b[0] - a[0])
    }

    /// The heading change (rad) across `p` on `line`, read on 10 m chords either side.
    fn turn_across(line: &[Pt], p: Pt) -> f64 {
        let s = linear::project(line, p);
        let a = linear::interpolate(line, s - 10.0).unwrap();
        let b = linear::interpolate(line, s + 10.0).unwrap();
        wrap_angle(heading(p, b) - heading(a, p))
    }

    /// The heading step (rad, signed) at the entry of `c` on `line`: the circle's own heading at its
    /// tangent point minus the heading of the line's segment into that point. `None` when the
    /// tangent point is no longer a vertex of `line` (a later circle replaced it).
    fn entry_kink(line: &[Pt], c: &Circle) -> Option<f64> {
        let i = line.iter().position(|&p| p == c.tangent_point)?;
        let prev = line[..i].iter().rev().find(|&&p| p != c.tangent_point)?;
        let r = [
            c.tangent_point[0] - c.centre[0],
            c.tangent_point[1] - c.centre[1],
        ];
        let along = if c.side == "right" {
            [r[1], -r[0]]
        } else {
            [-r[1], r[0]]
        };
        let circle_heading = libm::atan2(along[1], along[0]);
        Some(wrap_angle(circle_heading - heading(*prev, c.tangent_point)))
    }

    /// Heading continuity at the exit joins. At every exit join still on the final line (the arc's
    /// end E and the join Q), the heading change across it on 10 m chords is no more than a circle
    /// of R_min turns over the same chords, plus 0.25 degree. Before 0.3.0 (the crossing exit) this
    /// failed: on this river at R_min 300 m the crossing joins left on the final line turned 6.1,
    /// 6.2 and 7.3 degrees across 10 m chords, against 1.9 for the circle.
    #[test]
    fn every_exit_join_on_the_synthetic_river_is_tangent() {
        let g = river();
        for r_min in [255.0, 300.0] {
            let res = rolling_circle(&g, r_min, 10.0, 2, &sides(), None, None).unwrap();
            let line = &res.centreline;
            let circle_turn = 2.0 * libm::asin(10.0 / (2.0 * r_min));
            let bound = circle_turn + 0.25f64.to_radians();
            let mut joins = 0;
            for c in &res.circles {
                assert!(
                    c.tangent_exit,
                    "R_min {r_min}: a fallback at {:?}",
                    c.contact_point
                );
                for p in [c.exit_point, c.contact_point] {
                    if linear::point_line_distance(p, line) > 1e-6 {
                        continue; // a later circle replaced this stretch
                    }
                    joins += 1;
                    let t = turn_across(line, p).abs();
                    assert!(
                        t <= bound,
                        "R_min {r_min}: {:.3} deg across {p:?}, bound {:.3}",
                        t.to_degrees(),
                        bound.to_degrees()
                    );
                }
            }
            assert!(
                joins >= 16,
                "R_min {r_min}: only {joins} joins on the final line"
            );
        }
    }

    /// Heading continuity at the arc entries (#283). At every entry still on the final line, the
    /// circle's heading at its tangent point equals the heading of the line's segment into that
    /// point, to 1e-6 rad, at rolling steps of 10 m and 25 m and R_min 255 m and 300 m. Before #283
    /// the circle was placed tangent to the direction read on the resample, and this failed: the
    /// worst entry kinks were 0.56 deg (step 10 m, R_min 300 m), 0.57 deg (25 m, 300 m), 0.88 deg
    /// (10 m, 255 m) and 0.89 deg (25 m, 255 m).
    #[test]
    fn every_arc_entry_on_the_synthetic_river_is_tangent() {
        let g = river();
        for step in [10.0, 25.0] {
            for r_min in [255.0, 300.0] {
                let res = rolling_circle(&g, r_min, step, 2, &sides(), None, None).unwrap();
                let mut entries = 0;
                let mut worst = 0.0f64;
                for c in &res.circles {
                    let Some(kink) = entry_kink(&res.centreline, c) else {
                        continue; // a later circle replaced this entry
                    };
                    entries += 1;
                    worst = worst.max(kink.abs());
                }
                assert!(
                    worst <= 1e-6,
                    "step {step}, R_min {r_min}: an entry kinks {:.4} deg",
                    worst.to_degrees()
                );
                assert!(
                    entries >= 12,
                    "step {step}, R_min {r_min}: only {entries} entries on the final line"
                );
            }
        }
    }

    /// Later passes never add a crossing exit (#283): every circle of pass 2 or later took the
    /// tangent exit, at R_min 255 m and 300 m and rolling steps of 10 m and 25 m. Without the
    /// guard, with the vertex entry, this failed: 4 pass-2 circles fell back to the crossing exit at
    /// a 25 m step and R_min 300 m (none in the other three cases), and their kinks read 265.6 m on
    /// a 10 m resample.
    #[test]
    fn later_passes_never_add_a_crossing_exit() {
        let g = river();
        let mut fell_back = Vec::new();
        for step in [10.0, 25.0] {
            for r_min in [255.0, 300.0] {
                let res = rolling_circle(&g, r_min, step, 2, &sides(), None, None).unwrap();
                let n = res
                    .circles
                    .iter()
                    .filter(|c| c.pass >= 2 && !c.tangent_exit)
                    .count();
                if n > 0 {
                    fell_back.push(format!("step {step}, R_min {r_min}: {n}"));
                }
            }
        }
        assert!(fell_back.is_empty(), "pass-2 crossing exits: {fell_back:?}");
    }

    /// R_min held on short chords, read as `docs/known-issues.md` reads them: the line resampled
    /// every 10 m and every 25 m, three consecutive points, at rolling steps of 10 m and 25 m. Before
    /// 0.3.0, at R_min 300 m and a 10 m step, these read 111 m and 288 m. Before #283, at a 25 m step,
    /// the 10 m resample read 280 m (R_min 300 m) and 227 m (R_min 255 m). The tolerance is `RADIUS_TOL` (2 %), the reading tolerance
    /// of `radius_ok`.
    #[test]
    fn rolled_lines_hold_r_min_on_10_and_25_m_resamples() {
        let g = river();
        for r_min in [255.0, 300.0] {
            for step in [10.0, 25.0] {
                let res = rolling_circle(&g, r_min, step, 2, &sides(), None, None).unwrap();
                for chord in [10.0, 25.0] {
                    let pts = resample_line(&res.centreline, chord).unwrap();
                    let (rad, at) = min_radius_of_line(&pts);
                    assert!(
                        rad >= (1.0 - RADIUS_TOL) * r_min,
                        "R_min {r_min}, step {step}, {chord} m resample: {rad} at {:?}",
                        pts[at]
                    );
                }
                assert!(res.radius_ok, "{}", res.measured_min_radius_m);
            }
        }
    }

    #[test]
    fn a_tangent_exit_on_a_3_4_5_triangle() {
        // The old line runs from inside the bend to V = (400, 0), turning left there onto the
        // x axis. The tangent from V touches the circle at E = (0, 0) with heading 0.
        let line = [[200.0, 10.0], [400.0, 0.0], [800.0, 0.0]];
        let e = find_tangent_exit(&line, [0.0, 300.0], 300.0, "left", 0.0).unwrap();
        assert!(
            crate::geom::dist(e.exit_point, [0.0, 0.0]) < 1e-9,
            "{:?}",
            e.exit_point
        );
        assert_eq!(e.join_point, [400.0, 0.0]);
        assert!((e.join_chainage - crate::geom::dist(line[0], line[1])).abs() < 1e-12);
        // The same, mirrored across the x axis, on the right.
        let mirrored: Vec<Pt> = line.iter().map(|p| [p[0], -p[1]]).collect();
        let e = find_tangent_exit(&mirrored, [0.0, -300.0], 300.0, "right", 0.0).unwrap();
        assert!(
            crate::geom::dist(e.exit_point, [0.0, 0.0]) < 1e-9,
            "{:?}",
            e.exit_point
        );
        assert_eq!(e.join_point, [400.0, 0.0]);
    }

    #[test]
    fn no_tangent_exit_when_the_old_line_turns_away_or_ends() {
        // Turns right at V (away from the straight): the error grows, so no exit.
        let away = [[200.0, 10.0], [400.0, 0.0], [800.0, -100.0]];
        assert_eq!(
            find_tangent_exit(&away, [0.0, 300.0], 300.0, "left", 0.0),
            None
        );
        // Ends before the error reaches zero.
        let ends = [[200.0, 10.0], [400.0, 0.0]];
        assert_eq!(
            find_tangent_exit(&ends, [0.0, 300.0], 300.0, "left", 0.0),
            None
        );
        // Degenerate inputs give no exit, never a panic.
        assert_eq!(find_tangent_exit(&[], [0.0, 0.0], 300.0, "left", 0.0), None);
        assert_eq!(
            find_tangent_exit(&away, [0.0, 300.0], 0.0, "left", 0.0),
            None
        );
        assert_eq!(
            find_tangent_exit(&away, [0.0, 300.0], 300.0, "left", f64::NAN),
            None
        );
        let repeated = [[200.0, 10.0], [400.0, 0.0], [400.0, 0.0], [800.0, 0.0]];
        assert!(find_tangent_exit(&repeated, [0.0, 300.0], 300.0, "left", 0.0).is_some());
    }

    /// A tight hook at the end of a line: the circle crosses it, but the line ends before the
    /// straight can meet it, so that circle takes the crossing exit. The run still returns a line.
    #[test]
    fn a_bend_the_straight_cannot_reach_falls_back_to_the_crossing_exit() {
        let mut line: Vec<Pt> = (0..=50).map(|i| [-500.0 + 10.0 * i as f64, 0.0]).collect();
        for k in 1..=18 {
            let a = (k as f64 * 5.0).to_radians();
            line.push([100.0 * libm::sin(a), 100.0 - 100.0 * libm::cos(a)]);
        }
        line.push([100.0, 130.0]);
        let g = Geometry::LineString(line.clone());
        let res = rolling_circle(&g, 300.0, 10.0, 2, &sides(), None, None).unwrap();
        assert!(!res.circles.is_empty());
        let fell_back: Vec<&Circle> = res.circles.iter().filter(|c| !c.tangent_exit).collect();
        assert!(!fell_back.is_empty(), "{:?}", res.circles);
        for c in fell_back {
            assert_eq!(c.exit_point, c.contact_point);
        }
        assert_eq!(res.centreline[0], line[0]);
        assert_eq!(
            res.centreline[res.centreline.len() - 1],
            line[line.len() - 1]
        );
    }
}
