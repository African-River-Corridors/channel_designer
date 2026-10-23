//! The screening report's "Largest vessel" section (spec 0016 T3): the series table, the chart, the
//! per-kilometre strip, and the bank caveat in "Data and limits". Every number is the study's.
//!
//! Label: property

mod common;

use common::{html_text, report, synthetic};
use owd_core::envelope::Settings;
use owd_format::design::{Role, Session};
use owd_report::Format;

fn with_sweep() -> (String, serde_json::Value) {
    let mut s = Session::open(&synthetic()).unwrap();
    s.run_envelope(Settings::default(), true).unwrap();
    let e = s.envelope_result().unwrap().clone();
    (s.to_text(), e)
}

/// The text of the section, from its title to the next section's.
fn section(html: &str) -> &str {
    let start = html
        .find("Largest vessel The largest")
        .expect("the section");
    let end = html[start..]
        .find("Constraint stretches")
        .map_or(html.len(), |e| start + e);
    &html[start..end]
}

#[test]
fn the_section_gives_the_sweep_as_the_study_holds_it() {
    let (design, e) = with_sweep();
    let r = report(&design, None);
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    let s = section(&html);
    let largest = match e["largest"]["label"].as_str() {
        Some(_) => format!(
            "{} x {} m",
            e["largest"]["loa_m"].as_f64().unwrap(),
            e["largest"]["beam_m"].as_f64().unwrap()
        ),
        None => "none".to_string(),
    };
    assert!(
        s.contains(&format!("passes the whole reach: {largest}")),
        "{s}"
    );
    assert!(s.contains("PIANC-C") && s.contains("draught 2 m"), "{s}");
    for size in e["sizes"].as_array().unwrap() {
        let label = size["label"].as_str().unwrap();
        let count = size["stretch_count"].as_u64().unwrap();
        assert!(s.contains(&format!("{label} ")), "{label}: {s}");
        assert!(s.contains(&format!(" {count} ")), "{label} {count}: {s}");
    }
    for asset in ["envelope-sizes", "envelope-strip"] {
        let svg = String::from_utf8(r.assets[asset].clone()).unwrap();
        assert!(svg.starts_with("<svg"), "{asset}");
    }
    // The bank caveat, in the box.
    let start = html.find("Data and limits Reach").unwrap();
    assert!(
        html[start..].contains("Largest vessel the answer is only as good as the banks"),
        "{}",
        &html[start..start + 2000]
    );
    // Every format renders.
    for f in [Format::Pdf, Format::Docx] {
        assert!(!r.render(f).unwrap().bytes.is_empty());
    }
}

#[test]
fn without_a_sweep_the_section_says_how_to_make_one() {
    let r = report(&synthetic(), None);
    let html = html_text(&r.render(Format::Html).unwrap().bytes);
    let at = html
        .find("Largest vessel The largest vessel the river allows is not in the design yet (")
        .expect("the note");
    assert!(
        html[at..at + 120].contains("owd envelope"),
        "{}",
        &html[at..at + 120]
    );
    assert!(!r.assets.contains_key("envelope-sizes"));
}

#[test]
fn a_stale_sweep_says_so() {
    let (design, _) = with_sweep();
    let mut s = Session::open(&design).unwrap();
    let left = &s.document()["inputs"]["left_bank"];
    let mut moved = left.clone();
    for p in moved["coordinates"].as_array_mut().unwrap() {
        p[1] = serde_json::json!(p[1].as_f64().unwrap() + 1.0);
    }
    let crs = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
    let text = format!(
        r#"{{"type":"LineString",{crs},"coordinates":{}}}"#,
        moved["coordinates"]
    );
    s.import(&[(Role::LeftBank, text.as_str())]).unwrap();
    let html = html_text(
        &report(&s.to_text(), None)
            .render(Format::Html)
            .unwrap()
            .bytes,
    );
    assert!(section(&html).contains("stale"), "{}", section(&html));
}
