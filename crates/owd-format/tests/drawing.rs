//! Label: property
//! The cross-section gallery (spec 0004, ticket 1) through its public seam, `drawing::sheets_from_text`,
//! `drawing::sheets`, `drawing::render` and `drawing::check`, on the web example (the synthetic river,
//! its DEM, every stage fresh): `otd` accepts every drawing document, every drawn value equals the station
//! walk's result, the same design gives the same bytes, and a drawn value that differs is refused with
//! the sheet and the check named.
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, Session, Stage, Terrain};
use owd_format::drawing::{self, DrawError, DrawOptions, Sheet, SheetKind, check};
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
    let o = DrawOptions {
        sheets: vec![SheetKind::CrossSections],
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
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
    // the spacing sections alone: no extra-section rule (bend middles, worst stations)
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::CrossSections],
        style: owd_format::drawing::Style::builtin()
            .with_overrides(&[("sections.extra".to_string(), serde_json::json!([]))])
            .unwrap(),
        ..DrawOptions::default()
    };
    let sheets = drawing::sheets_from_text(&text, Some(&dem), &o).unwrap();
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

// ---- the sheet style (spec 0004, ticket 2) ----------------------------------------------------

use owd_format::drawing::Style;
use serde_json::json;

fn drawn_with(style: Style) -> Vec<Sheet> {
    let (text, dem) = example();
    let o = DrawOptions {
        style,
        sheets: vec![SheetKind::CrossSections],
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
}

fn style_note(sheet: &Sheet) -> String {
    sheet.document["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .find(|n| n.starts_with("SHEET STYLE "))
        .expect("every sheet names its style")
}

fn drawn_sections(sheets: &[Sheet]) -> usize {
    sheets.iter().flat_map(viewports).count()
}

#[test]
fn the_built_in_style_is_valid_and_holds_the_defaults() {
    let s = Style::builtin();
    assert!(
        Style::from_overlay(s.value()).is_ok(),
        "the default passes its own schema"
    );
    let g = s.gallery();
    assert_eq!(
        (g.section_spacing_m, g.min_columns, g.sheet_size.as_str()),
        (500.0, 2, "A3")
    );
    assert!(g.bend_sections && g.grid && g.hatch && g.water);
    // A partial file that repeats a default changes nothing, so it has the same hash.
    let same = Style::from_overlay(&json!({"sections": {"spacing_m": 500}})).unwrap();
    assert_eq!(same.hash(), s.hash());
}

#[test]
fn an_invalid_style_is_refused_with_the_field_named() {
    let e = Style::from_text(r#"{"gallery": {"min_columns": 0}}"#)
        .unwrap_err()
        .to_string();
    assert!(e.contains("/gallery/min_columns"), "{e}");
    let e = Style::from_text(r#"{"sections": {"spacing_m": "wide"}}"#)
        .unwrap_err()
        .to_string();
    assert!(e.contains("/sections/spacing_m"), "{e}");
    let e = Style::from_text(r#"{"gallery": {"scales": [300]}}"#)
        .unwrap_err()
        .to_string();
    assert!(e.contains("/gallery/scales"), "{e}");
    let e = Style::from_text(r#"{"nonsense": 1}"#)
        .unwrap_err()
        .to_string();
    assert!(e.contains("nonsense"), "{e}");
    assert!(
        Style::from_text("not json")
            .unwrap_err()
            .to_string()
            .contains("not JSON")
    );
    let e = Style::builtin()
        .with_overrides(&[("sections.nope".to_string(), json!(1))])
        .unwrap_err()
        .to_string();
    assert!(e.contains("`sections.nope` is not a member"), "{e}");
    let e = Style::builtin()
        .with_overrides(&[("sheets.xs.size".to_string(), json!("A9"))])
        .unwrap_err()
        .to_string();
    assert!(e.contains("/sheets/xs/size"), "{e}");
}

#[test]
fn one_changed_parameter_changes_the_hash_printed_on_the_sheet() {
    let base = Style::builtin();
    let finer = base
        .clone()
        .with_overrides(&[("sections.spacing_m".to_string(), json!(250))])
        .unwrap();
    assert_ne!(base.hash(), finer.hash());
    let a = drawn_with(base.clone());
    let b = drawn_with(finer.clone());
    assert!(
        style_note(&a[0]).ends_with(&base.hash()),
        "{}",
        style_note(&a[0])
    );
    assert!(
        style_note(&b[0]).ends_with(&finer.hash()),
        "{}",
        style_note(&b[0])
    );
    assert_ne!(style_note(&a[0]), style_note(&b[0]));
    // The hash is of the style after the overrides, so an override equals the same value in a file.
    let from_file = Style::from_overlay(&json!({"sections": {"spacing_m": 250}})).unwrap();
    assert_eq!(from_file.hash(), finer.hash());
    // The same style gives the same bytes.
    assert_eq!(drawn_with(finer.clone()), b);
}

#[test]
fn the_gallery_reads_its_spacing_and_extra_sections_from_the_style() {
    let base = drawn_sections(&drawn_with(Style::builtin()));
    let finer = Style::builtin()
        .with_overrides(&[("sections.spacing_m".to_string(), json!(250))])
        .unwrap();
    let n = drawn_sections(&drawn_with(finer));
    assert!(
        n > base,
        "250 m gives more sections than 500 m: {n} and {base}"
    );
    // Taking the bend rule out removes the mid-bend sections.
    let (text, dem) = example();
    let mut s = Session::open(&text).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.set_sizing("PIANC-B", "CEMT_Va", None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as Terrain))
        .unwrap();
    let count_bends = |style: Style| {
        let o = DrawOptions {
            style,
            ..DrawOptions::default()
        };
        drawing::sheets(&s, &t as Terrain, &o)
            .unwrap()
            .iter()
            .flat_map(viewports)
            .filter(|v| v["entities"].to_string().contains("MID-BEND"))
            .count()
    };
    assert!(count_bends(Style::builtin()) > 0);
    let off = Style::from_overlay(&json!({"sections": {"extra": []}})).unwrap();
    assert_eq!(count_bends(off), 0);
}

#[test]
fn a_layer_turned_off_leaves_the_sheet_without_it() {
    let off = Style::from_overlay(&json!({"layers": {"GRID": false, "HATCH": false}})).unwrap();
    for sheet in drawn_with(off) {
        let doc = sheet.document.to_string();
        assert!(!doc.contains("\"GRID\"") && !doc.contains("\"HATCH\""));
        assert!(drawing::render(&sheet).is_ok());
    }
    assert!(
        drawn_with(Style::builtin())[0]
            .document
            .to_string()
            .contains("\"HATCH\"")
    );
}

#[test]
fn the_style_sets_the_sheet_size_and_the_scale() {
    let a4 = Style::from_overlay(
        &json!({"sheets": {"xs": {"size": "A2"}}, "gallery": {"scales": [2000]}}),
    )
    .unwrap();
    for sheet in drawn_with(a4) {
        assert_eq!(sheet.document["sheet"]["size"], "A2");
        assert!(viewports(&sheet).iter().all(|v| v["scale"] == 2000));
        assert!(drawing::render(&sheet).is_ok());
    }
    // A scale at which the panels do not fit is refused, with the style named.
    let tight = Style::from_overlay(&json!({"gallery": {"scales": [200]}})).unwrap();
    let (text, dem) = example();
    let o = DrawOptions {
        style: tight,
        ..DrawOptions::default()
    };
    let e = drawing::sheets_from_text(&text, Some(&dem), &o)
        .unwrap_err()
        .to_string();
    assert!(e.contains("sheet style"), "{e}");
}

#[test]
fn the_gallery_builder_holds_no_threshold_that_the_style_does_not_set() {
    // A search of the builder: the only numeric constants left are the standard's text heights, the
    // gap between two heading items and the model-space stride of the panels (no design choice).
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/drawing/gallery.rs"),
    )
    .unwrap();
    let consts: Vec<&str> = src
        .lines()
        .filter(|l| l.trim_start().starts_with("const ") || l.contains(" const "))
        .filter(|l| l.contains(": f64") || l.contains(": usize") || l.contains(": u32"))
        .map(|l| {
            l.split(':')
                .next()
                .unwrap()
                .trim()
                .trim_start_matches("pub(super) ")
                .trim_start_matches("const ")
        })
        .collect();
    assert_eq!(
        consts,
        ["H_TITLE", "H_TEXT", "ITEM_GAP_MM", "PANEL_STRIDE_M"],
        "{consts:?}"
    );
    assert!(!src.contains("impl Default for GalleryParams"));
}

// ---- the typical section (spec 0004, ticket 3) --------------------------------------------------

fn typical_sheets() -> Vec<Sheet> {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::TypicalSection],
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
}

fn typical_facts() -> owd_format::drawing::Facts {
    let (text, _) = example();
    owd_format::drawing::typical_facts(&Session::open(&text).unwrap()).unwrap()
}

fn entity_of<'a>(sheet: &'a Sheet, viewport: &str, id: &str) -> &'a Value {
    sheet.document["viewports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == viewport)
        .unwrap()["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id}"))
}

#[test]
fn the_typical_section_is_accepted_and_its_numbers_follow_the_scheme() {
    let sheets = typical_sheets();
    assert_eq!(sheets.len(), 1, "one in all, until the reaches land");
    let s = &sheets[0];
    assert!(s.number.contains("-TS-001"), "{}", s.number);
    let out = drawing::render(s).expect("otd accepts it");
    // no DXF warning; the layout review's warnings (a label in the water) are allowed
    assert!(
        out.warnings.iter().all(|w| w.starts_with("warning `")),
        "{:?}",
        out.warnings
    );
    let norm: Value = serde_json::from_str(&out.normalised).unwrap();
    assert_eq!(s.document["status"], "concept");
    assert!(norm["sheet"]["size"].is_string());
}

#[test]
fn every_dimension_value_equals_the_design_s_result_and_the_standard_prints_it() {
    let f = typical_facts();
    let s = &typical_sheets()[0];
    let len = |vp: &str, id: &str, axis: usize| {
        let e = entity_of(s, vp, id);
        (e["to"]["point_m"][axis].as_f64().unwrap() - e["from"]["point_m"][axis].as_f64().unwrap())
            .abs()
    };
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(close(len("ts", "ts-width", 0), f.bottom_width_m));
    assert!(close(len("ts", "ts-depth", 1), f.design_depth_m));
    assert!(close(
        len("ts-detail", "ts-ukc", 1),
        f.design_depth_m - f.draught_m
    ));
    // The standard computes the text of a dimension from its geometry: read it back.
    let out = drawing::render(s).unwrap();
    let norm: Value = serde_json::from_str(&out.normalised).unwrap();
    let texts: Vec<String> = norm["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "text")
        .filter_map(|e| e["text"].as_str().map(str::to_string))
        .collect();
    for want in [
        format!("{:.2}", f.bottom_width_m),
        format!("{:.2}", f.design_depth_m),
        format!("{:.2}", f.design_depth_m - f.draught_m),
    ] {
        assert!(
            texts.contains(&want),
            "no dimension text {want} in {texts:?}"
        );
    }
    assert!(owd_format::drawing::check::typical(&typical_sheets(), &f).is_empty());
}

#[test]
fn each_value_has_a_note_that_names_its_source() {
    let s = &typical_sheets()[0];
    let notes: Vec<&str> = s.document["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    for (start, source) in [
        ("BOTTOM WIDTH:", "RULESET PIANC-WG141@0.3.0 (SE_QUALITY B)"),
        ("DESIGN DEPTH:", "RULESET PIANC-WG141@0.3.0 (SE_QUALITY B)"),
        (
            "UNDER-KEEL CLEARANCE:",
            "RULESET PIANC-WG141@0.3.0 (SE_QUALITY B)",
        ),
        ("SIDE SLOPE:", "VOLUMES"),
        ("DESIGN WATER LEVEL:", "WATER PROFILE"),
    ] {
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with(start) && n.contains(source)),
            "no note `{start}` naming `{source}` in {notes:?}"
        );
    }
}

