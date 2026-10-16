//! Vectorised twin of the cut models, for many cross-sections at once. Mirrors
//! `volumes/cut_model_grid.py`.
//!
//! Same rules as [`crate::cut_model`] (and the same constants, imported, never restated), laid out
//! for an `(n_sections, n_offsets)` rake so a search scorer can evaluate thousands of sections
//! quickly.
//!
//! Inputs, all aligned on one offset vector `offs` (m, signed):
//! - `tmpl2`   (n, m) caller's fairway template (used where |offset| <= hb)
//! - `gz2`     (n, m) ground elevation
//! - `finite2` (n, m) true where `gz2` is known
//! - `hb`      scalar or (n,) bottom half-width

use crate::cut_model::{
    ABOVE_SLOPE_H_PER_V, BERM_M, DAYLIGHT_RUN_M, PRISM_INTRUDE_M, WET_SLOPE_H_PER_V,
};
use crate::py_max;

/// An `(n, m)` rake: one row per section.
pub type Matrix = Vec<Vec<f64>>;
/// A true/false rake of the same shape.
pub type Mask = Vec<Vec<bool>>;

/// The bottom half-width of each section.
#[derive(Debug, Clone, PartialEq)]
pub enum HalfWidths {
    Scalar(f64),
    PerSection(Vec<f64>),
    /// A value that is neither a scalar nor a vector, with the shape Python would report.
    Invalid(Vec<usize>),
}

