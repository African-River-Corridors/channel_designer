//! The river search and the whole river through the app state (spec 0012, T13 to T15), on recorded
//! answers: Photon for "Ank" and the Ankobra relation from Overpass, both recorded on 2026-10-22
//! (`examples/reach/photon_ank.json`, `examples/reach/overpass_ankobra.json`, © OpenStreetMap
//! contributors).
//!
//! Label: property

use owd_reach::ReplayFetch;
use owd_shell::reach::{ReachEndpoints, plan_restore, read_river, search_rivers};
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
    let hits = search_rivers(&f, &endpoints(), "Ank", None, false).unwrap();
    let ank = hits
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["osm_id"] == "relation/3328713")
        .expect("listed");
    assert_eq!(ank["label"], "Ankobra River \u{2014} Western Region, Ghana");
    assert_eq!(
        search_rivers(&f, &endpoints(), "An", None, false).unwrap(),
        json!([])
    );
    assert_eq!(f.requests().len(), 1);
}

#[test]
fn choosing_a_river_starts_a_study_at_its_mouth_and_a_cut_needs_no_new_request() {
    let f = fetch();
    let river = read_river(&f, &endpoints(), "relation/3328713", &[]).unwrap();
    let mut s = AppState::new_design("t", "2026-10-22");
    s.start_river(river, Some("osm-photon"), "2026-10-22")
        .unwrap();
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
fn when_photon_fails_it_says_so_and_nominatim_answers_once() {
    let f = ReplayFetch::new().with_response_containing(
        "nominatim.example/search?q=Ankobra&",
        ex("nominatim_ankobra.json"),
    );
    let at = ReachEndpoints {
        nominatim: "https://nominatim.example/search".into(),
        ..endpoints()
    };
    let e = search_rivers(&f, &at, "Ankobra", None, false).unwrap_err();
    assert!(e.contains("Photon) is not answering"), "{e}");
    assert!(e.contains("Press Enter"), "{e}");
    let hits = search_rivers(&f, &at, "Ankobra", None, true).unwrap();
    assert_eq!(hits[0]["osm_id"], "relation/3328713");
    let mut s = AppState::new_design("t", "2026-10-22");
    let river = read_river(
        &ReplayFetch::new()
            .with_response_containing("rel%283328713%29", ex("overpass_ankobra.json")),
        &at,
        "relation/3328713",
        &[],
    )
    .unwrap();
    s.start_river(river, Some("osm-nominatim"), "2026-10-22")
        .unwrap();
    let p = &s.snapshot()["document"]["provenance"]["route"]["settings"];
    assert_eq!(p["river_search"]["source"], "osm-nominatim");
    assert_eq!(p["osm_id"], "relation/3328713");
}

#[test]
fn a_cut_with_no_river_chosen_says_what_to_do() {
    let mut s = AppState::new_design("t", "2026-10-22");
    let e = s
        .reach_cut([0.0, 0.0], [0.1, 0.0], &[], "2026-10-22")
        .unwrap_err();
    assert!(e.contains("choose a river"), "{e}");
}

/// Save a study, open it again: the panel draws the same route (#318).
fn reopen(s: &mut AppState) -> AppState {
    let path = Path::new("/tmp/study.owd");
    let text = s.save_text_for(path);
    AppState::open(path, &text, None).unwrap()
}

#[test]
fn an_opened_study_from_a_river_draws_its_route_again_and_keeps_its_document() {
    let f = fetch();
    let river = read_river(&f, &endpoints(), "relation/3328713", &[]).unwrap();
    let mut s = AppState::new_design("t", "2026-10-22");
    s.start_river(river, Some("osm-photon"), "2026-10-22")
        .unwrap();
    let line = s.snapshot()["reach_view"]["river"]["line"].clone();
    let mid = &line[line.as_array().unwrap().len() / 2];
    let mid = [mid[0].as_f64().unwrap(), mid[1].as_f64().unwrap()];
    s.reach_cut([-2.2688538, 4.8983929], mid, &[], "2026-10-22")
        .unwrap();
    let before = s.snapshot();

    let mut opened = reopen(&mut s);
    let view = &opened.snapshot()["reach_view"];
    assert_eq!(view["route"], Value::Null);
    assert_eq!(view["restore"], true);
    let doc = opened.snapshot()["document"].clone();
    let restored = plan_restore(&f, &endpoints(), &doc).unwrap();
    opened.restore_reach(restored).unwrap();
    let after = opened.snapshot();
    assert_eq!(after["reach_view"]["route"], before["reach_view"]["route"]);
    assert_eq!(
        after["reach_view"]["length_m"],
        before["reach_view"]["length_m"]
    );
    assert_eq!(after["reach_view"]["river"]["osm_id"], "relation/3328713");
    assert_eq!(after["reach_view"]["restore"], Value::Null);
    assert_eq!(
        after["document"], doc,
        "drawing the route again changes nothing in the document"
    );
    assert!(!opened.is_dirty());
}

#[test]
fn a_study_read_again_after_a_fetch_keeps_its_route_and_another_does_not() {
    let f = fetch();
    let river = read_river(&f, &endpoints(), "relation/3328713", &[]).unwrap();
    let mut s = AppState::new_design("t", "2026-10-22");
    s.start_river(river, Some("osm-photon"), "2026-10-22")
        .unwrap();
    let route = s.snapshot()["reach_view"]["route"].clone();
    let opened = reopen(&mut s);
    let next = s.reopened(opened);
    assert_eq!(next.snapshot()["reach_view"]["route"], route);
    // A different study read in its place starts with nothing held.
    let next = s.reopened(AppState::new_design("t", "2026-10-22"));
    assert_eq!(next.snapshot()["reach_view"]["route"], Value::Null);
}

#[test]
fn an_opened_study_routed_from_points_is_planned_again_from_its_points() {
    let net = ex("synthetic/network.json");
    let f = ReplayFetch::new().with_response_containing("out%20geom", net);
    let at = endpoints();
    let mut s = AppState::new_design("t", "2026-10-22");
    s.reach_route(&f, &at, None, [-6.0, 5.0], [-5.94, 5.0], &[], "2026-10-22")
        .unwrap();
    let before = s.snapshot()["reach_view"]["route"].clone();
    let mut opened = reopen(&mut s);
    let doc = opened.snapshot()["document"].clone();
    opened
        .restore_reach(plan_restore(&f, &at, &doc).unwrap())
        .unwrap();
    assert_eq!(opened.snapshot()["reach_view"]["route"], before);
}

#[test]
fn a_study_from_a_river_is_named_by_date_river_and_kilometres_from_the_mouth() {
    let f = fetch();
    let river = read_river(&f, &endpoints(), "relation/3328713", &[]).unwrap();
    let mut s = AppState::new_design("t", "2026-10-22");
    s.start_river(river, Some("osm-photon"), "2026-10-22")
        .unwrap();
    assert_eq!(
        s.study_folder_name("2026-10-22").unwrap(),
        "2026-10-22 Ankobra River km 0-166"
    );
    // A reach cut from a point up the river to the far end starts at that point's kilometre.
    let line = s.snapshot()["reach_view"]["river"]["line"].clone();
    let line = line.as_array().unwrap();
    let mid = &line[line.len() / 2];
    let mid = [mid[0].as_f64().unwrap(), mid[1].as_f64().unwrap()];
    s.reach_cut(mid, [-2.1349229, 5.7294889], &[], "2026-10-22")
        .unwrap();
    let name = s.study_folder_name("2026-10-22").unwrap();
    let km: Vec<f64> = name
        .rsplit(' ')
        .next()
        .unwrap()
        .split('-')
        .map(|k| k.parse().unwrap())
        .collect();
    assert!(km[0] > 40.0 && km[0] < 130.0, "{name}");
    assert_eq!(km[1], 166.0, "{name}");
    assert!(
        AppState::new_design("t", "2026-10-22")
            .study_folder_name("2026-10-22")
            .is_none()
    );
}
