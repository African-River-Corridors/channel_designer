//! Bend catalogue and the TIGHT / GENTLE / STRAIGHT classification of a natural river. Mirrors
//! `alignment/bends.py`.
//!
//! `e` is the smallest lateral corridor half-width about the natural centreline inside which some
//! curve of radius >= R_min can traverse the bend. It is found exactly: a forward-reachability DP
//! over (lateral offset, heading error) with |curvature| <= 1/R_min, and bisection on the
//! half-width. Every grid index is rounded as numpy's `rint` rounds it, from the same arithmetic,
//! so the reachable sets, and with them the classes, are the Python's.

use crate::geom::overlay::{Loc, locate_all};
use crate::geom::{Polygon, Pt, linear};
use crate::numpy::{box_convolve, linspace, py_floordiv, sign};
use crate::rules::rulesets::{classify_bend, lateral_room_m};

pub const STEP_M: f64 = 10.0;
pub const N_PSI: usize = 41;
pub const PSI_MAX_DEG: f64 = 12.0;
pub const N_Y: usize = 81;
pub const E_MAX_M: f64 = 300.0;
pub const E_TOL_M: f64 = 0.5;

/// Python `math.radians` and numpy `np.radians`: `x * (pi / 180)`.
fn radians(d: f64) -> f64 {
    d * (core::f64::consts::PI / 180.0)
}

/// Python 3.12 `sum()` of floats: Neumaier's compensated sum.
pub fn py_sum(xs: impl IntoIterator<Item = f64>) -> f64 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for x in xs {
        let t = s + x;
        if s.abs() >= x.abs() {
            c += (s - t) + x;
        } else {
            c += (x - t) + s;
        }
        s = t;
    }
    if c != 0.0 && c.is_finite() { s + c } else { s }
}

/// `resample(line, step)`: stations `i * step` for `i < int(length // step) + 1`, and their points.
pub fn resample(line: &[Pt], step: f64) -> (Vec<f64>, Vec<Pt>) {
    let n = py_floordiv(linear::length(line), step) as usize + 1;
    let s: Vec<f64> = (0..n).map(|i| i as f64 * step).collect();
    let xy = s
        .iter()
        .map(|&d| linear::interpolate(line, d).unwrap_or([f64::NAN; 2]))
        .collect();
    (s, xy)
}

/// `curvature(xy)`: signed curvature by the three-point circumradius, positive left.
pub fn curvature(xy: &[Pt]) -> Vec<f64> {
    let n = xy.len();
    let mut k = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        let ([ax, ay], [bx, by], [cx, cy]) = (xy[i - 1], xy[i], xy[i + 1]);
        let cross = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
        let a = libm::hypot(bx - ax, by - ay);
        let b = libm::hypot(cx - bx, cy - by);
        let c = libm::hypot(ax - cx, ay - cy);
        if a * b * c < 1e-9 {
            continue;
        }
        k[i] = 2.0 * cross / (a * b * c);
    }
    if n >= 2 {
        k[0] = k[1];
        k[n - 1] = k[n - 2];
    }
    k
}

/// One bend unit: a signed lobe of the smoothed curvature.
#[derive(Debug, Clone, PartialEq)]
pub struct BendUnit {
    pub i0: usize,
    pub i1: usize,
    pub s0: f64,
    pub s1: f64,
    pub length_m: f64,
    pub sign: i64,
    pub kappa_max: f64,
    pub r_min_natural_m: f64,
    pub deflection_rad: f64,
}

