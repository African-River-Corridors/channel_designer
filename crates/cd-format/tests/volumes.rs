//! The volumes stage and the DEM reference, on the synthetic river given in UTM 30N: the stage's
//! numbers equal `section_volumes` called directly on the same sampler, a changed or missing DEM
//! file marks `volumes` stale, and a stage with no DEM, or no readable one, refuses with a message
//! that names the DEM.
//!
//! Label: property

use cd_core::cut_model::{WIDEN_RMAX, widened_hb_from_shape};
use cd_core::geom::{Polygon, Pt};
use cd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_core::volumes::sections::{HalfWidth, Level, section_volumes};
use cd_format::design::{AlignMethod, DemCheck, DesignError, Role, Session, Stage, StageStatus};
use cd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn river() -> cd_core::synthetic::SyntheticRiver {
    synthetic_river(RiverParams::default()).unwrap()
}

/// A stand-in DEM: the file's bytes (any bytes; the session only hashes them) and a `Dem` value.
fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    }
}

fn terrain(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let r = river();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
        .collect()
}

fn aligned() -> Session {
    let r = river();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let centre: Vec<Pt> = r.centreline.iter().map(shift).collect();
    let mut s = Session::new("River", "2026-10-16");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s
}

const DEM_BYTES: &[u8] = b"pretend this is a GeoTIFF";

fn with_dem() -> Session {
    let mut s = aligned();
    s.set_dem("dem/river.tif", DEM_BYTES, &dem());
    s
}

fn settings() -> Value {
    json!({"water_level_start_m": river().water_level(0.0), "water_surface_slope": 0.0, "cut_model": "b"})
}

#[test]
fn the_volumes_equal_section_volumes_called_directly() {
    let mut s = with_dem();
    s.run_volumes(&settings(), Some(&terrain)).unwrap();
    let rows = s.document()["stages"]["volumes"]["results"]["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 1);
    let line: Vec<Pt> = s.document()["options"][0]["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect();
    let shape = channel_shape(&RulesetRegistry::new(), "PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    let ring: Vec<Pt> = river().polygon.exterior.iter().map(shift).collect();
    let poly = [Polygon {
        exterior: ring,
        holes: vec![],
    }];
    let wl0 = river().water_level(0.0);
    let wl = move |_s: f64| wl0;
    let hb = |xy: &[Pt]| widened_hb_from_shape(xy, &shape, WIDEN_RMAX);
    let direct = section_volumes(
        &line,
        &terrain,
        &poly,
        &Level::AtChainage(&wl),
        shape.design_depth_m,
        &HalfWidth::FromStations(&hb),
        "b",
        3.0,
        25.0,
        200.0,
        1.0,
        None,
        None,
        40,
    )
    .unwrap();
    let r = &rows[0];
    assert_eq!(r["id"], "rolling-circle");
    assert_eq!(r["dredge_m3"].as_f64().unwrap(), direct.dredge_m3());
    assert_eq!(r["excav_m3"].as_f64().unwrap(), direct.excav_m3());
    assert_eq!(r["max_face_h_m"].as_f64().unwrap(), direct.max_face_h_m());
    assert!(direct.dredge_m3() > 0.0 && direct.excav_m3() > 0.0);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    // A failed check stays with the option's numbers.
    assert_eq!(
        r["radius_ok"],
        s.document()["options"][0]["checks"]["radius_ok"]
    );
}

#[test]
fn a_changed_dem_file_warns_on_open_and_marks_volumes_stale() {
    let mut s = with_dem();
    s.run_volumes(&settings(), Some(&terrain)).unwrap();
    let saved = Session::open(&s.to_text()).unwrap();

    assert_eq!(saved.dem_check(Some(DEM_BYTES)), DemCheck::Matches);
    assert_eq!(saved.dem_warning(Some(DEM_BYTES)), None);
    assert_eq!(
        saved.stage_status_with_dem(Stage::Volumes, Some(DEM_BYTES)),
        StageStatus::Fresh
    );

    let other: &[u8] = b"pretend this is another GeoTIFF";
    assert_eq!(saved.dem_check(Some(other)), DemCheck::Changed);
    let w = saved.dem_warning(Some(other)).unwrap();
    assert!(w.contains("dem/river.tif") && w.contains("changed"), "{w}");
    assert_eq!(
        saved.stage_status_with_dem(Stage::Volumes, Some(other)),
        StageStatus::Stale
    );
    // The earlier stages do not depend on the DEM file's bytes.
    assert_eq!(
        saved.stage_status_with_dem(Stage::Alignment, Some(other)),
        StageStatus::Fresh
    );

    // A missing file is a warning and a stale stage too.
    assert_eq!(saved.dem_check(None), DemCheck::Missing);
    assert!(saved.dem_warning(None).unwrap().contains("missing"));
    assert_eq!(
        saved.stage_status_with_dem(Stage::Volumes, None),
        StageStatus::Stale
    );
}

#[test]
fn a_new_dem_reference_marks_volumes_stale() {
    let mut s = with_dem();
    s.run_volumes(&settings(), Some(&terrain)).unwrap();
    s.set_dem("dem/other.tif", b"another", &dem());
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
}

#[test]
fn volumes_refuses_without_a_dem_and_names_the_dem_it_cannot_read() {
    let mut s = aligned();
    let e = s.run_volumes(&settings(), Some(&terrain)).unwrap_err();
    assert!(
        matches!(&e, DesignError::Input(m) if m.contains("needs a DEM")),
        "{e}"
    );
    let mut s = with_dem();
    let e = s.run_volumes(&settings(), None).unwrap_err().to_string();
    assert!(e.contains("dem/river.tif") && e.contains("missing"), "{e}");
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::NotRun);
}

#[test]
fn volumes_needs_a_water_level_and_a_fresh_alignment() {
    let mut s = with_dem();
    let e = s
        .run_volumes(&Value::Null, Some(&terrain))
        .unwrap_err()
        .to_string();
    assert!(e.contains("water_level_start_m"), "{e}");
    // The re-route's water level is the default.
    let mut s = with_dem();
    s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
    assert_eq!(
        s.run_volumes(&settings(), Some(&terrain)).unwrap_err(),
        DesignError::StaleUpstream {
            stage: Stage::Volumes,
            run_first: Stage::Sizing
        }
    );
}

#[test]
fn run_stale_reruns_volumes_when_alignment_changes() {
    let mut s = with_dem();
    s.run_volumes(&settings(), Some(&terrain)).unwrap();
    let before = s.document()["stages"]["volumes"]["results"].clone();
    s.align(AlignMethod::RollingCircle, &json!({"step_m": 20.0}), None)
        .unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
    let ran = s.run_stale_with(Some(&terrain)).unwrap();
    assert_eq!(ran, vec![Stage::Volumes]);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    assert_ne!(s.document()["stages"]["volumes"]["results"], before);
}

#[test]
fn a_saved_document_reloads_with_the_same_volumes() {
    let mut s = with_dem();
    s.run_volumes(&settings(), Some(&terrain)).unwrap();
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(again, s);
    assert_eq!(again.stage_status(Stage::Volumes), StageStatus::Fresh);
}
