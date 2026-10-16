//! GeoPackage import: the same river from a GeoPackage and from GeoJSON gives the same document.
//!
//! Two GeoPackages are used: one written by this test (the synthetic river, in the shape GDAL
//! writes: a header, then WKB), and `tests/data/gdal/river.gpkg`, written by GDAL 3.10.2
//! (`ogr2ogr -f GPKG`) from the three GeoJSON files beside it.
//!
//! Label: property

use cd_core::geom::Geometry;
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_format::design::geopackage::{self, is_geopackage, layers};
use cd_format::design::{Role, Session};
use rusqlite::{Connection, MAIN_DB};
use serde_json::json;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/gdal")
            .join(name),
    )
    .unwrap()
}

fn text(name: &str) -> String {
    String::from_utf8(data(name)).unwrap()
}

// --- writing a GeoPackage, as GDAL does -------------------------------------------------------

/// A GeoPackage geometry blob: header (no envelope) and little-endian WKB.
fn blob(srs_id: i32, wkb: &[u8]) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0, 0b0000_0001];
    b.extend(srs_id.to_le_bytes());
    b.extend(wkb);
    b
}

fn wkb_points(pts: &[[f64; 2]]) -> Vec<u8> {
    let mut b = (pts.len() as u32).to_le_bytes().to_vec();
    for p in pts {
        b.extend(p[0].to_le_bytes());
        b.extend(p[1].to_le_bytes());
    }
    b
}

fn wkb_line(pts: &[[f64; 2]]) -> Vec<u8> {
    let mut b = vec![1];
    b.extend(2u32.to_le_bytes());
    b.extend(wkb_points(pts));
    b
}

fn wkb_polygon(ring: &[[f64; 2]]) -> Vec<u8> {
    let mut b = vec![1];
    b.extend(3u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(wkb_points(ring));
    b
}

/// A feature table to write: its name, its geometry type name and its geometry blobs.
type Table<'a> = (&'a str, &'a str, Vec<Vec<u8>>);

/// A GeoPackage in memory with the given feature tables.
fn gpkg(srs_id: i32, layers: &[Table]) -> Vec<u8> {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(&format!(
        "CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT, srs_id INTEGER PRIMARY KEY, organization TEXT,
           organization_coordsys_id INTEGER, definition TEXT, description TEXT);
         INSERT INTO gpkg_spatial_ref_sys VALUES ('x', {srs_id}, 'EPSG', {srs_id}, 'x', '');
         CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT, identifier TEXT, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT, column_name TEXT, geometry_type_name TEXT,
           srs_id INTEGER, z INTEGER, m INTEGER);"
    ))
    .unwrap();
    for (name, kind, blobs) in layers {
        c.execute_batch(&format!(
            "CREATE TABLE {name} (fid INTEGER PRIMARY KEY, geom BLOB);
             INSERT INTO gpkg_contents VALUES ('{name}', 'features', '{name}', {srs_id});
             INSERT INTO gpkg_geometry_columns VALUES ('{name}', 'geom', '{kind}', {srs_id}, 0, 0);"
        ))
        .unwrap();
        for b in blobs {
            c.execute(&format!("INSERT INTO {name} (geom) VALUES (?1)"), [b])
                .unwrap();
        }
    }
    c.serialize(MAIN_DB).unwrap().to_vec()
}

type Pts = Vec<[f64; 2]>;

fn river() -> (Pts, Pts, Pts) {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let shift = |p: &[f64; 2]| [p[0] + 500_000.0, p[1] + 600_000.0];
    let pts = |g: &Geometry| match g {
        Geometry::LineString(l) => l.iter().map(shift).collect::<Vec<[f64; 2]>>(),
        other => panic!("a bank is a line, found {other:?}"),
    };
    (
        pts(&river.left_bank),
        pts(&river.right_bank),
        river.polygon.exterior.iter().map(shift).collect(),
    )
}

