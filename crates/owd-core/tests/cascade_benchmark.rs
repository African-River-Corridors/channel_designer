//! Benchmark of `search_cascade` on a larger problem than the parity cases (run with
//! `cargo test --release -p owd-core --test cascade_benchmark -- --ignored --nocapture`). The same
//! problem is timed against the frozen Python in the PR that adds this file.
//!
//! Label: property. No fixed number: it prints a time, and only checks that a cascade is found.

use owd_core::optimise::cascade::{CascadeConfig, search_cascade};

#[test]
#[ignore = "benchmark: run on request"]
fn search_cascade_on_forty_sites_and_five_lifts() {
    let sites: Vec<f64> = (1..=40).map(|i| 250.0 * f64::from(i)).collect();
    let mut cfg = CascadeConfig::new(sites, vec![0.5, 1.0, 1.5, 2.0, 2.5], 10_250.0, 8.0);
    cfg.min_spacing_m = 500.0;
    cfg.level_cap_m = Some(10.0);
    let cfg = cfg.validated().unwrap();
    let reach = |a: f64, b: f64, level: f64| -> Result<f64, String> {
        let mid = 0.5 * (a + b);
        let t = (mid - 5000.0) / 1500.0;
        Ok(((b - a) * (100.0 + 400.0 * libm::exp(-(t * t)))) * libm::exp(-0.4 * level))
    };
    let structure = |lift: f64| 3.0e5 + 1.5e5 * libm::pow(lift, 1.5);
    let t = std::time::Instant::now();
    let res = search_cascade(&cfg, &reach, &structure).unwrap();
    println!(
        "rust search_cascade: {:.1} ms, {} states, best {:?}",
        t.elapsed().as_secs_f64() * 1e3,
        res.states_explored,
        res.best().map(|c| c.total_cost)
    );
    assert!(res.best().is_some());
}
