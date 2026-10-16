//! Ticket 30: explicit and recursive weir placement agree on a shared case.
//!
//! `pools` cases `weirs_shared_recursion` and `weirs_shared_explicit` place a cascade on one bed
//! profile, the second from the first's weirs (chainage, crest as the pool level, height H). Run
//! through the Rust core, both give the same weirs and the same water levels and grades.
//! (`bed_m` is not compared: the recursion reports the level the bed rose to, the explicit
//! placement samples the smoothed bed at the next station.)
//!
//! Label: property

use cd_format::parity::{Tolerance, compare};
use serde_json::Value;
use std::path::Path;

fn case(id: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/cases/pools.json");
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("pools cases")).expect("json");
    let c = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no case {id}"));
    let out = cd_format::eval("pools", &c["input"]).expect("eval");
    out["ok"].clone()
}

fn tolerance() -> Tolerance {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/tolerance.json");
    Tolerance::from_json(&std::fs::read_to_string(path).expect("tolerance.json"))
        .expect("tolerance")
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
    // The explicit cascade is the frozen Python's recursion, so the two Rust placements agree to
    // the parity tolerance (a last-bit difference in a chainage is allowed, as everywhere).
    let tol = tolerance();
    let rec = case("place_weirs/weirs_shared_recursion");
    let exp = case("place_weirs/weirs_shared_explicit");
    let n = rec.as_array().map_or(0, Vec::len);
    assert!(n >= 3, "the shared case should be a cascade, got {n} weirs");
    if let Err(d) = compare(&without_bed(&rec), &without_bed(&exp), tol) {
        panic!("the weirs differ:\n{}", d.join("\n"));
    }
    let lr = case("water_levels/levels_shared_recursion");
    let le = case("water_levels/levels_shared_explicit");
    if let Err(d) = compare(&lr, &le, tol) {
        panic!("water levels and grades differ:\n{}", d.join("\n"));
    }
}
