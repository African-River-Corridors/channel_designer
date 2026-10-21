//! Label: property
//! The plan sheets (spec 0004, ticket 5) through their public seams, `drawing::sheets_from_text`,
//! `drawing::render` and `drawing::plan::labels`: on the web example (the synthetic river, its DEM,
//! every stage fresh), and on synthetic options for the label rules.
use owd_format::drawing::plan::{LabelRule, labels};
use owd_format::drawing::{self, DrawOptions, Sheet, SheetKind, Style};
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

fn plan_with(style: Style) -> Vec<Sheet> {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::Plan],
        style,
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
}

fn plan() -> Vec<Sheet> {
    plan_with(Style::builtin())
}

fn entities(sheet: &Sheet) -> Vec<Value> {
    sheet.document["viewports"][0]["entities"]
        .as_array()
        .unwrap()
        .clone()
}

fn style_with(overrides: &[(&str, Value)]) -> Style {
    Style::builtin()
        .with_overrides(
            &overrides
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap()
}

fn ids(sheet: &Sheet, prefix: &str) -> Vec<String> {
    entities(sheet)
        .iter()
        .filter_map(|e| e["id"].as_str())
        .filter(|id| id.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

fn by_id(sheet: &Sheet, id: &str) -> Value {
    entities(sheet)
        .into_iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("{}: no {id}", sheet.number))
}

#[test]
fn otd_accepts_every_plan_sheet_and_makes_every_output() {
    let sheets = plan();
    assert!(sheets.len() >= 2, "an overview and at least one strip");
    assert!(sheets[0].number.ends_with("-PL-001"));
    for sheet in &sheets {
        assert_eq!(sheet.document["standard_version"], "1.1");
        let out = drawing::render(sheet).expect("the standard's schema and checks pass");
        assert!(out.pdf.starts_with(b"%PDF"));
        assert!(
            out.svg.contains("data:image/jpeg;base64,"),
            "the relief is drawn"
        );
        let files: Vec<String> = out.files().into_iter().map(|(n, _)| n).collect();
        assert!(
            files.iter().any(|n| n.ends_with("-relief.jpg")),
            "{files:?}"
        );
    }
}

#[test]
fn the_same_design_gives_the_same_plan_bytes() {
    let (a, b) = (plan(), plan());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.document_text(), y.document_text());
        assert_eq!(x.extra, y.extra, "the relief JPEG");
    }
}

/// The strips of the example at a larger scale, so that the line takes several.
fn strips() -> Vec<Sheet> {
    plan_with(style_with(&[("plan.strip_scale", json!(1000))]))
}

fn match_chainage(sheet: &Sheet, which: &str) -> f64 {
    let text = by_id(sheet, &format!("match-{which}-label"))["text"]
        .as_str()
        .unwrap()
        .to_string();
    let ch = text
        .strip_prefix("MATCH LINE CH ")
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let (km, m) = ch.split_once('+').unwrap();
    km.parse::<f64>().unwrap() * 1000.0 + m.parse::<f64>().unwrap()
}

