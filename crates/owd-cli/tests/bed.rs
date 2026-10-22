//! `owd bed` and the bed model in the volumes (spec 0012, ticket T7): a straight channel with a
//! known depth gives a known dredge volume, every output says the depth is assumed, and a
//! document with no bed model is untouched.
//!
//! Label: sourced - hand calculation (the parabola and the cut model's trapezoid, integrated by
//! hand: see `expected_dredge_area`)

use owd_core::geom::Pt;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

const SHIFT: Pt = [500_000.0, 600_000.0];
const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const LABEL: &str = "depth assumed, not measured";

const LENGTH_M: f64 = 2000.0;
const WIDTH_M: f64 = 100.0;
const DEPTH_M: f64 = 3.0;
/// The ground beside and under the river in the DEM: the DEM sees only the water surface (here
/// level with the bank top), which is why the bed model is needed.
const DEM_Z: f64 = 4.0;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn ok(args: &[&str]) -> Output {
    let o = run(args);
    assert!(
        o.status.code() == Some(0),
        "{args:?}: {} {}",
        err(&o),
        out(&o)
    );
    o
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-bed-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A flat float32 GeoTIFF of `DEM_Z` over the river, UTM 30N, 10 m pixels.
fn write_dem(path: &Path) {
    let (x0, y1, px) = (SHIFT[0] - 300.0, SHIFT[1] + 400.0, 10.0);
    let (w, h) = (260usize, 80usize);
    let data = vec![DEM_Z as f32; w * h];
    let mut enc = TiffEncoder::new(std::fs::File::create(path).unwrap()).unwrap();
    let mut img = enc
        .new_image::<colortype::Gray32Float>(w as u32, h as u32)
        .unwrap();
    img.encoder()
        .write_tag(Tag::Unknown(33550), &[px, px, 0.0][..])
        .unwrap();
    img.encoder()
        .write_tag(Tag::Unknown(33922), &[0.0, 0.0, 0.0, x0, y1, 0.0][..])
        .unwrap();
    let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 32630];
    img.encoder()
        .write_tag(Tag::Unknown(34735), &keys[..])
        .unwrap();
    img.write_data(&data).unwrap();
}

/// A straight river `LENGTH_M` long and `WIDTH_M` wide with a DEM, sized, aligned, at water level 0.
fn build(name: &str) -> (PathBuf, String) {
    let dir = work_dir(name);
    let at = |x: f64, y: f64| [x + SHIFT[0], y + SHIFT[1]];
    let h = WIDTH_M / 2.0;
    let ring = [
        at(-100.0, -h),
        at(LENGTH_M + 100.0, -h),
        at(LENGTH_M + 100.0, h),
        at(-100.0, h),
        at(-100.0, -h),
    ];
    let w = |file: &str, text: String| {
        let p = dir.join(file);
        std::fs::write(&p, text).unwrap();
        p.to_str().unwrap().to_string()
    };
    let polygon = w(
        "polygon.geojson",
        format!(
            r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
            json!([ring])
        ),
    );
    let centre = w(
        "centre.geojson",
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!([at(0.0, 0.0), at(LENGTH_M, 0.0)])
        ),
    );
    write_dem(&dir.join("dem.tif"));
    let doc = dir.join("t.owd").to_str().unwrap().to_string();
    ok(&["new", &doc, "--title", "t", "--date", "2026-10-21"]);
    ok(&[
        "import",
        &doc,
        "--centreline",
        &centre,
        "--polygon",
        &polygon,
        "--dem",
        dir.join("dem.tif").to_str().unwrap(),
        "--water-level",
        "0",
    ]);
    ok(&[
        "size",
        &doc,
        "--rule-set",
        "pianc-wg141",
        "--set-input",
        "se_quality=B",
        "--vessel",
        "CEMT_IV",
        "--draught",
        "2.5",
    ]);
    ok(&["align", &doc, "--method", "rolling-circle"]);
    (dir, doc)
}

