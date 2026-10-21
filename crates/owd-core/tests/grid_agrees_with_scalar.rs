//! The vectorised grid twin and the scalar cut models agree on the same profiles.
//!
//! Label: property. No fixed number: both sides are computed here, from the same rules.

use owd_core::cut_model::{
    WET_SLOPE_H_PER_V, cut_model_a_bench_gate, cut_model_a_face, cut_model_b_daylight_limit,
};
use owd_core::volumes::cut_model_grid::{HalfWidths, cut_model_a_template, cut_model_b_template};

const DEPTH: f64 = 3.25;
const HB: f64 = 11.2;
const WL: f64 = 7.0;
const GRADE: f64 = WL - DEPTH;

fn offsets() -> Vec<f64> {
    (-120..=120).map(f64::from).collect()
}

/// Ground: a knoll on the right, a high wall on the far right, low elsewhere.
fn knoll(offs: &[f64]) -> Vec<f64> {
    let o_wl = HB + 3.0 * DEPTH;
    offs.iter()
        .map(|&o| {
            if o > 0.0 && (o_wl - 4.0..=o_wl + 6.0).contains(&o) {
                WL + 3.0
            } else if o > 0.0 && (60.0..=80.0).contains(&o) {
                WL + 60.0
            } else {
                WL - 4.0
            }
        })
        .collect()
}

fn wall(offs: &[f64]) -> Vec<f64> {
    let start = HB + 3.0 * DEPTH - 5.0;
    offs.iter()
        .map(|&o| {
            if o.abs() >= start {
                WL + 10.0
            } else {
                WL - 5.0
            }
        })
        .collect()
}

fn template(offs: &[f64]) -> Vec<f64> {
    offs.iter()
        .map(|o| GRADE + (o.abs() - HB).max(0.0) / 3.0)
        .collect()
}

#[test]
fn model_a_grid_equals_the_scalar_face_with_the_scalar_bench_gate() {
    let offs = offsets();
    let rows = vec![knoll(&offs), wall(&offs), knoll(&offs)];
    let tmpl: Vec<Vec<f64>> = rows.iter().map(|_| template(&offs)).collect();
    let finite = vec![vec![true; offs.len()]; rows.len()];
    let grid = cut_model_a_template(
        &tmpl,
        &rows,
        &finite,
        &offs,
        &HalfWidths::Scalar(HB),
        GRADE,
        WL,
        WET_SLOPE_H_PER_V,
    )
    .unwrap();
    for (i, ground) in rows.iter().enumerate() {
        let bench = cut_model_a_bench_gate(&offs, ground, HB, WL, DEPTH, None, WET_SLOPE_H_PER_V);
        for (j, &o) in offs.iter().enumerate() {
            let side: i8 = if o < 0.0 { -1 } else { 1 };
            // Offset 0 has no side in the grid (neither left nor right): Model A face.
            let use_bench = if o == 0.0 { false } else { bench.get(side) };
            let scalar =
                cut_model_a_face(o.abs(), use_bench, HB, GRADE, WL, DEPTH, WET_SLOPE_H_PER_V);
            let fairway = o.abs() <= HB;
            let want = if fairway { tmpl[i][j] } else { scalar };
            assert_eq!(grid[i][j], want, "section {i}, offset {o}");
        }
    }
}

#[test]
fn model_b_grid_closes_the_works_where_the_scalar_daylight_limit_does() {
    let offs = offsets();
    let ground = knoll(&offs);
    let tmpl = vec![template(&offs)];
    let gz = vec![ground.clone()];
    let finite = vec![vec![true; offs.len()]];
    let hb = HalfWidths::Scalar(HB);
    let run_m = 15.0;
    let model_a = cut_model_a_template(
        &tmpl,
        &gz,
        &finite,
        &offs,
        &hb,
        GRADE,
        WL,
        WET_SLOPE_H_PER_V,
    )
    .unwrap();
    let grid = cut_model_b_template(
        &tmpl,
        &gz,
        &finite,
        &offs,
        &hb,
        GRADE,
        WL,
        WET_SLOPE_H_PER_V,
        run_m,
        None,
    )
    .unwrap();
    let bench = cut_model_a_bench_gate(&offs, &ground, HB, WL, DEPTH, None, WET_SLOPE_H_PER_V);
    let limits = cut_model_b_daylight_limit(
        &offs,
        &ground,
        HB,
        GRADE,
        WL,
        DEPTH,
        bench,
        None,
        WET_SLOPE_H_PER_V,
        Some(run_m),
    );
    assert!(
        limits.get(1).is_finite(),
        "this knoll must close the works on the right"
    );
    for (j, &o) in offs.iter().enumerate() {
        let side: i8 = if o < 0.0 { -1 } else { 1 };
        let limit = limits.get(side);
        if o != 0.0 && o.abs() < limit {
            // Inside the works Model B is Model A's surface.
            assert_eq!(grid[0][j], model_a[0][j], "offset {o} is inside the works");
        }
        if o != 0.0 && o.abs() > limit + 1.0 {
            // The grid limit is quantised to the rake step (1 m here): past it, no cut.
            assert_eq!(
                grid[0][j],
                f64::INFINITY,
                "offset {o} lies beyond the works"
            );
        }
    }
}
