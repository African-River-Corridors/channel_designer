//! Helpers the report tests share: the examples, and the report read back as text.
#![allow(dead_code)]

use owd_format::design::Session;
use owd_report::{Control, Kind, Report, Request, build};
use std::path::PathBuf;

pub const DATE: &str = "2026-10-22";
pub const NOTICE: &str = "Screening — not for construction";

pub fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// The web example (a design with an option and its volumes, no fits answer) and its DEM.
pub fn web() -> (String, Vec<u8>) {
    let dir = examples().join("web");
    (
        std::fs::read_to_string(dir.join("river-example.owd")).unwrap(),
        std::fs::read(dir.join("river-dem.tif")).unwrap(),
    )
}

/// The web example with a fits answer (plan only), run here: the example file holds none.
pub fn web_with_fits() -> (String, Vec<u8>) {
    let (design, dem) = web();
    let mut s = Session::open(&design).unwrap();
    s.run_fits(None).unwrap();
    (s.to_text(), dem)
}

/// The synthetic example: no DEM and no option.
pub fn synthetic() -> String {
    std::fs::read_to_string(examples().join("synthetic-river.owd")).unwrap()
}

pub fn request(control: Control) -> Request {
    Request {
        kind: Kind::Screening,
        option: None,
        control,
        requested_by: None,
    }
}

pub fn report(design: &str, dem: Option<&[u8]>) -> Report {
    build(design, dem, &request(Control::draft(DATE))).unwrap()
}

/// The visible text of an HTML report: styles and scripts dropped, tags made spaces, entities read,
/// white space squashed.
pub fn html_text(html: &[u8]) -> String {
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

pub fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of each PDF page.
pub fn pdf_pages(pdf: &[u8]) -> Vec<String> {
    let d = lopdf::Document::load_mem(pdf).unwrap();
    d.get_pages()
        .keys()
        .map(|&p| squash(&d.extract_text(&[p]).unwrap()))
        .collect()
}

/// Each needle occurs in `hay`, in order.
pub fn in_order(hay: &str, needles: &[String]) {
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
pub fn after<'a>(hay: &'a str, caption: &str) -> &'a str {
    let i = hay
        .rfind(caption)
        .unwrap_or_else(|| panic!("{caption:?} is missing"));
    &hay[i + caption.len()..]
}

/// The printed form of a number: thousands separators, `places` decimals.
pub fn grouped(x: f64, places: usize) -> String {
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
