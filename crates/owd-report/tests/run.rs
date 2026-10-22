//! The run report (spec 0014, ticket 5): every hash it prints equals one worked out here by a second
//! path (SHA-256 of the bytes with `sha2`, over the canonical text, the trace or the sheet file), a
//! changed DEM byte shows as a mismatch, and the versions are the workspace's pins.
//!
//! Label: property. No number is pinned.

mod common;

use common::*;
use owd_format::design::Session;
use owd_format::design::hash::canonical;
use owd_format::drawing::{self, DrawOptions};
use owd_report::{Control, Format, Kind, Report, Request, build};
use sha2::{Digest, Sha256};

fn run_report(design: &str, dem: Option<&[u8]>) -> Report {
    let req = Request {
        kind: Kind::Run,
        option: None,
        control: Control::draft(DATE),
        requested_by: Some("owd report test --kind run".into()),
    };
    build(design, dem, &req).unwrap()
}

/// SHA-256 as the report prints it: four groups of 16 hex digits.
fn sha(bytes: &[u8]) -> String {
    let h: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (0..4)
        .map(|i| h[i * 16..i * 16 + 16].to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn html(r: &Report) -> String {
    html_text(&r.render(Format::Html).unwrap().bytes)
}

#[test]
fn both_examples_render_in_three_formats_and_ord_accepts_them() {
    let (design, dem) = web();
    for (name, r) in [
        ("web", run_report(&design, Some(&dem))),
        ("synthetic", run_report(&synthetic(), None)),
    ] {
        assert!(r.number.ends_with("-RR"), "{}", r.number);
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
fn every_hash_is_worked_out_again() {
    let (design, dem) = web();
    let s = Session::open(&design).unwrap();
    let d = s.document();
    let text = html(&run_report(&design, Some(&dem)));

    // The request.
    assert!(text.contains("owd report test --kind run"), "{text}");

    // Each input, over its canonical text.
    let inputs: Vec<String> = d["inputs"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, v)| !v.is_null())
        .flat_map(|(k, v)| [k.clone(), sha(canonical(v).as_bytes())])
        .collect();
    in_order(after(&text, "The inputs and their hashes"), &inputs);

    // The DEM, over its bytes, recorded and given.
    in_order(
        after(&text, "The DEM file"),
        &[sha(&dem), sha(&dem), "they match".into()],
    );

    // The stages' recorded dependency hashes.
    let deps: Vec<String> = ["sizing", "alignment", "volumes"]
        .iter()
        .flat_map(|st| {
            let h = d["stages"][st]["depends_on"].as_str().unwrap();
            let g: Vec<String> = (0..4).map(|i| h[i * 16..i * 16 + 16].to_string()).collect();
            [st.to_string(), g.join(" ")]
        })
        .collect();
    in_order(after(&text, "Each stage: status"), &deps);

    // The results: the document, the two traces, and each sheet's normalised file as `owd draw`
    // writes it.
    let mut results = vec![
        sha(canonical(d).as_bytes()),
        sha(canonical(&s.rule_trace(None, 10.0).unwrap()).as_bytes()),
        sha(canonical(&s.rule_trace(Some("rolling-circle"), 10.0).unwrap()).as_bytes()),
    ];
    let files = drawing::files_from_text(&design, Some(&dem), &DrawOptions::default()).unwrap();
    for (name, bytes) in files {
        if name.ends_with(".normalised.json") {
            results.push(format!(
                "Sheet {} {}",
                name.trim_end_matches(".normalised.json").to_uppercase(),
                sha(&bytes)
            ));
        }
    }
    assert!(results.len() > 3);
    in_order(after(&text, "The hashes of the results"), &results);
}

#[test]
fn a_changed_dem_byte_shows_as_a_mismatch() {
    let (design, dem) = web();
    let mut changed = dem.clone();
    changed[1000] ^= 1;
    let text = html(&run_report(&design, Some(&changed)));
    in_order(
        after(&text, "The DEM file"),
        &[sha(&dem), sha(&changed), "they differ".into()],
    );
    assert!(!text.contains("they match"));
}

#[test]
fn the_versions_are_the_workspace_pins() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
    )
    .unwrap();
    let tag = |dep: &str| {
        let l = manifest
            .lines()
            .find(|l| l.starts_with(&format!("{dep} = ")))
            .unwrap();
        l.split("tag = \"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    let (design, dem) = web();
    let text = html(&run_report(&design, Some(&dem)));
    in_order(
        after(&text, "The versions that made this result"),
        &[
            format!("tag {}", tag("ord")),
            format!("tag {}", tag("otd")),
            format!("tag {}", tag("arc-rules")),
        ],
    );
}
