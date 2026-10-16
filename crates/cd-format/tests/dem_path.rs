//! Label: property
//! The DEM is identified by its hash: a design saved to another folder, with the DEM path changed to
//! match, keeps its results fresh (spec 0003, ticket 74).
use cd_format::design::terrain::dem_sampler;
use cd_format::design::{AlignMethod, Session, Stage, StageStatus, Terrain};
use cd_format::geotiff::Dem;
use serde_json::Value;

#[test]
fn moving_the_dem_path_keeps_the_stages_fresh_and_a_changed_file_still_makes_them_stale() {
    let (w, h) = (80usize, 12usize);
    let data = (0..w * h)
        .map(|i| 40.0 + libm::sin((i % w) as f64 / 6.0))
        .collect();
    let dem = Dem {
        width: w,
        height: h,
        data,
        geotransform: [100.0, 0.0, 499_800.0, 0.0, -100.0, 600_600.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/synthetic-river.cdesign"),
    )
    .unwrap();
    let mut s = Session::open(&text).unwrap();
    s.set_dem("dem.tif", b"the file", &dem);
    s.set_water_profile(&[[0.0, 38.0]]).unwrap();
    let t = dem_sampler(&dem, Some(32630)).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as Terrain))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    s.set_dem_path("../elsewhere/dem.tif");
    assert_eq!(s.dem_path(), Some("../elsewhere/dem.tif"));
    assert_eq!(
        s.stage_status(Stage::Volumes),
        StageStatus::Fresh,
        "a moved path is not a changed DEM"
    );
    // A different file (another hash) is still a change.
    s.set_dem("dem.tif", b"another file", &dem);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
}
