//! "Start from a river" through the app state, every answer recorded: the river arm of the lower
//! Sassandra (a synthetic Overpass way), the JRC window (a dropped tile), the Sentinel-2 scene of
//! 23 December 2023, the SWORD subset and Hydrocron series, and a GLO-30 window as the DEM.
//!
//! Label: property

use owd_reach::ReplayFetch;
use owd_shell::reach::{ReachEndpoints, ReachStep};
use owd_shell::state::AppState;
use serde_json::Value;
use std::path::Path;

fn ex(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/reach")
            .join(name),
    )
    .unwrap()
}

fn endpoints() -> ReachEndpoints {
    ReachEndpoints {
        overpass: "https://overpass.example/api/interpreter".into(),
        stac: "https://stac.example/v1/search".into(),
        sword_record: "https://zenodo.example/record".into(),
        hydrocron: "https://hydrocron.example/timeseries".into(),
    }
}

fn fetch() -> ReplayFetch {
    let s2 = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/29/N/RF/2023/12/S2B_29NRF_20231223_0_L2A";
    let zip = ex("sword_v17c_shp_subset.zip");
    let record = format!(
        r#"{{"files":[{{"key":"SWORD_v17c_shp.zip","size":{},"links":{{"self":"https://zenodo.example/sword.zip"}}}}]}}"#,
        zip.len()
    );
    let mut f = ReplayFetch::new()
        .with_response_containing("overpass.example", ex("synthetic/sassandra_arm.json"))
        .with_response_containing("stac.example", ex("stac_search_sassandra.json"))
        .with_response("https://zenodo.example/record", record)
        .with_response("https://zenodo.example/sword.zip", zip);
    for b in ["B03", "B04", "B08", "B11", "SCL"] {
        f = f.with_response(
            &format!("{s2}/{b}.tif"),
            ex(&format!("s2_sassandra_{b}.tif")),
        );
    }
    for id in [
        "14560100021",
        "14560100033",
        "14560100041",
        "14560100051",
        "14560100061",
    ] {
        f = f.with_response_containing(
            &format!("feature_id={id}&"),
            ex(&format!("hydrocron_{id}.json")),
        );
    }
    f
}

fn step(s: &mut AppState, f: &ReplayFetch, step: ReachStep) {
    let job = s.begin_reach(step).unwrap();
    let job = job
        .run(f, &endpoints(), "2024-03-01")
        .unwrap_or_else(|e| panic!("{step:?}: {e}"));
    s.keep_reach(job).unwrap();
}

