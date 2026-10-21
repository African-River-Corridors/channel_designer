//! Exports: GeoJSON (project CRS and WGS84) and CSV carry "Screening — not for construction", the
//! checks of each option, and a failed check on the option that failed it. A stale or missing DEM
//! shows in the volumes status, never as fresh.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_format::design::crs::Utm;
use owd_format::design::export::SCREENING_LABEL;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn zigzag() -> Vec<Pt> {
    (0..=60)
        .map(|i| {
            [
                500_000.0 + 50.0 * f64::from(i),
                600_000.0 + if i % 2 == 0 { 8.0 } else { -8.0 },
            ]
        })
        .collect()
}

fn straight() -> Vec<Pt> {
    vec![[500_000.0, 600_000.0], [501_500.0, 600_000.0]]
}

fn design(line: &[Pt], vessel: &str) -> Session {
    let mut s = Session::new("Export test", "2026-10-16");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(line)
        ),
    )])
    .unwrap();
    s.set_sizing("PIANC-B", vessel, Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s
}

#[test]
fn the_csv_has_the_label_a_header_and_a_failed_checks_column_that_names_the_failure() {
    let s = design(&zigzag(), "CEMT_Va");
    let csv = s.export_csv(None);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], format!("# {SCREENING_LABEL}"));
    let header: Vec<&str> = lines[1].split(',').collect();
    let col = header.iter().position(|h| *h == "failed_checks").unwrap();
    let row: Vec<&str> = lines[2].split(',').collect();
    assert_eq!(row[col], "R_min", "{csv}");
    assert_eq!(
        row[header.iter().position(|h| *h == "radius_ok").unwrap()],
        "false"
    );
    assert_eq!(row[0], "rolling-circle");
    // A passing option has an empty failed_checks cell.
    let ok = design(&straight(), "CEMT_IV").export_csv(None);
    let row: Vec<&str> = ok.lines().nth(2).unwrap().split(',').collect();
    assert_eq!(row[col], "");
}

#[test]
fn the_geojson_has_the_label_the_checks_and_the_failure_in_both_crs() {
    let s = design(&zigzag(), "CEMT_Va");
    let project: Value = serde_json::from_str(&s.export_geojson(false, None).unwrap()).unwrap();
    let wgs: Value = serde_json::from_str(&s.export_geojson(true, None).unwrap()).unwrap();
    for fc in [&project, &wgs] {
        assert_eq!(fc["screening"], SCREENING_LABEL);
        let f = &fc["features"][0];
        assert_eq!(f["properties"]["screening"], SCREENING_LABEL);
        assert_eq!(f["properties"]["failed_checks"], json!(["R_min"]));
        assert_eq!(f["properties"]["checks"]["radius_ok"], false);
        assert!(f["properties"]["checks"]["measured_min_radius_m"].is_number());
    }
    // The project export names its CRS; the WGS84 one is plain RFC 7946.
    assert_eq!(
        project["crs"]["properties"]["name"],
        "urn:ogc:def:crs:EPSG::32630"
    );
    assert!(wgs.get("crs").is_none());
    // The two agree: WGS84 back to UTM is the project line, to a millimetre.
    let u = Utm {
        zone: 30,
        south: false,
    };
    let (pc, wc) = (
        project["features"][0]["geometry"]["coordinates"]
            .as_array()
            .unwrap(),
        wgs["features"][0]["geometry"]["coordinates"]
            .as_array()
            .unwrap(),
    );
    assert_eq!(pc.len(), wc.len());
    for (p, w) in pc.iter().zip(wc) {
        let (lon, lat) = (w[0].as_f64().unwrap(), w[1].as_f64().unwrap());
        assert!(
            (-6.0..=0.0).contains(&lon) && (5.0..=6.0).contains(&lat),
            "{lon} {lat}"
        );
        let [x, y] = u.forward(lon, lat).unwrap();
        assert!(
            (x - p[0].as_f64().unwrap()).abs() < 1e-3 && (y - p[1].as_f64().unwrap()).abs() < 1e-3
        );
    }
}

#[test]
fn an_export_with_no_options_says_what_to_run() {
    let s = Session::new("empty", "2026-10-16");
    let e = s.export_geojson(false, None).unwrap_err().to_string();
    assert!(e.contains("alignment"), "{e}");
    // The CSV of nothing is the label and the header.
    assert_eq!(s.export_csv(None).lines().count(), 2);
}

#[test]
fn volumes_show_in_the_exports_with_their_status() {
    let mut s = design(&straight(), "CEMT_IV");
    let dem = Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    };
    s.set_dem("d.tif", b"dem", &dem);
    let flat = |xs: &[f64], _ys: &[f64]| vec![3.0; xs.len()];
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    let csv = s.export_csv(Some(b"dem"));
    let header: Vec<&str> = csv.lines().nth(1).unwrap().split(',').collect();
    let row: Vec<&str> = csv.lines().nth(2).unwrap().split(',').collect();
    let at = |name: &str| row[header.iter().position(|h| *h == name).unwrap()];
    assert_eq!(at("volumes_status"), "fresh");
    assert!(at("dredge_m3").parse::<f64>().is_ok() && at("tallest_face_m").parse::<f64>().is_ok());
    // A changed DEM file: the numbers stay, marked stale.
    let stale = s.export_csv(Some(b"another dem"));
    let stale_row: Vec<&str> = stale.lines().nth(2).unwrap().split(',').collect();
    let stale_at = |name: &str| stale_row[header.iter().position(|h| *h == name).unwrap()];
    assert_eq!(stale_at("volumes_status"), "stale", "{stale}");
    let fc: Value = serde_json::from_str(&s.export_geojson(false, Some(b"dem")).unwrap()).unwrap();
    assert_eq!(fc["features"][0]["properties"]["volumes_status"], "fresh");
    assert!(fc["features"][0]["properties"]["volumes"]["dredge_m3"].is_number());
}
