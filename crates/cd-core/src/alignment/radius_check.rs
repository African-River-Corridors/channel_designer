//! Measured minimum radius of a design line, read on a stated chord (`alignment/radius_check.py`).
//!
//! Every step that straightens a line to R_min reports what it achieved, not what it aimed for.
//! Two readings: on a dense line (`min_radius_on_chord`, the rolling circle) and on a polyline's
//! own vertices (`min_radius_on_vertices`, the route). `radius_ok` allows `RADIUS_TOL` (2 %) below
//! R_min; it is a reading tolerance, not a design relaxation.

use super::PyError;
use crate::geom::Pt;
use crate::numpy::interp;
use crate::py_min;

pub const MAX_STRIDE_M: f64 = 10.0;
pub const RADIUS_TOL: f64 = 0.02;

/// One vessel length at the package's default R_min = 3 x LOA (PIANC).
pub fn default_check_chord(r_min: f64) -> f64 {
    r_min / 3.0
}

/// Minimum circumradius of `(pts[i-k], pts[i], pts[i+k])` over all `i`; infinity if there is none.
/// NaN anywhere gives NaN, as `np.min` does.
fn three_point_min(pts: &[Pt], k: usize) -> f64 {
    if pts.len() < 2 * k + 1 {
        return f64::INFINITY;
    }
    let mut min = f64::INFINITY;
    for i in 0..pts.len() - 2 * k {
        let (a_, b_, c_) = (pts[i], pts[i + k], pts[i + 2 * k]);
        let a = libm::hypot(b_[0] - c_[0], b_[1] - c_[1]);
        let b = libm::hypot(a_[0] - c_[0], a_[1] - c_[1]);
        let c = libm::hypot(a_[0] - b_[0], a_[1] - b_[1]);
        let area =
            0.5 * ((b_[0] - a_[0]) * (c_[1] - a_[1]) - (c_[0] - a_[0]) * (b_[1] - a_[1])).abs();
        let r = if area > 1e-9 {
            a * b * c / (4.0 * area)
        } else {
            f64::INFINITY
        };
        if r.is_nan() {
            return f64::NAN;
        }
        if r < min {
            min = r;
        }
    }
    min
}

/// Points every exactly `h` metres from the start; the remainder (< h) is dropped.
fn resample_exact(coords: &[Pt], h: f64) -> Vec<Pt> {
    let mut cum = Vec::with_capacity(coords.len());
    let mut total = 0.0;
    cum.push(0.0);
    for w in coords.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        total += (dx * dx + dy * dy).sqrt();
        cum.push(total);
    }
    let xs: Vec<f64> = coords.iter().map(|p| p[0]).collect();
    let ys: Vec<f64> = coords.iter().map(|p| p[1]).collect();
    // `np.arange(0.0, total + 1e-9, h)`: ceil((stop - start) / step) values, `i * h`.
    let n = ((total + 1e-9) / h).ceil().max(0.0) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 * h;
            [interp(t, &cum, &xs), interp(t, &cum, &ys)]
        })
        .collect()
}

/// Minimum three-point radius (m) of `line` read on a chord of exactly `chord_m`: forward from the
/// start and backward from the end, half a chord apart. Infinity for a straight line, or a line
/// shorter than the chord.
pub fn min_radius_on_chord(line: &[Pt], chord_m: f64) -> Result<f64, PyError> {
    if !(chord_m.is_finite() && chord_m > 0.0) {
        return Err(PyError::value("chord_m must be finite and positive."));
    }
    let half = chord_m / 2.0;
    let k = ((half / MAX_STRIDE_M).ceil() as usize).max(1);
    let h = half / k as f64;
    let forward = three_point_min(&resample_exact(line, h), k);
    let reversed: Vec<Pt> = line.iter().rev().copied().collect();
    let backward = three_point_min(&resample_exact(&reversed, h), k);
    Ok(py_min(forward, backward))
}

/// Minimum three-point radius (m) on a polyline's own vertices, arms `k` vertices long.
pub fn min_radius_on_vertices(pts: &[Pt], k: i64) -> Result<f64, PyError> {
    if k < 1 {
        return Err(PyError::value("k must be at least 1."));
    }
    Ok(three_point_min(pts, k as usize))
}

/// True when `measured_m` holds `r_min` within the reading tolerance (false on NaN).
pub fn radius_ok(measured_m: f64, r_min: f64) -> bool {
    measured_m >= (1.0 - RADIUS_TOL) * r_min
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn circle(r: f64, ds: f64, sweep: f64) -> Vec<Pt> {
        let n = (sweep / (ds / r)).ceil() as usize;
        (0..n)
            .map(|i| {
                let th = i as f64 * ds / r;
                [r * libm::sin(th), r * (1.0 - libm::cos(th))]
            })
            .collect()
    }

    #[test]
    fn vertices_on_a_circle_read_its_radius() {
        let r = min_radius_on_vertices(&circle(150.0, 100.0, 5.0), 1).unwrap();
        assert!((r - 150.0).abs() < 1e-6, "{r}");
    }

    #[test]
    fn a_straight_line_reads_infinity_and_a_short_one_too() {
        assert_eq!(
            min_radius_on_chord(&[[0.0, 0.0], [1000.0, 0.0]], 100.0),
            Ok(f64::INFINITY)
        );
        assert_eq!(
            min_radius_on_chord(&[[0.0, 0.0], [40.0, 0.0], [60.0, 30.0]], 100.0),
            Ok(f64::INFINITY)
        );
    }

    #[test]
    fn a_chord_that_is_not_finite_and_positive_is_refused() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let e = min_radius_on_chord(&[[0.0, 0.0], [10.0, 0.0]], bad).unwrap_err();
            assert_eq!(e.message, "chord_m must be finite and positive.");
        }
        assert_eq!(
            min_radius_on_vertices(&[], 0).unwrap_err().message,
            "k must be at least 1."
        );
    }

    #[test]
    fn radius_ok_allows_two_percent_and_is_false_for_nan() {
        assert!(radius_ok(294.0, 300.0));
        assert!(!radius_ok(293.9, 300.0));
        assert!(radius_ok(f64::INFINITY, 300.0));
        assert!(!radius_ok(f64::NAN, 300.0));
        assert!(radius_ok(0.0, 0.0));
    }
}
