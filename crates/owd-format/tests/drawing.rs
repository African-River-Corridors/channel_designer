//! Label: property
//! The cross-section gallery (spec 0004, ticket 1) through its public seam, `drawing::sheets_from_text`,
//! `drawing::sheets`, `drawing::render` and `drawing::check`, on the web example (the synthetic river,
//! its DEM, every stage fresh): `otd` accepts every drawing document, every drawn value equals the station
//! walk's result, the same design gives the same bytes, and a drawn value that differs is refused with
//! the sheet and the check named.
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, Session, Stage, Terrain};
use owd_format::drawing::{self, DrawError, DrawOptions, Sheet, check};
use owd_format::geotiff;
use serde_json::Value;
use std::path::PathBuf;

fn web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

fn example() -> (String, Vec<u8>) {
    (
        std::fs::read_to_string(web().join("river-example.owd")).unwrap(),
        std::fs::read(web().join("river-dem.tif")).unwrap(),
    )
}

fn gallery() -> Vec<Sheet> {
    let (text, dem) = example();
    drawing::sheets_from_text(&text, Some(&dem), &DrawOptions::default()).unwrap()
}

/// The station walk of the example, called directly on the same DEM.
fn walk() -> owd_core::volumes::sections::SectionVolumes {
    let (text, dem) = example();
    let s = Session::open(&text).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.option_section_walk("rolling-circle", &t as Terrain)
        .unwrap()
}

fn viewports(sheet: &Sheet) -> Vec<Value> {
    sheet.document["viewports"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["id"].as_str().unwrap().starts_with("xs-"))
        .cloned()
        .collect()
}

#[test]
fn otd_accepts_every_document_and_makes_every_output() {
    let sheets = gallery();
    assert!(!sheets.is_empty());
    for sheet in &sheets {
        let out = drawing::render(sheet).expect("the standard's schema and checks pass");
        assert!(out.pdf.starts_with(b"%PDF"));
        assert!(out.svg.contains("<svg"));
        assert!(out.dxf.contains("SECTION"));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let norm: Value = serde_json::from_str(&out.normalised).unwrap();
        assert!(norm["sheet"]["size"] == "A3");
        if let Ok(dir) = std::env::var("OWD_DRAW_DUMP") {
            std::fs::create_dir_all(&dir).unwrap();
            for (name, data) in out.files() {
                std::fs::write(PathBuf::from(&dir).join(name), data).unwrap();
            }
        }
    }
}

#[test]
fn drawn_values_equal_the_walk_s_results() {
    let sheets = gallery();
    assert!(check::gallery(&sheets, &walk()).is_empty());
}

#[test]
fn sections_are_every_500_m_and_the_ground_is_the_walk_s_profile() {
    let w = walk();
    let sheets = gallery();
    let drawn: Vec<f64> = sheets
        .iter()
        .flat_map(viewports)
        .map(|v| {
            let id = v["id"].as_str().unwrap();
            let t = v["entities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == format!("{id}-chainage"))
                .unwrap()["text"]
                .as_str()
                .unwrap()
                .to_string();
            let (km, m) = t.strip_prefix("CH ").unwrap().split_once('+').unwrap();
            km.parse::<f64>().unwrap() * 1000.0 + m.parse::<f64>().unwrap()
        })
        .collect();
    let last = w.stations.last().unwrap().chainage_m;
    let expected = (last / 500.0).floor() as usize + 1;
    assert_eq!(drawn.len(), expected, "one section per 500 m: {drawn:?}");
    for (i, got) in drawn.iter().enumerate() {
        let near = w
            .stations
            .iter()
            .map(|s| (s.chainage_m - i as f64 * 500.0).abs())
            .fold(f64::INFINITY, f64::min);
        let station = w
            .stations
            .iter()
            .find(|s| (s.chainage_m - got).abs() < 0.06)
            .unwrap();
        assert!(
            ((station.chainage_m - i as f64 * 500.0).abs() - near).abs() < 1e-9,
            "section {i} is the station nearest to {} m",
            i * 500
        );
    }
}

#[test]
fn a_wrong_drawn_value_is_refused_naming_the_sheet_and_the_check() {
    let w = walk();
    let mut sheets = gallery();
    let number = sheets[0].number.clone();
    // The first section's dredge area is drawn one square metre too small.
    let vp = sheets[0].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap();
    let e = vp
        .iter_mut()
        .find(|e| e["id"] == "xs-001-dredge")
        .expect("section 1 draws its dredge area");
    let area = w.stations[0].dredge_area_m2 - 1.0;
    e["text"] = Value::String(format!("DREDGE {area:.1} M²"));
    let found = check::gallery(&sheets, &w);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].sheet, number);
    assert_eq!(found[0].check, "section-area");
    assert!(found[0].to_string().contains(&number), "{}", found[0]);
}

