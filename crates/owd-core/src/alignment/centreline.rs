//! Smoothed river centreline from a pair of bank lines (paired cross-section method). Mirrors
//! `alignment/centreline.py`.
//!
//! Each bank is resampled, mapped monotonically onto the opposite bank, and the midpoints are
//! blended, Chaikin-smoothed and moving-averaged. Errors are the Python `ValueError` messages.

use crate::geom::{Pt, linear};
use crate::numpy::{chainages, interp, isclose, linspace, norm, searchsorted_right};

/// `ensure_orientation`: flip `right` if it runs the opposite way to `left`.
pub fn ensure_orientation(left: &[Pt], right: &[Pt]) -> (Vec<Pt>, Vec<Pt>) {
    let d = |a: Pt, b: Pt| norm(a[0] - b[0], a[1] - b[1]);
    let same = d(left[0], right[0]);
    let reverse = d(left[0], right[right.len() - 1]);
    let mut r = right.to_vec();
    if reverse < same {
        r.reverse();
    }
    (left.to_vec(), r)
}

/// `resample_polyline`: uniform spacing `step` (m), at least three samples.
pub fn resample_polyline(points: &[Pt], step: f64) -> Result<Vec<Pt>, String> {
    if points.len() < 2 {
        return Err("Polyline must contain at least two vertices.".into());
    }
    if step <= 0.0 {
        return Err("Resample step must be positive.".into());
    }
    let cum = chainages(points);
    let total = cum[cum.len() - 1];
    if total == 0.0 {
        return Err("Polyline has zero length.".into());
    }
    Ok(resample_at(points, &cum, total, step))
}

/// The resampling shared by the three `resample_*` functions: `max(ceil(total / step) + 1, 3)`
/// samples by `np.interp` on each coordinate.
pub(crate) fn resample_at(points: &[Pt], cum: &[f64], total: f64, step: f64) -> Vec<Pt> {
    let n = ((total / step).ceil() as usize + 1).max(3);
    let xs: Vec<f64> = points.iter().map(|p| p[0]).collect();
    let ys: Vec<f64> = points.iter().map(|p| p[1]).collect();
    linspace(0.0, total, n)
        .into_iter()
        .map(|t| [interp(t, cum, &xs), interp(t, cum, &ys)])
        .collect()
}

/// `reparameterize_fraction`: the points and their fraction of the total length.
pub fn reparameterize_fraction(points: &[Pt]) -> Result<(Vec<Pt>, Vec<f64>), String> {
    let cum = chainages(points);
    let total = cum[cum.len() - 1];
    if total == 0.0 {
        return Err("Polyline has zero length.".into());
    }
    Ok((points.to_vec(), cum.iter().map(|c| c / total).collect()))
}

/// `moving_average`: an odd window (an even one grows by one), edges padded by repetition.
pub fn moving_average(points: &[Pt], window: i64) -> Vec<Pt> {
    if window <= 1 {
        return points.to_vec();
    }
    let w = if window % 2 == 0 { window + 1 } else { window } as usize;
    let pad = w / 2;
    let n = points.len();
    // `np.pad(..., mode="edge")`
    let padded: Vec<Pt> = (0..n + 2 * pad)
        .map(|i| points[(i as isize - pad as isize).clamp(0, n as isize - 1) as usize])
        .collect();
    let col = |k: usize| -> Vec<f64> {
        let a: Vec<f64> = padded.iter().map(|p| p[k]).collect();
        crate::numpy::box_convolve(&a, w, false)
    };
    let (x, y) = (col(0), col(1));
    x.into_iter().zip(y).map(|(x, y)| [x, y]).collect()
}

/// `chaikin_smoothing`: corner cutting at 1/4 and 3/4, the ends kept.
pub fn chaikin_smoothing(points: &[Pt], iterations: i64) -> Vec<Pt> {
    let mut pts = points.to_vec();
    for _ in 0..iterations.max(0) {
        let mut next = vec![pts[0]];
        for w in pts.windows(2) {
            let (p0, p1) = (w[0], w[1]);
            next.push([0.75 * p0[0] + 0.25 * p1[0], 0.75 * p0[1] + 0.25 * p1[1]]);
            next.push([0.25 * p0[0] + 0.75 * p1[0], 0.25 * p0[1] + 0.75 * p1[1]]);
        }
        next.push(pts[pts.len() - 1]);
        pts = next;
    }
    pts
}

