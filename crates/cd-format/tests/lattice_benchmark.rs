//! Benchmark of `optimise_pools` through the parity seam (run with
//! `cargo test --release -p cd-format --test lattice_benchmark -- --ignored --nocapture`). The same
//! two problems are timed against the frozen Python in the PR that adds this file.
//!
//! Label: property. No fixed number: it prints times, and checks only that pools were replaced.

use serde_json::{Value, json};
use std::path::Path;
use std::time::Instant;

fn pools_case() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/cases/lattice.json");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "optimise_pools/pools_two_pools")
        .expect("the parity example")["input"]
        .clone()
}

fn time(label: &str, input: &Value) {
    let t = Instant::now();
    let out = cd_format::eval("lattice", input).unwrap();
    let ok = &out["ok"];
    println!(
        "rust {label}: {:.0} ms, {} pools, {} replaced",
        t.elapsed().as_secs_f64() * 1e3,
        ok["pools"].as_array().unwrap().len(),
        ok["n_replaced"]
    );
    assert!(ok["n_replaced"].as_u64().unwrap() >= 1);
}

#[test]
#[ignore = "benchmark: run on request"]
fn optimise_pools_on_the_parity_example_and_on_a_default_grid() {
    let example = pools_case();
    time("parity example (6 m cells, 120 m corridor)", &example);
    // The same problem on the package's own defaults: 2 m cells and a 350 m corridor.
    let mut finer = example.clone();
    finer["args"]["cell_m"] = json!(2.0);
    finer["args"]["corridor_half_m"] = json!(350.0);
    time(
        "same problem, default grid (2 m cells, 350 m corridor)",
        &finer,
    );
}
