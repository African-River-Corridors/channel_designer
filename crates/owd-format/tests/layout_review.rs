//! Label: property
//! `otd`'s layout review in `owd draw` (spec 0004, ticket 7), through `drawing::render`, the call
//! `owd draw` and `owd.draw` make for every sheet: every sheet of the fixture designs passes with no
//! error, and a sheet made to overlap two texts is refused with the sheet and the finding named.
use owd_format::drawing::{self, DrawError, DrawOptions, Sheet, Style};
use serde_json::{Value, json};
use std::path::PathBuf;

fn web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

fn sheets(style: Style) -> Vec<Sheet> {
    let text = std::fs::read_to_string(web().join("river-example.owd")).unwrap();
    let dem = std::fs::read(web().join("river-dem.tif")).unwrap();
    let o = DrawOptions {
        style,
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(&text, Some(&dem), &o).unwrap()
}

#[test]
fn every_sheet_of_the_example_passes_the_review_with_no_error() {
    // every kind at the default style, and the plan again in strips at 1:1000
    let strips = Style::builtin()
        .with_overrides(&[("plan.strip_scale".to_string(), json!(1000))])
        .unwrap();
    let mut all = sheets(Style::builtin());
    all.extend(
        sheets(strips)
            .into_iter()
            .filter(|s| s.number.contains("-PL-")),
    );
    assert!(all.len() > 8, "{} sheets", all.len());
    for sheet in &all {
        let out = drawing::render(sheet).unwrap_or_else(|e| panic!("{}: {e}", sheet.number));
        // warnings are allowed; they are printed, not failed
        assert!(
            out.warnings.iter().all(|w| !w.starts_with("error")),
            "{}: {:?}",
            sheet.number,
            out.warnings
        );
    }
}

#[test]
fn overlapping_texts_fail_the_sheet_with_the_finding_named() {
    let mut sheet = sheets(Style::builtin())
        .into_iter()
        .find(|s| s.number.contains("-XS-"))
        .unwrap();
    // a second text on top of the first text of the first panel
    let vp = &mut sheet.document["viewports"][0];
    let first: Value = vp["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "text")
        .cloned()
        .unwrap();
    let mut twin = first.clone();
    twin["id"] = json!("overlapping-twin");
    twin["text"] = json!("OVERLAP");
    vp["entities"].as_array_mut().unwrap().push(twin);
    match drawing::render(&sheet) {
        Err(DrawError::Layout {
            sheet: number,
            findings,
        }) => {
            assert_eq!(number, sheet.number);
            assert!(
                findings
                    .iter()
                    .any(|f| f.starts_with("error `text-overlap`")
                        && f.contains("OVERLAP")
                        && f.contains("overlapping-twin")),
                "{findings:?}"
            );
            let message = DrawError::Layout {
                sheet: number.clone(),
                findings,
            }
            .to_string();
            assert!(
                message.starts_with(&format!(
                    "sheet {number}: layout review: error `text-overlap`"
                )),
                "{message}"
            );
        }
        other => panic!("expected a layout review error, got {other:?}"),
    }
}

#[test]
fn a_warning_is_reported_but_does_not_fail_the_sheet() {
    // the typical section labels its vessel inside the water hatch: a warning
    let sheet = sheets(Style::builtin())
        .into_iter()
        .find(|s| s.number.contains("-TS-"))
        .unwrap();
    let out = drawing::render(&sheet).expect("a warning does not fail the sheet");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.starts_with("warning `text-on-hatch`")),
        "{:?}",
        out.warnings
    );
}
