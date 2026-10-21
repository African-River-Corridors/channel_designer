//! `cdesign fetch-dem` against a recorded mirror (a folder laid out like the public bucket), so no
//! test touches the network: the tiles and their hashes and the licence notice are written next to the
//! document and recorded in it, a tile whose hash differs is never overwritten, and tiles on one grid
//! are joined into one DEM.
//!
//! Label: property

use owd_format::design::dem_source::{COPERNICUS_NOTICE, TileId, write_geotiff};
use owd_format::design::hash::sha256_hex;
use owd_format::geotiff::{read_bytes, read_file};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cdesign"))
        .args(args)
        .output()
        .expect("cdesign runs")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cdesign-fetch-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn recorded_mirror() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mirror")
        .to_str()
        .unwrap()
        .to_string()
}

/// A WGS84 polygon around (`lon`, `lat`), a few hundred metres across.
fn polygon(dir: &Path, name: &str, lon: f64, lat: f64) -> String {
    let d = 0.002;
    let ring = json!([[
        [lon - d, lat - d],
        [lon + d, lat - d],
        [lon + d, lat + d],
        [lon - d, lat + d],
        [lon - d, lat - d]
    ]]);
    let p = dir.join(name);
    std::fs::write(
        &p,
        json!({"type": "Polygon", "coordinates": ring}).to_string(),
    )
    .unwrap();
    p.to_str().unwrap().to_string()
}

