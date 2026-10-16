//! Navigation channel limits: left and right offset lines with curvature-adaptive widening.
//! Mirrors `alignment/limits.py`.

use crate::geom::{Pt, linear, merge};
use crate::numpy::{chainages, linspace, norm, searchsorted_left, searchsorted_right};

/// `resample_line`: uniform spacing along the line, at least three samples.
pub fn resample_line(line: &[Pt], step: f64) -> Result<Vec<Pt>, String> {
    if step <= 0.0 {
        return Err("Sample step must be positive.".into());
    }
    if line.len() < 2 {
        return Err("Line must contain at least two vertices.".into());
    }
    let cum = chainages(line);
    let total = cum[cum.len() - 1];
    if total <= 0.0 {
        return Err("Line has zero length, cannot resample.".into());
    }
    Ok(super::centreline::resample_at(line, &cum, total, step))
}

/// `circumcircle`: the centre and radius of the circle through three points, or `None` when they
/// are (nearly) collinear (`|d| < 1e-9`).
pub fn circumcircle(p0: Pt, p1: Pt, p2: Pt) -> Option<(Pt, f64)> {
    let ([ax, ay], [bx, by], [cx, cy]) = (p0, p1, p2);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-9 {
        return None;
    }
    let a2 = ax * ax + ay * ay;
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    Some(([ux, uy], libm::hypot(ux - ax, uy - ay)))
}

/// `compute_curvature_radius`: the circumradius at each interior point (infinite where the
/// points are collinear); the ends copy their neighbours.
pub fn compute_curvature_radius(points: &[Pt]) -> Vec<f64> {
    let n = points.len();
    let mut r = vec![f64::INFINITY; n];
    if n < 3 {
        return r;
    }
    for i in 1..n - 1 {
        if let Some((_, rad)) = circumcircle(points[i - 1], points[i], points[i + 1])
            && rad > 0.0
        {
            r[i] = rad;
        }
    }
    r[0] = r[1];
    r[n - 1] = r[n - 2];
    r
}

/// `compute_normals`: unit left normals from central-difference tangents.
pub fn compute_normals(points: &[Pt]) -> Vec<Pt> {
    let n = points.len();
    if n < 2 {
        return vec![[0.0, 0.0]; n];
    }
    let sub = |a: Pt, b: Pt| [a[0] - b[0], a[1] - b[1]];
    (0..n)
        .map(|i| {
            let mut t = if i == 0 {
                sub(points[1], points[0])
            } else if i == n - 1 {
                sub(points[n - 1], points[n - 2])
            } else {
                sub(points[i + 1], points[i - 1])
            };
            let len = norm(t[0], t[1]);
            if len > 0.0 {
                t = [t[0] / len, t[1] / len];
            }
            [-t[1], t[0]]
        })
        .collect()
}

/// `compute_chainages`.
pub fn compute_chainages(points: &[Pt]) -> Vec<f64> {
    chainages(points)
}

/// `rolling_average_radius`: the mean of the finite, positive radii within `window` metres of
/// chainage either side.
pub fn rolling_average_radius(radii: &[f64], chainages: &[f64], window: f64) -> Vec<f64> {
    (0..radii.len())
        .map(|i| {
            let a = searchsorted_left(chainages, chainages[i] - window);
            let b = searchsorted_right(chainages, chainages[i] + window);
            let v: Vec<f64> = radii[a..b.max(a)]
                .iter()
                .copied()
                .filter(|r| r.is_finite() && *r > 0.0)
                .collect();
            if v.is_empty() {
                f64::INFINITY
            } else {
                mean(&v)
            }
        })
        .collect()
}

/// `np.mean`: numpy's pairwise sum, then the division.
pub(crate) fn mean(v: &[f64]) -> f64 {
    pairwise_sum(v) / v.len() as f64
}

