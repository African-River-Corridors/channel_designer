//! Issue #366: when every bend of the water-aware rolling circle is infeasible, the option says
//! "no feasible line". It is a failed check, its screening verdict is `not-navigable`, and the
//! volumes stage does not cost it as a design.
//!
//! Label: property (a 40 m synthetic river, where no bend holds the channel, against a 300 m one).

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, SyntheticRiver, synthetic_river};
use owd_format::design::export::failed_checks;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn river(width: f64) -> SyntheticRiver {
    synthetic_river(RiverParams {
        width_m: width,
        ..RiverParams::default()
    })
    .unwrap()
}

fn aligned(width: f64) -> Session {
    let r = river(width);
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let centre: Vec<Pt> = r.centreline.iter().map(shift).collect();
    let mut s = Session::new("River", "2026-10-22");
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
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s
}

fn option(s: &Session) -> Value {
    s.document()["options"][0].clone()
}

fn with_volumes(width: f64) -> Session {
    let mut s = aligned(width);
    let dem = Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    s.set_dem("dem/river.tif", b"a stand-in", &dem);
    let r = river(width);
    s.set_water_fall(r.water_level(0.0), 0.0).unwrap();
    let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
            .collect()
    };
    s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
        .unwrap();
    s
}

#[test]
fn a_river_too_narrow_at_every_bend_gives_no_feasible_line() {
    let s = aligned(40.0);
    let o = option(&s);
    let rungs = o["details"]["rungs"].as_array().unwrap();
    assert!(
        !rungs.is_empty() && rungs.iter().all(|r| r == "infeasible"),
        "{rungs:?}"
    );
    assert_eq!(o["checks"]["feasible"]["ok"], false, "{}", o["checks"]);
    assert_eq!(o["checks"]["feasible"]["note"], "no feasible line");
    assert!(
        failed_checks(&o).contains(&"no_feasible_line"),
        "{:?}",
        failed_checks(&o)
    );
}

#[test]
fn a_wide_river_gives_a_feasible_line() {
    let o = option(&aligned(300.0));
    assert_eq!(o["checks"]["feasible"]["ok"], true, "{}", o["checks"]);
    assert!(!failed_checks(&o).contains(&"no_feasible_line"));
}

#[test]
fn volumes_do_not_cost_an_option_with_no_feasible_line() {
    let s = with_volumes(40.0);
    let row = &s.document()["stages"]["volumes"]["results"]["options"][0];
    assert_eq!(row["no_feasible_line"], true, "{row}");
    for k in ["dredge_m3", "excav_m3", "total_m3", "max_face_h_m"] {
        assert!(row[k].is_null(), "{k}: {}", row[k]);
    }
    assert_eq!(row["screening"]["verdict"], "not-navigable");
}

#[test]
fn volumes_cost_a_feasible_option_as_before() {
    let s = with_volumes(300.0);
    let row = &s.document()["stages"]["volumes"]["results"]["options"][0];
    assert!(row["no_feasible_line"].is_null(), "{row}");
    assert!(row["total_m3"].as_f64().is_some(), "{row}");
}