#[test]
fn a_reach_becomes_a_study_with_marked_inputs_a_level_a_bed_and_an_answer() {
    let f = fetch();
    let mut s = AppState::new_design("Untitled", "2024-03-01");
    s.reach_add_tile(
        "occurrence_10W_10Nv1_4_2021.tif",
        ex("jrc_occurrence_sassandra_estuary.tif"),
    )
    .unwrap();
    assert!(s.reach_add_tile("dem.tif", vec![1, 2]).is_err());
    s.reach_route(
        &f,
        &endpoints(),
        None,
        [-6.0715, 4.966],
        [-6.07, 4.983],
        &[],
        "2024-03-01",
    )
    .unwrap();
    let snap = s.snapshot();
    let view = &snap["reach_view"];
    assert!(view["length_m"].as_f64().unwrap() > 1_500.0, "{view}");
    assert_eq!(view["jrc_tiles"][0]["given"], true, "{view}");
    assert!(!view["dem_tiles"].as_array().unwrap().is_empty());
    assert_eq!(
        snap["document"]["provenance"]["route"]["source"],
        "osm-overpass"
    );
    assert_eq!(snap["dirty"], true);

    step(&mut s, &f, ReachStep::Banks);
    assert_eq!(
        s.snapshot()["document"]["provenance"]["left_bank"]["source"],
        "jrc-gsw"
    );
    step(&mut s, &f, ReachStep::EdgeFit);
    let snap = s.snapshot();
    for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
        let p = &snap["document"]["provenance"][input];
        assert_eq!(p["source"], "sentinel2-l2a", "{input}");
        assert_eq!(p["mark"], "default");
    }

    // The level: no SWORD reach follows this arm closely, so the DEM gives it.
    s.load_dem(
        Path::new("glo30_sassandra_lower.tif"),
        ex("glo30_sassandra_lower.tif"),
        "2024-03-01",
    )
    .unwrap();
    assert_eq!(
        s.snapshot()["document"]["provenance"]["dem"]["mark"],
        "user"
    );
    step(&mut s, &f, ReachStep::Level { percentile: None });
    let snap = s.snapshot();
    assert!(
        !snap["document"]["inputs"]["water_level"]["points"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let notices: Vec<String> = serde_json::from_value(snap["notices"].clone()).unwrap();
    assert!(
        notices.iter().any(|n| n.contains("OpenStreetMap")),
        "{notices:?}"
    );
    assert!(
        notices.iter().any(|n| n.contains("Sentinel")),
        "{notices:?}"
    );

    s.run_fits().unwrap();
    let snap = s.snapshot();
    assert!(snap["document"]["fits"].is_object());

    assert_eq!(snap["fits_status"], "fresh");
    let stretches: &Value = &snap["reach_view"]["fits_stretches"]["PIANC-B"];
    assert!(
        stretches.as_object().is_some_and(|c| !c.is_empty()),
        "{stretches}"
    );

    s.set_bed(&[[0.0, 3.0]], Some("dry"), "2024-03-01").unwrap();
    assert_eq!(
        s.snapshot()["document"]["provenance"]["bed"]["mark"],
        "user"
    );
    // A step on a design that changed meanwhile is not kept.
    let job = s
        .begin_reach(ReachStep::Banks)
        .unwrap()
        .run(&f, &endpoints(), "2024-03-01")
        .unwrap();
    s.set_bed(&[[0.0, 2.0]], None, "2024-03-01").unwrap();
    assert!(s.keep_reach(job).unwrap_err().contains("changed"));
}

#[test]
fn with_a_bed_model_a_level_and_the_dem_the_answer_is_full() {
    use owd_format::design::Role;
    use owd_format::geotiff::Dem;
    // The straight, 400 m wide river of owd-format's tests/fits_full.rs, with a flat DEM at 12 m.
    const UTM30: &str =
        r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let mut s = AppState::new_design("t", "2026-10-21");
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        serde_json::json!([
            at(0.0, -200.0),
            at(3000.0, -200.0),
            at(3000.0, 200.0),
            at(0.0, 200.0),
            at(0.0, -200.0)
        ])
    );
    let centre = format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        serde_json::json!([at(0.0, 0.0), at(3000.0, 0.0)])
    );
    s.load_inputs(
        &[(Role::RiverPolygon, polygon), (Role::Centreline, centre)],
        "2026-10-21",
    )
    .unwrap();
    s.set_water(10.0, None).unwrap();
    s.run_fits().unwrap();
    assert!(
        s.snapshot()["document"]["fits"]["results"]
            .get("mode")
            .is_none(),
        "plan only"
    );
    s.set_bed(
        &[
            [0.0, 4.0],
            [1100.0, 4.0],
            [1200.0, 1.5],
            [1500.0, 1.5],
            [1600.0, 4.0],
        ],
        Some("dry"),
        "2026-10-21",
    )
    .unwrap();
    let (w, h) = (120, 40);
    let dem = Dem {
        width: w,
        height: h,
        data: vec![12.0; w * h],
        geotransform: [50.0, 0.0, 499_500.0, 0.0, -50.0, 601_000.0],
        nodata: None,
        epsg: Some(32630),
    };
    let bytes = owd_format::design::dem_source::write_geotiff(&dem).unwrap();
    s.load_dem(Path::new("dem.tif"), bytes, "2026-10-21")
        .unwrap();
    s.run_fits().unwrap();
    let r = &s.snapshot()["document"]["fits"]["results"];
    assert_eq!(r["mode"], "full");
    assert!(
        r["rulesets"][0]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["depth_basis"] == "depth assumed, not measured")
    );
}
