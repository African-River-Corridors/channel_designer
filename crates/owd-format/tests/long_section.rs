//! Label: property
//! The long section (spec 0004, ticket 10) through its public seam, `drawing::sheets_from_text`,
//! `drawing::render` and `drawing::check`, on the web example (the synthetic river, its DEM, every
//! stage fresh): `otd` accepts the sheet (standard 1.2, with a vertical exaggeration), makes every
//! output and passes the layout review; every drawn level and chainage equals the result of the
//! app's long-section call; the exaggeration is stated on the sheet in words; the style's factor
//! overrides "auto"; and the same design gives the same bytes.
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Terrain};
use owd_format::drawing::{self, DrawOptions, Sheet, SheetKind, Style, check};
use owd_format::geotiff;
use serde_json::{Value, json};
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

fn drawn(style: Style) -> Vec<Sheet> {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::LongSection],
        style,
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
}

fn long_section() -> Vec<Sheet> {
    drawn(Style::builtin())
}

/// The app's long-section series of the example, called directly on the same DEM.
fn series() -> Value {
    let (text, dem) = example();
    let s = Session::open(&text).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let walk = s
        .option_section_walk("rolling-circle", &t as Terrain)
        .unwrap();
    s.long_section("rolling-circle", &walk).unwrap()
}

fn viewport(sheet: &Sheet) -> &Value {
    sheet.document["viewports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "ls")
        .unwrap()
}

fn entity_mut<'a>(sheet: &'a mut Sheet, id: &str) -> &'a mut Value {
    sheet.document["viewports"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|v| v["id"] == "ls")
        .unwrap()["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id}"))
}

/// The ratios a 1.2 document may use (clauses 7.3.1, 7.3.4 and 7.3.5).
fn allowed(n: u64) -> bool {
    [
        1u64, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 1250, 2000, 2500, 5000, 10000, 20000,
        50000, 100000, 200000, 500000, 1000000, 2000000, 5000000, 10000000,
    ]
    .contains(&n)
}

