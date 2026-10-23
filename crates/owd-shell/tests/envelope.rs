//! Spec 0016 T4: the app's "Largest vessel" panel runs the sweep through the shell, and the map gets
//! the per-kilometre strip in WGS84, cut from the plan line.
//!
//! Label: property

use owd_format::design::Role;
use owd_shell::state::AppState;
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A straight river 3.2 km long and 400 m wide, in UTM 30N, with both banks.
fn study() -> AppState {
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let mut s = AppState::new_design("t", "2026-10-23");
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!([
            at(0.0, -200.0),
            at(3200.0, -200.0),
            at(3200.0, 200.0),
            at(0.0, 200.0),
            at(0.0, -200.0)
        ])
    );
    let line = |y: f64| {
        let pts: Vec<[f64; 2]> = (0..=32).map(|i| at(100.0 * f64::from(i), y)).collect();
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(pts)
        )
    };
    s.load_inputs(
        &[
            (Role::RiverPolygon, polygon),
            (Role::LeftBank, line(200.0)),
            (Role::RightBank, line(-200.0)),
            (Role::Centreline, line(0.0)),
        ],
        "2026-10-23",
    )
    .unwrap();
    s
}

#[test]
fn the_largest_vessel_panel_runs_the_sweep_and_draws_the_per_km_strip() {
    let mut s = study();
    assert_eq!(s.snapshot()["envelope_status"], "not_run");
    s.run_envelope().unwrap();
    let snap = s.snapshot();
    assert_eq!(snap["envelope_status"], "fresh");
    let r = &snap["document"]["stages"]["envelope"]["results"];
    let sizes = r["sizes"].as_array().unwrap();
    assert_eq!(sizes.len(), 5, "the default series");
    let per_km = r["per_km"].as_array().unwrap();
    assert!(per_km.len() >= 3, "{}", r["per_km"]);
    let strip = snap["reach_view"]["envelope_strip"].as_array().unwrap();
    assert_eq!(strip.len(), per_km.len());
    for (row, k) in strip.iter().zip(per_km) {
        assert_eq!(row["largest"], k["largest"]["label"]);
        // The rank is the size's place in the series, for the colour: none where no size passes.
        let rank = sizes
            .iter()
            .position(|z| z["label"] == k["largest"]["label"]);
        assert_eq!(row["rank"], rank.map_or(Value::Null, |i| json!(i)), "{row}");
        assert_eq!(row["sizes"], json!(sizes.len()));
        // The km, cut from the plan line, in longitude and latitude (UTM 30N: near 3° W, 5.4° N).
        let line = row["line"].as_array().unwrap();
        assert!(line.len() >= 2, "{row}");
        for p in line {
            let (lon, lat) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
            assert!(
                (-3.1..-2.9).contains(&lon) && (5.3..5.5).contains(&lat),
                "{p}"
            );
        }
    }
    // A change to the river makes the answer stale; nothing reruns it.
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!([
            at(0.0, -150.0),
            at(3200.0, -150.0),
            at(3200.0, 150.0),
            at(0.0, 150.0),
            at(0.0, -150.0)
        ])
    );
    s.load_inputs(&[(Role::RiverPolygon, polygon)], "2026-10-23")
        .unwrap();
    assert_eq!(s.snapshot()["envelope_status"], "stale");
}
