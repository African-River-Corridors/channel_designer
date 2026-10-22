//! `owd qa-overlay` on the recorded Sassandra window (spec 0013, tickets T5 and T6): the banks seeded
//! from Sentinel-2, drawn over the scene they came from, as a PNG and a KMZ.
//!
//! Label: property

use owd_cli::qa_overlay::{OverlayFlags, qa_overlay_with};
use owd_format::design::Session;
use owd_reach::banks::BankParams;
use owd_reach::edgefit::SceneQuery;
use owd_reach::seed::{SeedParams, banks_from_s2, record_s2_banks};
use owd_reach::sentinel2::{SEARCH_ENDPOINT, item_url};
use owd_reach::{DroppedFiles, ReplayFetch};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn ex(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/reach").join(name)).unwrap()
}

/// The search, each scene by its id, and the band windows of both scenes.
fn fetch() -> ReplayFetch {
    let stac: Value = serde_json::from_slice(&ex("stac_search_s2win.json")).unwrap();
    let mut f = ReplayFetch::new().with_response_containing("earth-search.aws.element84.com/v1/search", ex("stac_search_s2win.json"));
    for item in stac["features"].as_array().unwrap() {
        let id = item["id"].as_str().unwrap();
        f = f.with_response(&item_url(SEARCH_ENDPOINT, id), item.to_string().into_bytes());
        let date = item["properties"]["datetime"].as_str().unwrap()[..10].replace('-', "");
        for (key, band) in [("blue", "B02"), ("green", "B03"), ("red", "B04"), ("nir", "B08"), ("swir16", "B11"), ("scl", "SCL")] {
            f = f.with_response(item["assets"][key]["href"].as_str().unwrap(), ex(&format!("s2win_{date}_{band}.tif")));
        }
    }
    f
}

fn work_dir() -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("owd-qa-overlay");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_overlay_of_the_seeded_window_is_drawn_on_its_scene_as_a_png_and_a_kmz() {
    let f = fetch();
    let jrc = DroppedFiles::new().with_file("occurrence_10W_10Nv1_4_2021.tif", ex("jrc_occurrence_s2win.tif"));
    let trace: Value = serde_json::from_slice(&ex("s2win_trace.json")).unwrap();
    let route: Vec<[f64; 2]> = serde_json::from_value(trace["trace"]["coordinates"].clone()).unwrap();
    let q = SceneQuery {
        endpoint: SEARCH_ENDPOINT.to_string(),
        from: "2024-12-01".into(),
        to: "2025-03-01".into(),
        months: vec![12, 1, 2],
        max_cloud_pct: 20.0,
    };
    let params = BankParams { threshold_pct: 30.0, ..BankParams::default() };
    let b = banks_from_s2(&f, Some(&jrc), &route, &params, &q, &SeedParams::default()).unwrap();
    let mut s = Session::new("Sassandra km 148 to 152", "2026-10-22");
    record_s2_banks(&mut s, &b).unwrap();
    let dir = work_dir();
    let doc = dir.join("window.owd");
    std::fs::write(&doc, s.to_text()).unwrap();
    let (png, kmz) = (dir.join("window.png"), dir.join("window.kmz"));
    let flags = OverlayFlags {
        doc: doc.to_str().unwrap().into(),
        png: png.to_str().unwrap().into(),
        kmz: Some(kmz.to_str().unwrap().into()),
        from_km: None,
        to_km: None,
        scene: Vec::new(),
        stac: None,
        json: true,
    };
    if let Err(e) = qa_overlay_with(flags, &f) {
        panic!("{}", e.message);
    }
    // The PNG decodes; the map is at least the window at 10 m (upscaled up to 4 times) plus the legend.
    let p = png::Decoder::new(std::io::Cursor::new(std::fs::read(&png).unwrap())).read_info().unwrap();
    assert!(p.info().width >= 300 && p.info().height >= 300, "{} x {}", p.info().width, p.info().height);
    // The KMZ holds the KML and the image, the scene's notice in the KML.
    let bytes = std::fs::read(&kmz).unwrap();
    let url = "file:///window.kmz";
    let r = ReplayFetch::new().with_response(url, bytes.clone());
    let z = owd_reach::zip::RemoteZip::open(&r, url, bytes.len() as u64).unwrap();
    let kml = String::from_utf8(z.read(&r, z.member("doc.kml").unwrap()).unwrap()).unwrap();
    assert!(kml.contains("Contains modified Copernicus Sentinel data 2025"), "{kml}");
    assert!(kml.contains("Left bank") && kml.contains("Right bank") && kml.contains("Centreline"));
    assert!(z.member("overlay.png").is_some());
    // A copy for the build notes and the pull request.
    if let Ok(out) = std::env::var("OWD_QA_OVERLAY_OUT") {
        std::fs::copy(&png, Path::new(&out).join("s2win-qa-overlay.png")).unwrap();
        std::fs::copy(&kmz, Path::new(&out).join("s2win-qa-overlay.kmz")).unwrap();
    }
}