#[test]
fn every_strip_match_line_meets_the_next_strips() {
    let sheets = strips();
    assert!(sheets.len() >= 4, "{} sheets", sheets.len());
    let strips = &sheets[1..];
    for pair in strips.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert_eq!(match_chainage(a, "end"), match_chainage(b, "start"));
        let see = by_id(a, "match-end-label")["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(see.ends_with(&b.number), "{see}");
        let back = by_id(b, "match-start-label")["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(back.ends_with(&a.number), "{back}");
        // the two lines lie on one line: each crosses the design line at the same point
        let (ea, eb) = (by_id(a, "match-end"), by_id(b, "match-start"));
        let dir = |e: &Value| {
            let (f, t) = (&e["from_m"], &e["to_m"]);
            let (dx, dy) = (
                t[0].as_f64().unwrap() - f[0].as_f64().unwrap(),
                t[1].as_f64().unwrap() - f[1].as_f64().unwrap(),
            );
            let l = (dx * dx + dy * dy).sqrt();
            (dx / l, dy / l)
        };
        let ((ax, ay), (bx, by)) = (dir(&ea), dir(&eb));
        assert!((ax * by - ay * bx).abs() < 1e-9, "parallel");
        let (px, py) = (
            ea["from_m"][0].as_f64().unwrap(),
            ea["from_m"][1].as_f64().unwrap(),
        );
        let (qx, qy) = (
            eb["from_m"][0].as_f64().unwrap(),
            eb["from_m"][1].as_f64().unwrap(),
        );
        assert!(
            ((qx - px) * ay - (qy - py) * ax).abs() < 1e-6,
            "on one line"
        );
    }
    assert!(
        ids(&strips[0], "match-start").is_empty(),
        "the first strip starts the line"
    );
    assert!(
        ids(strips.last().unwrap(), "match-end").is_empty(),
        "the last strip ends it"
    );
}

#[test]
fn match_and_section_lines_are_at_the_results_and_a_moved_one_is_refused() {
    let (text, dem) = example();
    let session = owd_format::design::Session::open(&text).unwrap();
    let grid = owd_format::geotiff::read_bytes(&dem).unwrap();
    let t = owd_format::design::terrain::dem_sampler(&grid, Some(32630)).unwrap();
    let walk = session
        .option_section_walk("rolling-circle", &t as owd_format::design::Terrain)
        .unwrap();
    let o = DrawOptions {
        sheets: vec![SheetKind::Plan],
        style: style_with(&[("plan.strip_scale", json!(1000))]),
        ..DrawOptions::default()
    };
    let mut sheets = strips();
    assert!(drawing::check::plan(&sheets, &walk, &session, "rolling-circle", &o).is_empty());
    let sections: usize = sheets
        .iter()
        .map(|s| {
            ids(s, "xs-")
                .iter()
                .filter(|i| !i.ends_with("-label"))
                .count()
        })
        .sum();
    assert!(sections >= 5, "{sections} section lines");
    // move a match line by a metre: the check names the sheet
    let entities = sheets[1].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap();
    let e = entities
        .iter_mut()
        .find(|e| e["id"] == "match-end")
        .unwrap();
    let x = e["from_m"][0].as_f64().unwrap();
    e["from_m"][0] = json!(x + 1.0);
    e["to_m"][0] = json!(e["to_m"][0].as_f64().unwrap() + 1.0);
    let found = drawing::check::plan(&sheets, &walk, &session, "rolling-circle", &o);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].check, "match-line");
    assert_eq!(found[0].sheet, sheets[1].number);
    // move a section line: refused too
    let mut sheets = strips();
    let (si, id) = sheets
        .iter()
        .enumerate()
        .find_map(|(i, s)| {
            ids(s, "xs-")
                .into_iter()
                .find(|id| !id.ends_with("-label"))
                .map(|id| (i, id))
        })
        .unwrap();
    let entities = sheets[si].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap();
    let e = entities
        .iter_mut()
        .find(|e| e["id"] == id.as_str())
        .unwrap();
    e["from_m"][1] = json!(e["from_m"][1].as_f64().unwrap() + 2.0);
    let found = drawing::check::plan(&sheets, &walk, &session, "rolling-circle", &o);
    assert!(found.iter().any(|f| f.check == "section-line"), "{found:?}");
}

// ---- the label rules, on synthetic options --------------------------------------------------

