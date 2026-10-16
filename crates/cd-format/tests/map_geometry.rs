//! Label: property
//! The plan map's geometry on the synthetic river (spec 0003, ticket 69).
use cd_format::design::{AlignMethod, Session};
use serde_json::Value;

fn aligned() -> Session {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/synthetic-river.cdesign"),
    )
    .unwrap();
    let mut s = Session::open(&text).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s
}

fn lon_lat_ok(v: &Value) -> bool {
    let c = v.as_array().unwrap();
    // The synthetic river sits in UTM 30N near 3 degrees west, 5.4 degrees north.
    (-3.2..-2.9).contains(&c[0].as_f64().unwrap()) && (5.3..5.5).contains(&c[1].as_f64().unwrap())
}

#[test]
fn the_map_geometry_is_wgs84_with_every_option_and_its_marks() {
    let m = aligned().map_geometry().unwrap();
    assert_eq!(m["crs"], "EPSG:4326");
    assert_eq!(m["banks"].as_array().unwrap().len(), 2);
    for b in m["banks"].as_array().unwrap() {
        assert!(
            b["line"]["coordinates"]
                .as_array()
                .unwrap()
                .iter()
                .all(lon_lat_ok)
        );
    }
    assert!(
        m["centreline"]["coordinates"]
            .as_array()
            .unwrap()
            .iter()
            .all(lon_lat_ok)
    );
    let opts = m["options"].as_array().unwrap();
    assert_eq!(opts.len(), 1);
    let o = &opts[0];
    assert_eq!(o["id"], "rolling-circle");
    assert!(
        o["line"]["coordinates"]
            .as_array()
            .unwrap()
            .iter()
            .all(lon_lat_ok)
    );
    let ticks = o["ticks"].as_array().unwrap();
    assert!(ticks.len() >= 5 && ticks[0]["label"] == "0+000");
    assert!(ticks.iter().all(|t| lon_lat_ok(&t["position"])));
    let bbox = m["bbox"].as_array().unwrap();
    assert!(bbox[0].as_f64() < bbox[2].as_f64() && bbox[1].as_f64() < bbox[3].as_f64());
}

#[test]
fn a_design_with_no_project_crs_is_refused_with_the_reason() {
    let mut s = Session::new("Empty", "2026-10-16");
    // A new design has no inputs and no CRS: the map has nothing to draw, and says why.
    let e = s.map_geometry().unwrap_err().to_string();
    assert!(e.contains("CRS"), "{e}");
    let _ = &mut s;
}
