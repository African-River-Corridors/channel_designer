//! The screening report (spec 0014, ticket 1) through the public API, read back as HTML text and as
//! PDF text (a second reader, `lopdf`): every number in it is the document's, ORD accepts the
//! document, the output is the same bytes twice, document control is checked, and the "Data and
//! limits" box follows the document.
//!
//! Label: property. A report value equals the document value; no number is pinned.

use owd_format::design::Session;
use owd_report::{Control, Format, Kind, Report, Request, build};
use serde_json::Value;
use std::path::PathBuf;

const DATE: &str = "2026-10-22";
const NOTICE: &str = "Screening — not for construction";

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// The web example (a design with an option, its volumes and a fits answer) and its DEM.
fn web() -> (String, Vec<u8>) {
    let dir = examples().join("web");
    (
        std::fs::read_to_string(dir.join("river-example.owd")).unwrap(),
        std::fs::read(dir.join("river-dem.tif")).unwrap(),
    )
}

/// The web example with a fits answer (plan only), run here: the example file holds none.
fn web_with_fits() -> (String, Vec<u8>) {
    let (design, dem) = web();
    let mut s = Session::open(&design).unwrap();
    s.run_fits(None).unwrap();
    (s.to_text(), dem)
}

/// The synthetic example: no DEM and no option.
fn synthetic() -> String {
    std::fs::read_to_string(examples().join("synthetic-river.owd")).unwrap()
}

fn request(control: Control) -> Request {
    Request {
        kind: Kind::Screening,
        option: None,
        control,
    }
}

fn report(design: &str, dem: Option<&[u8]>) -> Report {
    build(design, dem, &request(Control::draft(DATE))).unwrap()
}

/// The visible text of an HTML report: styles and scripts dropped, tags made spaces, entities read,
/// white space squashed.
fn html_text(html: &[u8]) -> String {
    let s = String::from_utf8(html.to_vec()).unwrap();
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        out.push(' ');
        let after = &rest[i..];
        let skip_to = if after.starts_with("<style") {
            after.find("</style>").map(|j| j + 8)
        } else if after.starts_with("<script") {
            after.find("</script>").map(|j| j + 9)
        } else {
            after.find('>').map(|j| j + 1)
        };
        rest = &after[skip_to.unwrap_or(after.len())..];
    }
    out.push_str(rest);
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    squash(&out)
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of each PDF page.
fn pdf_pages(pdf: &[u8]) -> Vec<String> {
    let d = lopdf::Document::load_mem(pdf).unwrap();
    d.get_pages()
        .keys()
        .map(|&p| squash(&d.extract_text(&[p]).unwrap()))
        .collect()
}

/// Each needle occurs in `hay`, in order.
fn in_order(hay: &str, needles: &[String]) {
    let mut at = 0;
    for n in needles {
        let n = squash(n);
        match hay[at..].find(&n) {
            Some(i) => at += i + n.len(),
            None => panic!("{n:?} is missing, or out of order, after byte {at}"),
        }
    }
}

/// The text after the last place `caption` occurs: a table's own rows, not the contents list or an
/// earlier table.
fn after<'a>(hay: &'a str, caption: &str) -> &'a str {
    let i = hay
        .rfind(caption)
        .unwrap_or_else(|| panic!("{caption:?} is missing"));
    &hay[i + caption.len()..]
}

/// The printed form of a number: thousands separators, `places` decimals.
fn grouped(x: f64, places: usize) -> String {
    let s = format!("{:.*}", places, x);
    let (int, frac) = s
        .split_once('.')
        .map_or((s.as_str(), None), |(i, f)| (i, Some(f)));
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));
    let mut g = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            g.push(',');
        }
        g.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{g}.{f}"),
        None => format!("{sign}{g}"),
    }
}

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
    assert!(before.contains("Reach not recorded"), "{before}");

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
