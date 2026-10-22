//! The river search and the whole river through the app state (spec 0012, T13 to T15), on recorded
//! answers: Photon for "Ank" and the Ankobra relation from Overpass, both recorded on 2026-10-22
//! (`examples/reach/photon_ank.json`, `examples/reach/overpass_ankobra.json`, © OpenStreetMap
//! contributors).
//!
//! Label: property

use owd_reach::ReplayFetch;
use owd_shell::reach::{ReachEndpoints, read_river, search_rivers};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::Path;

fn ex(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/reach")
            .join(name),
    )
    .unwrap()
}

fn endpoints() -> ReachEndpoints {
    ReachEndpoints {
        overpass: "https://overpass.example/api/interpreter".into(),
        photon: "https://photon.example/api/".into(),
        ..ReachEndpoints::default()
    }
}

fn fetch() -> ReplayFetch {
    ReplayFetch::new()
        .with_response_containing("photon.example/api/?q=Ank&", ex("photon_ank.json"))
        .with_response_containing("rel%283328713%29", ex("overpass_ankobra.json"))
}

#[test]
fn ank_lists_the_ankobra_and_short_text_asks_nothing() {
    let f = fetch();
    let hits = search_rivers(&f, &endpoints(), "Ank", None).unwrap();
    let ank = hits
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["osm_id"] == "relation/3328713")
        .expect("listed");
    assert_eq!(ank["label"], "Ankobra River \u{2014} Western Region, Ghana");
    assert_eq!(
        search_rivers(&f, &endpoints(), "An", None).unwrap(),
        json!([])
    );
    assert_eq!(f.requests().len(), 1);
}

#[test]
fn choosing_a_river_starts_a_study_at_its_mouth_and_a_cut_needs_no_new_request() {
    let f = fetch();
    let river = read_river(&f, &endpoints(), "relation/3328713", &[]).unwrap();
    let mut s = AppState::new_design("t", "2026-10-22");
    s.start_river(river, "2026-10-22").unwrap();
    let snap = s.snapshot();
    let view = &snap["reach_view"];
    assert_eq!(view["start"], json!([-2.2688538, 4.8983929]));
    assert_eq!(view["end"], json!([-2.1349229, 5.7294889]));
    assert_eq!(view["river"]["osm_id"], "relation/3328713");
    let whole = view["length_m"].as_f64().unwrap();
    assert_eq!(view["river"]["length_m"].as_f64().unwrap(), whole);
    let doc: &Value = &snap["document"];
    assert_eq!(
        doc["provenance"]["route"]["settings"]["osm_id"],
        "relation/3328713"
    );
    assert_eq!(
        doc["provenance"]["route"]["settings"]["river_search"]["source"],
        "osm-photon"
    );
    assert_eq!(doc["project"]["title"], "Ankobra River");

    // Drag the end down to the middle of the river: a shorter reach, and no new request.
    let asked = f.requests().len();
    let line = view["river"]["line"].as_array().unwrap();
    let mid = &line[line.len() / 2];
    let mid = [mid[0].as_f64().unwrap(), mid[1].as_f64().unwrap()];
    s.reach_cut([-2.2688538, 4.8983929], mid, &[], "2026-10-22")
        .unwrap();
    let cut = s.snapshot()["reach_view"].clone();
    assert!(cut["length_m"].as_f64().unwrap() < whole);
    assert_eq!(cut["end"], json!(mid));
    assert_eq!(cut["river"]["length_m"].as_f64().unwrap(), whole);
    assert_eq!(f.requests().len(), asked);
}

#[test]
fn a_cut_with_no_river_chosen_says_what_to_do() {
    let mut s = AppState::new_design("t", "2026-10-22");
    let e = s
        .reach_cut([0.0, 0.0], [0.1, 0.0], &[], "2026-10-22")
        .unwrap_err();
    assert!(e.contains("choose a river"), "{e}");
}
