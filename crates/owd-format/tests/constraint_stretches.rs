//! Label: property
//! Constraint stretches on the drawings (spec 0004, ticket 9), through `drawing::sheets_from_text`,
//! `drawing::check` and `drawing::plan`: on the web example, whose stored screening has width, depth,
//! radius and infeasible stretches, every covered stretch has one band and one label on the plan with
//! its own values, its worst station is a gallery section once, and turning the rules off removes all of
//! it. A design with no stretch shows none.
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Terrain};
use owd_format::drawing::plan::{CONSTRAINT_KINDS, LabelRule, labels, stretch_text};
use owd_format::drawing::{self, DrawOptions, Sheet, SheetKind, Style};
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

fn options(kinds: Vec<SheetKind>, overrides: &[(&str, Value)]) -> DrawOptions {
    let style = Style::builtin()
        .with_overrides(
            &overrides
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    DrawOptions {
        sheets: kinds,
        style,
        ..DrawOptions::default()
    }
}

fn draw(o: &DrawOptions) -> Vec<Sheet> {
    let (text, dem) = example();
    drawing::sheets_from_text(&text, Some(&dem), o).unwrap()
}

/// The stored stretches of the example's option.
fn stored() -> Vec<Value> {
    let (text, _) = example();
    let doc: Value = serde_json::from_str(&text).unwrap();
    doc["stages"]["volumes"]["results"]["options"][0]["screening"]["constraint_stretches"]
        .as_array()
        .unwrap()
        .clone()
}

fn texts(sheet: &Sheet, prefix: &str) -> Vec<(String, String)> {
    sheet.document["viewports"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|v| v["entities"].as_array().unwrap().iter())
        .filter(|e| e["id"].as_str().is_some_and(|i| i.starts_with(prefix)))
        .map(|e| {
            (
                e["id"].as_str().unwrap().to_string(),
                e["text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn session_and_walk() -> (Session, owd_core::volumes::sections::SectionVolumes) {
    let (text, dem) = example();
    let s = Session::open(&text).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let walk = s
        .option_section_walk("rolling-circle", &t as Terrain)
        .unwrap();
    (s, walk)
}

#[test]
fn every_covered_stretch_has_one_band_and_one_label_with_its_own_values() {
    let stretches = stored();
    let covered: Vec<usize> = (0..stretches.len())
        .filter(|&i| CONSTRAINT_KINDS.contains(&stretches[i]["kind"].as_str().unwrap()))
        .collect();
    assert!(
        covered.len() >= 5,
        "the example has width, depth and radius stretches"
    );
    for scale in [5000, 1000] {
        let o = options(vec![SheetKind::Plan], &[("plan.strip_scale", json!(scale))]);
        let sheets = draw(&o);
        let strips = &sheets[1..];
        for &i in &covered {
            let label: Vec<String> = strips
                .iter()
                .flat_map(|s| texts(s, &format!("stretch-{i}")))
                .filter(|(id, _)| id == &format!("stretch-{i}"))
                .map(|(_, t)| t)
                .collect();
            assert_eq!(label.len(), 1, "1:{scale}: stretch {i}: {label:?}");
            let fields: Vec<String> = ["kind", "deficit", "works", "chainage"]
                .map(String::from)
                .to_vec();
            assert_eq!(label[0], stretch_text(&stretches[i], &fields));
            let bands = strips
                .iter()
                .flat_map(entity_ids)
                .filter(|id| id == &format!("stretch-{i}-band-0"))
                .count();
            assert_eq!(bands, 1, "1:{scale}: stretch {i}");
        }
        // no band or label for a stretch that is not covered (the infeasible ones have their own rule)
        for (i, x) in stretches.iter().enumerate() {
            if !covered.contains(&i) {
                assert!(
                    strips.iter().all(|s| !entity_ids(s)
                        .iter()
                        .any(|id| id.starts_with(&format!("stretch-{i}")))),
                    "{x}"
                );
            }
        }
        let (session, walk) = session_and_walk();
        assert!(drawing::check::plan(&sheets, &walk, &session, "rolling-circle", &o).is_empty());
    }
}

fn entity_ids(sheet: &Sheet) -> Vec<String> {
    sheet.document["viewports"][0]["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn a_label_that_differs_from_its_stretch_is_refused() {
    let o = options(vec![SheetKind::Plan], &[]);
    let mut sheets = draw(&o);
    let (session, walk) = session_and_walk();
    // write a deficit 0.2 m off on the first width stretch's label
    let i = stored()
        .iter()
        .position(|x| {
            x["kind"] == "width" && x["end_chainage_m"].as_f64() > x["start_chainage_m"].as_f64()
        })
        .unwrap();
    let id = format!("stretch-{i}");
    let entities = sheets[1].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap();
    let e = entities
        .iter_mut()
        .find(|e| e["id"] == id.as_str())
        .unwrap();
    let d = stored()[i]["deficit"].as_f64().unwrap();
    let text = e["text"]
        .as_str()
        .unwrap()
        .replace(&format!("{d:.1} M"), &format!("{:.1} M", d + 0.2));
    e["text"] = json!(text);
    let found = drawing::check::plan(&sheets, &walk, &session, "rolling-circle", &o);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].check, "constraint-stretch");
    assert!(found[0].message.contains("deficit"), "{}", found[0].message);
    // and a band taken away leaves the stretch with no band
    let mut sheets = draw(&o);
    let band = format!("stretch-{i}-band-0");
    sheets[1].document["viewports"][0]["entities"]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["id"] != band.as_str());
    let found = drawing::check::plan(&sheets, &walk, &session, "rolling-circle", &o);
    assert!(
        found.iter().any(|f| f.message.contains("0 bands")),
        "{found:?}"
    );
}

/// The chainages of the gallery's sections, from their headings.
fn section_chainages(sheets: &[Sheet]) -> Vec<f64> {
    sheets
        .iter()
        .flat_map(|s| {
            s.document["viewports"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| {
                    let id = v["id"].as_str()?;
                    let t = v["entities"]
                        .as_array()?
                        .iter()
                        .find(|e| e["id"] == format!("{id}-chainage").as_str())?;
                    let (km, m) = t["text"].as_str()?.strip_prefix("CH ")?.split_once('+')?;
                    Some(km.parse::<f64>().ok()? * 1000.0 + m.parse::<f64>().ok()?)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn each_worst_station_is_a_gallery_section_once() {
    let (_, walk) = session_and_walk();
    let nearest = |c: f64| {
        walk.stations
            .iter()
            .map(|s| s.chainage_m)
            .min_by(|a, b| (a - c).abs().total_cmp(&(b - c).abs()))
            .unwrap()
    };
    let on = section_chainages(&draw(&options(vec![SheetKind::CrossSections], &[])));
    assert!(!on.is_empty());
    for x in stored() {
        let Some(w) = x["worst_chainage_m"].as_f64() else {
            continue;
        };
        let at = nearest(w);
        let n = on
            .iter()
            .filter(|c| (**c - at).abs() <= 0.05 + 1e-9)
            .count();
        assert_eq!(n, 1, "worst station CH {w} (station {at}): {on:?}");
    }
    let off = section_chainages(&draw(&options(
        vec![SheetKind::CrossSections],
        &[("sections.extra", json!([{"rule": "bend-middle"}]))],
    )));
    assert!(
        off.len() < on.len(),
        "{} sections with, {} without",
        on.len(),
        off.len()
    );
}

#[test]
fn turning_the_rules_off_removes_bands_labels_and_extra_sections() {
    let off = options(
        vec![SheetKind::Plan, SheetKind::CrossSections],
        &[
            ("sections.extra", json!([{"rule": "bend-middle"}])),
            (
                "plan.label_rules",
                json!([{"rule": "bend-tighter-than", "k": 1}, {"rule": "infeasible-stretch"}]),
            ),
        ],
    );
    let sheets = draw(&off);
    assert!(
        sheets
            .iter()
            .all(|s| entity_ids(s).iter().all(|id| !id.starts_with("stretch-")))
    );
    let on = draw(&options(vec![SheetKind::CrossSections], &[]));
    let gallery_off: Vec<Sheet> = sheets
        .into_iter()
        .filter(|s| s.number.contains("-XS-"))
        .collect();
    assert!(section_chainages(&gallery_off).len() < section_chainages(&on).len());
}

#[test]
fn a_design_with_no_stretch_shows_none() {
    let option = json!({"line": {"type": "LineString", "coordinates": [[0, 0], [1000, 0]]}, "checks": {"r_min_m": 300}});
    let rules = vec![LabelRule::ConstraintStretch {
        fields: vec!["kind".into()],
    }];
    assert!(labels(&option, &[], &rules).is_empty());
    // an infeasible stretch is not this rule's
    let infeasible =
        json!([{"kind": "infeasible", "start_chainage_m": 10.0, "end_chainage_m": 20.0}]);
    assert!(labels(&option, infeasible.as_array().unwrap(), &rules).is_empty());
}

#[test]
fn the_fields_of_a_label_are_the_styles() {
    let x = json!({"kind": "width", "start_chainage_m": 4200.0, "end_chainage_m": 4650.0, "deficit": 12.04, "works": "bank-cut"});
    let all: Vec<String> = ["kind", "deficit", "works", "chainage"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        stretch_text(&x, &all),
        "WIDTH SHORT · 12.0 M · BANK CUT · CH 4+200.0 TO 4+650.0"
    );
    assert_eq!(
        stretch_text(&x, &["chainage".into(), "kind".into()]),
        "CH 4+200.0 TO 4+650.0 · WIDTH SHORT"
    );
}
