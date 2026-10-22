//! The Sassandra acceptance window (spec 0013, ticket T6): km 148 to 152 of the spec 0012 worked
//! example, where the river opens into the Soubre reservoir and the JRC mask saw a 17 m channel in
//! 200 m of water. Recorded fixtures (`examples/reach/s2win_*`, `tools/make_s2_window_fixture.py`):
//! the hand-drawn trace, two dry-season scenes of tile 29NQG, and the JRC window.
//!
//! Label: sourced - the open-water widths in `examples/reach/s2win_widths.json`, measured on the scene
//! across the trace (method in `examples/reach/s2win_README.md`). The other checks are properties.

use owd_format::design::midline::{bank_offsets, midline_offsets};
use owd_reach::banks::{BankParams, StretchKind};
use owd_reach::edgefit::{FitParams, SceneQuery, fit_to_scene};
use owd_reach::seed::{SeedParams, banks_from_s2};
use owd_reach::{DroppedFiles, ReplayFetch};
use serde_json::Value;

const EX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/reach/");

fn file(name: &str) -> Vec<u8> {
    std::fs::read(format!("{EX}{name}")).unwrap()
}

fn fetch() -> ReplayFetch {
    let stac: Value = serde_json::from_slice(&file("stac_search_s2win.json")).unwrap();
    let mut f = ReplayFetch::new().with_response_containing(
        "earth-search.aws.element84.com/v1/search",
        file("stac_search_s2win.json"),
    );
    for item in stac["features"].as_array().unwrap() {
        let date = item["properties"]["datetime"].as_str().unwrap()[..10].replace('-', "");
        for (key, band) in [
            ("blue", "B02"),
            ("green", "B03"),
            ("red", "B04"),
            ("nir", "B08"),
            ("swir16", "B11"),
            ("scl", "SCL"),
        ] {
            let href = item["assets"][key]["href"].as_str().unwrap();
            f = f.with_response(href, file(&format!("s2win_{date}_{band}.tif")));
        }
    }
    f
}

fn jrc() -> DroppedFiles {
    DroppedFiles::new().with_file(
        "occurrence_10W_10Nv1_4_2021.tif",
        file("jrc_occurrence_s2win.tif"),
    )
}

fn trace() -> Vec<[f64; 2]> {
    let v: Value = serde_json::from_slice(&file("s2win_trace.json")).unwrap();
    serde_json::from_value(v["trace"]["coordinates"].clone()).unwrap()
}

fn query() -> SceneQuery {
    SceneQuery {
        endpoint: owd_reach::sentinel2::SEARCH_ENDPOINT.to_string(),
        from: "2024-12-01".into(),
        to: "2025-03-01".into(),
        months: vec![12, 1, 2],
        max_cloud_pct: 20.0,
    }
}

