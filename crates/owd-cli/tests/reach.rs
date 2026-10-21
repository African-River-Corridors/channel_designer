//! `owd reach` end to end against a local server that serves the recorded Overpass answers
//! (`examples/reach/synthetic/`), so no test touches the network: a route from a start and an end, a
//! via point that picks a branch, a trace with a gap, and the document and notice that come out.
//!
//! Label: property

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NETWORK: &str = include_str!("../../../examples/reach/synthetic/network.json");
const SEARCH: &str = include_str!("../../../examples/reach/synthetic/search_alpha.json");

/// Serve the two recorded answers on a free port until the test process ends. The query is in the
/// request line: `out%20geom` asks for the network, `out%20tags%20bb` for the name search.
fn serve() -> String {
    serve_network(NETWORK.to_string())
}

fn serve_network(network: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/interpreter", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 8192];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let first = req.lines().next().unwrap_or("").to_string();
            let ua_ok = req
                .to_lowercase()
                .contains("user-agent: open-waterway-designer/");
            let body = if !ua_ok {
                None
            } else if first.contains("out%20geom") {
                Some(network.as_str())
            } else if first.contains("out%20tags%20bb") {
                Some(SEARCH)
            } else {
                None
            };
            let resp = match body {
                Some(b) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                    b.len()
                ),
                None => {
                    "HTTP/1.1 406 Not Acceptable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                }
            };
            let _ = s.write_all(resp.as_bytes());
        }
    });
    url
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-reach-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn doc(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_river_a_start_and_an_end_make_a_document_with_a_route_and_the_osm_notice() {
    let server = serve();
    let dir = work_dir("points");
    let file = dir.join("reach.owd");
    let o = run(&[
        "reach",
        file.to_str().unwrap(),
        "--river",
        "Alpha",
        "--start",
        "-6.0,5.0",
        "--end",
        "-5.94,5.0",
        "--overpass",
        &server,
        "--date",
        "2026-10-21",
        "--json",
    ]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "{err}");
    let result: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(result["ways"], serde_json::json!([101, 103, 104]));
    assert_eq!(result["as_drawn_stretches"], 0);
    let d = doc(&file);
    assert_eq!(d["project"]["title"], "Alpha");
    assert_eq!(
        d["reach"]["route"][0]["way_ids"],
        serde_json::json!([101, 103, 104])
    );
    assert_eq!(
        d["provenance"]["route"]["notice"],
        "\u{a9} OpenStreetMap contributors"
    );
    // The new document passes `owd validate`.
    let v = run(&["validate", file.to_str().unwrap()]);
    assert_eq!(
        v.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&v.stderr)
    );
}

