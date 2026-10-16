//! Reprojection against PROJ. `tests/data/proj_points.csv` was printed by PROJ 9.7.1 `cs2cs`
//! (`+proj=longlat +datum=WGS84` to `+proj=utm`), 68 points across the zones, both hemispheres
//! and latitudes to 84 degrees.
//!
//! Label: snapshot (what PROJ printed)

use cd_format::design::crs::{Crs, Utm};

fn points() -> Vec<(f64, f64, Utm, f64, f64)> {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/proj_points.csv"),
    )
    .expect("proj_points.csv");
    text.lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let utm = Utm {
                zone: f[2].parse().unwrap(),
                south: f[3] == "S",
            };
            (
                f[0].parse().unwrap(),
                f[1].parse().unwrap(),
                utm,
                f[4].parse().unwrap(),
                f[5].parse().unwrap(),
            )
        })
        .collect()
}

#[test]
fn forward_matches_proj_to_one_millimetre() {
    let pts = points();
    assert!(pts.len() >= 60);
    for (lon, lat, utm, e, n) in pts {
        let [x, y] = utm.forward(lon, lat).unwrap();
        assert!(
            (x - e).abs() < 1e-3 && (y - n).abs() < 1e-3,
            "({lon}, {lat}) zone {}: got ({x}, {y}), PROJ ({e}, {n})",
            utm.zone
        );
    }
}

#[test]
fn inverse_returns_the_start_to_a_nanodegree() {
    for (lon, lat, utm, e, n) in points() {
        let [lo, la] = utm.inverse(e, n).unwrap();
        assert!(
            (lo - lon).abs() < 1e-8 && (la - lat).abs() < 1e-8,
            "({lon}, {lat}): got ({lo}, {la})"
        );
    }
}

#[test]
fn the_zone_comes_from_the_longitude_and_the_hemisphere_from_the_latitude() {
    assert_eq!(
        Utm::for_point(-2.1, 5.2),
        Utm {
            zone: 30,
            south: false
        }
    );
    assert_eq!(Utm::for_point(-2.1, 5.2).epsg(), 32630);
    assert_eq!(Utm::for_point(36.8, -1.3).epsg(), 32737);
    assert_eq!(Utm::for_point(179.99, 10.0).zone, 60);
    assert_eq!(Utm::for_point(-180.0, 10.0).zone, 1);
}

#[test]
fn only_wgs84_and_wgs84_utm_are_accepted_and_the_message_says_what_to_convert_to() {
    assert_eq!(Crs::parse("EPSG:4326"), Ok(Crs::Wgs84));
    assert_eq!(
        Crs::parse("urn:ogc:def:crs:EPSG::32630").unwrap().epsg(),
        32630
    );
    assert_eq!(Crs::parse("urn:ogc:def:crs:OGC:1.3:CRS84"), Ok(Crs::Wgs84));
    let msg = Crs::parse("EPSG:2154").unwrap_err();
    assert!(
        msg.contains("EPSG:2154") && msg.contains("WGS84") && msg.contains("326xx"),
        "{msg}"
    );
    assert!(Crs::parse("EPSG:32661").is_err());
    assert!(Crs::parse("local grid").unwrap_err().contains("local grid"));
}
