//! The screening report's figures and its rule-trace appendix (spec 0014, ticket 2): each figure is
//! the sheet `owd draw` makes, a design that cannot make a sheet leaves the figure out with a note,
//! and the appendix holds the rows `owd trace` rebuilds.
//!
//! Label: property. A report value equals the value of the code it reuses; no number is pinned.

mod common;

use common::*;
use owd_format::design::Session;
use owd_format::design::hash::hash_value;
use owd_format::drawing::{self, DrawOptions, SheetKind};
use owd_report::Format;
use serde_json::Value;

/// The number and SVG of each sheet of `kind`, as `owd draw --sheets <kind>` writes them.
fn sheet_svgs(design: &str, dem: &[u8], kind: SheetKind) -> Vec<(String, String)> {
    let options = DrawOptions {
        sheets: vec![kind],
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(design, Some(dem), &options)
        .unwrap()
        .iter()
        .map(|s| (s.number.clone(), drawing::render(s).unwrap().svg))
        .collect()
}

#[test]
fn each_figure_is_its_sheet() {
    let (design, dem) = web();
    let r = report(&design, Some(&dem));
    let plan = sheet_svgs(&design, &dem, SheetKind::Plan);
    let ls = sheet_svgs(&design, &dem, SheetKind::LongSection);
    assert!(
        r.assets["key-map"] == plan[0].1.as_bytes(),
        "the key map is not the plan overview"
    );
    assert!(
        r.assets["long-section"] == ls[0].1.as_bytes(),
        "the long section is not its sheet"
    );
    // The figures are placed, with the sheets' numbers in their captions.
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    in_order(
        &html,
        &[
            format!("Key map: the plan overview, sheet {}", plan[0].0),
            format!("sheet {}", ls[0].0),
        ],
    );
}

#[test]
fn a_design_with_no_dem_and_no_option_leaves_the_figures_out() {
    let r = report(&synthetic(), None);
    assert!(r.assets.is_empty());
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    assert!(html.contains("The key map is left out:"), "{html}");
    assert!(html.contains("The long section is left out:"), "{html}");
    for f in Format::ALL {
        r.render(f).unwrap();
    }
}

fn rows(t: &Value) -> Vec<Value> {
    t["rows"].as_array().unwrap().clone()
}

#[test]
fn the_appendix_holds_the_rule_trace() {
    let (design, dem) = web();
    let s = Session::open(&design).unwrap();
    let channel = s.rule_trace(None, 10.0).unwrap();
    let option = s.rule_trace(Some("rolling-circle"), 10.0).unwrap();
    let html = html_text(
        &report(&design, Some(&dem))
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    let appendix = after(&html, "Appendix A Rule trace");

    // The channel rows, every one, in order.
    let ch: Vec<String> = rows(&channel)
        .iter()
        .flat_map(|r| {
            [
                r["rule"].as_str().unwrap().into(),
                r["outcome"].as_str().unwrap().into(),
            ]
        })
        .collect();
    assert!(!ch.is_empty());
    in_order(
        after(appendix, "The Rule Trace of the channel and its reaches"),
        &ch,
    );
    assert!(appendix.contains(&hash_value(&channel)));

    // The option: the rows that do not pass, in order, and the passing rows counted per Rule.
    let all = rows(&option);
    let not_pass: Vec<String> = all
        .iter()
        .filter(|r| r["outcome"] != "pass")
        .flat_map(|r| {
            [
                r["rule"].as_str().unwrap().into(),
                r["outcome"].as_str().unwrap().into(),
            ]
        })
        .collect();
    assert!(!not_pass.is_empty());
    in_order(
        after(appendix, "The station rows that do not pass"),
        &not_pass,
    );
    let mut counts: Vec<(String, usize)> = Vec::new();
    for r in all.iter().filter(|r| r["outcome"] == "pass") {
        let rule = r["rule"].as_str().unwrap().to_string();
        match counts.iter_mut().find(|(n, _)| *n == rule) {
            Some(e) => e.1 += 1,
            None => counts.push((rule, 1)),
        }
    }
    let passes: Vec<String> = counts.iter().map(|(r, n)| format!("{r} {n}")).collect();
    in_order(
        after(appendix, "The passing station rows, counted per Rule"),
        &passes,
    );
    assert!(
        appendix.contains(&format!("{} rows", all.len())),
        "{appendix}"
    );
    assert!(appendix.contains(&hash_value(&option)));
}
