//! Ticket 30: explicit and recursive weir placement agree on a shared case.
//!
//! `pools` cases `weirs_shared_recursion` and `weirs_shared_explicit` place a cascade on one bed
//! profile, the second from the first's weirs (chainage, crest as the pool level, height H). Run
//! through the Rust core, both give the same weirs and the same water levels and grades.
//! (`bed_m` is not compared: the recursion reports the level the bed rose to, the explicit
//! placement samples the smoothed bed at the next station.)
//!
//! Label: property

use serde_json::Value;
use std::path::Path;

fn case(id: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/weir_inputs.json");
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("pools cases")).expect("json");
    let c = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no case {id}"));
    let out = owd_format::eval("pools", &c["input"]).expect("eval");
    out["ok"].clone()
}

/// Floats agree to a relative 1e-9 (absolute 1e-9 near zero); everything else exactly.
fn compare(a: &Value, b: &Value) -> Result<(), Vec<String>> {
    fn walk(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) if x.is_f64() || y.is_f64() => {
                let (x, y) = (
                    x.as_f64().unwrap_or(f64::NAN),
                    y.as_f64().unwrap_or(f64::NAN),
                );
                if x != y && (x - y).abs() > (1e-9 * x.abs().max(y.abs())).max(1e-9) {
                    out.push(format!("{path}: {x} vs {y}"));
                }
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    walk(p, q, &format!("{path}[{i}]"), out);
                }
            }
            (Value::Object(x), Value::Object(y)) if x.len() == y.len() => {
                for (k, p) in x {
                    match y.get(k) {
                        Some(q) => walk(p, q, &format!("{path}.{k}"), out),
                        None => out.push(format!("{path}.{k}: missing")),
                    }
                }
            }
            _ if a == b => {}
            _ => out.push(format!("{path}: {a} vs {b}")),
        }
    }
    let mut out = Vec::new();
    walk(a, b, "$", &mut out);
    if out.is_empty() { Ok(()) } else { Err(out) }
}

/// The weirs without `bed_m` (see the module notes).
fn without_bed(weirs: &Value) -> Value {
    let mut w = weirs.clone();
    for x in w.as_array_mut().expect("a list of weirs") {
        x.as_object_mut().expect("a weir").remove("bed_m");
    }
    w
}

#[test]
fn explicit_and_recursive_weirs_agree_on_the_shared_profile() {
    // The explicit cascade is the recursion's, so the two placements agree to
    // the 1e-9 tolerance (a last-bit difference in a chainage is allowed, as everywhere).
    let rec = case("place_weirs/weirs_shared_recursion");
    let exp = case("place_weirs/weirs_shared_explicit");
    let n = rec.as_array().map_or(0, Vec::len);
    assert!(n >= 3, "the shared case should be a cascade, got {n} weirs");
    if let Err(d) = compare(&without_bed(&rec), &without_bed(&exp)) {
        panic!("the weirs differ:\n{}", d.join("\n"));
    }
    let lr = case("water_levels/levels_shared_recursion");
    let le = case("water_levels/levels_shared_explicit");
    if let Err(d) = compare(&lr, &le) {
        panic!("water levels and grades differ:\n{}", d.join("\n"));
    }
}