#[test]
fn a_via_point_picks_the_other_branch() {
    let server = serve();
    let file = work_dir("via").join("reach.owd");
    let o = run(&[
        "reach",
        file.to_str().unwrap(),
        "--start",
        "-6.0,5.0",
        "--end",
        "-5.94,5.0",
        "--via",
        "-5.97,5.012",
        "--overpass",
        &server,
        "--json",
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        doc(&file)["reach"]["route"][0]["way_ids"],
        serde_json::json!([101, 102, 104])
    );
}

#[test]
fn a_trace_with_a_gap_flags_the_stretch_it_kept_as_drawn() {
    let server = serve();
    let dir = work_dir("trace");
    let trace = dir.join("trace.geojson");
    std::fs::write(
        &trace,
        r#"{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[-5.95,5.0],[-5.94,5.0],[-5.90,5.0],[-5.88,5.0]]}}"#,
    )
    .unwrap();
    let file = dir.join("reach.owd");
    let o = run(&[
        "reach",
        file.to_str().unwrap(),
        "--trace",
        trace.to_str().unwrap(),
        "--overpass",
        &server,
    ]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out.contains("kept as drawn"), "{out}");
    assert!(out.contains("OpenStreetMap contributors"), "{out}");
    let d = doc(&file);
    let kinds: Vec<&str> = d["reach"]["route"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["waterway", "as-drawn", "waterway"]);
    assert_eq!(d["reach"]["trace"].as_array().unwrap().len(), 4);
}

#[test]
fn bad_arguments_and_a_busy_server_exit_2_with_a_plain_message() {
    let server = serve();
    let file = work_dir("bad").join("reach.owd");
    let f = file.to_str().unwrap();
    let o = run(&[
        "reach",
        f,
        "--start",
        "oops",
        "--end",
        "-5.94,5.0",
        "--overpass",
        &server,
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("not `lon,lat`"));
    let o = run(&["reach", f]);
    assert_eq!(o.status.code(), Some(2));
    // A server that is not there: the failure names the URL.
    let o = run(&[
        "reach",
        f,
        "--start",
        "-6.0,5.0",
        "--end",
        "-5.94,5.0",
        "--overpass",
        "http://127.0.0.1:1/api/interpreter",
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot fetch http://127.0.0.1:1/"));
    assert!(!file.exists());
}

/// One synthetic way along the river arm of the recorded JRC window (see `jrc_occurrence_sassandra_estuary.tif`).
fn arm_network() -> String {
    let pts = [
        [-6.0715, 4.966],
        [-6.0715, 4.9703],
        [-6.0705, 4.974],
        [-6.0697, 4.977],
        [-6.0695, 4.98],
        [-6.07, 4.983],
    ];
    let nodes: Vec<String> = (0..pts.len()).map(|i| (900 + i).to_string()).collect();
    let geom: Vec<String> = pts
        .iter()
        .map(|p| format!(r#"{{"lat":{},"lon":{}}}"#, p[1], p[0]))
        .collect();
    format!(
        r#"{{"elements":[{{"type":"way","id":901,"nodes":[{}],"geometry":[{}],"tags":{{"waterway":"river"}}}}]}}"#,
        nodes.join(","),
        geom.join(",")
    )
}

fn jrc_tile(dir: &Path) -> PathBuf {
    let tile = dir.join("occurrence_10W_10Nv1_4_2021.tif");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/reach/jrc_occurrence_sassandra_estuary.tif"),
        &tile,
    )
    .unwrap();
    tile
}

#[test]
fn banks_from_the_jrc_mask_go_into_the_document_with_the_notice_and_the_flags() {
    let server = serve_network(arm_network());
    let dir = work_dir("banks");
    let tile = jrc_tile(&dir);
    let file = dir.join("reach.owd");
    let o = run(&[
        "reach",
        file.to_str().unwrap(),
        "--start",
        "-6.0715,4.966",
        "--end",
        "-6.07,4.983",
        "--banks",
        "--jrc-tile",
        tile.to_str().unwrap(),
        "--overpass",
        &server,
        "--json",
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let result: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(result["banks"]["mask_threshold_pct"], 75.0);
    assert_eq!(result["banks"]["low_confidence_stretches"], 3);
    let d = doc(&file);
    for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
        assert_eq!(d["provenance"][input]["source"], "jrc-gsw");
        assert_eq!(d["provenance"][input]["mark"], "default");
        assert!(d["inputs"][input].is_object(), "{input}");
    }
    assert_eq!(
        d["low_confidence_width"]["stretches"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(d["project"]["crs"]["epsg"], 32629);
    let v = run(&["validate", file.to_str().unwrap()]);
    assert_eq!(
        v.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&v.stderr)
    );
    // Both notices are in the document's provenance.
    assert!(
        d["provenance"]["route"]["notice"]
            .as_str()
            .unwrap()
            .contains("OpenStreetMap")
    );
}

#[test]
fn a_river_the_mask_cannot_see_or_a_missing_tile_exits_2_with_what_to_do() {
    let server = serve();
    let dir = work_dir("banks-bad");
    let tile = jrc_tile(&dir);
    let file = dir.join("reach.owd");
    let f = file.to_str().unwrap();
    // The synthetic network lies where the recorded window has no water at 75 %.
    let o = run(&[
        "reach",
        f,
        "--start",
        "-6.0,5.0",
        "--end",
        "-5.98,5.0",
        "--banks",
        "--jrc-tile",
        tile.to_str().unwrap(),
        "--overpass",
        &server,
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("Lower the threshold"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    // A route in another tile than the one given.
    let o = run(&[
        "reach",
        f,
        "--start",
        "-6.0,5.0",
        "--end",
        "-5.94,5.0",
        "--banks",
        "--jrc-tile",
        dir.join("nope.tif").to_str().unwrap(),
        "--overpass",
        &server,
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!file.exists());
}

#[test]
fn edge_fit_moves_the_mask_banks_onto_a_recorded_scene_and_records_the_scene() {
    // In process, with every answer recorded: the river arm (Overpass), the JRC window (a dropped
    // tile) and the Sentinel-2 search and band windows of 23 December 2023.
    use owd_cli::commands::{ReachFlags, reach_with};
    use owd_reach::{DroppedFiles, ReplayFetch};
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/reach");
    let base = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/29/N/RF/2023/12/S2B_29NRF_20231223_0_L2A";
    let mut fetch = ReplayFetch::new()
        .with_response_containing("out%20geom", arm_network())
        .with_response_containing(
            "earth-search.aws.element84.com/v1/search",
            std::fs::read(ex.join("stac_search_sassandra.json")).unwrap(),
        );
    for band in ["B03", "B04", "B08", "B11", "SCL"] {
        fetch = fetch.with_response(
            &format!("{base}/{band}.tif"),
            std::fs::read(ex.join(format!("s2_sassandra_{band}.tif"))).unwrap(),
        );
    }
    let jrc = DroppedFiles::new().with_file(
        "occurrence_10W_10Nv1_4_2021.tif",
        std::fs::read(ex.join("jrc_occurrence_sassandra_estuary.tif")).unwrap(),
    );
    let dir = work_dir("edge-fit");
    let file = dir.join("reach.owd");
    let flags = |edge_fit: bool, max_cloud: Option<f64>| ReachFlags {
        banks: true,
        jrc_tile: Vec::new(),
        mask_threshold: None,
        edge_fit,
        max_cloud,
        stac: None,
        file: file.to_str().unwrap().to_string(),
        river: None,
        start: Some("-6.0715,4.966".into()),
        end: Some("-6.07,4.983".into()),
        via: Vec::new(),
        trace: None,
        title: Some("Edge fit".into()),
        overpass: None,
        // Two years back from here holds the 2023-24 dry season.
        date: Some("2024-03-01".into()),
        json: true,
    };
    if let Err(e) = reach_with(flags(true, None), &fetch, &jrc) {
        panic!("{}", e.message);
    }
    let d = doc(&file);
    for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
        let p = &d["provenance"][input];
        assert_eq!(p["source"], "sentinel2-l2a", "{input}");
        assert_eq!(p["date"], "2023-12-23");
        assert_eq!(p["settings"]["scene_id"], "S2B_29NRF_20231223_0_L2A");
        assert!(
            p["notice"]
                .as_str()
                .unwrap()
                .contains("Contains modified Copernicus Sentinel data 2023")
        );
    }
    assert_eq!(d["low_confidence_width"]["source"], "sentinel2-l2a");
    let v = run(&["validate", file.to_str().unwrap()]);
    assert_eq!(
        v.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&v.stderr)
    );
    // No scene clear enough: the run stops and says what to do.
    let Err(e) = reach_with(flags(true, Some(0.001)), &fetch, &jrc) else {
        panic!("a cloud limit no scene meets must fail");
    };
    assert!(e.message.contains("no Sentinel-2 scene"), "{}", e.message);
    assert!(e.message.contains("Raise the cloud limit"), "{}", e.message);
}