#[test]
fn otd_accepts_the_sheet_makes_every_output_and_it_passes_the_layout_review() {
    let sheets = long_section();
    assert_eq!(sheets.len(), 1, "the example's line fits one sheet");
    for sheet in &sheets {
        assert!(sheet.number.contains("-LS-"), "{}", sheet.number);
        assert_eq!(sheet.document["standard_version"], "1.2");
        // render runs the standard's checks and otd's layout review: an error fails it
        let out = drawing::render(sheet).unwrap_or_else(|e| panic!("{e}"));
        assert!(out.pdf.starts_with(b"%PDF"));
        assert!(out.svg.contains("<svg"));
        assert!(out.dxf.contains("SECTION"));
        assert!(
            out.warnings.iter().all(|w| !w.starts_with("error")),
            "{:?}",
            out.warnings
        );
        let norm: Value = serde_json::from_str(&out.normalised).unwrap();
        // the title block says AS SHOWN with an exaggerated viewport (clause 14.4.4)
        let scale_cell = norm["entities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["text"] == "AS SHOWN");
        assert!(scale_cell, "the title block scale is AS SHOWN");
        if let Ok(dir) = std::env::var("OWD_DRAW_DUMP") {
            std::fs::create_dir_all(&dir).unwrap();
            for (name, data) in out.files() {
                std::fs::write(PathBuf::from(&dir).join(name), data).unwrap();
            }
        }
    }
}

#[test]
fn every_drawn_level_and_chainage_equals_the_app_s_long_section() {
    let sheets = long_section();
    let s = series();
    assert_eq!(check::long_section(&sheets, &s), []);
    // the bed, the water level and the design bed are drawn at every station
    let vp = viewport(&sheets[0]);
    let count = |key: &str| -> usize {
        vp["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["id"].as_str().unwrap().starts_with(&format!("ls-{key}-")))
            .map(|e| e["points_m"].as_array().unwrap().len())
            .sum()
    };
    let stations = s["chainage_m"].as_array().unwrap().len();
    let finite = |k: &str| {
        s[k].as_array()
            .unwrap()
            .iter()
            .filter(|v| v.as_f64().is_some())
            .count()
    };
    assert_eq!(count("invert"), finite("design_invert_m"));
    assert_eq!(count("wl"), finite("water_level_m"));
    assert_eq!(count("bed"), finite("bed_m"));
    assert!(count("invert") == stations, "{stations} stations");
}

#[test]
fn a_moved_value_is_refused_naming_the_sheet_and_the_check() {
    let s = series();
    let number = long_section()[0].number.clone();
    // a design bed point half a metre too low
    let mut sheets = long_section();
    let e = entity_mut(&mut sheets[0], "ls-invert-1");
    let z = e["points_m"][3][1].as_f64().unwrap();
    e["points_m"][3][1] = json!(z - 0.5);
    let found = check::long_section(&sheets, &s);
    assert!(!found.is_empty());
    for f in &found {
        assert_eq!(f.sheet, number);
        assert_eq!(f.check, "long-section-series");
        assert!(f.to_string().contains(&number), "{f}");
    }
    assert!(
        found[0].message.contains("design bed point"),
        "{}",
        found[0]
    );
    // a chainage moved along the line
    let mut sheets = long_section();
    let e = entity_mut(&mut sheets[0], "ls-bed-1");
    let x = e["points_m"][2][0].as_f64().unwrap();
    e["points_m"][2][0] = json!(x + 3.0);
    let checks: Vec<&str> = check::long_section(&sheets, &s)
        .iter()
        .map(|f| f.check)
        .collect();
    assert!(checks.contains(&"long-section-series"), "{checks:?}");
    // a heading level one centimetre off, and an axis label off its tick
    let mut sheets = long_section();
    let wl = s["water_level_m"][0].as_f64().unwrap();
    *entity_mut(&mut sheets[0], "ls-start-wl") = {
        let mut t = entity_mut(&mut sheets[0], "ls-start-wl").clone();
        t["text"] = json!(format!("WL {:+.2}", wl + 0.01));
        t
    };
    let label = sheets[0].document["viewports"][0]["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"].as_str().unwrap().starts_with("ls-clabel-"))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = entity_mut(&mut sheets[0], &label);
    let x = e["anchor_m"][0].as_f64().unwrap();
    e["anchor_m"][0] = json!(x + 100.0);
    let checks: Vec<&str> = check::long_section(&sheets, &s)
        .iter()
        .map(|f| f.check)
        .collect();
    assert_eq!(checks, ["long-section-level", "long-section-axis"]);
    // and `owd draw` fails on such a sheet: the check runs inside `sheets`
    let mut sheets = long_section();
    sheets[0].document["notes"][0] =
        json!("VERTICAL EXAGGERATION ×2: LEVELS DRAWN AT 1:1, CHAINAGE AT 1:2.");
    let found = check::long_section(&sheets, &s);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].check, "long-section-exaggeration");
}

#[test]
fn the_exaggeration_is_stated_on_the_sheet_in_words() {
    let sheet = &long_section()[0];
    let vp = viewport(sheet);
    let n = vp["scale"].as_u64().unwrap();
    let e = vp["vertical_exaggeration"]
        .as_u64()
        .expect("the example's levels need an exaggeration");
    assert!(e >= 2);
    // the furniture's words, in the normalised content (clause 7.9.11)
    let out = drawing::render(sheet).unwrap();
    let words = format!("Vertical exaggeration ×{e}");
    assert!(out.normalised.contains(&words), "{words}");
    assert!(
        out.normalised.contains(&format!("Vertical 1:{}", n / e)),
        "the vertical scale bar"
    );
    // the notes give the factor and both scales in words, never by initials
    let note = sheet.document["notes"][0].as_str().unwrap();
    assert_eq!(
        note,
        format!(
            "VERTICAL EXAGGERATION ×{e}: LEVELS DRAWN AT 1:{}, CHAINAGE AT 1:{n}.",
            n / e
        )
    );
    let text = sheet.document_text();
    for initials in ["V.E.", " VE ", "VE:", "V.E"] {
        assert!(!text.contains(initials), "{initials}");
    }
}

