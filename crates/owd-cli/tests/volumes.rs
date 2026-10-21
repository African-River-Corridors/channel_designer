//! `owd import --dem` and `owd volumes` (spec 0002, ticket 62), on the synthetic river and
//! a GeoTIFF DEM written from its terrain: the numbers equal the core called directly on the same
//! DEM, a changed DEM warns and marks `volumes` stale, and a missing DEM names its path and refuses.
//!
//! Label: property

use owd_core::cut_model::{WIDEN_RMAX, widened_hb_from_shape};
use owd_core::geom::{Polygon, Pt};
use owd_core::rules::rulesets::{RulesetRegistry, channel_shape};
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_core::volumes::sections::{HalfWidth, Level, section_volumes};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

const SHIFT: Pt = [500_000.0, 600_000.0];
const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

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

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-vol-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The synthetic river's terrain as a float32 GeoTIFF in UTM 30N, 10 m pixels, with `bump` metres
/// added so a second file differs from the first.
fn write_dem(path: &Path, bump: f32) {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let (x0, y1, px) = (SHIFT[0] - 400.0, SHIFT[1] + 900.0, 10.0);
    let (w, h) = (700usize, 180usize);
    let data: Vec<f32> = (0..h)
        .flat_map(|r| (0..w).map(move |c| (r, c)))
        .map(|(r, c)| {
            let (x, y) = (x0 + (c as f64 + 0.5) * px, y1 - (r as f64 + 0.5) * px);
            river.terrain(x - SHIFT[0], y - SHIFT[1]) as f32 + bump
        })
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut enc = TiffEncoder::new(file).unwrap();
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
    let river = synthetic_river(RiverParams::default()).unwrap();
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

/// A document with the river, a DEM next to it, sizing and the rolling circle. Returns the paths.
fn build(dir: &Path) -> (String, PathBuf) {
    let (c, p) = river_inputs(dir);
    let doc = dir.join("river.cdesign").to_str().unwrap().to_string();
    let dem = dir.join("river-dem.tif");
    write_dem(&dem, 0.0);
    let steps: [&[&str]; 4] = [
        &["new", &doc, "--title", "River", "--date", "2026-10-16"],
        &[
            "import",
            &doc,
            "--centreline",
            &c,
            "--polygon",
            &p,
            "--dem",
            dem.to_str().unwrap(),
        ],
        &[
            "size",
            &doc,
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
        ],
        &["align", &doc, "--method", "rolling-circle"],
    ];
    for s in steps {
        let o = run(s);
        assert!(
            code(&o) == 0 || (s[0] == "align" && code(&o) == 1),
            "{s:?}: {}",
            err(&o)
        );
    }
    (doc, dem)
}

fn read_doc(doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

const VOLUME_FLAGS: [&str; 6] = [
    "--model",
    "b",
    "--water-level",
    "0.0",
    "--water-slope",
    "0.0",
];

#[test]
fn import_records_the_dem_by_relative_path_hash_crs_and_nodata() {
    let dir = work_dir("ref");
    let (doc, dem) = build(&dir);
    let d = &read_doc(&doc)["inputs"]["dem"];
    assert_eq!(d["path"], "river-dem.tif", "relative to the document");
    assert_eq!(d["crs"], "EPSG:32630");
    assert!(d["nodata"].is_null());
    let bytes = std::fs::read(&dem).unwrap();
    let want: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    assert_eq!(d["sha256"], want);
    assert!(!std::fs::read_to_string(&doc).unwrap().contains("pixel"));
}

#[test]
fn volumes_equal_the_core_called_directly_on_the_same_dem() {
    let dir = work_dir("direct");
    let (doc, dem) = build(&dir);
    let mut args = vec!["volumes", &doc as &str, "--json"];
    args.extend(VOLUME_FLAGS);
    let o = run(&args);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));
    let res: Value = serde_json::from_str(&out(&o)).unwrap();
    let row = &res["options"][0];

    let saved = read_doc(&doc);
    let line: Vec<Pt> = saved["options"][0]["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect();
    let dem = owd_format::geotiff::read_file(&dem).unwrap();
    let terrain = |xs: &[f64], ys: &[f64]| dem.sample_many(xs, ys);
    let shape = channel_shape(&RulesetRegistry::new(), "PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    let ring: Vec<Pt> = synthetic_river(RiverParams::default())
        .unwrap()
        .polygon
        .exterior
        .iter()
        .map(|p| [p[0] + SHIFT[0], p[1] + SHIFT[1]])
        .collect();
    let poly = [Polygon {
        exterior: ring,
        holes: vec![],
    }];
    let wl = |_s: f64| 0.0;
    let hb = |xy: &[Pt]| widened_hb_from_shape(xy, &shape, WIDEN_RMAX);
    let direct = section_volumes(
        &line,
        &terrain,
        &poly,
        &Level::AtChainage(&wl),
        shape.design_depth_m,
        &HalfWidth::FromStations(&hb),
        "b",
        3.0,
        25.0,
        200.0,
        1.0,
        None,
        None,
        40,
    )
    .unwrap();
    assert_eq!(row["dredge_m3"].as_f64().unwrap(), direct.dredge_m3());
    assert_eq!(row["excav_m3"].as_f64().unwrap(), direct.excav_m3());
    assert_eq!(row["max_face_h_m"].as_f64().unwrap(), direct.max_face_h_m());
    assert!(direct.dredge_m3() > 0.0);
    assert_eq!(saved["stages"]["volumes"]["results"]["options"][0], *row);
    let st = run(&["status", &doc, "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&out(&st)).unwrap()["stages"]["volumes"],
        "fresh"
    );
}

#[test]
fn a_changed_dem_warns_on_open_and_marks_volumes_stale() {
    let dir = work_dir("changed");
    let (doc, dem) = build(&dir);
    let mut args = vec!["volumes", &doc as &str];
    args.extend(VOLUME_FLAGS);
    run(&args);
    write_dem(&dem, 0.5);
    let st = run(&["status", &doc, "--json"]);
    let v: Value = serde_json::from_str(&out(&st)).unwrap();
    assert_eq!(v["stages"]["volumes"], "stale");
    assert_eq!(v["stale"], json!(["volumes"]));
    let w = v["warnings"][0].as_str().unwrap();
    assert!(w.contains("river-dem.tif") && w.contains("changed"), "{w}");
    assert!(
        err(&st).contains("warning") && err(&st).contains("changed"),
        "{}",
        err(&st)
    );
    // `run` will not read a changed DEM, and `volumes` says how to accept it.
    let o = run(&["volumes", &doc]);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("river-dem.tif") && err(&o).contains("--dem"),
        "{}",
        err(&o)
    );
    // Accepting it records the new hash and the stage is fresh again.
    let mut args = vec!["volumes", &doc as &str, "--dem", dem.to_str().unwrap()];
    args.extend(VOLUME_FLAGS);
    assert!(code(&run(&args)) <= 1);
    let st = run(&["status", &doc, "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&out(&st)).unwrap()["stages"]["volumes"],
        "fresh"
    );
}

#[test]
fn a_missing_dem_names_its_path_and_volumes_refuses_to_run() {
    let dir = work_dir("missing");
    let (doc, dem) = build(&dir);
    std::fs::remove_file(&dem).unwrap();
    let st = run(&["status", &doc]);
    assert!(
        err(&st).contains("river-dem.tif") && err(&st).contains("missing"),
        "{}",
        err(&st)
    );
    let mut args = vec!["volumes", &doc as &str];
    args.extend(VOLUME_FLAGS);
    let o = run(&args);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("river-dem.tif") && err(&o).contains("missing"),
        "{}",
        err(&o)
    );
    assert!(read_doc(&doc)["stages"]["volumes"]["results"].is_null());
}

#[test]
fn volumes_without_a_dem_says_to_import_one() {
    let dir = work_dir("nodem");
    let (c, p) = river_inputs(&dir);
    let doc = dir.join("r.cdesign").to_str().unwrap().to_string();
    run(&["new", &doc, "--title", "R", "--date", "2026-10-16"]);
    run(&["import", &doc, "--centreline", &c, "--polygon", &p]);
    run(&["size", &doc, "--ruleset", "PIANC-B", "--vessel", "CEMT_IV"]);
    run(&["align", &doc, "--method", "rolling-circle"]);
    let o = run(&["volumes", &doc]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("needs a DEM"), "{}", err(&o));
}

#[test]
fn run_reruns_a_stale_volumes_stage() {
    let dir = work_dir("run");
    let (doc, _) = build(&dir);
    let mut args = vec!["volumes", &doc as &str];
    args.extend(VOLUME_FLAGS);
    run(&args);
    run(&["align", &doc, "--method", "rolling-circle", "--step", "20"]);
    let o = run(&["run", &doc, "--json"]);
    assert!(code(&o) <= 1, "{}", err(&o));
    assert_eq!(
        serde_json::from_str::<Value>(&out(&o)).unwrap()["ran"],
        json!(["volumes"])
    );
}

// --- the optimise stage (ticket 63) ---------------------------------------------------------

#[test]
fn optimise_adds_the_lattice_option_and_volumes_measure_it() {
    let dir = work_dir("optimise");
    let (c, p) = river_inputs(&dir);
    // A short straight design line along the river's first kilometre.
    let line = [[SHIFT[0], SHIFT[1]], [SHIFT[0] + 1000.0, SHIFT[1]]];
    std::fs::write(
        &c,
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(line)
        ),
    )
    .unwrap();
    let doc = dir.join("pools.cdesign").to_str().unwrap().to_string();
    let dem = dir.join("river-dem.tif");
    write_dem(&dem, 0.0);
    for s in [
        vec!["new", &doc, "--title", "Pools", "--date", "2026-10-16"],
        vec![
            "import",
            &doc,
            "--centreline",
            &c,
            "--polygon",
            &p,
            "--dem",
            dem.to_str().unwrap(),
        ],
        vec![
            "size",
            &doc,
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
        ],
        vec!["align", &doc, "--method", "rolling-circle"],
        vec!["import", &doc, "--water-level", "0"],
    ] {
        assert_eq!(code(&run(&s)), 0, "{s:?}");
    }
    let o = run(&[
        "optimise", &doc, "--json", "--sites", "500", "--lifts", "0.5,1",
    ]);
    assert!(code(&o) <= 1, "{}", err(&o));
    let res: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(res["option"]["method"], "lattice");
    assert!(!res["pools"].as_array().unwrap().is_empty());
    let saved = read_doc(&doc);
    assert_eq!(saved["options"].as_array().unwrap().len(), 2);
    let st = run(&["status", &doc, "--json"]);
    let st: Value = serde_json::from_str(&out(&st)).unwrap();
    assert_eq!(st["stages"]["optimise"], "fresh");

    let o = run(&["volumes", &doc, "--json"]);
    assert!(code(&o) <= 1, "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    let ids: Vec<&str> = v["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["rolling-circle", "lattice"]);
    assert_eq!(v["options"][1]["water_source"], "optimise");
}

#[test]
fn optimise_without_a_water_level_says_what_it_needs() {
    let dir = work_dir("optimise-need");
    let (doc, _) = build(&dir);
    let o = run(&["optimise", &doc]);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("water level") && err(&o).contains("--water-level"),
        "{}",
        err(&o)
    );
}

