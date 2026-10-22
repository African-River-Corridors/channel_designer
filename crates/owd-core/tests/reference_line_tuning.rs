//! Label: tuning harness (spec 0016 T1), not a check. It sweeps the reference line's pull towards
//! the seed on windows of a study and prints, for each weight, how smooth and how central the line
//! is. Ignored: run it by hand on a study of your own.
//!
//! ```text
//! OWD_TUNE_STUDY=study.owd OWD_TUNE_WINDOWS=16-36,72-84 \
//!   cargo test -p owd-core --test reference_line_tuning -- --ignored --nocapture
//! ```
//!
//! Each window is a range of kilometres along the study's centreline. `OWD_TUNE_WEIGHTS` (comma
//! separated) sets the weights.

use owd_core::alignment::fairway_fit::{Fixed, fairway_fit, max_curvature_step};
use owd_core::alignment::reference_line::{refit_params_for_tuning, seed_mid_line};
use owd_core::geom::linear::{ProjectIndex, length, point_line_distance};
use owd_core::geom::{Polygon, Pt};
use std::time::Instant;

fn pts(v: &serde_json::Value) -> Vec<Pt> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

/// The points of `bank` whose chainage on `centre` lies in `[a, b]` (m).
fn clip(bank: &[Pt], centre: &ProjectIndex, a: f64, b: f64) -> Vec<Pt> {
    bank.iter()
        .copied()
        .filter(|p| {
            let s = centre.project(*p);
            a <= s && s <= b
        })
        .collect()
}

/// Signed curvature every 10 m through points 20 m either side (three-point circle).
fn curvatures(line: &[Pt]) -> Vec<f64> {
    let n = (length(line) / 10.0) as usize;
    let at = |s: f64| owd_core::geom::linear::interpolate(line, s).unwrap();
    (2..n.saturating_sub(2))
        .map(|i| {
            let s = 10.0 * i as f64;
            let (a, b, c) = (at(s - 20.0), at(s), at(s + 20.0));
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
            let d = |p: Pt, q: Pt| libm::hypot(p[0] - q[0], p[1] - q[1]);
            let den = d(a, b) * d(b, c) * d(a, c);
            if den > 0.0 {
                2.0 * (u[0] * v[1] - u[1] * v[0]) / den
            } else {
                0.0
            }
        })
        .collect()
}

/// (sign changes of curvature per km, smallest radius, mean off-centre share, smallest bank gap).
/// The off-centre share is |d_left − d_right| / (d_left + d_right): 0 on the mid-line, 1 at a bank.
fn metrics(line: &[Pt], left: &[Pt], right: &[Pt]) -> (f64, f64, f64, f64) {
    let k = curvatures(line);
    let flips = k.windows(2).filter(|w| w[0] * w[1] < 0.0).count() as f64;
    let km = length(line) / 1000.0;
    let rmin = 1.0 / k.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    let n = (length(line) / 50.0) as usize;
    let (mut share, mut gap) = (0.0, f64::MAX);
    for i in 0..=n {
        let p = owd_core::geom::linear::interpolate(line, 50.0 * i as f64).unwrap();
        let (dl, dr) = (point_line_distance(p, left), point_line_distance(p, right));
        share += (dl - dr).abs() / (dl + dr);
        gap = gap.min(dl.min(dr));
    }
    (flips / km, rmin, share / (n + 1) as f64, gap)
}

#[test]
#[ignore]
fn sweep_the_pull_weight() {
    let Ok(path) = std::env::var("OWD_TUNE_STUDY") else {
        eprintln!("set OWD_TUNE_STUDY to a study file");
        return;
    };
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let inp = &doc["inputs"];
    let (left, right, centre) = (
        pts(&inp["left_bank"]["coordinates"]),
        pts(&inp["right_bank"]["coordinates"]),
        pts(&inp["centreline"]["coordinates"]),
    );
    let rings: Vec<Vec<Pt>> = inp["river_polygon"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(pts)
        .collect();
    let water = vec![Polygon {
        exterior: rings[0].clone(),
        holes: rings[1..].to_vec(),
    }];
    let ci = ProjectIndex::new(&centre);
    let windows = std::env::var("OWD_TUNE_WINDOWS").unwrap_or("16-36".into());
    let weights: Vec<f64> = std::env::var("OWD_TUNE_WEIGHTS")
        .unwrap_or("0,0.01,0.1,0.3,1,3,10,100".into())
        .split(',')
        .map(|w| w.parse().unwrap())
        .collect();
    for w in windows.split(',') {
        let (a, b) = w.split_once('-').unwrap();
        let (a, b): (f64, f64) = (
            a.parse::<f64>().unwrap() * 1e3,
            b.parse::<f64>().unwrap() * 1e3,
        );
        let (l, r) = (clip(&left, &ci, a, b), clip(&right, &ci, a, b));
        let seed = seed_mid_line(&l, &r).unwrap();
        let (f, rm, sh, g) = metrics(&seed, &l, &r);
        println!(
            "window {w} km: seed  flips/km {f:5.2}  R_min {rm:7.0} m  off-centre {sh:.3}  bank gap {g:6.1} m"
        );
        for &wt in &weights {
            let mut p = refit_params_for_tuning(&seed, &l, &r);
            p.seed_pull_weight = wt;
            let t = Instant::now();
            let fit = fairway_fit(&seed, &water, &Fixed::default(), &p).unwrap();
            let (f, rm, sh, g) = metrics(&fit.line, &l, &r);
            let jump = max_curvature_step(&fit.line, 20.0, 10.0);
            println!(
                "  pull {wt:6}: flips/km {f:5.2}  R_min {rm:7.0} m  off-centre {sh:.3}  bank gap {g:6.1} m  dκ {jump:.5}  {:.1} s",
                t.elapsed().as_secs_f64()
            );
        }
    }
}