fn read_doc(doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

fn volumes(doc: &str) -> Value {
    let o = ok(&["volumes", doc, "--json", "--model", "a"]);
    serde_json::from_str::<Value>(&out(&o)).unwrap()["options"][0].clone()
}

/// The dredge area (m2) of one station of the straight channel, by hand. The river is 100 m wide
/// and 3 m deep at the centreline: the bed is `z_b(y) = -3 (1 - (y/50)^2)` below a water level of
/// 0. The design channel (model A) has a flat bottom of half-width `hb` at `-d` (the design depth)
/// and wet side slopes of 3 horizontal to 1 vertical: `z_t(y) = -d + (|y| - hb) / 3` beyond `hb`.
/// Material is dredged where the bed is above the design: between `z_b` and `-d` on the bottom,
/// and between `z_b` and `z_t` on the slopes, out to `y*` where they meet.
fn expected_dredge_area(hb: f64, d: f64) -> f64 {
    let (a, w) = (DEPTH_M, WIDTH_M / 2.0);
    // On the slope: z_b = z_t  =>  -a + a y^2 / w^2 = -d + (y - hb)/3.
    // a/w^2 y^2 - y/3 + (d - a + hb/3) = 0.
    let (qa, qb, qc) = (a / (w * w), -1.0 / 3.0, d - a + hb / 3.0);
    // The first crossing, going out from the centreline.
    let y_star = (-qb - (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
    assert!(
        y_star > hb,
        "the bed must meet the slope outside the bottom"
    );
    // Integrals of z_b - z_t. Bottom, 0..hb: z_b + d. Slope, hb..y*: z_b - z_t.
    let zb_int = |lo: f64, hi: f64| -a * (hi - lo) + a * (hi.powi(3) - lo.powi(3)) / (3.0 * w * w);
    let bottom = zb_int(0.0, hb) + d * hb;
    // z_t integrated over the slope: -d (y*-hb) + ((y*-hb)^2)/6.
    let zt_int = -d * (y_star - hb) + (y_star - hb).powi(2) / 6.0;
    let slope = zb_int(hb, y_star) - zt_int;
    2.0 * (bottom + slope)
}

#[test]
fn a_straight_channel_with_a_known_depth_gives_the_hand_calculated_dredge() {
    let (_dir, doc) = build("dredge");
    let d = read_doc(&doc)["stages"]["sizing"]["results"].clone();
    let (hb, depth) = (
        d["bottom_width_straight_m"].as_f64().unwrap() / 2.0,
        d["design_depth_m"].as_f64().unwrap(),
    );
    ok(&["bed", &doc, "--depth", "3", "--season", "dry"]);
    let row = volumes(&doc);
    let stations = row["stations"].as_f64().unwrap();
    let got = row["dredge_m3"].as_f64().unwrap() / (stations * 25.0);
    let want = expected_dredge_area(hb, depth);
    assert!(
        (got - want).abs() < 0.01 * want,
        "dredge area per station {got} m2, by hand {want} m2 (hb {hb} m, design depth {depth} m)"
    );
    assert_eq!(row["depth_basis"], LABEL);
}

#[test]
fn without_a_bed_model_the_dem_is_the_bed_and_nothing_says_depth_is_assumed() {
    let (dir, doc) = build("none");
    let row = volumes(&doc);
    assert!(row.get("depth_basis").is_none(), "{row}");
    assert!(!std::fs::read_to_string(&doc).unwrap().contains("bed_model"));
    let csv = out(&ok(&["export", &doc, "--csv"]));
    assert!(!csv.contains(LABEL), "{csv}");
    // The DEM puts the ground at 4 m over the whole river: more to dredge than with a 3 m bed.
    let none = row["dredge_m3"].as_f64().unwrap();
    ok(&["bed", &doc, "--depth", "3"]);
    let with = volumes(&doc)["dredge_m3"].as_f64().unwrap();
    assert!(with < none, "{with} against {none}");
    drop(dir);
}

#[test]
fn the_label_is_in_the_csv_the_geojson_and_the_sections() {
    let (_dir, doc) = build("label");
    ok(&["bed", &doc, "--depth", "3"]);
    volumes(&doc);
    let csv = out(&ok(&["export", &doc, "--csv"]));
    assert!(
        csv.lines()
            .any(|l| l.starts_with("# ") && l.contains(LABEL)),
        "{csv}"
    );
    assert!(csv.lines().last().unwrap().contains(LABEL), "{csv}");
    let g: Value = serde_json::from_str(&out(&ok(&["export", &doc, "--geojson"]))).unwrap();
    assert_eq!(g["depth_basis"], LABEL);
    assert!(
        g["features"][0]["properties"]["notes"]
            .as_str()
            .unwrap()
            .contains(LABEL)
    );
    assert_eq!(
        g["features"][0]["properties"]["volumes"]["depth_basis"],
        LABEL
    );
}

#[test]
fn a_bed_model_makes_only_the_volumes_stale_and_a_survey_replaces_it() {
    let (_dir, doc) = build("stale");
    volumes(&doc);
    let status = |doc: &str| -> Value {
        serde_json::from_str(&out(&ok(&["status", doc, "--json"]))).unwrap()
    };
    assert_eq!(status(&doc)["stages"]["volumes"], "fresh");
    ok(&["bed", &doc, "--depth", "3"]);
    let s = status(&doc);
    assert_eq!(s["stages"]["volumes"], "stale");
    assert_eq!(s["stages"]["alignment"], "fresh");
    assert_eq!(s["stages"]["sizing"], "fresh");
    // A table is read along the river.
    let table = work_dir("stale-table").join("depths.csv");
    std::fs::write(&table, "chainage_m,depth_m\n0,2\n2000,4\n").unwrap();
    ok(&["bed", &doc, "--table", table.to_str().unwrap()]);
    assert_eq!(
        read_doc(&doc)["inputs"]["bed_model"]["points"],
        json!([[0.0, 2.0], [2000.0, 4.0]])
    );
    // Taking it out (the survey) leaves everything else as it was.
    let mut before = read_doc(&doc);
    ok(&["bed", &doc, "--clear"]);
    let after = read_doc(&doc);
    before["inputs"]
        .as_object_mut()
        .unwrap()
        .remove("bed_model");
    before.as_object_mut().unwrap().remove("provenance");
    assert_eq!(before, after);
    assert!(ok(&["validate", &doc]).status.success());
}

#[test]
fn a_bad_depth_is_refused_with_exit_2() {
    let (_dir, doc) = build("bad");
    for args in [
        vec!["bed", &doc, "--depth", "-1"],
        vec!["bed", &doc],
        vec!["bed", &doc, "--clear", "--depth", "2"],
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", err(&o));
    }
}
