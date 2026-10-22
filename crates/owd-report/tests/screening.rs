//! The screening report (spec 0014, ticket 1) through the public API, read back as HTML text and as
//! PDF text (a second reader, `lopdf`): every number in it is the document's, ORD accepts the
//! document, the output is the same bytes twice, document control is checked, and the "Data and
//! limits" box follows the document.
//!
//! Label: property. A report value equals the document value; no number is pinned.

mod common;

use common::*;
use owd_format::design::Session;
use owd_report::{Control, Format, build};
use serde_json::Value;

/// A stored volume as the report prints it: whole cubic metres, or "not computed".
fn volume(v: &Value) -> String {
    v.as_f64().map_or("not computed".into(), |x| grouped(x, 0))
}

#[test]
fn both_examples_render_in_three_formats_and_ord_accepts_them() {
    let (design, dem) = web();
    for (name, r) in [
        ("web", report(&design, Some(&dem))),
        ("synthetic", report(&synthetic(), None)),
    ] {
        let errors: Vec<String> = r
            .check()
            .unwrap()
            .into_iter()
            .filter(|f| f.starts_with("error "))
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        for f in Format::ALL {
            let out = r.render(f).unwrap();
            assert!(!out.bytes.is_empty(), "{name} {f:?}");
        }
    }
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let (design, dem) = web();
    for f in Format::ALL {
        let a = report(&design, Some(&dem)).render(f).unwrap().bytes;
        let b = report(&design, Some(&dem)).render(f).unwrap().bytes;
        assert!(a == b, "{f:?} differs between two runs");
    }
}