fn doc_from(items: &[(Role, String)]) -> Session {
    let mut s = Session::new("Synthetic river", "2026-10-16");
    let refs: Vec<(Role, &str)> = items.iter().map(|(r, t)| (*r, t.as_str())).collect();
    s.import(&refs).unwrap();
    s
}

#[test]
fn the_synthetic_river_imports_from_a_geopackage_and_from_geojson_to_the_same_document() {
    let (l, r, ring) = river();
    let bytes = gpkg(
        32630,
        &[
            ("left_bank", "LINESTRING", vec![blob(32630, &wkb_line(&l))]),
            ("right_bank", "LINESTRING", vec![blob(32630, &wkb_line(&r))]),
            (
                "river_polygon",
                "POLYGON",
                vec![blob(32630, &wkb_polygon(&ring))],
            ),
        ],
    );
    assert!(is_geopackage(&bytes));
    let from_gpkg = doc_from(&[
        (
            Role::LeftBank,
            geopackage::to_geojson(&bytes, Some("left_bank")).unwrap(),
        ),
        (
            Role::RightBank,
            geopackage::to_geojson(&bytes, Some("right_bank")).unwrap(),
        ),
        (
            Role::RiverPolygon,
            geopackage::to_geojson(&bytes, Some("river_polygon")).unwrap(),
        ),
    ]);
    let line = |p: &[[f64; 2]]| {
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(p)
        )
    };
    let from_json = doc_from(&[
        (Role::LeftBank, line(&l)),
        (Role::RightBank, line(&r)),
        (
            Role::RiverPolygon,
            format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ]);
    assert_eq!(from_gpkg.to_text(), from_json.to_text());
    assert_eq!(from_gpkg.document()["project"]["crs"]["epsg"], 32630);
    assert_eq!(
        from_gpkg.document()["project"]["source_crs"]["left_bank"],
        "EPSG:32630"
    );
}

#[test]
fn a_geopackage_written_by_gdal_imports_to_the_same_document_as_its_geojson_source() {
    let bytes = data("river.gpkg");
    let names: Vec<String> = layers(&bytes)
        .unwrap()
        .into_iter()
        .map(|l| l.table)
        .collect();
    assert_eq!(names, ["left_bank", "right_bank", "river_polygon"]);
    let from_gpkg = doc_from(&[
        (
            Role::LeftBank,
            geopackage::to_geojson(&bytes, Some("left_bank")).unwrap(),
        ),
        (
            Role::RightBank,
            geopackage::to_geojson(&bytes, Some("right_bank")).unwrap(),
        ),
        (
            Role::RiverPolygon,
            geopackage::to_geojson(&bytes, Some("river_polygon")).unwrap(),
        ),
    ]);
    let from_json = doc_from(&[
        (Role::LeftBank, text("left.geojson")),
        (Role::RightBank, text("right.geojson")),
        (Role::RiverPolygon, text("polygon.geojson")),
    ]);
    assert_eq!(from_gpkg.to_text(), from_json.to_text());
}

#[test]
fn a_file_with_several_layers_needs_one_named_and_the_error_lists_them() {
    let e = geopackage::to_geojson(&data("river.gpkg"), None).unwrap_err();
    assert!(
        e.contains("3 feature layers")
            && e.contains("left_bank (linestring)")
            && e.contains("FILE#LAYER"),
        "{e}"
    );
    let e = geopackage::to_geojson(&data("river.gpkg"), Some("nope")).unwrap_err();
    assert!(
        e.contains("no feature layer `nope`") && e.contains("river_polygon"),
        "{e}"
    );
}

#[test]
fn a_single_layer_needs_no_name() {
    let (l, _, _) = river();
    let bytes = gpkg(
        32630,
        &[("banks", "LINESTRING", vec![blob(32630, &wkb_line(&l))])],
    );
    let g = geopackage::to_geojson(&bytes, None).unwrap();
    assert!(g.contains("FeatureCollection") && g.contains("EPSG:32630"));
}