/// `bend_units`: split at sign changes of the curvature smoothed over `smooth_n` points.
pub fn bend_units(s: &[f64], kappa: &[f64], smooth_n: usize) -> Vec<BendUnit> {
    let ks = box_convolve(kappa, smooth_n, true);
    let sg: Vec<f64> = ks
        .iter()
        .map(|&k| if sign(k) == 0.0 { 1.0 } else { sign(k) })
        .collect();
    let mut idx = vec![0];
    idx.extend((1..sg.len()).filter(|&i| sg[i] != sg[i - 1]));
    idx.push(sg.len() - 1);
    let mut out = Vec::new();
    for w in idx.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b < a + 2 {
            continue;
        }
        let seg = &ks[a..=b];
        let kmax = seg.iter().map(|k| k.abs()).fold(f64::NEG_INFINITY, |m, k| {
            if k > m || k.is_nan() { k } else { m }
        });
        out.push(BendUnit {
            i0: a,
            i1: b,
            s0: s[a],
            s1: s[b],
            length_m: s[b] - s[a],
            sign: sg[a] as i64,
            kappa_max: kmax,
            r_min_natural_m: if kmax > 1e-9 {
                1.0 / kmax
            } else {
                f64::INFINITY
            },
            deflection_rad: crate::numpy::np_sum(seg).abs() * (s[1] - s[0]),
        });
    }
    out
}

/// `feasible_within`: can a curve with |k| <= 1/r_min traverse the window inside +-half_width?
pub fn feasible_within(
    kappa: &[f64],
    ds: f64,
    half_width: f64,
    r_min: f64,
    n_y: usize,
    n_psi: usize,
) -> bool {
    if half_width <= 0.0 {
        return kappa.iter().all(|k| k.abs() <= 1.0 / r_min + 1e-12);
    }
    let ys = linspace(-half_width, half_width, n_y);
    let psis: Vec<f64> = linspace(-PSI_MAX_DEG, PSI_MAX_DEG, n_psi)
        .into_iter()
        .map(radians)
        .collect();
    let (dy, dpsi) = (ys[1] - ys[0], psis[1] - psis[0]);
    let k_max = 1.0 / r_min;
    let argmin_abs = |v: &[f64]| {
        let mut best = 0;
        for (i, x) in v.iter().enumerate() {
            if x.abs() < v[best].abs() {
                best = i;
            }
        }
        best
    };
    let mut reach = vec![false; n_y * n_psi];
    reach[argmin_abs(&ys) * n_psi + argmin_abs(&psis)] = true;
    let k_cmds = linspace(-k_max, k_max, 9);
    let psi_off = radians(PSI_MAX_DEG);
    for &k_nat in &kappa[..kappa.len().saturating_sub(1)] {
        let states: Vec<usize> = (0..reach.len()).filter(|&i| reach[i]).collect();
        if states.is_empty() {
            return false;
        }
        let mut next = vec![false; n_y * n_psi];
        for &kc in &k_cmds {
            for &st in &states {
                let (y0, p0) = (ys[st / n_psi], psis[st % n_psi]);
                let y1 = y0 + p0 * ds;
                let p1 = p0 + (kc - k_nat) * ds;
                let j = ((y1 + half_width) / dy).round_ties_even();
                let m = ((p1 + psi_off) / dpsi).round_ties_even();
                if j >= 0.0 && j < n_y as f64 && m >= 0.0 && m < n_psi as f64 {
                    next[j as usize * n_psi + m as usize] = true;
                }
            }
        }
        reach = next;
    }
    reach.iter().any(|&r| r)
}

