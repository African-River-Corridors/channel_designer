//! River width measured perpendicular to the centreline, bank to bank. Mirrors
//! `alignment/widths.py`.
//!
//! At each station a cross-line is cast along the local normal and intersected with the left and
//! right bank lines.

use crate::geom::{Pt, dist, linear};
use crate::numpy::linspace;

/// `resample_distances`: `max(ceil(total / spacing) + 1, 2)` stations from 0 to the end.
pub fn resample_distances(total_length: f64, spacing: f64) -> Result<Vec<f64>, String> {
    if spacing <= 0.0 {
        return Err("Sampling spacing must be positive.".into());
    }
    let steps = ((total_length / spacing).ceil() as usize + 1).max(2);
    let mut d = linspace(0.0, total_length, steps);
    let last = d.len() - 1;
    d[last] = total_length;
    Ok(d)
}

/// One width record (`measure_widths` row).
#[derive(Debug, Clone, PartialEq)]
pub struct WidthRow {
    pub chainage_m: f64,
    pub width_m: f64,
    pub left: Pt,
    pub right: Pt,
    pub sample_index: usize,
}

/// `measure_widths`: the bank-to-bank width every `spacing` metres, and the number of stations
/// where it failed.
pub fn measure_widths(
    centreline: &[Pt],
    left_bank: &[Pt],
    right_bank: &[Pt],
    spacing: f64,
    cross_span: f64,
) -> Result<(Vec<WidthRow>, usize), String> {
    if cross_span <= 0.0 {
        return Err("Cross-span must be positive.".into());
    }
    let total = linear::length(centreline);
    let mut rows = Vec::new();
    let mut failures = 0;
    for (idx, chainage) in resample_distances(total, spacing)?.into_iter().enumerate() {
        let point = linear::interpolate(centreline, chainage).unwrap_or([f64::NAN; 2]);
        let Some((tx, ty)) = estimate_tangent(centreline, total, chainage, spacing) else {
            failures += 1;
            continue;
        };
        let (nx, ny) = normal_from_tangent(tx, ty);
        let (ox, oy) = (nx * cross_span, ny * cross_span);
        let cross = [
            [point[0] + ox, point[1] + oy],
            [point[0] - ox, point[1] - oy],
        ];
        let (Some(l), Some(r)) = (
            nearest_intersection(&cross, left_bank, point),
            nearest_intersection(&cross, right_bank, point),
        ) else {
            failures += 1;
            continue;
        };
        let width = dist(l, r);
        if width <= 0.0 {
            failures += 1;
            continue;
        }
        rows.push(WidthRow {
            chainage_m: chainage,
            width_m: width,
            left: l,
            right: r,
            sample_index: idx,
        });
    }
    Ok((rows, failures))
}

fn estimate_tangent(line: &[Pt], length: f64, chainage: f64, step: f64) -> Option<(f64, f64)> {
    let ahead = linear::interpolate(line, crate::py_min(chainage + step, length))?;
    let behind = linear::interpolate(line, crate::py_max(chainage - step, 0.0))?;
    let (dx, dy) = (ahead[0] - behind[0], ahead[1] - behind[1]);
    let n = libm::hypot(dx, dy);
    if n == 0.0 {
        None
    } else {
        Some((dx / n, dy / n))
    }
}

fn normal_from_tangent(tx: f64, ty: f64) -> (f64, f64) {
    let (nx, ny) = (-ty, tx);
    let n = libm::hypot(nx, ny);
    if n == 0.0 {
        (0.0, 0.0)
    } else {
        (nx / n, ny / n)
    }
}

/// The meeting point of the cross-line and the bank nearest `origin` (points, and the ends of any
/// overlap, as `_iter_points` reads them). The first nearest wins a tie.
fn nearest_intersection(cross: &[Pt], bank: &[Pt], origin: Pt) -> Option<Pt> {
    let (points, lines) = linear::line_intersection(cross, bank);
    let ends = lines.iter().flat_map(|l| [l[0], l[l.len() - 1]]);
    let mut best: Option<(Pt, f64)> = None;
    for p in points.into_iter().chain(ends) {
        let d = dist(p, origin);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((p, d));
        }
    }
    best.map(|(p, _)| p)
}
