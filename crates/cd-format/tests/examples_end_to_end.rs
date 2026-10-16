//! End-to-end parity: the two worked examples, `examples/quickstart.py` and
//! `examples/optimise_pools.py`, rebuilt step by step through the Rust core and held to what the
//! frozen Python prints (`parity/examples/reference.json`, written by `python -m parity.examples`).
//!
//! Each Python step is one call through the parity seam (`cd_format::eval`), chained the way the
//! example chains its objects. Every printed line is rebuilt as text, with Python's number
//! formatting, and compared exactly; every design line is compared vertex by vertex, and every
//! volume within the ADR 0005 tolerances.
//!
//! Needs no Python: it reads the committed reference only.
//!
//! Label: snapshot. The reference is what the frozen Python printed. Never hand-edit it.

use cd_core::pyfmt::{comma_fixed, fixed};
use cd_format::parity::{Tolerance, compare};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity")
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}")))
        .unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn tolerance() -> Tolerance {
    let text = std::fs::read_to_string(parity_dir().join("tolerance.json")).unwrap();
    Tolerance::from_json(&text).unwrap()
}

/// One call through the seam; a Python refusal or a seam error fails the test with its text.
fn call(entry_point: &str, function: &str, args: Value) -> Value {
    let out = cd_format::eval(entry_point, &json!({ "fn": function, "args": args }))
        .unwrap_or_else(|e| panic!("{entry_point}.{function}: {e}"));
    match out.get("ok") {
        Some(v) => v.clone(),
        None => panic!("{entry_point}.{function} refused: {out}"),
    }
}

fn f(v: &Value) -> f64 {
    match v {
        Value::String(s) if s == "float:inf" => f64::INFINITY,
        _ => v.as_f64().unwrap_or_else(|| panic!("not a number: {v}")),
    }
}

/// `f"{x:>w,.0f}"` and `f"{x:w,.0f}"` (numbers align right by default).
fn right_comma(x: f64, width: usize) -> String {
    format!("{:>width$}", comma_fixed(x, 0))
}

fn expect_close(what: &str, expected: &Value, actual: &Value) {
    if let Err(diffs) = compare(expected, actual, tolerance()) {
        panic!("{what}: {}", diffs.join("; "));
    }
}

fn volumes_check(name: &str, reference: &Value, v: &Value) {
    let got = json!({
        "dredge_m3": v["dredge_m3"], "excav_m3": v["excav_m3"], "total_m3": v["total_m3"],
        "max_face_h_m": v["max_face_h_m"], "n_stations": v["stations"].as_array().unwrap().len(),
    });
    expect_close(&format!("volumes of {name}"), reference, &got);
}

fn line_coords(geom: &Value) -> Value {
    geom["coords"].clone()
}

fn same_line(name: &str, reference: &Value, coords: &Value) {
    expect_close(&format!("design line {name}"), reference, coords);
}

#[test]
fn quickstart_matches_the_frozen_python() {
    let reference = read(&parity_dir().join("examples/reference.json"))["quickstart"].clone();
    let river = call("synthetic", "synthetic_river", json!({}))["river"].clone();
    let (centreline, polygon) = (river["centreline"].clone(), river["polygon"].clone());

    // 2. Rules.
    let shape = call(
        "rulesets",
        "channel_shape",
        json!({"ruleset_id": "PIANC-B", "vessel_standard_id": "CEMT_IV", "draught_m": 2.5}),
    );
    let (bottom, r_min, depth) = (
        f(&shape["bottom_width_straight_m"]),
        f(&shape["min_bend_radius_m"]),
        f(&shape["design_depth_m"]),
    );
    let mut stdout = vec![format!(
        "{} x {}: bottom {} m, R_min {} m, depth {} m",
        shape["ruleset_id"].as_str().unwrap(),
        shape["vessel_standard_id"].as_str().unwrap(),
        fixed(bottom, 1),
        fixed(r_min, 0),
        fixed(depth, 2)
    )];
    expect_close(
        "shape",
        &reference["shape"],
        &json!({
            "bottom_width_straight_m": bottom, "min_bend_radius_m": r_min, "design_depth_m": depth,
        }),
    );

    // 3. Alignment: the bends tighter than R_min, then straightened.
    let tight = call(
        "meanders",
        "find_meanders",
        json!({"centreline": centreline, "max_radius": r_min}),
    );
    let rolled = call(
        "rolling_circle",
        "rolling_circle",
        json!({"centreline": centreline, "min_radius": r_min, "step": 10.0}),
    );
    let rolled_line = rolled["result"]["centreline"].clone();
    let n_arcs = rolled["n_arcs"].as_u64().unwrap();
    stdout.push(format!(
        "{} bends tighter than R_min; {} arcs spliced in",
        tight.as_array().unwrap().len(),
        n_arcs
    ));
    assert_eq!(n_arcs, reference["n_arcs"].as_u64().unwrap());

    // 4. Least-cost route.
    let route = call(
        "route",
        "optimise_route",
        json!({
            "line": rolled_line, "terrain": {"kind": "synthetic_river"}, "river_polygon": polygon,
            "grade": {"kind": "water_level_minus", "depth": depth},
            "bottom_width": bottom, "depth": depth, "side_slope": 3.0, "r_min": r_min,
            "corridor_sweep": [40, 80, 120],
        }),
    );
    let route_line = route["line"].clone();

    // 5. Volumes under Cut Model B, with bend widening from the ruleset.
    let hb = json!({"kind": "widened_from_shape", "ruleset_id": "PIANC-B", "vessel_standard_id": "CEMT_IV"});
    for (name, line) in [
        ("rolling circle", &rolled_line),
        ("least-cost", &route_line),
    ] {
        let v = call(
            "sections",
            "section_volumes",
            json!({
                "line": line, "terrain": {"kind": "synthetic_river"}, "river_polygon": polygon,
                "wl": {"kind": "synthetic_water_level"}, "depth": depth, "hb": hb,
                "model": "b", "maxw_m": 150.0,
            }),
        );
        volumes_check(name, &reference["volumes"][name], &v);
        stdout.push(format!(
            "{name:>14}: dredge {} m3   dry cut {} m3   tallest face {} m",
            right_comma(f(&v["dredge_m3"]), 9),
            right_comma(f(&v["excav_m3"]), 9),
            format!("{:>4}", fixed(f(&v["max_face_h_m"]), 1)),
        ));
    }
    same_line(
        "rolling circle",
        &reference["lines"]["rolling circle"],
        &line_coords(&rolled_line),
    );
    same_line(
        "least-cost",
        &reference["lines"]["least-cost"],
        &line_coords(&route_line),
    );
    assert_eq!(
        json!(stdout),
        reference["stdout"],
        "the printed lines differ"
    );
}