#[test]
fn a_moved_ground_point_and_a_wrong_water_level_are_refused() {
    let w = walk();
    let mut sheets = gallery();
    {
        let entities = sheets[0].document["viewports"][0]["entities"]
            .as_array_mut()
            .unwrap();
        let ground = entities
            .iter_mut()
            .find(|e| e["id"] == "xs-001-ground-1")
            .unwrap();
        let y = ground["points_m"][3][1].as_f64().unwrap();
        ground["points_m"][3][1] = serde_json::json!(y + 0.5);
        let wl = entities
            .iter_mut()
            .find(|e| e["id"] == "xs-001-wl-line")
            .unwrap();
        wl["to_m"][1] = serde_json::json!(w.stations[0].wl + 1.0);
    }
    let checks: Vec<&str> = check::gallery(&sheets, &w)
        .iter()
        .map(|f| f.check)
        .collect();
    assert_eq!(checks, ["section-profile", "section-level"]);
}

#[test]
fn the_same_design_gives_the_same_bytes_and_numbers_follow_the_scheme() {
    let (text, dem) = example();
    let a = drawing::files_from_text(&text, Some(&dem), &DrawOptions::default()).unwrap();
    let b = drawing::files_from_text(&text, Some(&dem), &DrawOptions::default()).unwrap();
    assert_eq!(a, b);
    for sheet in gallery() {
        let n = sheet.number;
        assert!(n.starts_with("OWD-") && n.contains("-XS-"), "{n}");
        assert_eq!(
            sheet.document["title_block"]["fields"]["document_number"],
            n
        );
        assert_eq!(sheet.document["status"], "concept");
        assert!(
            sheet.document["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x.as_str().unwrap().starts_with("SCREENING"))
        );
    }
}

#[test]
fn the_title_block_names_the_design_the_option_and_the_revision() {
    let s = &gallery()[0];
    let f = &s.document["title_block"]["fields"];
    assert!(f["owner"].as_str().unwrap().contains("SYNTHETIC RIVER"));
    assert!(
        f["supplementary_title"]
            .as_str()
            .unwrap()
            .ends_with("ROLLING CIRCLE")
    );
    assert_eq!(s.document["revisions"]["entries"][0]["rev"], "A");
    assert_eq!(s.document["revisions"]["entries"][0]["date"], "2026-10-21");
    let opts = DrawOptions {
        revision: "B".into(),
        date: Some("2026-11-01".into()),
        ..DrawOptions::default()
    };
    let (text, dem) = example();
    let b = drawing::sheets_from_text(&text, Some(&dem), &opts).unwrap();
    assert_eq!(b[0].document["revisions"]["entries"][0]["rev"], "B");
    assert_eq!(
        b[0].document["revisions"]["entries"][0]["date"],
        "2026-11-01"
    );
}

#[test]
fn the_design_must_be_ready_and_the_dem_must_be_there() {
    let (text, dem) = example();
    let o = DrawOptions::default();
    let e = drawing::sheets_from_text(&text, None, &o)
        .unwrap_err()
        .to_string();
    assert!(e.contains("is missing"), "{e}");
    let mut changed = dem.clone();
    changed[200] ^= 1;
    let e = drawing::sheets_from_text(&text, Some(&changed), &o)
        .unwrap_err()
        .to_string();
    assert!(e.contains("has changed"), "{e}");
    let e = drawing::sheets_from_text(
        &text,
        Some(&dem),
        &DrawOptions {
            option: Some("nope".into()),
            ..DrawOptions::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("no design line option `nope`"), "{e}");
    // A design whose volumes are stale is not drawn.
    let mut s = Session::open(&text).unwrap();
    s.set_water_profile(&[[0.0, 1.0]]).unwrap();
    let e = drawing::sheets_from_text(&s.to_text(), Some(&dem), &o).unwrap_err();
    assert!(
        matches!(e, DrawError::Input(_)) && e.to_string().contains("stale"),
        "{e}"
    );
}

#[test]
fn a_tight_bend_adds_a_section_at_its_middle() {
    let (text, dem) = example();
    let mut s = Session::open(&text).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    // A larger vessel class needs a larger radius than the river's bends give.
    s.set_sizing("PIANC-B", "CEMT_Va", None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as Terrain))
        .unwrap();
    let map = s.map_geometry().unwrap();
    let bends: Vec<f64> = map["options"][0]["tight_bends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["chainage_m"].as_f64().unwrap())
        .collect();
    assert!(
        !bends.is_empty(),
        "the example needs a tight bend for this test"
    );
    let sheets = drawing::sheets(&s, &t as Terrain, &DrawOptions::default()).unwrap();
    let drawn: Vec<Value> = sheets.iter().flat_map(viewports).collect();
    let tagged = drawn
        .iter()
        .filter(|v| {
            v["entities"].as_array().unwrap().iter().any(|e| {
                e["id"].as_str().is_some_and(|i| i.ends_with("-title"))
                    && e["text"].as_str().unwrap().ends_with("MID-BEND")
            })
        })
        .count();
    assert_eq!(tagged, bends.len(), "one mid-bend section per bend");
}