/// The JRC threshold of the worked example (75 % sees almost nothing there).
fn params() -> BankParams {
    BankParams {
        threshold_pct: 30.0,
        ..BankParams::default()
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn percentile(mut v: Vec<f64>, q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The measured open water: median of the transects.
fn measured_median() -> f64 {
    let v: Value = serde_json::from_slice(&file("s2win_widths.json")).unwrap();
    median(
        v["transects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["width_m"].as_f64().unwrap())
            .collect(),
    )
}

/// The three acceptance checks on a pair of banks and their centreline.
fn accept(what: &str, left: &[[f64; 2]], right: &[[f64; 2]], centre: &[[f64; 2]], widths: &[f64]) {
    let want = measured_median();
    let got = median(widths.iter().copied().filter(|w| *w > 0.0).collect());
    assert!(
        (got - want).abs() <= 0.2 * want,
        "{what}: median width {got:.0} m, measured {want:.0} m"
    );
    for (i, o) in bank_offsets(centre, left, right).iter().enumerate() {
        if let (Some(l), Some(r)) = o {
            assert!(
                l > r,
                "{what}: station {i} has both banks on one side ({l:.1}, {r:.1})"
            );
        }
    }
    let off: Vec<f64> = midline_offsets(centre, left, right)
        .into_iter()
        .flatten()
        .collect();
    assert!(
        off.len() * 10 >= centre.len() * 9,
        "{what}: {} of {} stations meet both banks",
        off.len(),
        centre.len()
    );
    let (p95, max) = (
        percentile(off.clone(), 0.95),
        off.iter().copied().fold(0.0, f64::max),
    );
    assert!(
        p95 <= 5.0 && max <= 10.0,
        "{what}: centreline off the mid-line by {p95:.1} m (95th percentile), {max:.1} m at most"
    );
    eprintln!(
        "{what}: median width {got:.0} m (measured {want:.0} m); mid-line offset p95 {p95:.2} m, max {max:.2} m"
    );
}

#[test]
fn the_sentinel_2_seed_finds_the_open_water_of_km_148_to_152() {
    let f = fetch();
    let j = jrc();
    let b = banks_from_s2(
        &f,
        Some(&j),
        &trace(),
        &params(),
        &query(),
        &SeedParams::default(),
    )
    .unwrap();
    // The clearer of the two scenes over the river, every part.
    for p in &b.parts {
        assert_eq!(p.scenes[0].id, "S2B_29NQG_20250205_0_L2A", "{p:?}");
    }
    // At each transect, the seed's width at the nearest station is within a quarter of the measure.
    let v: Value = serde_json::from_slice(&file("s2win_widths.json")).unwrap();
    for t in v["transects"].as_array().unwrap().iter().skip(1) {
        // (The first transect, at the islands below km 148, crosses into a side channel.)
        let (x, y) = (t["x"].as_f64().unwrap(), t["y"].as_f64().unwrap());
        let i = (0..b.banks.centreline.len())
            .min_by(|a, c| {
                let d =
                    |k: usize| (b.banks.centreline[k][0] - x).hypot(b.banks.centreline[k][1] - y);
                d(*a).total_cmp(&d(*c))
            })
            .unwrap();
        let (want, got) = (t["width_m"].as_f64().unwrap(), b.banks.widths[i]);
        assert!(
            (got - want).abs() <= 0.25 * want,
            "at {} m: {got:.0} m, measured {want:.0} m",
            t["chainage_m"]
        );
    }
    accept(
        "seed",
        &b.banks.left,
        &b.banks.right,
        &b.banks.centreline,
        &b.banks.widths,
    );
    // The JRC mask alone, on the same trace and threshold, misses much of this water.
    let jb = owd_reach::banks::banks_from_jrc(&j, &trace(), &params()).unwrap();
    let jm = median(jb.widths.iter().copied().filter(|w| *w > 0.0).collect());
    let want = measured_median();
    assert!(
        (jm - want).abs() > 0.2 * want,
        "the JRC mask alone gives {jm:.0} m, measured {want:.0} m"
    );
}

#[test]
fn the_edge_fit_on_the_seed_keeps_the_acceptance() {
    let f = fetch();
    let j = jrc();
    let b = banks_from_s2(
        &f,
        Some(&j),
        &trace(),
        &params(),
        &query(),
        &SeedParams::default(),
    )
    .unwrap();
    let fit = fit_to_scene(&f, &b.banks, &FitParams::default(), &query()).unwrap();
    accept(
        "edge fit",
        &fit.fit.left,
        &fit.fit.right,
        &fit.fit.centreline,
        &fit.fit.widths,
    );
    // On its own scene the seed is already on the edge: the fit moves it a little, never to the limit
    // over most of the window.
    let limit = fit
        .fit
        .stretches
        .iter()
        .filter(|s| s.kind == StretchKind::AtShiftLimit)
        .map(|s| s.to_m - s.from_m)
        .sum::<f64>();
    let length = owd_format::design::midline::arc(&fit.fit.centreline)
        .last()
        .copied()
        .unwrap();
    assert!(
        limit < 0.25 * length,
        "{limit:.0} m of {length:.0} m at the shift limit"
    );
}
