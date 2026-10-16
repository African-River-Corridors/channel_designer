//! numpy and CPython float routines the port must reproduce, written out. Shared by the modules
//! that read as `np.interp` / `np.linspace` / `%` in the frozen Python.
//!
//! Label: property (the exact-value tests are the parity cases)

/// `np.mod` for floats: the result takes the sign of the divisor, as Python's `%`.
pub(crate) fn np_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) { m + b } else { m }
    } else {
        libm::copysign(0.0, b)
    }
}

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
    // largest j with xp[j] <= x
    let j = xp.partition_point(|&v| v <= x) - 1;
    if j == n - 1 {
        return fp[j];
    }
    if xp[j] == x {
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

/// `ndarray.sum()` of a 1-D float array: numpy's pairwise summation. The reduction starts from the
/// first element and adds the pairwise sum of the rest (blocks of 8 below 128 elements).
pub(crate) fn np_sum(xs: &[f64]) -> f64 {
    match xs.split_first() {
        None => 0.0,
        Some((&first, rest)) => first + pairwise(rest),
    }
}

fn pairwise(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0;
        for &x in a {
            res += x;
        }
        res
    } else if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let body = n - n % 8;
        let mut i = 8;
        while i < body {
            for (j, rj) in r.iter_mut().enumerate() {
                *rj += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for &x in &a[i..] {
            res += x;
        }
        res
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        pairwise(&a[..n2]) + pairwise(&a[n2..])
    }
}

/// `np.allclose(a, b, atol=atol)` for two points, with numpy's default `rtol` of 1e-5. It is not
/// symmetric: the relative part scales with `b`.
pub(crate) fn allclose2(a: [f64; 2], b: [f64; 2], atol: f64) -> bool {
    (0..2).all(|i| (a[i] - b[i]).abs() <= atol + 1e-5 * b[i].abs())
}

/// `math.isclose(a, b)` with the default `rel_tol` of 1e-9.
pub(crate) fn isclose(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

/// `np.searchsorted(a, v, side="left")`: how many elements are below `v` (`a` is sorted).
pub(crate) fn searchsorted_left(a: &[f64], v: f64) -> usize {
    a.partition_point(|&x| x < v)
}

/// `np.searchsorted(a, v, side="right")`: how many elements are at or below `v`.
pub(crate) fn searchsorted_right(a: &[f64], v: f64) -> usize {
    a.partition_point(|&x| x <= v)
}

// --- added for the alignment port (ticket 27) ---

use crate::geom::Pt;

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
        let mut ddmod = np_mod(dd + PI, 2.0 * PI) - PI;
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
mod alignment_tests {
    //! Label: sourced - numpy 2.x and CPython 3.12 results for the same inputs
    use super::*;

    #[test]
    fn floor_division_and_modulo_follow_python() {
        assert_eq!(py_floordiv(7.0, 2.0), 3.0);
        assert_eq!(py_floordiv(-7.0, 2.0), -4.0);
        assert_eq!(py_floordiv(0.3, 0.1), 2.0);
        assert_eq!(np_mod(-1.0, 3.0), 2.0);
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