/// numpy's pairwise summation (`pairwise_sum` in `loops_utils.h`): blocks of up to 8 summed in
/// eight lanes, halves of more than 128 summed recursively.
pub(crate) fn pairwise_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut s = 0.0;
        for &x in a {
            s += x;
        }
        s
    } else if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let mut i = 8;
        while i < n - (n % 8) {
            for k in 0..8 {
                r[k] += a[i + k];
            }
            i += 8;
        }
        let mut s = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for &x in &a[i..] {
            s += x;
        }
        s
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        pairwise_sum(&a[..n2]) + pairwise_sum(&a[n2..])
    }
}

/// The widening parameters of `compute_extra_width` and the offset-line builders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Widening {
    pub loa: f64,
    pub extra_width_coefficient: f64,
    pub min_radius_length_multiple: f64,
    /// `None` (or zero) uses the continuous `c * loa² / R`; otherwise four radius bands.
    pub max_extra_width_radius: Option<f64>,
    pub radius_threshold: f64,
}

/// `compute_extra_width`: extra channel width at tight bends, from 100 m in from each end.
pub fn compute_extra_width(
    radii: &[f64],
    chainages: &[f64],
    total_length: f64,
    w: &Widening,
) -> Vec<f64> {
    let mut extra = vec![0.0; radii.len()];
    if w.loa <= 0.0 || w.extra_width_coefficient <= 0.0 {
        return extra;
    }
    let upper_ch = crate::py_max(total_length - 100.0, 0.0);
    let mask: Vec<bool> = radii
        .iter()
        .zip(chainages)
        .map(|(&r, &c)| {
            c >= 100.0 && c <= upper_ch && r.is_finite() && r > 0.0 && r < w.radius_threshold
        })
        .collect();
    if !mask.iter().any(|&m| m) {
        return extra;
    }
    let loa2 = w.loa * w.loa;
    let banded = w
        .max_extra_width_radius
        .is_some_and(|m| m != 0.0 && !m.is_nan())
        && w.min_radius_length_multiple > 0.0
        && w.loa > 0.0
        && w.max_extra_width_radius.is_some_and(|m| m > 0.0);
    if banded {
        let mut min_radius = w.loa * w.min_radius_length_multiple;
        if min_radius <= 0.0 {
            min_radius = 1.0;
        }
        let upper = crate::py_max(w.max_extra_width_radius.unwrap_or(0.0), min_radius + 1.0);
        let edges = linspace(min_radius, upper, 5);
        let limits = &edges[1..];
        let mut widths: Vec<f64> = edges[..4]
            .iter()
            .map(|e| w.extra_width_coefficient * loa2 / e)
            .collect();
        widths.push(0.0);
        for (i, &m) in mask.iter().enumerate() {
            if m {
                let b = searchsorted_right(limits, radii[i]).min(widths.len() - 1);
                extra[i] = widths[b];
            }
        }
    } else {
        for (i, &m) in mask.iter().enumerate() {
            if m {
                extra[i] = w.extra_width_coefficient * loa2 / radii[i];
            }
        }
    }
    extra
}

/// The result of `build_variable_offset_line`.
#[derive(Debug, Clone, PartialEq)]
pub struct OffsetLines {
    pub left: Vec<Pt>,
    pub right: Vec<Pt>,
    pub smoothed_radii: Vec<f64>,
    pub extra: Vec<f64>,
}

/// The result of `build_channel_limits`: each boundary as its lines (a multiline).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelLimits {
    pub left: Vec<Vec<Pt>>,
    pub right: Vec<Vec<Pt>>,
    pub diagnostics: LimitsDiagnostics,
}