#[test]
fn a_wrong_dimension_slope_or_missing_note_is_refused_naming_the_check() {
    let f = typical_facts();
    let good = typical_sheets();
    // The bottom width dimension points one metre too far.
    let mut s = good.clone();
    let vp = s[0].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap();
    let e = vp.iter_mut().find(|e| e["id"] == "ts-width").unwrap();
    let x = e["to"]["point_m"][0].as_f64().unwrap();
    e["to"]["point_m"][0] = serde_json::json!(x + 1.0);
    let found = owd_format::drawing::check::typical(&s, &f);
    assert!(
        found
            .iter()
            .any(|x| x.check == "typical-dimension" && x.sheet == s[0].number),
        "{found:?}"
    );
    // The slope label says another slope.
    let mut s = good.clone();
    let vp = s[0].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap();
    let e = vp.iter_mut().find(|e| e["id"] == "ts-slope").unwrap();
    e["text"] = Value::String("SIDE SLOPE 2.0:1 (H:V)".into());
    let found = owd_format::drawing::check::typical(&s, &f);
    assert!(
        found.iter().any(|x| x.check == "typical-slope"),
        "{found:?}"
    );
    // A note that names no source.
    let mut s = good;
    let notes = s[0].document["notes"].as_array_mut().unwrap();
    notes.retain(|n| !n.as_str().unwrap().starts_with("UNDER-KEEL"));
    let found = owd_format::drawing::check::typical(&s, &f);
    assert!(
        found.iter().any(|x| x.check == "typical-source"),
        "{found:?}"
    );
}

