//! Rolling-circle meander straightening (`alignment/rolling_circle.py`).
//!
//! Per side and per pass: resample the line every `step`; at each sample place a circle of radius
//! `min_radius` tangent to the line on that side; find the furthest forward point where the circle
//! touches the line again (more than `min_chainage_gap` along it); if there is one, replace the
//! line between the tangent point and that touch by the circle arc and resume from the touch.
//!
//! Search class (ADR 0005): the one discrete choice is the circle contact. The circle is the
//! 512-sided polygon GEOS builds for `quad_segs=128`, never an exact circle.

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
    pub contact_point: Pt,
    pub start_chainage_m: f64,
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

/// Points along the circle arc from `start_point` to `end_point`, turning to `side`.
pub fn create_arc_between_points(
    centre: Pt,
    radius: f64,
    start_point: Pt,
    end_point: Pt,
    side: &str,
    num_segments: i64,
) -> Vec<Pt> {
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
    let tol = 1e-9;
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
            let mut idx = start_index(&chainages);
            while idx + 1 < samples.len() {
                let Ok((centre, _)) = place_circle_at_point(&samples, idx as i64, min_radius, side)
                else {
                    idx += 1;
                    continue;
                };
                let tangent_point = samples[idx];
                let tangent_distance = linear::project(&line, tangent_point);
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
                circles.push(Circle {
                    pass: pass_index + 1,
                    side: side.clone(),
                    centre: contact.centre,
                    radius_m: min_radius,
                    tangent_point: contact.tangent_point,
                    contact_point: contact.contact_point,
                    start_chainage_m: tangent_distance,
                    end_chainage_m: contact.contact_chainage,
                });
                let arc = create_arc_between_points(
                    contact.centre,
                    min_radius,
                    contact.tangent_point,
                    contact.contact_point,
                    side,
                    32,
                );
                line = replace_segment_with_arc(
                    &line,
                    tangent_distance,
                    contact.contact_chainage,
                    &arc,
                )?;
                (samples, chainages) = resampled(&line)?;
                let updated = linear::project(&line, contact.contact_point);
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