/// `e_exact`: the smallest corridor half-width that admits an R_min-feasible curve (infinite
/// above `E_MAX_M`), by bisection to `E_TOL_M`.
pub fn e_exact(kappa: &[f64], ds: f64, r_min: f64) -> f64 {
    if feasible_within(kappa, ds, 0.0, r_min, N_Y, N_PSI) {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0, E_MAX_M);
    if !feasible_within(kappa, ds, hi, r_min, N_Y, N_PSI) {
        return f64::INFINITY;
    }
    while hi - lo > E_TOL_M {
        let mid = 0.5 * (lo + hi);
        if feasible_within(kappa, ds, mid, r_min, N_Y, N_PSI) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// `e_closed_form`: the closed form, kept only as a reported cross-check.
pub fn e_closed_form(r_nat: f64, defl: f64, r_min: f64) -> f64 {
    if !r_nat.is_finite() || r_nat >= r_min || defl <= 0.0 {
        return 0.0;
    }
    (r_min - r_nat)
        * (1.0 / libm::cos(crate::py_min(defl, core::f64::consts::PI * 0.98) / 2.0) - 1.0)
}

/// `contiguous_wetted_width`: walk out each side from `centre` and stop at the first exit.
pub fn contiguous_wetted_width(
    river: &[Polygon],
    centre: Pt,
    nx: f64,
    ny: f64,
    max_half_m: f64,
    probe_m: f64,
) -> f64 {
    let mut half = [0.0; 2];
    for (k, sgn) in [1.0, -1.0].into_iter().enumerate() {
        let mut d = 0.0;
        while d < max_half_m {
            let p = [
                centre[0] + sgn * (d + probe_m) * nx,
                centre[1] + sgn * (d + probe_m) * ny,
            ];
            if locate_all(p, river) != Loc::Interior {
                break;
            }
            d += probe_m;
        }
        half[k] = d;
    }
    py_sum(half)
}

/// One classified bend (`classify_bends` row).
#[derive(Debug, Clone, PartialEq)]
pub struct BendRow {
    pub unit: BendUnit,
    /// `None` when no half-width up to `E_MAX_M` admits a curve.
    pub e_m: Option<f64>,
    pub e_closed_form_m: f64,
    pub wetted_width_m: f64,
    pub room_m: f64,
    pub klass: &'static str,
}

/// `classify_bends`: the bend catalogue of `natural` against the river polygon.
pub fn classify_bends(
    natural: &[Pt],
    river: &[Polygon],
    r_min: f64,
    top_width: f64,
    bottom_width: f64,
    bank_cut_fraction: f64,
    step: f64,
) -> Vec<BendRow> {
    let (s, xy) = resample(natural, step);
    let kappa = curvature(&xy);
    bend_units(&s, &kappa, 5)
        .into_iter()
        .map(|b| {
            let e = e_exact(&kappa[b.i0..=b.i1], step, r_min);
            let mid = linear::interpolate(natural, 0.5 * (b.s0 + b.s1)).unwrap_or([f64::NAN; 2]);
            let j1 = b.i1.min(xy.len() - 1);
            let th = libm::atan2(xy[j1][1] - xy[b.i0][1], xy[j1][0] - xy[b.i0][0]);
            let w = contiguous_wetted_width(river, mid, -libm::sin(th), libm::cos(th), 200.0, 1.0);
            let room = lateral_room_m(
                if w > 0.0 { w } else { top_width },
                top_width,
                bottom_width,
                bank_cut_fraction,
            );
            let klass = if e.is_finite() {
                classify_bend(e, room, 1e-9)
            } else {
                "T"
            };
            BendRow {
                e_closed_form_m: e_closed_form(b.r_min_natural_m, b.deflection_rad, r_min),
                e_m: e.is_finite().then_some(e),
                wetted_width_m: w,
                room_m: room,
                klass,
                unit: b,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Label: snapshot - the frozen Python's `e_exact` for the same inputs
    use super::*;

    #[test]
    fn e_exact_on_a_straight_a_gentle_arc_and_a_hairpin() {
        assert_eq!(e_exact(&[0.0; 20], 10.0, 500.0), 0.0);
        assert_eq!(e_exact(&[1.0 / 300.0; 10], 10.0, 500.0), 3.80859375);
        assert_eq!(e_exact(&[1.0 / 100.0; 30], 10.0, 500.0), f64::INFINITY);
    }

    #[test]
    fn neumaier_sum_keeps_the_small_terms() {
        // Label: property
        assert_eq!(py_sum([1e16, 1.0, -1e16]), 1.0);
    }
}
