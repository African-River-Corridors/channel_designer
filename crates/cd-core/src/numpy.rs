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

/// `np.arange(start, stop, step)` for floats: `ceil((stop - start) / step)` values. numpy fills
/// them as `start + i * delta`, where `delta = (start + step) - start` (which is not always
/// `step`, to the last bit).
pub(crate) fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil();
    if n.is_nan() || n <= 0.0 {
        return Vec::new();
    }
    let n = n as usize;
    let delta = (start + step) - start;
    (0..n)
        .map(|i| match i {
            0 => start,
            1 => start + step,
            _ => start + i as f64 * delta,
        })
        .collect()
}

/// `ndarray.min()` of floats: NaN if any value is NaN. Panics on an empty slice, as numpy raises.
pub(crate) fn np_min(xs: &[f64]) -> f64 {
    let mut m = xs[0];
    for &x in xs {
        if x.is_nan() {
            return f64::NAN;
        }
        if x < m {
            m = x;
        }
    }
    m
}

/// `np.argmin`: the first index of the least value (the first NaN, if there is one).
pub(crate) fn argmin(xs: &[f64]) -> usize {
    let mut best = 0;
    for (i, &x) in xs.iter().enumerate() {
        if x.is_nan() {
            return i;
        }
        if x < xs[best] {
            best = i;
        }
    }
    best
}
