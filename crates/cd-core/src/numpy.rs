//! The numpy and Python float operations the alignment port needs, written out so that each one
//! rounds as numpy's or CPython's does.

use crate::geom::Pt;

/// `np.linspace(start, stop, num)` with the endpoint: `i * step + start`, last value `stop`.
pub(crate) fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return Vec::new();
    }
    if num == 1 {
        return vec![start];
    }
    let div = (num - 1) as f64;
    let delta = stop - start;
    let step = delta / div;
    let mut y: Vec<f64> = (0..num)
        .map(|i| {
            if step == 0.0 {
                (i as f64 / div) * delta + start
            } else {
                i as f64 * step + start
            }
        })
        .collect();
    y[num - 1] = stop;
    y
}

/// `np.interp(x, xp, fp)` for one point: flat outside the range.
pub(crate) fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if x.is_nan() {
        return x;
    }
    if x > xp[n - 1] {
        return fp[n - 1];
    }
    if x < xp[0] {
        return fp[0];
    }
    let j = xp.partition_point(|&v| v <= x) - 1;
    if j == n - 1 || xp[j] == x {
        return fp[j];
    }
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    let mut r = slope * (x - xp[j]) + fp[j];
    if r.is_nan() {
        r = slope * (x - xp[j + 1]) + fp[j + 1];
        if r.is_nan() && fp[j] == fp[j + 1] {
            r = fp[j];
        }
    }
    r
}

/// `np.linalg.norm` of a 2-vector: `sqrt(x*x + y*y)`.
pub(crate) fn norm(dx: f64, dy: f64) -> f64 {
    (dx * dx + dy * dy).sqrt()
}

/// `concatenate(([0], cumsum(norm(diff(points), axis=1))))`: chainage at each point.
pub(crate) fn chainages(points: &[Pt]) -> Vec<f64> {
    let mut out = Vec::with_capacity(points.len());
    let mut acc = 0.0;
    out.push(0.0);
    for w in points.windows(2) {
        acc += norm(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        out.push(acc);
    }
    out
}

/// `np.searchsorted(a, v, side="left")`.
pub(crate) fn searchsorted_left(a: &[f64], v: f64) -> usize {
    a.partition_point(|&x| x < v)
}

/// `np.searchsorted(a, v, side="right")`.
pub(crate) fn searchsorted_right(a: &[f64], v: f64) -> usize {
    a.partition_point(|&x| x <= v)
}

/// `np.mod` / Python `%` for floats: the result takes the sign of the divisor.
pub(crate) fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 && ((b < 0.0) != (m < 0.0)) {
        m + b
    } else {
        m
    }
}

/// Python `a // b` for floats (CPython `float_floor_div`).
pub(crate) fn py_floordiv(a: f64, b: f64) -> f64 {
    let m = a % b;
    let mut div = (a - m) / b;
    if m != 0.0 && ((b < 0.0) != (m < 0.0)) {
        div -= 1.0;
    }
    if div == 0.0 {
        return 0.0f64.copysign(a / b);
    }
    let mut f = div.floor();
    if div - f > 0.5 {
        f += 1.0;
    }
    f
}

/// `np.unwrap(p)` with the default period 2π.
pub(crate) fn unwrap(p: &[f64]) -> Vec<f64> {
    use core::f64::consts::PI;
    let mut out = p.to_vec();
    let mut correction = 0.0;
    for i in 1..p.len() {
        let dd = p[i] - p[i - 1];
        let mut ddmod = py_mod(dd + PI, 2.0 * PI) - PI;
        if ddmod == -PI && dd > 0.0 {
            ddmod = PI;
        }
        let mut ph = ddmod - dd;
        if dd.abs() < PI {
            ph = 0.0;
        }
        correction += ph;
        out[i] = p[i] + correction;
    }
    out
}

/// `np.convolve(a, np.ones(n) / n, mode)` for `"valid"` and `"same"`: each output is the sum, in
/// order, of `a[j] * (1 / n)` over the overlapping `j`.
pub(crate) fn box_convolve(a: &[f64], n: usize, same: bool) -> Vec<f64> {
    let w = 1.0 / n as f64;
    let len = a.len() as isize;
    let (count, first) = if same {
        (a.len().max(n), ((n - 1) / 2) as isize - (n - 1) as isize)
    } else {
        (a.len().max(n) - a.len().min(n) + 1, 0)
    };
    (0..count as isize)
        .map(|i| {
            let mut s = 0.0;
            for j in (i + first)..(i + first + n as isize) {
                if (0..len).contains(&j) {
                    s += a[j as usize] * w;
                }
            }
            s
        })
        .collect()
}

/// Python `math.isclose(a, b)` with the defaults (`rel_tol=1e-9`, `abs_tol=0`).
pub(crate) fn isclose(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

/// `np.sign`: -1, 0 or +1 (NaN stays NaN).
pub(crate) fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    //! Label: sourced - numpy 2.x and CPython 3.12 results for the same inputs
    use super::*;

    #[test]
    fn floor_division_and_modulo_follow_python() {
        assert_eq!(py_floordiv(7.0, 2.0), 3.0);
        assert_eq!(py_floordiv(-7.0, 2.0), -4.0);
        assert_eq!(py_floordiv(0.3, 0.1), 2.0);
        assert_eq!(py_mod(-1.0, 3.0), 2.0);
    }

    #[test]
    fn unwrap_removes_a_jump_of_two_pi() {
        let u = unwrap(&[3.0, -3.0]);
        assert!((u[1] - (-3.0 + 2.0 * core::f64::consts::PI)).abs() < 1e-15);
    }

    #[test]
    fn box_convolve_same_and_valid_lengths() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(box_convolve(&a, 5, true).len(), 6);
        assert_eq!(box_convolve(&a, 5, false).len(), 2);
        assert!((box_convolve(&a, 3, false)[0] - 2.0).abs() < 1e-15);
        // numpy: np.convolve([1, 2, 3, 4, 5, 6], np.ones(4) / 4, "same")[0] == (1 + 2) / 4
        assert_eq!(box_convolve(&a, 4, true)[0], 0.75);
    }
}