#[test]
fn auto_picks_a_factor_whose_vertical_scale_is_an_allowed_ratio() {
    let vp = viewport(&long_section()[0]).clone();
    let n = vp["scale"].as_u64().unwrap();
    let e = vp["vertical_exaggeration"].as_u64().unwrap();
    assert!(n.is_multiple_of(e));
    assert!(allowed(n) && allowed(n / e), "1:{n}, vertical 1:{}", n / e);
    // the smallest such factor that fills half the plot: one step smaller fills less than half
    let p = Style::builtin().long_section();
    let frame_h = vp["frame"]["height_mm"].as_f64().unwrap();
    let plot_h = frame_h - p.band_top_mm - p.band_bottom_mm;
    let window_h =
        vp["window"]["ymax_m"].as_f64().unwrap() - vp["window"]["ymin_m"].as_f64().unwrap();
    let kv = frame_h / window_h;
    let k = 1000.0 / n as f64;
    assert!((kv - k * e as f64).abs() < 1e-9 * kv);
    let smaller = (2..e)
        .rev()
        .find(|f| n.is_multiple_of(*f) && allowed(n / f));
    let s = series();
    let levels: Vec<f64> = ["bed_m", "water_level_m", "design_invert_m"]
        .iter()
        .flat_map(|k| s[*k].as_array().unwrap().iter().filter_map(Value::as_f64))
        .collect();
    let range = levels.iter().copied().fold(f64::MIN, f64::max)
        - levels.iter().copied().fold(f64::MAX, f64::min)
        + p.below_invert_m
        + p.above_ground_m;
    assert!(range * k * e as f64 >= p.auto_min_fill * plot_h);
    assert!(range * k * e as f64 <= plot_h);
    if let Some(f) = smaller {
        assert!(
            range * k * (f as f64) < p.auto_min_fill * plot_h,
            "×{f} would do"
        );
    }
}

#[test]
fn the_style_s_number_overrides_auto_and_an_invalid_one_is_refused() {
    let fixed = Style::builtin()
        .with_overrides(&[("long_section.vertical_exaggeration".into(), json!(10))])
        .unwrap();
    let sheets = drawn(fixed);
    let vp = viewport(&sheets[0]);
    assert_eq!(vp["vertical_exaggeration"], 10);
    assert!(drawing::render(&sheets[0]).is_ok());
    assert!(
        sheets[0].document["notes"][0]
            .as_str()
            .unwrap()
            .starts_with("VERTICAL EXAGGERATION ×10: ")
    );
    assert_eq!(check::long_section(&sheets, &series()), []);
    // 1 or a fraction is refused by the style; 3 gives no ratio at any scale of the style
    for bad in [json!(1), json!(2.5), json!("big")] {
        let e = Style::builtin()
            .with_overrides(&[("long_section.vertical_exaggeration".into(), bad.clone())])
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("/long_section/vertical_exaggeration"),
            "{bad}: {e}"
        );
    }
    let three = Style::builtin()
        .with_overrides(&[("long_section.vertical_exaggeration".into(), json!(3))])
        .unwrap();
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::LongSection],
        style: three,
        ..DrawOptions::default()
    };
    let e = drawing::sheets_from_text(&text, Some(&dem), &o)
        .unwrap_err()
        .to_string();
    assert!(e.contains("vertical exaggeration ×3"), "{e}");
}

#[test]
fn a_long_line_splits_over_sheets_at_one_scale() {
    // only 1:500 allowed: the line does not fit one sheet
    let style = Style::builtin()
        .with_overrides(&[("long_section.scales".into(), json!([500]))])
        .unwrap();
    let sheets = drawn(style);
    assert!(sheets.len() >= 2, "{} sheets", sheets.len());
    assert_eq!(check::long_section(&sheets, &series()), []);
    for sheet in &sheets {
        assert_eq!(viewport(sheet)["scale"], 500);
        drawing::render(sheet).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn the_same_design_gives_the_same_bytes() {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::LongSection],
        ..DrawOptions::default()
    };
    let a = drawing::files_from_text(&text, Some(&dem), &o).unwrap();
    let b = drawing::files_from_text(&text, Some(&dem), &o).unwrap();
    assert!(!a.is_empty());
    assert_eq!(a, b);
}