#[test]
fn optimise_pools_matches_the_frozen_python() {
    let reference = read(&parity_dir().join("examples/reference.json"))["optimise_pools"].clone();
    let params = json!({
        "length_m": 3000.0, "wavelength_m": 2400.0, "amplitude_m": 120.0,
        "hill_at": [1500.0, -60.0], "hill_height_m": 10.0, "hill_radius_m": 80.0,
    });
    let river = call("synthetic", "synthetic_river", params.clone())["river"].clone();
    let polygon = river["polygon"].clone();
    let terrain = json!({"kind": "synthetic_river", "params": params});
    let line = json!({"geom": "LineString", "coords": [[0.0, 0.0], [3000.0, 0.0]]});
    let (depth, half_width, r_min) = (3.25, 15.0, 255.0);

    let cfg = json!({
        "candidate_sites_m": [500, 1000, 1500, 2000, 2500], "lifts_m": [0.5, 1.0],
        "terminal_m": 3000.0, "terminal_min_level_m": 0.5, "min_spacing_m": 1000.0,
    });
    let levels = call("cascade", "cascade_levels", json!({"cfg": cfg}));
    let table = call(
        "cascade",
        "build_level_table",
        json!({
            "line": line, "terrain": terrain, "river_polygon": polygon, "levels_m": levels,
            "depth_m": depth, "hb": half_width, "model": "b", "maxw_m": 120.0,
        }),
    );
    let search = call(
        "cascade",
        "search_cascade",
        json!({
            "cfg": cfg,
            "reach": {"kind": "earthworks", "table": table, "wet_weight": 1.0, "dry_weight": 1.5},
            "structure": {"kind": "power", "fixed": 1.0e5, "coeff": 2.0e5, "power": 1.0},
        }),
    );
    let best = search["best"].clone();
    let pools = search["cascades_pools"][0].clone();
    let weirs: Vec<(f64, f64)> = best["weirs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (f(&w["chainage_m"]), f(&w["pool_level_m"])))
        .collect();
    expect_close(
        "weirs",
        &reference["weirs"],
        &Value::Array(
            weirs
                .iter()
                .map(|&(c, l)| json!({"chainage_m": c, "pool_level_m": l}))
                .collect(),
        ),
    );
    expect_close("pools", &reference["pools"], &pools);

    let out = call(
        "lattice",
        "optimise_pools",
        json!({
            "baseline": line, "terrain": terrain, "river_polygon": polygon, "pools": pools,
            "depth_m": depth,
            "section": {"half_width": half_width, "side_slope_h_per_v": 3.0, "dry_weight": 1.5,
                        "rake_half_m": 80.0, "rake_step_m": 5.0, "ds_step_m": 10.0},
            "cfg": {"r_min_m": r_min, "pitch_m": 20.0, "n_heading": 24, "radius_multiples": [1, 2, 4]},
            "corridor_half_m": 180.0, "cell_m": 5.0,
        }),
    );
    assert_eq!(out["n_replaced"], reference["n_replaced"]);
    let optimised = out["line"].clone();

    let mut stdout = vec![format!(
        "weirs: {}",
        weirs
            .iter()
            .map(|&(c, l)| format!("{} m -> pool {} m", fixed(c, 0), fixed(l, 1)))
            .collect::<Vec<_>>()
            .join(", ")
    )];
    for (name, l) in [("straight", &line), ("optimised", &optimised)] {
        let v = call(
            "sections",
            "section_volumes",
            json!({
                "line": l, "terrain": terrain, "river_polygon": polygon,
                "wl": {"kind": "pool_levels", "line": line, "pools": pools},
                "depth": depth, "hb": half_width, "model": "b", "maxw_m": 120.0,
            }),
        );
        volumes_check(name, &reference["volumes"][name], &v);
        stdout.push(format!(
            "{name:>10}: dredge {} m3   dry cut {} m3",
            right_comma(f(&v["dredge_m3"]), 9),
            right_comma(f(&v["excav_m3"]), 9)
        ));
    }
    same_line(
        "optimised",
        &reference["lines"]["optimised"],
        &line_coords(&optimised),
    );
    assert_eq!(
        json!(stdout),
        reference["stdout"],
        "the printed lines differ"
    );
}