#[test]
fn a_typical_section_that_cannot_be_dimensioned_at_any_scale_says_so() {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::TypicalSection],
        style: Style::from_overlay(&json!({"typical": {"scales": [20]}})).unwrap(),
        ..DrawOptions::default()
    };
    let e = drawing::sheets_from_text(&text, Some(&dem), &o)
        .unwrap_err()
        .to_string();
    assert!(e.contains("typical section does not fit"), "{e}");
}

// ---- the setting-out table (spec 0004, ticket 4) ------------------------------------------------

fn so_with(style: Style) -> Vec<Sheet> {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::SettingOut],
        style,
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
}

fn so_table(spacing: f64) -> drawing::SettingOutTable {
    let (text, _) = example();
    drawing::setting_out_table(&Session::open(&text).unwrap(), "rolling-circle", spacing).unwrap()
}

fn csv_of(sheets: &[Sheet]) -> String {
    let (_, bytes) = sheets
        .iter()
        .flat_map(|s| s.extra.iter())
        .find(|(n, _)| n.ends_with(".csv"))
        .expect("the first sheet carries the CSV");
    String::from_utf8(bytes.clone()).unwrap()
}

fn drawn_rows(sheets: &[Sheet], viewport: &str, prefix: &str, cols: usize) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for s in sheets {
        let Some(vp) = s.document["viewports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == viewport)
        else {
            continue;
        };
        let text = |id: String| {
            vp["entities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .map(|e| e["text"].as_str().unwrap().to_string())
        };
        let mut r = 0;
        while text(format!("{prefix}-r{r}-c0")).is_some() {
            rows.push(
                (0..cols)
                    .map(|c| text(format!("{prefix}-r{r}-c{c}")).unwrap())
                    .collect(),
            );
            r += 1;
        }
    }
    rows
}

#[test]
fn the_setting_out_sheet_is_accepted_and_holds_the_zone_and_datum() {
    let sheets = so_with(Style::builtin());
    assert!(sheets[0].number.contains("-SO-001"), "{}", sheets[0].number);
    for s in &sheets {
        let out = drawing::render(s).expect("otd accepts it");
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let header = s.document["notes"][0].as_str().unwrap();
        assert!(
            header.contains("WGS 84 / UTM ZONE 30N") && header.contains("EPSG:32630"),
            "{header}"
        );
        assert!(header.contains("0.001 M"), "{header}");
    }
}

#[test]
fn every_coordinate_chainage_and_radius_equals_the_result_and_the_csv_holds_the_same_rows() {
    let sheets = so_with(Style::builtin());
    let t = so_table(100.0);
    assert!(owd_format::drawing::check::setting_out(&sheets, &t).is_empty());
    // An independent result: the station walk's own points, every 25 m of chainage.
    let w = walk();
    let drawn = drawn_rows(&sheets, "so-stations", "so-s", 5);
    assert_eq!(drawn.len(), t.stations.len());
    for (row, st) in drawn.iter().zip(&t.stations) {
        if let Some(s) = w
            .stations
            .iter()
            .find(|s| (s.chainage_m - st.chainage_m).abs() < 1e-9)
        {
            let e: f64 = row[1].parse().unwrap();
            let n: f64 = row[2].parse().unwrap();
            assert!(
                (e - s.x).abs() <= 5.1e-4 && (n - s.y).abs() <= 5.1e-4,
                "{row:?} vs walk ({}, {})",
                s.x,
                s.y
            );
            // The walk's heading is the direction of its segment too: bearing = 90 - heading.
            let b: f64 = row[3].parse().unwrap();
            let want = (90.0 - s.heading_rad.to_degrees()).rem_euclid(360.0);
            let diff = ((b - want + 180.0).rem_euclid(360.0) - 180.0).abs();
            assert!(
                diff < 5.0,
                "bearing {b} against the walk's heading {want} at {}",
                st.chainage_m
            );
        }
    }
    // The CSV: every sheet row is in it, in order.
    let csv = csv_of(&sheets);
    let lines: Vec<Vec<&str>> = csv
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .collect();
    let stations: Vec<&Vec<&str>> = lines.iter().filter(|l| l[0] == "station").collect();
    assert_eq!(stations.len(), drawn.len());
    for (c, d) in stations.iter().zip(&drawn) {
        assert_eq!(
            (c[2], c[3], c[4]),
            (d[1].as_str(), d[2].as_str(), d[3].as_str())
        );
    }
    let bends = drawn_rows(&sheets, "so-bends", "so-b", 4);
    assert_eq!(lines.iter().filter(|l| l[0] == "bend").count(), bends.len());
    assert!(!bends.is_empty(), "the example has bends");
}

#[test]
fn a_wrong_row_or_a_csv_that_differs_is_refused_naming_the_check() {
    let t = so_table(100.0);
    let good = so_with(Style::builtin());
    let mut s = good.clone();
    let e = s[0].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == "so-s-r2-c1")
        .unwrap();
    e["text"] = Value::String("500136.625".into());
    let found = owd_format::drawing::check::setting_out(&s, &t);
    assert!(
        found
            .iter()
            .any(|f| f.check == "setting-out-station" && f.sheet == s[0].number),
        "{found:?}"
    );
    let mut s = good;
    for (n, bytes) in &mut s[0].extra {
        if n.ends_with(".csv") {
            *bytes = String::from_utf8(bytes.clone())
                .unwrap()
                .replacen("500066.180", "500066.181", 1)
                .into_bytes();
        }
    }
    let found = owd_format::drawing::check::setting_out(&s, &t);
    assert!(
        found.iter().any(|f| f.check == "setting-out-csv"),
        "{found:?}"
    );
}

#[test]
fn the_spacing_comes_from_the_style_and_a_long_table_splits_over_sheets() {
    let wide = so_with(Style::from_overlay(&json!({"setting_out": {"spacing_m": 500}})).unwrap());
    let rows = |sheets: &[Sheet]| drawn_rows(sheets, "so-stations", "so-s", 5).len();
    assert_eq!(rows(&wide), so_table(500.0).stations.len());
    let fine = so_with(Style::from_overlay(&json!({"setting_out": {"spacing_m": 10}})).unwrap());
    assert!(fine.len() >= 2, "300 rows do not fit one sheet");
    assert_eq!(rows(&fine), so_table(10.0).stations.len());
    for s in &fine {
        assert!(drawing::render(s).is_ok());
    }
    assert!(owd_format::drawing::check::setting_out(&fine, &so_table(10.0)).is_empty());
}
