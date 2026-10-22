//! The section pack (spec 0014, ticket 4): each figure is the sheet `owd draw` makes, one per
//! gallery sheet, and the setting-out rows are the setting-out CSV `owd draw` writes.
//!
//! Label: property. A report value equals the value of the code it reuses; no number is pinned.

mod common;

use common::*;
use owd_format::drawing::{self, DrawOptions, SheetKind};
use owd_report::{Control, Format, Kind, Report, Request, build};

fn pack(design: &str, dem: Option<&[u8]>) -> Report {
    let req = Request {
        kind: Kind::Sections,
        option: None,
        control: Control::draft(DATE),
    };
    build(design, dem, &req).unwrap()
}

/// A drawn sheet: its number, its SVG and its extra files.
type Drawn = (String, String, Vec<(String, Vec<u8>)>);

/// Each sheet of `kind`, as `owd draw --sheets <kind>` makes it.
fn drawn(design: &str, dem: &[u8], kind: SheetKind) -> Vec<Drawn> {
    let options = DrawOptions {
        sheets: vec![kind],
        ..DrawOptions::default()
    };
    drawing::sheets_from_text(design, Some(dem), &options)
        .unwrap()
        .iter()
        .map(|s| {
            let o = drawing::render(s).unwrap();
            (o.number, o.svg, o.extra)
        })
        .collect()
}

#[test]
fn both_examples_render_in_three_formats_and_ord_accepts_them() {
    let (design, dem) = web();
    for (name, r) in [
        ("web", pack(&design, Some(&dem))),
        ("synthetic", pack(&synthetic(), None)),
    ] {
        assert!(r.number.ends_with("-SP"), "{}", r.number);
        let errors: Vec<String> = r
            .check()
            .unwrap()
            .into_iter()
            .filter(|f| f.starts_with("error "))
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        for f in Format::ALL {
            r.render(f).unwrap();
        }
    }
}

#[test]
fn each_figure_is_its_sheet() {
    let (design, dem) = web();
    let r = pack(&design, Some(&dem));
    let ts = drawn(&design, &dem, SheetKind::TypicalSection);
    let xs = drawn(&design, &dem, SheetKind::CrossSections);
    assert!(!xs.is_empty());
    assert!(r.assets["typical-section"] == ts[0].1.as_bytes());
    for (i, (_, svg, _)) in xs.iter().enumerate() {
        assert!(
            r.assets[&format!("cross-sections-{}", i + 1)] == svg.as_bytes(),
            "sheet {}",
            i + 1
        );
    }
    assert_eq!(
        r.assets
            .keys()
            .filter(|k| k.starts_with("cross-sections-"))
            .count(),
        xs.len()
    );
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    let mut captions = vec![format!("sheet {}", ts[0].0)];
    captions.extend(
        xs.iter()
            .enumerate()
            .map(|(i, (n, _, _))| format!("Cross sections, {} of {}, sheet {n}", i + 1, xs.len())),
    );
    in_order(&html, &captions);
}

#[test]
fn the_setting_out_rows_are_the_csv() {
    let (design, dem) = web();
    let so = drawn(&design, &dem, SheetKind::SettingOut);
    let csv = so
        .iter()
        .flat_map(|s| s.2.iter())
        .find(|(n, _)| n.ends_with(".csv"))
        .map(|(_, b)| String::from_utf8(b.clone()).unwrap())
        .unwrap();
    let (mut stations, mut bends) = (Vec::new(), Vec::new());
    for l in csv.lines().skip(1) {
        let c: Vec<&str> = l.split(',').collect();
        match c[0] {
            "station" => stations.push(
                c[1..6]
                    .iter()
                    .filter(|x| !x.is_empty())
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "bend" => bends.push([c[1], c[6], c[7], c[8]].join(" ")),
            _ => panic!("{l}"),
        }
    }
    assert!(!stations.is_empty() && !bends.is_empty());
    let html = html_text(
        &pack(&design, Some(&dem))
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    in_order(after(&html, "Setting out: the stations"), &stations);
    in_order(after(&html, "Setting out: the bends"), &bends);
}

#[test]
fn a_design_with_no_dem_and_no_option_leaves_the_sheets_out() {
    let r = pack(&synthetic(), None);
    assert!(r.assets.is_empty());
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    for what in [
        "typical section",
        "cross-section gallery",
        "setting-out table",
    ] {
        assert!(
            html.contains(&format!("The {what} is left out:")),
            "{what}: {html}"
        );
    }
}