/// `repr()` of a numpy shape tuple.
fn shape_repr(shape: &[usize]) -> String {
    match shape.len() {
        1 => format!("({},)", shape[0]),
        _ => format!(
            "({})",
            shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// One half-width per section, as a column (`hb[i]` for row `i`). Python's `_hb_column`.
fn hb_column(hb: &HalfWidths, n: usize) -> Result<Vec<f64>, String> {
    match hb {
        HalfWidths::Scalar(v) => Ok(vec![*v; n]),
        HalfWidths::PerSection(v) if v.len() == n => Ok(v.clone()),
        HalfWidths::PerSection(v) => Err(format!(
            "hb must be a scalar or an (n_sections,) vector; got shape {} for {n} sections",
            shape_repr(&[v.len()])
        )),
        HalfWidths::Invalid(shape) => Err(format!(
            "hb must be a scalar or an (n_sections,) vector; got shape {} for {n} sections",
            shape_repr(shape)
        )),
    }
}

/// Cut Model A design surface for every section. Returns `(n, m)`; +inf = no cut.
#[allow(clippy::too_many_arguments)]
pub fn cut_model_a_template(
    tmpl2: &Matrix,
    gz2: &Matrix,
    finite2: &Mask,
    offs: &[f64],
    hb: &HalfWidths,
    grade: f64,
    wl: f64,
    wet_slope: f64,
) -> Result<Matrix, String> {
    let n = gz2.len();
    let hbc = hb_column(hb, n)?;
    let depth = wl - grade;
    let r_wl = wet_slope * depth;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o_wl = hbc[i] + r_wl;
        let m = offs.len();
        let mut r2 = Vec::with_capacity(m);
        let mut t_a = Vec::with_capacity(m);
        let mut t_b = Vec::with_capacity(m);
        let (mut pen_left, mut pen_right) = (0.0_f64, 0.0_f64);
        for (j, &o) in offs.iter().enumerate() {
            let ao = o.abs();
            let r = ao - hbc[i];
            let prism = ao <= o_wl;
            let pen = if prism { o_wl - ao } else { 0.0 };
            let wet_line = grade + py_max(0.0, r) / wet_slope;
            let in_wet = r > 0.0 && r <= r_wl;
            let in_berm = r > r_wl && r <= r_wl + BERM_M;
            let in_face = r > r_wl + BERM_M;
            t_a.push(if in_wet { wet_line } else { f64::INFINITY });
            t_b.push(if in_wet {
                wet_line
            } else if in_berm {
                wl
            } else if in_face {
                wl + (r - r_wl - BERM_M) / ABOVE_SLOPE_H_PER_V
            } else {
                f64::INFINITY
            });
            r2.push(r);
            let intrude = finite2[i][j] && gz2[i][j] > wl && prism;
            if intrude && o < 0.0 {
                pen_left = py_max(pen_left, pen);
            }
            if intrude && o > 0.0 {
                pen_right = py_max(pen_right, pen);
            }
        }
        let b_l = pen_left > PRISM_INTRUDE_M;
        let b_r = pen_right > PRISM_INTRUDE_M;
        let row: Vec<f64> = (0..m)
            .map(|j| {
                if r2[j] <= 0.0 {
                    return tmpl2[i][j];
                }
                let use_b = (b_l && offs[j] < 0.0) || (b_r && offs[j] > 0.0);
                if use_b { t_b[j] } else { t_a[j] }
            })
            .collect();
        out.push(row);
    }
    Ok(out)
}

/// `(n, m)` true where the offset lies BEYOND the works under Cut Model B.
pub fn beyond_works_mask(
    comp: &Matrix,
    gz2: &Matrix,
    finite2: &Mask,
    offs: &[f64],
    o_wl: &[f64],
    run_m: f64,
    step: f64,
) -> Mask {
    let n = gz2.len();
    let m = offs.len();
    let ao: Vec<f64> = offs.iter().map(|o| o.abs()).collect();
    let l = std::cmp::max(1, (run_m / step).ceil() as i64) as usize;
    let mut beyond = vec![vec![false; m]; n];
    for positive in [true, false] {
        let mut order: Vec<usize> = (0..m)
            .filter(|&j| {
                if positive {
                    offs[j] > 0.0
                } else {
                    offs[j] < 0.0
                }
            })
            .collect();
        if order.is_empty() {
            continue;
        }
        order.sort_by(|&a, &b| {
            ao[a]
                .partial_cmp(&ao[b])
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let k = order.len();
        for i in 0..n {
            let daylit: Vec<bool> = order
                .iter()
                .map(|&j| !finite2[i][j] || !comp[i][j].is_finite() || gz2[i][j] <= comp[i][j])
                .collect();
            // prefix count of stretches that are NOT daylit
            let mut c = vec![0_i64; k + 1];
            for t in 0..k {
                c[t + 1] = c[t] + i64::from(!daylit[t]);
            }
            let mut first = k;
            for t in 0..k {
                let hi = t + l;
                if hi >= k {
                    break;
                }
                let win = c[hi + 1] - c[t] == 0;
                let cand = daylit[t] && ao[order[t]] > o_wl[i];
                if win && cand {
                    first = t;
                    break;
                }
            }
            for (t, &j) in order.iter().enumerate() {
                beyond[i][j] = t >= first;
            }
        }
    }
    beyond
}

/// Cut Model A's template, then close the works at sustained daylight. `step` of `None` is the
/// rake step from the distinct |offset| values.
///
/// The limit is quantised to the rake step: a search proxy. Quote volumes from the scalar
/// integrator ([`super::sections::section_volumes`]).
#[allow(clippy::too_many_arguments)]
pub fn cut_model_b_template(
    tmpl2: &Matrix,
    gz2: &Matrix,
    finite2: &Mask,
    offs: &[f64],
    hb: &HalfWidths,
    grade: f64,
    wl: f64,
    wet_slope: f64,
    run_m: f64,
    step: Option<f64>,
) -> Result<Matrix, String> {
    let comp = cut_model_a_template(tmpl2, gz2, finite2, offs, hb, grade, wl, wet_slope)?;
    let n = gz2.len();
    let hbc = hb_column(hb, n)?;
    let o_wl: Vec<f64> = hbc.iter().map(|h| h + wet_slope * (wl - grade)).collect();
    let step = match step {
        Some(s) => s,
        None => {
            // np.unique matters: a symmetric rake repeats every |offset|, and a plain
            // sort-and-diff then returns a step of 0.
            let mut u: Vec<f64> = offs.iter().map(|o| o.abs()).collect();
            u.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            u.dedup();
            if u.len() > 1 {
                u.windows(2)
                    .map(|w| w[1] - w[0])
                    .fold(f64::INFINITY, f64::min)
            } else {
                1.0
            }
        }
    };
    let beyond = beyond_works_mask(&comp, gz2, finite2, offs, &o_wl, run_m, step);
    Ok(comp
        .into_iter()
        .zip(beyond)
        .map(|(row, b)| {
            row.into_iter()
                .zip(b)
                .map(|(v, bey)| if bey { f64::INFINITY } else { v })
                .collect()
        })
        .collect())
}

/// Defaults of the Python keyword arguments.
pub const DEFAULT_WET_SLOPE: f64 = WET_SLOPE_H_PER_V;
pub const DEFAULT_RUN_M: f64 = DAYLIGHT_RUN_M;
