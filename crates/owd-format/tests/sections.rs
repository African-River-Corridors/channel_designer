//! Label: property
//! The long section and the cross-section on the worked example (spec 0003, ticket 72): what is
//! drawn is the station walk the volumes stage sums.
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, Session, Stage, Terrain};
use owd_format::geotiff::Dem;
use serde_json::Value;

fn dem() -> Dem {
    let (w, h) = (80usize, 12usize);
    let data = (0..w * h)
        .map(|i| {
            40.0 + 6.0 * libm::sin((i % w) as f64 / 6.0) + 4.0 * libm::cos((i / w) as f64 / 2.0)
        })
        .collect();
    Dem {
        width: w,
        height: h,
        data,
        geotransform: [100.0, 0.0, 499_800.0, 0.0, -100.0, 600_600.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    }
}

fn after_volumes(dem: &Dem) -> Session {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.owd"),
    )
    .unwrap();
    let mut s = Session::open(&text).unwrap();
    s.set_dem("dem.tif", b"dem", dem);
    s.set_water_profile(&[[0.0, 38.0]]).unwrap();
    let t = dem_sampler(dem, Some(32630)).unwrap();
    let ter: Terrain = &t;
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.run_stage_with(Stage::Volumes, Some(ter)).unwrap();
    s
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn the_long_section_sums_to_the_volumes_the_table_shows() {
    let d = dem();
    let s = after_volumes(&d);
    let t = dem_sampler(&d, Some(32630)).unwrap();
    let walk = s
        .option_section_walk("rolling-circle", &t as Terrain)
        .unwrap();
    let ls = s.long_section("rolling-circle", &walk).unwrap();
    let row = &s.document()["stages"]["volumes"]["results"]["options"][0];
    let n = ls["chainage_m"].as_array().unwrap().len();
    assert_eq!(n as u64, row["stations"].as_u64().unwrap());
    let sum = |k: &str| ls[k].as_array().unwrap().iter().map(f).sum::<f64>() * f(&ls["station_m"]);
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b.abs().max(1.0);
    assert!(close(sum("dredge_area_m2"), f(&row["dredge_m3"])), "dredge");
    assert!(close(sum("excav_area_m2"), f(&row["excav_m3"])), "dry cut");
    // The cut depth is the bed above the design invert, never negative.
    for i in 0..n {
        let (bed, inv, cut) = (
            f(&ls["bed_m"][i]),
            f(&ls["design_invert_m"][i]),
            f(&ls["cut_depth_m"][i]),
        );
        assert!(cut >= 0.0 && (cut - (bed - inv).max(0.0)).abs() < 1e-12);
    }
    assert_eq!(ls["positions"].as_array().unwrap().len(), n);
    assert_eq!(ls["screening"], "Screening — not for construction");
}

#[test]
fn a_cross_section_is_the_walk_s_station_and_its_cut_agrees_with_the_long_section() {
    let d = dem();
    let s = after_volumes(&d);
    let t = dem_sampler(&d, Some(32630)).unwrap();
    let walk = s
        .option_section_walk("rolling-circle", &t as Terrain)
        .unwrap();
    let ls = s.long_section("rolling-circle", &walk).unwrap();
    // Ask between two stations: the nearest one answers, and says which.
    let at = f(&ls["chainage_m"][10]) + 1.0;
    let cs = s.cross_section(&walk, at).unwrap();
    assert_eq!(cs["chainage_m"], ls["chainage_m"][10]);
    assert_eq!(cs["dredge_area_m2"], ls["dredge_area_m2"][10]);
    assert_eq!(cs["design_invert_m"], ls["design_invert_m"][10]);
    assert_eq!(cs["position"], ls["positions"][10]);
    let n = cs["offsets_m"].as_array().unwrap().len();
    assert_eq!(cs["terrain_m"].as_array().unwrap().len(), n);
    assert_eq!(cs["cut_face_m"].as_array().unwrap().len(), n);
    assert_eq!(cs["wet"].as_array().unwrap().len(), n);
}

#[test]
fn a_point_picked_on_the_map_gives_the_nearest_station() {
    let d = dem();
    let s = after_volumes(&d);
    let t = dem_sampler(&d, Some(32630)).unwrap();
    let walk = s
        .option_section_walk("rolling-circle", &t as Terrain)
        .unwrap();
    let ls = s.long_section("rolling-circle", &walk).unwrap();
    let p = &ls["positions"][25];
    let got = s.station_near(&walk, f(&p[0]), f(&p[1])).unwrap();
    assert_eq!(got, f(&ls["chainage_m"][25]));
}

#[test]
fn the_sections_need_fresh_volumes_and_a_known_option() {
    let d = dem();
    let mut s = after_volumes(&d);
    let t = dem_sampler(&d, Some(32630)).unwrap();
    assert!(
        s.option_section_walk("nope", &t as Terrain)
            .unwrap_err()
            .to_string()
            .contains("no option `nope`")
    );
    s.set_water_profile(&[[0.0, 37.0]]).unwrap();
    let e = s
        .option_section_walk("rolling-circle", &t as Terrain)
        .unwrap_err()
        .to_string();
    assert!(e.contains("stale"), "{e}");
}
