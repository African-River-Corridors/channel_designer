//! The quantity schedule (spec 0014, ticket 3): the option totals are the volumes stage's and
//! `owd export --csv`'s, each stretch row is `owd export --stretches`'s, and the tallest face in a
//! stretch is the tallest of the volumes stage's walk inside it (read through the long section).
//!
//! Label: property. A report value equals the document value; no number is pinned.

mod common;

use common::*;
use owd_format::design::Session;
use owd_format::design::terrain::dem_sampler;
use owd_report::{Control, Format, Kind, Report, Request, build};

fn schedule(design: &str, dem: Option<&[u8]>) -> Report {
    let req = Request {
        kind: Kind::Quantities,
        option: None,
        control: Control::draft(DATE),
    };
    build(design, dem, &req).unwrap()
}

/// The data rows of an export CSV, split on commas (the fields read here hold none).
fn csv_rows(csv: &str) -> Vec<Vec<String>> {
    csv.lines()
        .filter(|l| !l.starts_with('#'))
        .skip(1)
        .map(|l| l.split(',').map(String::from).collect())
        .collect()
}

fn f(s: &str) -> f64 {
    s.parse().unwrap()
}

#[test]
fn both_examples_render_in_three_formats_and_ord_accepts_them() {
    let (design, dem) = web();
    for (name, r) in [
        ("web", schedule(&design, Some(&dem))),
        ("synthetic", schedule(&synthetic(), None)),
    ] {
        assert!(r.number.ends_with("-QS"), "{}", r.number);
        let errors: Vec<String> = r
            .check()
            .unwrap()
            .into_iter()
            .filter(|f| f.starts_with("error "))
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        for fmt in Format::ALL {
            r.render(fmt).unwrap();
        }
    }
}

#[test]
fn the_totals_are_the_volumes_stage_and_the_csv_export() {
    let (design, dem) = web();
    let s = Session::open(&design).unwrap();
    let rows = csv_rows(&s.export_csv(Some(&dem)));
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    let html = html_text(
        &schedule(&design, Some(&dem))
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    let want = vec![
        grouped(f(&r[3]), 0),
        grouped(f(&r[4]), 0),
        grouped(f(&r[5]), 0),
        grouped(f(&r[6]), 2),
    ];
    in_order(after(&html, &format!("The whole option {}", r[1])), &want);
}

#[test]
fn each_stretch_row_is_the_stretch_export() {
    let (design, dem) = web();
    let s = Session::open(&design).unwrap();
    let mut rows = csv_rows(&s.export_stretches_csv(Some(&dem)));
    assert!(!rows.is_empty());
    rows.sort_by(|a, b| f(&a[2]).total_cmp(&f(&b[2])));
    let want: Vec<String> = rows
        .iter()
        .flat_map(|r| {
            [
                r[1].clone(),
                grouped(f(&r[2]) / 1000.0, 3),
                grouped(f(&r[3]) / 1000.0, 3),
                r[13].clone(),
                grouped(f(&r[14]), 0),
                grouped(f(&r[15]), 0),
            ]
        })
        .collect();
    let html = html_text(
        &schedule(&design, Some(&dem))
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    in_order(
        after(&html, "Dredge and dry cut by constraint stretch"),
        &want,
    );
}

#[test]
fn the_tallest_face_of_a_stretch_is_the_walks() {
    let (design, dem) = web();
    let s = Session::open(&design).unwrap();
    let grid = owd_format::geotiff::read_bytes(&dem).unwrap();
    let sampler = dem_sampler(&grid, Some(32630)).unwrap();
    let walk = s.option_section_walk("rolling-circle", &sampler).unwrap();
    let ls = s.long_section("rolling-circle", &walk).unwrap();
    let ch: Vec<f64> = ls["chainage_m"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let face: Vec<f64> = ls["max_face_h_m"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let mut rows = csv_rows(&s.export_stretches_csv(Some(&dem)));
    rows.sort_by(|a, b| f(&a[2]).total_cmp(&f(&b[2])));
    let want: Vec<String> = rows
        .iter()
        .map(|r| {
            let (a, b) = (f(&r[2]), f(&r[3]));
            let top = ch
                .iter()
                .zip(&face)
                .filter(|(c, h)| **c >= a && **c <= b && h.is_finite())
                .map(|(_, h)| *h)
                .fold(f64::NAN, f64::max);
            // the dry cut, then the face: anchors each face to its own row
            format!(
                "{} {}",
                grouped(f(&r[15]), 0),
                if top.is_nan() {
                    "—".into()
                } else {
                    grouped(top, 2)
                }
            )
        })
        .collect();
    let html = html_text(
        &schedule(&design, Some(&dem))
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    in_order(
        after(&html, "Dredge and dry cut by constraint stretch"),
        &want,
    );
}

#[test]
fn without_the_dem_the_faces_are_not_computed() {
    let (design, _) = web();
    let html = html_text(&schedule(&design, None).render(Format::Html).unwrap().bytes);
    assert!(html.contains("The faces are not computed:"), "{html}");
    assert!(html.contains("not computed"));
}
