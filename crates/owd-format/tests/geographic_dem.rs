//! A geographic DEM is sampled where it is: each query point in the project's UTM zone is converted to
//! longitude and latitude and sampled bilinearly in the DEM's own grid, with no warping. Held to what
//! rasterio reads from a warped (UTM, 10 m, bilinear) copy of the same file at 150 test points, within
//! the warp's own interpolation error (`tolerance_m`, derived in `tools/make_warp_fixture.py`).
//!
//! Label: snapshot (what rasterio 1.5 printed). Never hand-edit `warp_expected.json`.

use owd_format::design::terrain::dem_sampler;
use owd_format::geotiff::read_file;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dem")
}

#[test]
fn sampling_in_place_matches_a_warped_copy_within_the_warps_interpolation_error() {
    let exp: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("warp_expected.json")).unwrap())
            .unwrap();
    let tol = exp["tolerance_m"].as_f64().unwrap();
    let dem = read_file(&dir().join("f32_geographic.tif")).unwrap();
    assert_eq!(dem.epsg, Some(4326));
    let sample = dem_sampler(&dem, Some(exp["epsg"].as_u64().unwrap() as u32)).unwrap();
    let pts = exp["points"].as_array().unwrap();
    assert!(pts.len() >= 100);
    let (xs, ys): (Vec<f64>, Vec<f64>) = pts
        .iter()
        .map(|p| (p["x"].as_f64().unwrap(), p["y"].as_f64().unwrap()))
        .unzip();
    let got = sample(&xs, &ys);
    let mut worst: f64 = 0.0;
    for (g, p) in got.iter().zip(pts) {
        let want = p["z"].as_f64().unwrap();
        assert!(g.is_finite(), "a point inside the DEM gave no height");
        worst = worst.max((g - want).abs());
    }
    assert!(worst <= tol, "worst difference {worst} m against {tol} m");
    // The comparison can fail: a sampler that is wrong by one pixel is far outside the tolerance.
    let shifted: Vec<f64> = xs.iter().map(|x| x + 30.0).collect();
    let off = sample(&shifted, &ys);
    let off_worst = off
        .iter()
        .zip(pts)
        .filter(|(g, _)| g.is_finite())
        .map(|(g, p)| (g - p["z"].as_f64().unwrap()).abs())
        .fold(0.0, f64::max);
    assert!(
        off_worst > tol,
        "a 30 m shift stays inside the tolerance ({off_worst} m)"
    );
}

#[test]
fn a_dem_in_a_zone_other_than_the_designs_is_refused_naming_both() {
    let dem = read_file(&dir().join("f32_strip_none.tif")).unwrap(); // UTM 30N
    let e = dem_sampler(&dem, Some(32631)).err().unwrap().to_string();
    assert!(e.contains("EPSG:32630") && e.contains("EPSG:32631"), "{e}");
    let merc = read_file(&dir().join("f32_mercator_nodata.tif")).unwrap();
    let e = dem_sampler(&merc, Some(32630)).err().unwrap().to_string();
    assert!(e.contains("EPSG:3857") && e.contains("Convert"), "{e}");
}
