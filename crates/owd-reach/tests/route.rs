//! Routing on recorded Overpass answers (synthetic, from `tools/make_reach_fixtures.py`): a split, a
//! river-to-river crossing and a gap in the network each give the expected route, the document
//! holds way ids only, and the OSM notice is in the provenance and the exports.
//!
//! The network, in `examples/reach/synthetic/network.json`: river Alpha runs west to east (way 101),
//! splits at one node into a longer north branch (102) and a shorter south branch (103), joins again
//! and goes on (104). Stream Beta (201) crosses Alpha's way 104. River Gamma (301) lies 4.4 km east of
//! the end of 104 with no waterway between.
//!
//! Label: property (the expected routes are worked out from the drawing above)

use owd_format::design::Session;
use owd_reach::ReplayFetch;
use owd_reach::overpass::Overpass;
use owd_reach::reach::{plan_points, plan_trace, record_reach};
use owd_reach::route::Part;
use serde_json::Value;

const NETWORK: &str = include_str!("../../../examples/reach/synthetic/network.json");
const SEARCH: &str = include_str!("../../../examples/reach/synthetic/search_alpha.json");

// Test attribute that runs natively and, with wasm-bindgen-test, in wasm.
macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

fn fetch() -> ReplayFetch {
    ReplayFetch::new()
        .with_response_containing("out%20geom", NETWORK)
        .with_response_containing("out%20tags%20bb", SEARCH)
}

const START: [f64; 2] = [-6.0, 5.0];

t! {
fn at_a_split_the_shorter_branch_is_the_default_and_a_via_point_picks_the_other() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let end = [-5.94, 5.0];
    let direct = plan_points(&ov, None, START, end, &[]).unwrap();
    assert_eq!(direct.route.way_ids(), vec![101, 103, 104]);
    // A point on the north branch (about 1.3 km from its way) forces it.
    let north = plan_points(&ov, None, START, end, &[[-5.97, 5.012]]).unwrap();
    assert_eq!(north.route.way_ids(), vec![101, 102, 104]);
    assert!(north.route.length_m > direct.route.length_m);
    // Hand calculation (1 deg of longitude at 5 deg N = 110.8 km, 1 deg of latitude = 111.2 km):
    // 2 x 1108 m to the split, the south branch 1242 + 1108 + 1242 m, then 2 x 1108 m = 8024 m.
    assert!((direct.route.length_m - 8_024.0).abs() < 10.0, "{}", direct.route.length_m);
    // The north branch is 1444 + 1108 + 1444 m: 8428 m in all.
    assert!((north.route.length_m - 8_428.0).abs() < 10.0, "{}", north.route.length_m);
}
}

t! {
fn a_route_crosses_from_one_river_to_another() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let p = plan_points(&ov, Some("Alpha"), START, [-5.95, 4.985], &[]).unwrap();
    assert_eq!(p.route.way_ids(), vec![101, 103, 104, 201]);
    assert_eq!(p.route.as_drawn_count(), 0);
}
}

t! {
fn the_name_search_gives_candidates_with_their_extent() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let c = ov.search_rivers("Alpha").unwrap();
    assert_eq!(c[0].kind, "river");
    assert_eq!(c[0].way_ids, vec![101, 102, 103, 104]);
    assert_eq!(c[0].bbox, [-6.0, 4.99, -5.94, 5.012]);
    assert_eq!(c[1].kind, "canal");
}
}

t! {
fn a_click_far_from_any_waterway_is_refused_with_a_hint() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let e = plan_points(&ov, None, START, [-5.5, 5.5], &[]).unwrap_err();
    assert!(e.to_string().contains("no waterway within"), "{e}");
}
}

t! {
fn a_trace_picks_the_branch_it_follows() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let trace = [[-6.0, 5.0], [-5.98, 5.0], [-5.975, 5.012], [-5.965, 5.012], [-5.96, 5.0], [-5.94, 5.0]];
    let p = plan_trace(&ov, None, &trace).unwrap();
    assert_eq!(p.route.way_ids(), vec![101, 102, 104]);
    assert_eq!(p.route.as_drawn_count(), 0);
    assert_eq!(p.spec.trace.as_ref().unwrap().len(), 6);
}
}

t! {
fn a_gap_in_the_network_keeps_the_trace_as_drawn_and_flags_it() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let trace = [[-5.95, 5.0], [-5.94, 5.0], [-5.90, 5.0], [-5.88, 5.0]];
    let p = plan_trace(&ov, None, &trace).unwrap();
    let kinds: Vec<&str> = p.route.parts.iter().map(|p| match p {
        Part::Waterway { .. } => "waterway",
        Part::AsDrawn { .. } => "as-drawn",
    }).collect();
    assert_eq!(kinds, vec!["waterway", "as-drawn", "waterway"]);
    assert_eq!(p.route.way_ids(), vec![104, 301]);
    assert_eq!(p.route.as_drawn_count(), 1);
    // The drawn stretch runs from where the last waterway left the trace to where the next began:
    // 4.4 km of gap, less the 150 m snap at each end.
    let Part::AsDrawn { line } = &p.route.parts[1] else { unreachable!() };
    let len = owd_reach::geo::line_length_m(line);
    assert!((len - 4_130.0).abs() < 200.0, "{len}");
}
}

t! {
fn the_document_holds_way_ids_and_the_users_points_but_no_osm_coordinates() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let p = plan_points(&ov, Some("Alpha"), START, [-5.94, 5.0], &[]).unwrap();
    let mut s = Session::new("Reach test", "2026-10-21");
    record_reach(&mut s, &p, "2026-10-21").unwrap();
    let text = s.to_text();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["reach"]["route"][0]["way_ids"], serde_json::json!([101, 103, 104]));
    assert_eq!(doc["reach"]["river"], "Alpha");
    // Every OSM node coordinate of the fixture, other than the two the user clicked: none in the text.
    let net: Value = serde_json::from_str(NETWORK).unwrap();
    for w in net["elements"].as_array().unwrap() {
        for pt in w["geometry"].as_array().unwrap() {
            let (lon, lat) = (pt["lon"].as_f64().unwrap(), pt["lat"].as_f64().unwrap());
            let clicked = [START, [-5.94, 5.0]].iter().any(|c| c[0] == lon && c[1] == lat);
            if !clicked {
                assert!(!text.contains(&format!("[{lon},{lat}]")), "OSM node {lon},{lat} was written");
            }
        }
    }
    assert!(Session::open(&text).unwrap().validate().is_empty());
}
}

t! {
fn the_osm_notice_is_in_the_provenance_and_in_every_export() {
    let f = fetch();
    let ov = Overpass::new(&f);
    let p = plan_points(&ov, None, START, [-5.94, 5.0], &[]).unwrap();
    let mut s = Session::new("Reach test", "2026-10-21");
    record_reach(&mut s, &p, "2026-10-21").unwrap();
    assert_eq!(s.provenance("route").unwrap()["notice"], "\u{a9} OpenStreetMap contributors");
    assert_eq!(s.provenance("route").unwrap()["mark"], "default");
    assert_eq!(s.notices(), vec!["\u{a9} OpenStreetMap contributors"]);
}
}