/// `build_variable_offset_line`: the left and right lines, the smoothed radii and the extra widths.
pub fn build_variable_offset_line(
    line: &[Pt],
    base_offset: f64,
    w: &Widening,
    radius_sample_length: f64,
) -> Result<OffsetLines, String> {
    let step = crate::py_max(radius_sample_length, 0.5);
    let points = resample_line(line, step)?;
    let normals = compute_normals(&points);
    let radii = compute_curvature_radius(&points);
    let ch = chainages(&points);
    let smoothed = rolling_average_radius(&radii, &ch, 50.0);
    let total = ch.last().copied().unwrap_or(0.0);
    let extra = compute_extra_width(&smoothed, &ch, total, w);
    let offsets: Vec<f64> = extra.iter().map(|e| base_offset + e / 2.0).collect();
    let side = |s: f64| -> Vec<Pt> {
        points
            .iter()
            .zip(&normals)
            .zip(&offsets)
            .map(|((p, n), o)| {
                if s > 0.0 {
                    [p[0] + n[0] * o, p[1] + n[1] * o]
                } else {
                    [p[0] - n[0] * o, p[1] - n[1] * o]
                }
            })
            .collect()
    };
    Ok(OffsetLines {
        left: side(1.0),
        right: side(-1.0),
        smoothed_radii: smoothed,
        extra,
    })
}

/// `merge_lines`: one line as it is, several merged (`linemerge`).
pub fn merge_lines(lines: &[Vec<Pt>]) -> Result<Vec<Vec<Pt>>, String> {
    match lines.len() {
        0 => Err("Cannot merge an empty collection of line geometries.".into()),
        1 => Ok(vec![lines[0].clone()]),
        _ => Ok(merge::line_merge(lines)),
    }
}

/// The diagnostics of `build_channel_limits`.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitsDiagnostics {
    pub min_radius: Option<f64>,
    pub max_extra_width: f64,
    pub mean_extra_width: f64,
}

/// `build_channel_limits`: the left and right boundary lines (as multilines) from the
/// centreline lines, and the diagnostics.
pub fn build_channel_limits(
    centreline_lines: &[Vec<Pt>],
    base_offset: f64,
    w: &Widening,
    radius_sample_length: f64,
) -> Result<ChannelLimits, String> {
    let (mut lefts, mut rights) = (Vec::new(), Vec::new());
    let (mut all_r, mut all_e) = (Vec::new(), Vec::new());
    for line in centreline_lines {
        if line.is_empty() || linear::length(line) <= 0.0 {
            continue;
        }
        let OffsetLines {
            left: l,
            right: r,
            smoothed_radii: radii,
            extra,
        } = build_variable_offset_line(line, base_offset, w, radius_sample_length)?;
        if linear::length(&l) <= 0.0 || linear::length(&r) <= 0.0 {
            continue;
        }
        lefts.push(l);
        rights.push(r);
        all_r.extend(radii.iter().copied().filter(|r| r.is_finite() && *r > 0.0));
        all_e.extend(extra);
    }
    if lefts.is_empty() || rights.is_empty() {
        return Err("Failed to generate channel limits from the centreline geometries.".into());
    }
    let first_min = |v: &[f64]| v.iter().copied().reduce(crate::py_min);
    let diag = LimitsDiagnostics {
        min_radius: first_min(&all_r),
        max_extra_width: all_e.iter().copied().reduce(crate::py_max).unwrap_or(0.0),
        mean_extra_width: if all_e.is_empty() { 0.0 } else { mean(&all_e) },
    };
    Ok(ChannelLimits {
        left: merge_lines(&lefts)?,
        right: merge_lines(&rights)?,
        diagnostics: diag,
    })
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (circle through three points of a 10 m circle)
    use super::*;

    #[test]
    fn circumcircle_of_three_points_on_a_circle() {
        let (c, r) = circumcircle([10.0, 0.0], [0.0, 10.0], [-10.0, 0.0]).unwrap();
        assert_eq!((c, r), ([0.0, 0.0], 10.0));
        assert_eq!(circumcircle([0.0, 0.0], [1.0, 1.0], [2.0, 2.0]), None);
    }

    #[test]
    fn pairwise_sum_matches_a_plain_sum_on_integers() {
        let v: Vec<f64> = (1..=300).map(f64::from).collect();
        assert_eq!(pairwise_sum(&v), 45150.0);
    }
}