/// `map_points_monotonic`: project `source` onto `target`, never stepping back and never more
/// than `max_step` forward; the last point maps to the end of `target`.
pub fn map_points_monotonic(source: &[Pt], target: &[Pt], max_step: f64) -> Vec<Pt> {
    let total = linear::length(target);
    let mut prev = 0.0;
    let mut out = Vec::with_capacity(source.len());
    for (i, &p) in source.iter().enumerate() {
        let mut d = linear::project(target, p);
        if d < prev {
            d = prev;
        } else if d - prev > max_step {
            d = crate::py_min(prev + max_step, total);
        }
        if i == source.len() - 1 {
            d = total;
        }
        out.push(linear::interpolate(target, d).unwrap_or([f64::NAN, f64::NAN]));
        prev = d;
    }
    out
}

/// `compute_paired_centreline`.
pub fn compute_paired_centreline(
    left: &[Pt],
    right: &[Pt],
    resample_step: f64,
    smooth_window: i64,
    chaikin_iterations: i64,
) -> Result<Vec<Pt>, String> {
    let max_step = 5.0 * resample_step;
    let mid = |a: &[Pt], b: &[Pt]| -> Vec<Pt> {
        a.iter()
            .zip(b)
            .map(|(p, q)| [0.5 * (p[0] + q[0]), 0.5 * (p[1] + q[1])])
            .collect()
    };
    let right_on_left = map_points_monotonic(left, right, max_step);
    let left_mid = mid(left, &right_on_left);
    let left_on_right = map_points_monotonic(right, left, max_step);
    let right_mid = mid(&left_on_right, right);
    let (lc, lf) = reparameterize_fraction(&left_mid)?;
    let (rc, rf) = reparameterize_fraction(&right_mid)?;
    let n = left_mid.len().max(right_mid.len());
    let col = |c: &[Pt], k: usize| -> Vec<f64> { c.iter().map(|p| p[k]).collect() };
    let (lx, ly, rx, ry) = (col(&lc, 0), col(&lc, 1), col(&rc, 0), col(&rc, 1));
    let mut blended: Vec<Pt> = linspace(0.0, 1.0, n)
        .into_iter()
        .map(|t| {
            let a = [interp(t, &lf, &lx), interp(t, &lf, &ly)];
            let b = [interp(t, &rf, &rx), interp(t, &rf, &ry)];
            [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]
        })
        .collect();
    if chaikin_iterations > 0 {
        blended = chaikin_smoothing(&blended, chaikin_iterations);
    }
    if smooth_window > 1 {
        blended = moving_average(&blended, smooth_window);
    }
    resample_polyline(&blended, resample_step)
}

/// `build_centreline`: the smoothed centreline between two bank lines.
pub fn build_centreline(
    left_bank: &[Pt],
    right_bank: &[Pt],
    resample_step: f64,
    smooth_window: i64,
    chaikin_iterations: i64,
) -> Result<Vec<Pt>, String> {
    let left = resample_polyline(left_bank, resample_step)?;
    let right = resample_polyline(right_bank, resample_step)?;
    let (left, right) = ensure_orientation(&left, &right);
    compute_paired_centreline(
        &left,
        &right,
        resample_step,
        smooth_window,
        chaikin_iterations,
    )
}

/// `trim_polyline`: remove `cut_length` metres from both ends.
pub fn trim_polyline(points: &[Pt], cut_length: f64) -> Result<Vec<Pt>, String> {
    if cut_length <= 0.0 {
        return Ok(points.to_vec());
    }
    if points.len() < 2 {
        return Err("Cannot trim a centreline with fewer than two vertices.".into());
    }
    let cum = chainages(points);
    let total = cum[cum.len() - 1];
    if total <= 2.0 * cut_length {
        return Err(format!(
            "Centreline length {total:.2} m is too short for the requested end cut of {cut_length:.2} m at both ends."
        ));
    }
    let (start, end) = (cut_length, total - cut_length);
    let at = |d: f64| -> Pt {
        if d <= 0.0 {
            return points[0];
        }
        if d >= total {
            return points[points.len() - 1];
        }
        let i = searchsorted_right(&cum, d);
        let (prev, next) = (cum[i - 1], cum[i]);
        if isclose(d, prev) {
            return points[i - 1];
        }
        if isclose(d, next) {
            return points[i];
        }
        let r = (d - prev) / (next - prev);
        let (a, b) = (points[i - 1], points[i]);
        [a[0] + r * (b[0] - a[0]), a[1] + r * (b[1] - a[1])]
    };
    let mut out = vec![at(start)];
    out.extend(
        points
            .iter()
            .zip(&cum)
            .filter(|(_, c)| **c > start && **c < end)
            .map(|(p, _)| *p),
    );
    out.push(at(end));
    Ok(out)
}