/// A line: east along y = 0 for 500 m, a quarter circle of `radius` turning north, then north 500 m,
/// with the ruleset's least bend radius `r_min`, and the stored stretches given.
fn option(radius: Option<f64>, r_min: f64, stretches: Value) -> Value {
    let mut pts = vec![[0.0, 0.0], [500.0, 0.0]];
    if let Some(r) = radius {
        for i in 1..=18 {
            let a = (i as f64 * 5.0).to_radians();
            pts.push([500.0 + r * libm::sin(a), r - r * libm::cos(a)]);
        }
        pts.push([500.0 + r, r + 500.0]);
    } else {
        pts.push([1500.0, 0.0]);
    }
    json!({
        "id": "o", "line": {"type": "LineString", "coordinates": pts},
        "checks": {"r_min_m": r_min, "fits_in_water": {"ok": stretches.as_array().unwrap().is_empty(), "stretches": stretches}},
    })
}

fn both() -> Vec<LabelRule> {
    vec![
        LabelRule::BendTighterThan { k: 1.0 },
        LabelRule::InfeasibleStretch,
    ]
}

#[test]
fn a_clean_line_gives_no_label() {
    assert!(labels(&option(None, 300.0, json!([])), &[], &both()).is_empty());
    // a bend wider than the limit is not an exception either
    assert!(labels(&option(Some(900.0), 300.0, json!([])), &[], &both()).is_empty());
}

#[test]
fn one_tight_bend_gives_one_bend_label() {
    let found = labels(&option(Some(150.0), 300.0, json!([])), &[], &both());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].rule, "bend-tighter-than");
    assert!(found[0].text.starts_with("BEND R "), "{}", found[0].text);
    assert!(found[0].text.contains("< RMIN 300 M"), "{}", found[0].text);
    // k scales the limit: at k = 0.4 the 150 m bend is above 120 m and is not labelled
    assert!(
        labels(
            &option(Some(150.0), 300.0, json!([])),
            &[],
            &[LabelRule::BendTighterThan { k: 0.4 }]
        )
        .is_empty()
    );
}

#[test]
fn one_infeasible_stretch_gives_one_label() {
    let stretch =
        json!([{"start_chainage_m": 120.0, "end_chainage_m": 340.5, "reason": "too_narrow"}]);
    let found = labels(&option(None, 300.0, stretch), &[], &both());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].rule, "infeasible-stretch");
    assert_eq!(found[0].text, "TOO NARROW CH 0+120.0 TO 0+340.5");
    assert_eq!((found[0].start_m, found[0].end_m), (120.0, 340.5));
}

#[test]
fn turning_a_label_rule_off_removes_its_labels() {
    // the example holds stored infeasible stretches, so the default style labels them
    let on = plan();
    let n_on: usize = on.iter().map(|s| ids(s, "label-").len()).sum();
    assert!(n_on > 0);
    let off = plan_with(style_with(&[("plan.label_rules", json!([]))]));
    let n_off: usize = off.iter().map(|s| ids(s, "label-").len()).sum();
    assert_eq!(n_off, 0);
    let rules = off[0].document["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap().contains("LABEL RULES: NONE"));
    assert!(rules, "the notes say no rule is on");
    // and on the synthetic option, each rule alone gives only its own labels
    let opt = option(
        Some(150.0),
        300.0,
        json!([{"start_chainage_m": 10.0, "end_chainage_m": 20.0, "reason": "too_narrow"}]),
    );
    assert_eq!(labels(&opt, &[], &both()).len(), 2);
    assert!(
        labels(&opt, &[], &[LabelRule::InfeasibleStretch])
            .iter()
            .all(|l| l.rule == "infeasible-stretch")
    );
    assert!(labels(&opt, &[], &[]).is_empty());
}

#[test]
fn each_label_is_drawn_once_in_the_set() {
    let sheets = strips();
    let mut drawn: Vec<String> = sheets[1..]
        .iter()
        .flat_map(|s| {
            entities(s)
                .into_iter()
                .filter(|e| e["type"] == "text" && e["id"].as_str().unwrap().starts_with("label-"))
                .map(|e| e["text"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    let n = drawn.len();
    drawn.sort();
    drawn.dedup();
    assert_eq!(drawn.len(), n, "no label twice");
    assert_eq!(n, 3, "the example's three stored stretches: {drawn:?}");
}
