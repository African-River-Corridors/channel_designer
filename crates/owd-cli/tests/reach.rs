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
                Some(NETWORK)
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