#[test]
fn wgs84_layers_are_reprojected_like_wgs84_geojson() {
    let pts = [[-2.10, 5.20], [-2.09, 5.21], [-2.08, 5.22]];
    let bytes = gpkg(
        4326,
        &[("bank", "LINESTRING", vec![blob(4326, &wkb_line(&pts))])],
    );
    let from_gpkg = doc_from(&[(
        Role::LeftBank,
        geopackage::to_geojson(&bytes, None).unwrap(),
    )]);
    let from_json = doc_from(&[(
        Role::LeftBank,
        format!(r#"{{"type":"LineString","coordinates":{}}}"#, json!(pts)),
    )]);
    assert_eq!(
        from_gpkg.document()["inputs"],
        from_json.document()["inputs"]
    );
    assert_eq!(from_gpkg.document()["project"]["crs"]["epsg"], 32630);
}

#[test]
fn an_unsupported_crs_is_refused_by_the_import_with_its_message() {
    let bytes = gpkg(
        2154,
        &[(
            "bank",
            "LINESTRING",
            vec![blob(2154, &wkb_line(&[[1.0, 2.0], [3.0, 4.0]]))],
        )],
    );
    let g = geopackage::to_geojson(&bytes, None).unwrap();
    let mut s = Session::new("x", "2026-10-16");
    let e = s.import(&[(Role::LeftBank, &g)]).unwrap_err().to_string();
    assert!(e.contains("EPSG:2154") && e.contains("WGS84"), "{e}");
}

#[test]
fn z_values_and_multi_geometries_are_read_and_points_are_refused() {
    // A LineString Z (ISO type 1002): the z value is dropped.
    let mut z = vec![1];
    z.extend(1002u32.to_le_bytes());
    z.extend(2u32.to_le_bytes());
    for p in [[1.0f64, 2.0, 9.0], [3.0, 4.0, 9.0]] {
        for v in p {
            z.extend(v.to_le_bytes());
        }
    }
    let bytes = gpkg(32630, &[("z", "LINESTRING", vec![blob(32630, &z)])]);
    let g: serde_json::Value =
        serde_json::from_str(&geopackage::to_geojson(&bytes, None).unwrap()).unwrap();
    assert_eq!(
        g["features"][0]["geometry"]["coordinates"],
        json!([[1.0, 2.0], [3.0, 4.0]])
    );
    // A MultiLineString holding one line.
    let mut m = vec![1];
    m.extend(5u32.to_le_bytes());
    m.extend(1u32.to_le_bytes());
    m.extend(wkb_line(&[[1.0, 2.0], [3.0, 4.0]]));
    let bytes = gpkg(32630, &[("m", "MULTILINESTRING", vec![blob(32630, &m)])]);
    assert!(
        geopackage::to_geojson(&bytes, None)
            .unwrap()
            .contains("MultiLineString")
    );
    // Points are not lines.
    let mut p = vec![1];
    p.extend(1u32.to_le_bytes());
    p.extend(1.0f64.to_le_bytes());
    p.extend(2.0f64.to_le_bytes());
    let bytes = gpkg(32630, &[("pts", "POINT", vec![blob(32630, &p)])]);
    assert!(
        geopackage::to_geojson(&bytes, None)
            .unwrap_err()
            .contains("points")
    );
}

#[test]
fn a_file_that_is_not_a_geopackage_is_refused() {
    assert!(!is_geopackage(b"{\"type\":\"Point\"}"));
    assert!(
        geopackage::to_geojson(b"not a database at all", None)
            .unwrap_err()
            .contains("not a GeoPackage")
    );
    // A real SQLite file with no GeoPackage tables.
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE t (a)").unwrap();
    let bytes = c.serialize(MAIN_DB).unwrap().to_vec();
    assert!(
        geopackage::to_geojson(&bytes, None)
            .unwrap_err()
            .contains("not a GeoPackage")
    );
}