#[test]
fn every_number_is_the_documents() {
    let (design, dem) = web_with_fits();
    let doc: Value = serde_json::from_str(&design).unwrap();
    let r = report(&design, Some(&dem));
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    let pdf = pdf_pages(&r.render(Format::Pdf).unwrap().bytes).join(" ");

    // The design basis, in order.
    let basis: Vec<String> = doc["stages"]["sizing"]["results"]["design_basis"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|b| {
            [
                format!("{} m", grouped(b["value"].as_f64().unwrap(), 2)),
                b["rule"].as_str().unwrap().to_string(),
            ]
        })
        .collect();
    assert!(!basis.is_empty());
    in_order(after(&html, "The design basis"), &basis);

    // The option's verdict and the volumes stage's quantities.
    let v = &doc["stages"]["volumes"]["results"]["options"][0];
    let quantities = vec![
        v["name"].as_str().unwrap().to_string(),
        grouped(v["dredge_m3"].as_f64().unwrap(), 0),
        grouped(v["excav_m3"].as_f64().unwrap(), 0),
        grouped(v["total_m3"].as_f64().unwrap(), 0),
        grouped(v["max_face_h_m"].as_f64().unwrap(), 2),
    ];
    in_order(
        &html,
        &[v["screening"]["verdict"].as_str().unwrap().to_string()],
    );
    in_order(
        after(&html, "Works and quantities for each design line option"),
        &quantities,
    );
    in_order(
        after(&pdf, "Works and quantities for each design line option"),
        &quantities,
    );

    // Every constraint stretch, by start chainage: kind, chainages, dredge and dry cut.
    let mut list = v["screening"]["constraint_stretches"]
        .as_array()
        .unwrap()
        .clone();
    list.sort_by(|a, b| {
        a["start_chainage_m"]
            .as_f64()
            .unwrap()
            .total_cmp(&b["start_chainage_m"].as_f64().unwrap())
    });
    let rows: Vec<String> = list
        .iter()
        .flat_map(|s| {
            [
                s["kind"].as_str().unwrap().to_string(),
                grouped(s["start_chainage_m"].as_f64().unwrap() / 1000.0, 3),
                grouped(s["end_chainage_m"].as_f64().unwrap() / 1000.0, 3),
                volume(&s["dredge_m3"]),
                volume(&s["dry_cut_m3"]),
            ]
        })
        .collect();
    in_order(
        after(&html, "The constraint stretches of the chosen option"),
        &rows,
    );

    // The fits answer: every class, in order.
    let classes: Vec<String> = doc["fits"]["results"]["rule_sets"][0]["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["vessel_standard_id"].as_str().unwrap().to_string())
        .collect();
    in_order(&html, &classes);
}

#[test]
fn the_notice_is_on_every_pdf_page() {
    let (design, dem) = web();
    let pages = pdf_pages(
        &report(&design, Some(&dem))
            .render(Format::Pdf)
            .unwrap()
            .bytes,
    );
    assert!(pages.len() > 3);
    for (i, p) in pages.iter().enumerate() {
        assert!(p.contains(NOTICE), "page {} has no notice: {p}", i + 1);
    }
}

#[test]
fn a_status_needs_its_sign_offs() {
    let (design, dem) = web();
    let mut c = Control::draft(DATE);
    c.status = "for-checking".into();
    let r = build(&design, Some(&dem), &request(c.clone())).unwrap();
    let e = r.render(Format::Html).unwrap_err().to_string();
    assert!(e.contains("ORD004"), "{e}");

    c.status = "approved".into();
    c.revision = "0".into();
    c.prepared = Some("Engineer".into());
    c.checked = Some("Checker".into());
    c.approved = Some("Approver".into());
    let r = build(&design, Some(&dem), &request(c)).unwrap();
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    in_order(
        &html,
        &["Engineer".into(), "Checker".into(), "Approver".into()],
    );
}

/// The box's text in the HTML: from its title to the first section's prose.
fn data_box(design: &str, dem: Option<&[u8]>) -> String {
    let html = html_text(&report(design, dem).render(Format::Html).unwrap().bytes);
    let start = html.find("Data and limits Reach").expect("the box");
    let end = html[start..].find("This report screens").unwrap();
    html[start..start + end].to_string()
}

#[test]
fn the_data_and_limits_box_follows_the_document() {
    let (design, dem) = web();
    let before = data_box(&design, Some(&dem));
    assert!(before.contains("Depth not recorded"), "{before}");
    // No recorded reach: the box gives the centreline's own length, read from the input.
    let centre = s_centreline_km(&design);
    assert!(
        before.contains(&format!("Reach centreline {centre} km Coordinate")),
        "{before}"
    );

    // A bed model: the depth and its label.
    let mut s = Session::open(&design).unwrap();
    s.set_bed_model(&[[0.0, 3.0]], Some("dry"), DATE).unwrap();
    let with_bed = data_box(&s.to_text(), Some(&dem));
    assert!(
        with_bed.contains("Depth 3.00 m, dry season; depth assumed, not measured"),
        "{with_bed}"
    );

    // A changed fact changes the box: the depth.
    let mut s = Session::open(&design).unwrap();
    s.set_bed_model(&[[0.0, 4.5]], Some("dry"), DATE).unwrap();
    let other = data_box(&s.to_text(), Some(&dem));
    assert_ne!(with_bed, other);
    assert!(other.contains("Depth 4.50 m"), "{other}");

    // A DEM that is not the one the volumes read makes the volumes stale, and the box says so.
    let mut changed = dem.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    let stale = data_box(&design, Some(&changed));
    assert!(stale.contains("volumes stale"), "{stale}");
    assert!(!before.contains("volumes stale"), "{before}");
}

/// The centreline's length in km to three places, worked out here from the input.
fn s_centreline_km(design: &str) -> String {
    let v: Value = serde_json::from_str(design).unwrap();
    let c = v["inputs"]["centreline"]["coordinates"].as_array().unwrap();
    let m: f64 = c
        .windows(2)
        .map(|w| {
            let (a, b) = (&w[0], &w[1]);
            (b[0].as_f64().unwrap() - a[0].as_f64().unwrap())
                .hypot(b[1].as_f64().unwrap() - a[1].as_f64().unwrap())
        })
        .sum();
    format!("{:.3}", m / 1000.0)
}

#[test]
fn the_box_compares_the_centreline_with_the_route() {
    let (design, dem) = web();
    let centre: f64 = s_centreline_km(&design).parse().unwrap();
    let mut s = Session::open(&design).unwrap();
    // A route half as long as the centreline: the box says the centreline is twice the route.
    s.set_reach(serde_json::json!({
        "river": "Test river", "start": [-1.0, 5.4], "end": [-0.98, 5.42],
        "route": [{"kind": "as-drawn"}], "length_m": centre * 500.0,
    }))
    .unwrap();
    let b = data_box(&s.to_text(), Some(&dem));
    assert!(
        b.contains(&format!(
            "route {:.3} km; centreline {centre:.3} km, 2.00 times the route",
            centre / 2.0
        )),
        "{b}"
    );
}

#[test]
fn the_box_names_the_rule_sets_the_answers_used() {
    // The web example: sizing ran; with a fits answer, the box names its rule set too.
    let (design, dem) = web_with_fits();
    let b = data_box(&design, Some(&dem));
    assert!(
        b.contains("Rule Sets sizing: pianc-wg141@0.3.0, owd@0.3.0, se_quality B; fits answer: PIANC-B; every Rule unverified"),
        "{b}"
    );
    // Sizing not run: its selection is marked as unused.
    let mut v: Value = serde_json::from_str(&design).unwrap();
    for k in ["results", "depends_on", "engine_version"] {
        v["stages"]["sizing"][k] = Value::Null;
    }
    let text = v.to_string();
    Session::open(&text).expect("a design whose sizing has not run opens");
    let b = data_box(&text, Some(&dem));
    assert!(
        b.contains("se_quality B (selected; sizing not run); fits answer: PIANC-B"),
        "{b}"
    );
}

#[test]
fn a_design_with_no_option_says_so() {
    let html = html_text(
        &report(&synthetic(), None)
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    assert!(html.contains("no screened design line option"), "{html}");
    assert!(
        html.contains("no design line option, so it has no works"),
        "{html}"
    );
}

#[test]
fn an_unknown_option_is_refused() {
    let (design, dem) = web();
    let mut req = request(Control::draft(DATE));
    req.option = Some("no-such-option".into());
    let e = build(&design, Some(&dem), &req).unwrap_err().to_string();
    assert!(e.contains("no-such-option"), "{e}");
}