// --- run and export (ticket 64) --------------------------------------------------------------

/// The chain `new -> import -> size -> align -> optimise -> volumes` on a short straight line.
fn full_chain(dir: &Path) -> String {
    let (c, p) = river_inputs(dir);
    let line = [[SHIFT[0], SHIFT[1]], [SHIFT[0] + 1000.0, SHIFT[1]]];
    std::fs::write(
        &c,
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(line)
        ),
    )
    .unwrap();
    let doc = dir.join("chain.cdesign").to_str().unwrap().to_string();
    let dem = dir.join("river-dem.tif");
    write_dem(&dem, 0.0);
    let dem = dem.to_str().unwrap();
    for s in [
        vec!["new", &doc, "--title", "Chain", "--date", "2026-10-16"],
        vec![
            "import",
            &doc,
            "--centreline",
            &c,
            "--polygon",
            &p,
            "--dem",
            dem,
        ],
        vec![
            "size",
            &doc,
            "--ruleset",
            "PIANC-B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
        ],
        vec!["align", &doc, "--method", "rolling-circle"],
        vec!["import", &doc, "--water-level", "0"],
        vec!["optimise", &doc, "--sites", "500", "--lifts", "0.5,1"],
        vec!["volumes", &doc, "--water-level", "0", "--model", "b"],
    ] {
        let o = run(&s);
        assert!(code(&o) <= 1, "{s:?}: {}", err(&o));
    }
    doc
}

