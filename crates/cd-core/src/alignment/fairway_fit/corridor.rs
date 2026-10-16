//! The corridor QP of the fairway fit (ADR 0007): bounded least squares on lateral offsets.
//!
//! minimise Σ_r (c_r + Σ_j A_rj a_j)²  subject to  lo_j <= a_j <= hi_j
//!
//! Each row touches at most three neighbouring offsets, so `AᵀA` is banded (half-bandwidth 2).
//! Solved as a convex QP by Clarabel (interior point, pure Rust, Apache-2.0). If Clarabel ever has
//! to go, an own bounded least squares goes behind this same function.

use clarabel::algebra::CscMatrix;
use clarabel::solver::{
    DefaultSettingsBuilder, DefaultSolver, IPSolver, NonnegativeConeT, SolverStatus,
};

/// One row: a constant and up to three `(offset index, coefficient)` terms, indices within 2 of
/// each other.
pub(crate) struct Row {
    pub c: f64,
    pub terms: Vec<(usize, f64)>,
}

/// The narrowest box the solver is given (m). An interior-point method needs room between the
/// bounds; a box narrower than this is widened about its middle.
const MIN_BOX_M: f64 = 1e-3;

/// The offsets that minimise the rows' sum of squares inside the boxes, or `None` when the solver
/// does not reach a solution.
pub(crate) fn min_sum_squares_in_box(rows: &[Row], lo: &[f64], hi: &[f64]) -> Option<Vec<f64>> {
    let n = lo.len();
    if n == 0 {
        return Some(vec![]);
    }
    // P = 2 AᵀA (upper triangle, banded): band[j][k] = P(j - k, j). q = 2 Aᵀc.
    let mut band = vec![[0.0f64; 3]; n];
    let mut q = vec![0.0; n];
    for r in rows {
        for &(i, ai) in &r.terms {
            q[i] += 2.0 * ai * r.c;
            for &(j, aj) in &r.terms {
                if i <= j {
                    band[j][j - i] += 2.0 * ai * aj;
                }
            }
        }
    }
    // Scale the objective so its largest coefficient is about 1. The minimiser is unchanged; an
    // interior-point method on a corner seed (weights near the reweight cap, 10 m spacing) otherwise
    // stalls with InsufficientProgress and the pass is lost.
    let scale = band
        .iter()
        .flat_map(|b| b.iter())
        .chain(q.iter())
        .fold(0.0f64, |m, &v| m.max(v.abs()));
    if scale > 0.0 && scale.is_finite() {
        for b in band.iter_mut() {
            for v in b.iter_mut() {
                *v /= scale;
            }
        }
        for v in q.iter_mut() {
            *v /= scale;
        }
    }
    let (mut colptr, mut rowval, mut nzval) = (vec![0usize], vec![], vec![]);
    for (j, b) in band.iter().enumerate() {
        for k in (0..3).rev() {
            if k <= j {
                rowval.push(j - k);
                nzval.push(b[k]);
            }
        }
        colptr.push(rowval.len());
    }
    let p = CscMatrix::new(n, n, colptr, rowval, nzval);
    // a <= hi and -a <= -lo: A x + s = b, s >= 0.
    let a = CscMatrix::new(
        2 * n,
        n,
        (0..=n).map(|j| 2 * j).collect(),
        (0..n).flat_map(|j| [j, n + j]).collect(),
        (0..n).flat_map(|_| [1.0, -1.0]).collect(),
    );
    let mut b = Vec::with_capacity(2 * n);
    let mut nb = Vec::with_capacity(n);
    for j in 0..n {
        let (mut l, mut h) = (lo[j], hi[j]);
        if h - l < MIN_BOX_M {
            let m = (l + h) / 2.0;
            (l, h) = (m - MIN_BOX_M / 2.0, m + MIN_BOX_M / 2.0);
        }
        b.push(h);
        nb.push(-l);
    }
    b.extend(nb);
    let settings = DefaultSettingsBuilder::default()
        .verbose(false)
        .max_iter(200)
        .build()
        .ok()?;
    let cones = [NonnegativeConeT(2 * n)];
    let mut solver = DefaultSolver::new(&p, &q, &a, &b, &cones, settings).ok()?;
    solver.solve();
    match solver.solution.status {
        SolverStatus::Solved | SolverStatus::AlmostSolved => {
            // Interior-point answers sit a hair inside the box; clip the last bits.
            Some(
                solver
                    .solution
                    .x
                    .iter()
                    .zip(lo.iter().zip(hi))
                    .map(|(&x, (&l, &h))| if l <= h { x.clamp(l, h) } else { x })
                    .collect(),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation
    use super::*;

    #[test]
    fn an_unbounded_row_is_solved_exactly_and_a_box_clips_it() {
        // min (a0 - 2a1 + a2 + 3)² with a0 = a2 = 0 held by tight boxes: a1 = 1.5.
        let rows = [Row {
            c: 3.0,
            terms: vec![(0, 1.0), (1, -2.0), (2, 1.0)],
        }];
        let x = min_sum_squares_in_box(&rows, &[0.0, -10.0, 0.0], &[0.0, 10.0, 0.0]).unwrap();
        assert!((x[1] - 1.5).abs() < 1e-3, "{x:?}");
        // The box [-10, 1] holds a1 at its upper bound.
        let x = min_sum_squares_in_box(&rows, &[0.0, -10.0, 0.0], &[0.0, 1.0, 0.0]).unwrap();
        assert!((x[1] - 1.0).abs() < 1e-6, "{x:?}");
    }
}
