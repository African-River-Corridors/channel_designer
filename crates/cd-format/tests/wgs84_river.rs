//! The synthetic river given in WGS84 gives the same volumes, to 1e-6 relative, as the same river
//! given in UTM: the import reprojects to the right zone and the stages run on the result.
//!
//! Label: property

use cd_core::geom::Pt;
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_format::design::crs::Utm;
use cd_format::design::{AlignMethod, Role, Session, Stage};
use cd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn params() -> RiverParams {
    RiverParams::default()
}

fn terrain(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let r = synthetic_river(params()).unwrap();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
        .collect()
}

/// The river's two lines and polygon as GeoJSON text: in UTM 30N, or in WGS84 (no `crs` member).
fn inputs(wgs84: bool) -> Vec<(Role, String)> {
    let r = synthetic_river(params()).unwrap();
    let utm = Utm {
        zone: 30,
        south: false,
    };
    let conv = |p: &Pt| -> Pt {
        let q = [p[0] + SHIFT[0], p[1] + SHIFT[1]];
        if wgs84 {
            utm.inverse(q[0], q[1]).unwrap()
        } else {
            q
        }
    };
    let crs = if wgs84 { "" } else { UTM30 };
    // The design line stops a vertex short of each end of the polygon. A line that ends exactly on the
    // polygon's end edge puts a whole cross-section on the boundary, and the last bit of the
    // reprojection then decides which side of it every sample lands on. That knife edge is not what
    // this test is about.
    let n = r.centreline.len();
    let line: Vec<Pt> = r.centreline[1..n - 1].iter().map(conv).collect();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(conv).collect();
    vec![
        (
            Role::Centreline,
            format!(
                r#"{{"type":"LineString"{crs},"coordinates":{}}}"#,
                json!(line)
            ),
        ),
        (
            Role::RiverPolygon,
            format!(
                r#"{{"type":"Polygon"{crs},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ]
}

fn volumes(wgs84: bool) -> (Session, Value) {
    let mut s = Session::new("River", "2026-10-16");
    let texts = inputs(wgs84);
    let items: Vec<(Role, &str)> = texts.iter().map(|(r, t)| (*r, t.as_str())).collect();
    s.import(&items).unwrap();
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let dem = Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    };
    s.set_dem("dem.tif", b"x", &dem);
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
        .unwrap();
    let rows = s.document()["stages"]["volumes"]["results"]["options"][0].clone();
    (s, rows)
}

#[test]
fn a_wgs84_river_is_reprojected_and_gives_the_volumes_of_the_utm_river() {
    let (utm_s, utm) = volumes(false);
    let (wgs_s, wgs) = volumes(true);
    assert_eq!(wgs_s.document()["project"]["crs"]["epsg"], 32630);
    assert_eq!(
        wgs_s.document()["project"]["source_crs"]["centreline"],
        "EPSG:4326"
    );
    assert_eq!(
        utm_s.document()["project"]["source_crs"]["centreline"],
        "EPSG:32630"
    );
    for k in ["dredge_m3", "excav_m3", "total_m3", "max_face_h_m"] {
        let (a, b) = (utm[k].as_f64().unwrap(), wgs[k].as_f64().unwrap());
        assert!(a > 0.0);
        assert!(
            (a - b).abs() <= 1e-6 * a.abs(),
            "{k}: UTM {a}, WGS84 {b}, relative {:e}",
            ((a - b) / a).abs()
        );
    }
}
