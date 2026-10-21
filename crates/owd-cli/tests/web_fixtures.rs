//! The fixtures the web parity check and the web app's example run on (spec 0011, ticket T3), in
//! `examples/web/`: the synthetic river's terrain as a GeoTIFF, and two documents that have run
//! every stage and then had the draught changed, so every later stage is stale and `owd run` (and the
//! wasm session) must run it again.
//!
//! (A debug build runs the pool search far too slowly for the `test` job, so the test below checks only
//! that the files hold together; `tools/web_parity.mjs` runs them.)
//!
//! Rebuild them (they are committed, so a plain test run changes no file):
//!
//! ```text
//! cargo test --release -p owd-cli --test web_fixtures -- --ignored write_the_web_fixtures
//! ```
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, synthetic_river};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use tiff::encoder::compression::DeflateLevel;
use tiff::encoder::{Compression, TiffEncoder, colortype};
use tiff::tags::Tag;

const SHIFT: Pt = [500_000.0, 600_000.0];
const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A short river, so the pool search stays under a minute in a browser.
fn river() -> owd_core::synthetic::SyntheticRiver {
    synthetic_river(RiverParams {
        length_m: 2400.0,
        ..RiverParams::default()
    })
    .unwrap()
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

fn owd(args: &[&str]) {
    let o = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs");
    // Exit 1 means a check failed (the design is still written): fine for a fixture.
    let code = o.status.code().expect("exited normally");
    assert!(
        code == 0 || code == 1,
        "owd {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The synthetic river's terrain as a deflated float32 GeoTIFF in UTM 30N, 10 m pixels.
fn write_dem(path: &Path) {
    let river = river();
    let (x0, y1, px) = (SHIFT[0] - 200.0, SHIFT[1] + 600.0, 10.0);
    let (w, h) = (280usize, 120usize);
    let data: Vec<f32> = (0..h)
        .flat_map(|r| (0..w).map(move |c| (r, c)))
        .map(|(r, c)| {
            let (x, y) = (x0 + (c as f64 + 0.5) * px, y1 - (r as f64 + 0.5) * px);
            river.terrain(x - SHIFT[0], y - SHIFT[1]) as f32
        })
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut enc = TiffEncoder::new(file)
        .unwrap()
        .with_compression(Compression::Deflate(DeflateLevel::Best));
    let mut img = enc
        .new_image::<colortype::Gray32Float>(w as u32, h as u32)
        .unwrap();
    img.encoder()
        .write_tag(Tag::Unknown(33550), &[px, px, 0.0][..])
        .unwrap();
    img.encoder()
        .write_tag(Tag::Unknown(33922), &[0.0, 0.0, 0.0, x0, y1, 0.0][..])
        .unwrap();
    // GeoKeyDirectory: header, then projected CRS (model 1), pixel-is-area, EPSG 32630.
    let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 32630];
    img.encoder()
        .write_tag(Tag::Unknown(34735), &keys[..])
        .unwrap();
    img.write_data(&data).unwrap();
}

fn river_inputs(dir: &Path) -> (String, String) {
    let river = river();
    let shift = |p: &Pt| [p[0] + SHIFT[0], p[1] + SHIFT[1]];
    let centre: Vec<Pt> = river.centreline.iter().map(shift).collect();
    let ring: Vec<Pt> = river.polygon.exterior.iter().map(shift).collect();
    let c = dir.join("centreline.geojson");
    let p = dir.join("polygon.geojson");
    std::fs::write(
        &c,
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(centre)
        ),
    )
    .unwrap();
    std::fs::write(
        &p,
        format!(
            r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
            json!([ring])
        ),
    )
    .unwrap();
    (c.to_str().unwrap().into(), p.to_str().unwrap().into())
}

/// One document: the river and DEM, sizing, `steps`, then a changed draught so what ran is stale.
fn build(dir: &Path, name: &str, title: &str, steps: &[&[&str]], stale: bool) {
    let (c, p) = river_inputs(dir);
    let doc = dir.join(name).to_str().unwrap().to_string();
    let dem = dir.join("river-dem.tif").to_str().unwrap().to_string();
    let _ = std::fs::remove_file(&doc);
    owd(&["new", &doc, "--title", title, "--date", "2026-10-21"]);
    owd(&[
        "import",
        &doc,
        "--centreline",
        &c,
        "--polygon",
        &p,
        "--dem",
        &dem,
        "--water-level",
        "0.0",
        "--water-slope",
        "0.0",
    ]);
    owd(&[
        "size",
        &doc,
        "--ruleset",
        "PIANC-B",
        "--vessel",
        "CEMT_IV",
        "--draught",
        "2.5",
    ]);
    for s in steps {
        let mut args = vec![s[0], doc.as_str()];
        args.extend(
            s[1..]
                .iter()
                .map(|a| if *a == "@dem" { dem.as_str() } else { a }),
        );
        owd(&args);
    }
    if stale {
        owd(&[
            "size",
            &doc,
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.6",
        ]);
    }
}

#[test]
#[ignore = "writes examples/web: run on purpose, see the module docs"]
fn write_the_web_fixtures() {
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).unwrap();
    write_dem(&dir.join("river-dem.tif"));
    build(
        &dir,
        "river-rolling-circle.owd",
        "Synthetic river: rolling circle, pools, volumes",
        &[
            &["align", "--method", "rolling-circle"],
            &["optimise", "--sites", "1200"],
            &["volumes"],
        ],
        true,
    );
    build(
        &dir,
        "river-reroute.owd",
        "Synthetic river: least-cost re-route, volumes",
        &[
            &[
                "align",
                "--method",
                "reroute",
                "--dem",
                "@dem",
                "--water-level",
                "0.0",
                "--water-slope",
                "0.0",
            ],
            &["volumes"],
        ],
        true,
    );
    // The example the web app opens: every stage is fresh, so a visitor sees results at once.
    build(
        &dir,
        "river-example.owd",
        "Synthetic river: the example",
        &[&["align", "--method", "rolling-circle"], &["volumes"]],
        false,
    );
    for f in ["centreline.geojson", "polygon.geojson"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_web_fixtures_hold_the_dem_they_name_and_validate() {
    let dir = fixtures_dir();
    let dem = std::fs::read(dir.join("river-dem.tif")).expect("run write_the_web_fixtures");
    let want: String = Sha256::digest(&dem)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    for name in [
        "river-rolling-circle.owd",
        "river-reroute.owd",
        "river-example.owd",
    ] {
        let doc = read(&dir.join(name));
        assert_eq!(doc["inputs"]["dem"]["path"], "river-dem.tif", "{name}");
        assert_eq!(doc["inputs"]["dem"]["sha256"], want, "{name}: DEM hash");
        let status = Command::new(env!("CARGO_BIN_EXE_owd"))
            .args(["validate", dir.join(name).to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success(), "{name} validates");
    }
}