fn design(dir: &Path, lon: f64, lat: f64) -> String {
    let doc = dir.join("reach.cdesign").to_str().unwrap().to_string();
    let poly = polygon(dir, "reach.geojson", lon, lat);
    assert_eq!(
        code(&run(&[
            "new",
            &doc,
            "--title",
            "Reach",
            "--date",
            "2026-10-16"
        ])),
        0
    );
    let o = run(&["import", &doc, "--polygon", &poly]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    doc
}

fn read_doc(doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

const TILE: &str = "Copernicus_DSM_COG_10_N05_00_W003_00_DEM";

#[test]
fn the_tile_its_hash_and_the_licence_are_written_next_to_the_document_and_recorded_in_it() {
    let dir = work_dir("one");
    let doc = design(&dir, -2.57, 5.515);
    let o = run(&["fetch-dem", &doc, "--mirror", &recorded_mirror(), "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let res: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(res["downloaded"], 1);

    let rel = format!("reach-dem/{TILE}.tif");
    let file = dir.join(&rel);
    let bytes = std::fs::read(&file).unwrap();
    let d = read_doc(&doc);
    // The DEM reference: the tile, by relative path and hash, in WGS84.
    assert_eq!(d["inputs"]["dem"]["path"], rel.as_str());
    assert_eq!(d["inputs"]["dem"]["sha256"], sha256_hex(&bytes));
    assert_eq!(d["inputs"]["dem"]["crs"], "EPSG:4326");
    // Where it came from, with the hash of every tile, and the licence notice in the document.
    let src = &d["inputs"]["dem"]["source"];
    assert_eq!(src["name"], "Copernicus GLO-30");
    assert_eq!(src["licence"], COPERNICUS_NOTICE);
    assert_eq!(src["tiles"][0]["name"], TILE);
    assert_eq!(src["tiles"][0]["sha256"], sha256_hex(&bytes));
    assert_eq!(src["tiles"][0]["bytes"], bytes.len());
    assert!(
        src["tiles"][0]["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("{TILE}/{TILE}.tif"))
    );
    // The notice next to the tiles.
    let notice = std::fs::read_to_string(dir.join("reach-dem/NOTICE.txt")).unwrap();
    assert!(
        notice.contains(COPERNICUS_NOTICE) && notice.contains(&sha256_hex(&bytes)),
        "{notice}"
    );
    // The document still validates, and status reads the DEM without a warning.
    assert_eq!(code(&run(&["validate", &doc])), 0);
    let st = run(&["status", &doc]);
    assert!(!err(&st).contains("warning"), "{}", err(&st));
    // It is a DEM the tool reads.
    assert_eq!(read_file(&file).unwrap().epsg, Some(4326));
}

#[test]
fn a_second_run_reuses_the_file_and_a_file_with_another_hash_is_never_overwritten() {
    let dir = work_dir("overwrite");
    let doc = design(&dir, -2.57, 5.515);
    let m = recorded_mirror();
    assert_eq!(code(&run(&["fetch-dem", &doc, "--mirror", &m])), 0);
    let again = run(&["fetch-dem", &doc, "--mirror", &m, "--json"]);
    assert_eq!(code(&again), 0, "{}", err(&again));
    assert_eq!(
        serde_json::from_str::<Value>(&out(&again)).unwrap()["downloaded"],
        0
    );

    let file = dir.join(format!("reach-dem/{TILE}.tif"));
    let mut tampered = std::fs::read(&file).unwrap();
    tampered.push(0);
    std::fs::write(&file, &tampered).unwrap();
    let before = std::fs::read_to_string(&doc).unwrap();
    let o = run(&["fetch-dem", &doc, "--mirror", &m]);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("differs") && err(&o).contains("Refusing to overwrite"),
        "{}",
        err(&o)
    );
    assert_eq!(
        std::fs::read(&file).unwrap(),
        tampered,
        "the file is untouched"
    );
    assert_eq!(
        std::fs::read_to_string(&doc).unwrap(),
        before,
        "the document is untouched"
    );
}

#[test]
fn a_file_the_document_does_not_know_is_not_overwritten_either() {
    let dir = work_dir("unknown");
    let doc = design(&dir, -2.57, 5.515);
    std::fs::create_dir_all(dir.join("reach-dem")).unwrap();
    let file = dir.join(format!("reach-dem/{TILE}.tif"));
    std::fs::write(&file, b"mine").unwrap();
    let o = run(&["fetch-dem", &doc, "--mirror", &recorded_mirror()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("no record"), "{}", err(&o));
    assert_eq!(std::fs::read(&file).unwrap(), b"mine");
}

#[test]
fn a_reach_over_two_tiles_gets_one_joined_dem_and_both_hashes() {
    let dir = work_dir("two");
    // A mirror of two tiles on one grid: the recorded clip, and a copy of it one clip to the west.
    let mirror = dir.join("mirror");
    let clip = read_file(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/mirror/{TILE}/{TILE}.tif")),
    )
    .unwrap();
    let mut west = clip.clone();
    west.geotransform[2] -= clip.width as f64 * clip.geotransform[0];
    for (id, dem) in [
        (TileId { lon: -3, lat: 5 }, &clip),
        (TileId { lon: -4, lat: 5 }, &west),
    ] {
        let n = id.name();
        std::fs::create_dir_all(mirror.join(&n)).unwrap();
        std::fs::write(
            mirror.join(&n).join(format!("{n}.tif")),
            write_geotiff(dem).unwrap(),
        )
        .unwrap();
    }
    let doc = design(&dir, -3.0, 5.5);
    let o = run(&[
        "fetch-dem",
        &doc,
        "--mirror",
        mirror.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let d = read_doc(&doc);
    assert_eq!(d["inputs"]["dem"]["path"], "reach-dem.tif");
    assert_eq!(
        d["inputs"]["dem"]["source"]["tiles"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let joined = read_bytes(&std::fs::read(dir.join("reach-dem.tif")).unwrap()).unwrap();
    assert_eq!((joined.width, joined.height), (clip.width * 2, clip.height));
    assert_eq!(
        d["inputs"]["dem"]["sha256"],
        sha256_hex(&std::fs::read(dir.join("reach-dem.tif")).unwrap())
    );
    // Every pixel of both tiles is in the joined DEM, unchanged (stored as float32).
    for r in 0..clip.height {
        for c in 0..clip.width {
            assert_eq!(joined.value(r, c) as f32, west.value(r, c) as f32);
            assert_eq!(
                joined.value(r, clip.width + c) as f32,
                clip.value(r, c) as f32
            );
        }
    }
}

#[test]
fn a_mirror_without_the_tile_is_a_clear_error_and_changes_nothing() {
    let dir = work_dir("missing");
    let doc = design(&dir, -2.57, 5.515);
    let before = std::fs::read_to_string(&doc).unwrap();
    let o = run(&["fetch-dem", &doc, "--mirror", dir.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains(TILE), "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), before);
    // A document with no river has no extent.
    let empty = dir.join("empty.cdesign").to_str().unwrap().to_string();
    run(&["new", &empty, "--title", "E", "--date", "2026-10-16"]);
    let o = run(&["fetch-dem", &empty, "--mirror", &recorded_mirror()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("import the river"), "{}", err(&o));
}
