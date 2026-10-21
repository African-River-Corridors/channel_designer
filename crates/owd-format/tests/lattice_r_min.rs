//! Every design line the lattice search returns meets R_min by construction.
//!
//! For every converged result of every `lattice` case (`solve_lattice`, `optimise_pool` and each
//! pool of `optimise_pools`) this checks two things on the line the Rust returns:
//!
//! 1. every primitive in `prim_seq` is a straight or an arc of radius >= the case's `r_min_m`;
//! 2. the line itself, read as points, never turns tighter than `r_min_m`: the circumradius of
//!    every three consecutive vertices is >= `r_min_m` (within a relative 1e-9). Collinear triples have no circle and are skipped.
//!
//! Label: property. No fixed number: the bound is the case's own `r_min_m`.

use serde_json::Value;
use std::path::Path;

fn cases() -> Vec<Value> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lattice_pool_inputs.json");
    let text = std::fs::read_to_string(&path).expect("lattice_pool_inputs.json");
    let doc: Value = serde_json::from_str(&text).expect("lattice cases parse");
    doc["cases"].as_array().expect("cases").clone()
}

fn pt(v: &Value) -> [f64; 2] {
    [v[0].as_f64().expect("x"), v[1].as_f64().expect("y")]
}

/// The circumradius of three points, or `None` when they are collinear.
fn circumradius(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<f64> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-9 {
        return None;
    }
    let (a2, b2, c2) = (
        a[0] * a[0] + a[1] * a[1],
        b[0] * b[0] + b[1] * b[1],
        c[0] * c[0] + c[1] * c[1],
    );
    let ux = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
    let uy = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
    let (dx, dy) = (ux - a[0], uy - a[1]);
    Some((dx * dx + dy * dy).sqrt())
}

/// Check one result object. Returns true when it was a converged result that was checked.
fn check_result(id: &str, result: &Value, r_min: f64) -> bool {
    if result["converged"] != Value::Bool(true) {
        return false;
    }
    let tol = 1e-9;
    for p in result["prim_seq"].as_array().expect("prim_seq") {
        if let Some(r) = p[0].as_f64() {
            assert!(
                r >= r_min * (1.0 - tol),
                "{id}: primitive radius {r} < R_min {r_min}"
            );
        }
    }
    let line: Vec<[f64; 2]> = result["line"]["coords"]
        .as_array()
        .expect("line coords")
        .iter()
        .map(pt)
        .collect();
    for w in line.windows(3) {
        if let Some(r) = circumradius(w[0], w[1], w[2]) {
            assert!(
                r >= r_min * (1.0 - tol),
                "{id}: three vertices turn on radius {r} < R_min {r_min}"
            );
        }
    }
    true
}

#[test]
fn every_returned_lattice_line_meets_r_min() {
    let mut checked = 0;
    for case in cases() {
        let id = case["id"].as_str().expect("id").to_string();
        let input = &case["input"];
        let fn_name = input["fn"].as_str().expect("fn");
        if !matches!(
            fn_name,
            "solve_lattice" | "optimise_pool" | "optimise_pools"
        ) {
            continue;
        }
        let r_min = input["args"]["cfg"]["r_min_m"].as_f64().expect("r_min_m");
        let out = owd_format::eval("lattice", input).expect("eval");
        let Some(ok) = out.get("ok") else {
            continue; // a refusal returns no line
        };
        match fn_name {
            "solve_lattice" => checked += usize::from(check_result(&id, ok, r_min)),
            "optimise_pool" => checked += usize::from(check_result(&id, &ok["result"], r_min)),
            _ => {
                for (n, pool) in ok["pools"].as_array().expect("pools").iter().enumerate() {
                    checked +=
                        usize::from(check_result(&format!("{id}[{n}]"), &pool["result"], r_min));
                }
            }
        }
    }
    assert!(checked >= 10, "only {checked} converged lines were checked");
    println!("R_min held on {checked} returned lines");
}
