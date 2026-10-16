//! The design session on the synthetic river, given as GeoJSON in UTM and in WGS84.
//!
//! Label: property

use cd_core::geom::Geometry;
use cd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_format::design::crs::Utm;
use cd_format::design::{DesignError, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn line_json(pts: &[[f64; 2]], crs: &str) -> String {
    format!(
        r#"{{"type":"LineString"{crs},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// The synthetic river's banks and polygon as GeoJSON text in UTM zone 30N (the river's local
/// metres are used as eastings and northings offset into the zone).
fn river_utm() -> (String, String, String) {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let shift = |p: &[f64; 2]| [p[0] + 500_000.0, p[1] + 600_000.0];
    let pts = |g: &Geometry| match g {
        Geometry::LineString(l) => l.iter().map(shift).collect::<Vec<[f64; 2]>>(),
        other => panic!("a bank is a line, found {other:?}"),
    };
    let left = pts(&river.left_bank);
    let right = pts(&river.right_bank);
    let ring: Vec<[f64; 2]> = river.polygon.exterior.iter().map(shift).collect();
    (
        line_json(&left, UTM30),
        line_json(&right, UTM30),
        format!(
            r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
            json!([ring])
        ),
    )
}

fn imported() -> Session {
    let (l, r, p) = river_utm();
    let mut s = Session::new("Synthetic river", "2026-10-16");
    s.import(&[
        (Role::LeftBank, &l),
        (Role::RightBank, &r),
        (Role::RiverPolygon, &p),
    ])
    .unwrap();
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

#[test]
fn a_design_saves_reopens_and_validates_with_no_change() {
    let s = imported();
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    let text = s.to_text();
    let again = Session::open(&text).unwrap();
    assert_eq!(again.to_text(), text);
    assert_eq!(again, s);
    assert_eq!(s.document()["project"]["crs"]["epsg"], 32630);
    assert_eq!(s.document()["inputs"]["left_bank"]["type"], "LineString");
}

#[test]
fn a_saved_file_reloads_bit_for_bit() {
    let s = imported();
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(
        again.document()["inputs"]["river_polygon"],
        s.document()["inputs"]["river_polygon"]
    );
}

#[test]
fn sizing_results_equal_channel_shape_called_directly() {
    let s = imported();
    let direct = channel_shape(&RulesetRegistry::new(), "PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(
        r["bottom_width_straight_m"],
        json!(direct.bottom_width_straight_m)
    );
    assert_eq!(r["min_bend_radius_m"], json!(direct.min_bend_radius_m));
    assert_eq!(r["design_depth_m"], json!(direct.design_depth_m));
    assert_eq!(r["loa_m"], json!(direct.loa_m));
    assert_eq!(r["draught_m"], json!(2.5));
    assert_eq!(
        Session::preview_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap(),
        *r
    );
}

#[test]
fn changing_the_vessel_marks_sizing_stale_and_nothing_reruns() {
    let mut s = imported();
    let before = s.document()["stages"]["sizing"]["results"].clone();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    s.set_sizing("PIANC-B", "CEMT_Va", Some(2.5)).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
    assert_eq!(
        s.document()["stages"]["sizing"]["results"],
        before,
        "nothing reran"
    );
    // A ruleset change is an edit too.
    let mut s = imported();
    let other = RulesetRegistry::new()
        .iter()
        .map(|r| r.ruleset_id.clone())
        .find(|id| id != "PIANC-B")
        .unwrap();
    s.set_sizing(&other, "CEMT_IV", Some(2.5)).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
}

#[test]
fn an_unknown_ruleset_or_vessel_is_refused_and_changes_nothing() {
    let mut s = imported();
    let before = s.clone();
    for (rs, v, d) in [
        ("NOPE", "CEMT_IV", None),
        ("PIANC-B", "NOPE", None),
        ("PIANC-B", "CEMT_IV", Some(-1.0)),
    ] {
        assert!(
            matches!(s.set_sizing(rs, v, d), Err(DesignError::Input(_))),
            "{rs} {v} {d:?}"
        );
    }
    assert_eq!(s, before);
}

#[test]
fn wgs84_input_is_reprojected_to_the_zone_at_the_bank_centroid_and_the_zone_is_recorded() {
    let left = line_json(&[[-2.10, 5.20], [-2.09, 5.21], [-2.08, 5.22]], "");
    let right = line_json(&[[-2.10, 5.201], [-2.09, 5.211], [-2.08, 5.221]], "");
    let mut s = Session::new("Ankobra", "2026-10-16");
    let report = s
        .import(&[(Role::LeftBank, &left), (Role::RightBank, &right)])
        .unwrap();
    assert_eq!(report.project_epsg, 32630);
    assert_eq!(
        s.document()["project"]["crs"]["name"],
        "WGS 84 / UTM zone 30N"
    );
    assert_eq!(
        s.document()["project"]["source_crs"]["left_bank"],
        "EPSG:4326"
    );
    let p = &s.document()["inputs"]["left_bank"]["coordinates"][1];
    let [x, y] = Utm {
        zone: 30,
        south: false,
    }
    .forward(-2.09, 5.21)
    .unwrap();
    assert_eq!(p, &json!([x, y]));
}

#[test]
fn a_southern_trace_gets_a_southern_zone() {
    let line = line_json(&[[36.80, -1.30], [36.81, -1.31]], "");
    let mut s = Session::new("Tana", "2026-10-16");
    assert_eq!(
        s.import(&[(Role::Centreline, &line)]).unwrap().project_epsg,
        32737
    );
}

#[test]
fn an_unsupported_crs_is_refused_naming_it_and_what_to_convert_to_and_nothing_changes() {
    let good = line_json(&[[-2.10, 5.20], [-2.09, 5.21]], "");
    let bad = r#"{"type":"LineString","crs":{"type":"name","properties":{"name":"EPSG:2154"}},"coordinates":[[1,2],[3,4]]}"#.to_string();
    let mut s = Session::new("x", "2026-10-16");
    let before = s.clone();
    let e = s
        .import(&[(Role::LeftBank, &good), (Role::RightBank, &bad)])
        .unwrap_err();
    let DesignError::Input(msg) = e else {
        panic!("{e:?}")
    };
    assert!(
        msg.contains("right_bank") && msg.contains("EPSG:2154") && msg.contains("326xx"),
        "{msg}"
    );
    assert_eq!(s, before, "an import that is refused changes nothing");
}

#[test]
fn a_zone_that_differs_from_the_designs_is_refused() {
    let mut s = imported(); // zone 30N
    let other = r#"{"type":"LineString","crs":{"type":"name","properties":{"name":"EPSG:32631"}},
        "coordinates":[[500000,0],[500100,0]]}"#;
    let e = s
        .import(&[(Role::Centreline, other)])
        .unwrap_err()
        .to_string();
    assert!(e.contains("zone 31N") && e.contains("zone 30N"), "{e}");
}

#[test]
fn a_stage_that_is_not_built_yet_says_so() {
    let mut s = imported();
    assert!(matches!(
        s.run_stage(Stage::Alignment),
        Err(DesignError::NotAvailable(_))
    ));
}

#[test]
fn the_dem_warning_names_a_missing_or_changed_file() {
    let mut doc: Value = imported().document().clone();
    doc["inputs"]["dem"] = json!({
        "path": "dem.tif",
        "sha256": cd_format::design::hash::sha256_hex(b"abc"),
        "crs": "EPSG:4326", "nodata": -32768
    });
    let s = Session::open(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(s.dem_warning(Some(b"abc")), None);
    assert!(s.dem_warning(None).unwrap().contains("missing"));
    assert!(s.dem_warning(Some(b"abd")).unwrap().contains("changed"));
}

#[test]
fn the_published_schema_is_what_the_session_prints() {
    let file = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/cdesign.schema.json"),
    )
    .unwrap();
    assert_eq!(Session::schema_text(), file);
}

#[test]
fn open_refuses_text_that_is_not_json_and_names_each_schema_break() {
    assert!(matches!(Session::open("{"), Err(DesignError::Input(_))));
    let mut doc = imported().document().clone();
    doc["format_version"] = json!(2);
    doc["project"].as_object_mut().unwrap().remove("title");
    let Err(DesignError::Document(errs)) = Session::open(&doc.to_string()) else {
        panic!()
    };
    assert!(
        errs.iter().any(|e| e.contains("format_version")),
        "{errs:?}"
    );
    assert!(errs.iter().any(|e| e.contains("title")), "{errs:?}");
}