#[test]
fn the_chain_gives_the_same_document_as_run_on_the_same_inputs_and_settings() {
    let dir = work_dir("chain-run");
    let doc = full_chain(&dir);
    let chain = std::fs::read_to_string(&doc).unwrap();
    // The same file with every result cleared, as when the settings are edited and nothing has run.
    let mut v: Value = serde_json::from_str(&chain).unwrap();
    for stage in ["sizing", "alignment", "optimise", "volumes"] {
        for k in ["results", "depends_on", "engine_version"] {
            v["stages"][stage][k] = Value::Null;
        }
    }
    v["options"] = json!([]);
    let again = dir.join("again.cdesign");
    std::fs::write(
        &again,
        format!("{}\n", serde_json::to_string_pretty(&v).unwrap()),
    )
    .unwrap();
    let o = run(&["run", again.to_str().unwrap(), "--json"]);
    assert!(code(&o) <= 1, "{}", err(&o));
    let ran: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(
        ran["ran"],
        json!(["sizing", "alignment", "optimise", "volumes"])
    );
    assert_eq!(std::fs::read_to_string(&again).unwrap(), chain);
}

#[test]
fn export_writes_the_quantity_table_and_the_options_with_the_screening_label() {
    let dir = work_dir("export");
    let doc = full_chain(&dir);
    let csv = run(&["export", &doc, "--csv"]);
    assert_eq!(code(&csv), 0, "{}", err(&csv));
    let text = out(&csv);
    assert_eq!(
        text.lines().next().unwrap(),
        "# Screening — not for construction"
    );
    assert!(text.lines().nth(1).unwrap().contains("failed_checks"));
    assert!(
        text.contains("\nrolling-circle,") && text.contains("\nlattice,"),
        "{text}"
    );
    assert!(
        // `volumes_status`, then the `notes` column (spec 0005: the water-aware rolling circle
        // says "width only, no depth data").
        text.lines()
            .skip(2)
            .all(|l| l.contains(",fresh,") || l.ends_with(",fresh")),
        "{text}"
    );

    let g = run(&["export", &doc, "--geojson"]);
    let fc: Value = serde_json::from_str(&out(&g)).unwrap();
    assert_eq!(fc["screening"], "Screening — not for construction");
    assert_eq!(fc["features"].as_array().unwrap().len(), 2);
    let file = dir.join("wgs84.geojson");
    let w = run(&[
        "export",
        &doc,
        "--geojson",
        "--wgs84",
        "-o",
        file.to_str().unwrap(),
    ]);
    assert_eq!(code(&w), 0, "{}", err(&w));
    let fc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let c0 = &fc["features"][0]["geometry"]["coordinates"][0];
    assert!((-3.0..=-2.9).contains(&c0[0].as_f64().unwrap()), "{c0}");

    // The export never rewrites the document, and says what it needs.
    let none = run(&["export", &doc]);
    assert_eq!(code(&none), 2);
    assert!(err(&none).contains("--geojson"), "{}", err(&none));
}
