//! `owd fetch-dem` for a reach (spec 0012, T5): by `--bbox`, and from the reach of a study that has no
//! banks yet. A tile the source does not hold is named, the DEM is marked `default` with the Copernicus
//! notice, and the web path (tiles dropped in) gives the same DEM as the native fetch. No network: the
//! mirror is a folder laid out like the public bucket.
//!
//! Label: property

use owd_format::design::Session;
use owd_format::design::dem_source::{COPERNICUS_NOTICE, TileId, write_geotiff};
use owd_format::geotiff::read_file;
use owd_reach::DroppedFiles;
use owd_reach::dem::{DemTile, dem_from_tiles, tiles_for_bbox};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-fetch-reach-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const TILE: &str = "Copernicus_DSM_COG_10_N05_00_W003_00_DEM";

fn recorded_clip() -> owd_format::geotiff::Dem {
    read_file(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/mirror/{TILE}/{TILE}.tif")),
    )
    .unwrap()
}

fn recorded_mirror() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mirror")
        .to_str()
        .unwrap()
        .to_string()
}

/// A document with a reach and nothing else (no banks, no CRS).
fn reach_doc(dir: &Path, start: [f64; 2], end: [f64; 2]) -> String {
    let mut s = Session::new("Reach", "2026-10-21");
    s.set_reach(json!({
        "river": null, "start": start, "end": end, "via": [], "trace": null,
        "route": [{"kind": "waterway", "way_ids": [1]}]
    }))
    .unwrap();
    let doc = dir.join("reach.owd");
    std::fs::write(&doc, s.to_text()).unwrap();
    doc.to_str().unwrap().to_string()
}

fn read_doc(doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

#[test]
fn a_box_makes_a_dem_marked_default_with_the_copernicus_notice() {
    let dir = work_dir("bbox");
    let doc = dir.join("study.owd").to_str().unwrap().to_string();
    assert_eq!(
        run(&["new", &doc, "--title", "Box", "--date", "2026-10-21"])
            .status
            .code(),
        Some(0)
    );
    let o = run(&[
        "fetch-dem",
        &doc,
        "--bbox",
        "-2.58,5.50,-2.56,5.52",
        "--margin",
        "0",
        "--mirror",
        &recorded_mirror(),
        "--json",
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let d = read_doc(&doc);
    assert_eq!(d["inputs"]["dem"]["source"]["tiles"][0]["name"], TILE);
    assert_eq!(d["provenance"]["dem"]["source"], "cop-dem-glo30");
    assert_eq!(d["provenance"]["dem"]["mark"], "default");
    assert_eq!(d["provenance"]["dem"]["notice"], COPERNICUS_NOTICE);
    assert_eq!(run(&["validate", &doc]).status.code(), Some(0));
    // A bad box is a usage error.
    let o = run(&[
        "fetch-dem",
        &doc,
        "--bbox",
        "5,5,4,4",
        "--mirror",
        &recorded_mirror(),
    ]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_study_with_a_reach_and_no_banks_fetches_for_its_reach() {
    let dir = work_dir("reach");
    let doc = reach_doc(&dir, [-2.58, 5.50], [-2.56, 5.52]);
    let o = run(&["fetch-dem", &doc, "--mirror", &recorded_mirror()]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let d = read_doc(&doc);
    assert_eq!(d["inputs"]["dem"]["source"]["tiles"][0]["name"], TILE);
    // The reach is still there, and the document still validates.
    assert_eq!(d["reach"]["start"], json!([-2.58, 5.50]));
    assert_eq!(run(&["validate", &doc]).status.code(), Some(0));
}

#[test]
fn tiles_the_source_withholds_are_all_named_and_nothing_is_changed() {
    let dir = work_dir("withheld");
    // A reach over four tiles; the mirror holds only one of them.
    let doc = reach_doc(&dir, [-3.01, 4.99], [-2.99, 5.01]);
    let before = std::fs::read_to_string(&doc).unwrap();
    let o = run(&[
        "fetch-dem",
        &doc,
        "--margin",
        "0",
        "--mirror",
        &recorded_mirror(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    let e = err(&o);
    for n in ["N04_00_W004", "N04_00_W003", "N05_00_W004"] {
        assert!(e.contains(n), "{n} not named in: {e}");
    }
    assert!(
        !e.contains(&format!("{TILE},")) && !e.contains(&format!(", {TILE}")),
        "the tile that exists is named as withheld: {e}"
    );
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), before);
}

#[test]
fn the_web_path_gives_the_same_dem_as_the_native_fetch() {
    let dir = work_dir("parity");
    let mirror = dir.join("mirror");
    let clip = recorded_clip();
    let mut west = clip.clone();
    west.geotransform[2] -= clip.width as f64 * clip.geotransform[0];
    let tiles = [
        (TileId { lon: -3, lat: 5 }, &clip),
        (TileId { lon: -4, lat: 5 }, &west),
    ];
    let mut dropped = DroppedFiles::new();
    for (id, dem) in tiles {
        let n = id.name();
        let bytes = write_geotiff(dem).unwrap();
        std::fs::create_dir_all(mirror.join(&n)).unwrap();
        std::fs::write(mirror.join(&n).join(format!("{n}.tif")), &bytes).unwrap();
        dropped = dropped.with_file(&format!("{n}.tif"), bytes);
    }
    let doc = reach_doc(&dir, [-3.01, 5.50], [-2.99, 5.52]);
    let o = run(&[
        "fetch-dem",
        &doc,
        "--margin",
        "0",
        "--mirror",
        mirror.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let native = std::fs::read(dir.join("reach-dem.tif")).unwrap();
    // The web: the page lists the tiles as links, the user drops them in, owd-reach makes the DEM.
    let want = tiles_for_bbox([-3.01, 5.50, -2.99, 5.52], 0.0);
    assert_eq!(want.iter().map(|t| t.name()).collect::<Vec<_>>().len(), 2);
    let tiles: Vec<DemTile> = want;
    let web = write_geotiff(&dem_from_tiles(&dropped, &tiles).unwrap()).unwrap();
    assert_eq!(web, native);
}
