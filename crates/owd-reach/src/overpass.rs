//! OSM waterways through Overpass: a name search and the waterway graph for a bounding box. The
//! only network access is the `Fetch` it is given. OSM is ODbL: the geometry read here is used to
//! route, and only way ids reach the document.

use crate::error::ReachError;
use crate::fetch::{Fetch, FetchError};
use crate::geo::LonLat;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The public Overpass interpreter.
pub const DEFAULT_ENDPOINT: &str = "https://overpass-api.de/api/interpreter";

/// Public mirrors, tried in order when the main server fails (#319).
pub const MIRRORS: &[&str] = &[
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
];

/// The main server then the mirrors, as one setting: endpoints separated by commas.
pub fn default_endpoints() -> String {
    std::iter::once(DEFAULT_ENDPOINT)
        .chain(MIRRORS.iter().copied())
        .collect::<Vec<_>>()
        .join(",")
}

/// The host of an endpoint, for messages and the provenance.
pub fn host(endpoint: &str) -> &str {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// The waterway kinds OWD routes on.
const KINDS: &str = "^(river|stream|canal)$";

/// An OSM waterway with its geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Way {
    pub id: i64,
    pub nodes: Vec<i64>,
    /// One position per node.
    pub geom: Vec<LonLat>,
    pub name: Option<String>,
    pub kind: String,
}

/// A river found by name: every way with that name and kind, and their extent.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub kind: String,
    pub way_ids: Vec<i64>,
    /// `[west, south, east, north]`.
    pub bbox: [f64; 4],
}

/// Percent-encode a query value (RFC 3986 unreserved characters stay).
pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Escape text for an Overpass regular expression inside double quotes.
fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push('\\');
                out.push('\\');
                out.push(c);
            }
            '.' | '^' | '$' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}' => {
                out.push('\\');
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Overpass queries over a `Fetch`. A server that fails (an HTTP error, a timeout, or a busy page
/// instead of JSON) is asked once more, then the next server in the list is asked.
pub struct Overpass<'a> {
    fetch: &'a dyn Fetch,
    endpoints: Vec<String>,
    /// What happened, in order: which server failed and why, which answered.
    log: RefCell<Vec<String>>,
    answered: RefCell<Option<String>>,
}

/// Worth asking again or asking another server: the server failed, not the request.
fn retryable(e: &FetchError) -> bool {
    matches!(e, FetchError::Failed { .. })
}

impl<'a> Overpass<'a> {
    /// The main server, then the public mirrors.
    pub fn new(fetch: &'a dyn Fetch) -> Self {
        Self::with_endpoint(fetch, &default_endpoints())
    }

    /// Other Overpass servers: one, or several separated by commas, tried in that order.
    pub fn with_endpoint(fetch: &'a dyn Fetch, endpoints: &str) -> Self {
        let mut list: Vec<String> = endpoints
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect();
        if list.is_empty() {
            list.push(DEFAULT_ENDPOINT.to_string());
        }
        Overpass {
            fetch,
            endpoints: list,
            log: RefCell::new(Vec::new()),
            answered: RefCell::new(None),
        }
    }

    /// The request URL of a query on the first server.
    pub fn url(&self, query: &str) -> String {
        Self::url_at(&self.endpoints[0], query)
    }

    fn url_at(endpoint: &str, query: &str) -> String {
        format!("{endpoint}?data={}", urlencode(query))
    }

    /// The server that answered the last query.
    pub fn answered_by(&self) -> Option<String> {
        self.answered.borrow().clone()
    }

    /// What happened when a server failed, for the user: empty when the first server answered
    /// every query.
    pub fn notes(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    /// Ask each server in turn, each twice at most.
    fn ask(&self, query: &str) -> Result<Vec<u8>, ReachError> {
        let mut last: Option<ReachError> = None;
        for (i, endpoint) in self.endpoints.iter().enumerate() {
            let url = Self::url_at(endpoint, query);
            for _ in 0..2 {
                match self.fetch.get(&url) {
                    // Overpass answers JSON; a busy server can send an HTML page with status 200.
                    Ok(body) if body.trim_ascii_start().first() == Some(&b'{') => {
                        if i > 0 || last.is_some() {
                            self.log
                                .borrow_mut()
                                .push(format!("{} answered", host(endpoint)));
                        }
                        *self.answered.borrow_mut() = Some(endpoint.clone());
                        return Ok(body);
                    }
                    Ok(_) => {
                        last = Some(ReachError::Parse(format!(
                            "{} sent a page instead of JSON (busy)",
                            host(endpoint)
                        )));
                    }
                    Err(e) if retryable(&e) => last = Some(ReachError::Fetch(e)),
                    Err(e) => return Err(e.into()),
                }
            }
            if let Some(e) = &last {
                self.log
                    .borrow_mut()
                    .push(format!("{} failed ({e})", host(endpoint)));
            }
        }
        let tried: Vec<&str> = self.endpoints.iter().map(|e| host(e)).collect();
        Err(match last {
            Some(ReachError::Fetch(f)) => ReachError::Fetch(FetchError::Failed {
                url: tried.join(", "),
                reason: format!("no Overpass server answered ({f})"),
            }),
            Some(e) => e,
            None => ReachError::Input("no Overpass server to ask".into()),
        })
    }

    /// The query for rivers, streams and canals named `name` (any case).
    pub fn search_query(name: &str) -> String {
        format!(
            "[out:json][timeout:60];way[\"waterway\"~\"{KINDS}\"][\"name\"~\"^{}$\",i];out tags bb;",
            regex_escape(name.trim())
        )
    }

    /// The query for every waterway in a bounding box `[west, south, east, north]`.
    pub fn waterways_query(bbox: [f64; 4]) -> String {
        format!(
            "[out:json][timeout:120];way[\"waterway\"~\"{KINDS}\"]({},{},{},{});out geom;",
            bbox[1], bbox[0], bbox[3], bbox[2]
        )
    }

    /// Rivers called `name`, one candidate per name and kind, the one with most ways first.
    pub fn search_rivers(&self, name: &str) -> Result<Vec<Candidate>, ReachError> {
        if name.trim().is_empty() {
            return Err(ReachError::Input("the river name is empty".into()));
        }
        let body = self.ask(&Self::search_query(name))?;
        parse_candidates(&body)
    }

    /// The raw answer to a query.
    pub fn get(&self, query: &str) -> Result<Vec<u8>, ReachError> {
        self.ask(query)
    }

    /// Every waterway in a bounding box.
    pub fn waterways(&self, bbox: [f64; 4]) -> Result<Vec<Way>, ReachError> {
        let body = self.ask(&Self::waterways_query(bbox))?;
        parse_ways(&body)
    }
}

fn elements(body: &[u8]) -> Result<Vec<Value>, ReachError> {
    let v: Value = serde_json::from_slice(body).map_err(|e| {
        ReachError::Parse(format!(
            "the Overpass answer is not JSON ({e}); the server may be busy"
        ))
    })?;
    v["elements"]
        .as_array()
        .cloned()
        .ok_or_else(|| ReachError::Parse("the Overpass answer has no `elements`".into()))
}

/// Candidates from a `out tags bb` answer.
pub fn parse_candidates(body: &[u8]) -> Result<Vec<Candidate>, ReachError> {
    let mut by_key: BTreeMap<(String, String), Candidate> = BTreeMap::new();
    for e in elements(body)? {
        if e["type"] != "way" {
            continue;
        }
        let (Some(id), Some(b)) = (e["id"].as_i64(), e["bounds"].as_object()) else {
            continue;
        };
        let g = |k: &str| b.get(k).and_then(Value::as_f64);
        let (Some(w), Some(s), Some(ea), Some(n)) =
            (g("minlon"), g("minlat"), g("maxlon"), g("maxlat"))
        else {
            continue;
        };
        let name = e["tags"]["name"].as_str().unwrap_or("").to_string();
        let kind = e["tags"]["waterway"].as_str().unwrap_or("").to_string();
        let c = by_key
            .entry((name.clone(), kind.clone()))
            .or_insert(Candidate {
                name,
                kind,
                way_ids: Vec::new(),
                bbox: [w, s, ea, n],
            });
        c.way_ids.push(id);
        c.bbox = [
            c.bbox[0].min(w),
            c.bbox[1].min(s),
            c.bbox[2].max(ea),
            c.bbox[3].max(n),
        ];
    }
    let mut out: Vec<Candidate> = by_key.into_values().collect();
    out.sort_by(|a, b| {
        b.way_ids
            .len()
            .cmp(&a.way_ids.len())
            .then(a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Ways from an `out geom` answer. A way with no geometry, or with fewer than two nodes, is skipped.
pub fn parse_ways(body: &[u8]) -> Result<Vec<Way>, ReachError> {
    let mut ways = Vec::new();
    for e in elements(body)? {
        if e["type"] != "way" {
            continue;
        }
        let Some(id) = e["id"].as_i64() else { continue };
        let nodes: Vec<i64> = e["nodes"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        let geom: Vec<LonLat> = e["geometry"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some([p["lon"].as_f64()?, p["lat"].as_f64()?]))
                    .collect()
            })
            .unwrap_or_default();
        if nodes.len() < 2 || nodes.len() != geom.len() {
            continue;
        }
        ways.push(Way {
            id,
            nodes,
            geom,
            name: e["tags"]["name"].as_str().map(str::to_string),
            kind: e["tags"]["waterway"].as_str().unwrap_or("").to_string(),
        });
    }
    Ok(ways)
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::fetch::ReplayFetch;

    #[test]
    fn queries_name_the_kinds_and_escape_the_name() {
        let q = Overpass::search_query("Sassandra");
        assert!(
            q.contains("river|stream|canal") && q.contains("^Sassandra$"),
            "{q}"
        );
        assert!(Overpass::search_query("A.B (x)").contains("A\\\\.B \\\\(x\\\\)"));
        let w = Overpass::waterways_query([-6.7, 4.9, -6.0, 5.4]);
        assert!(w.contains("(4.9,-6.7,5.4,-6)"), "{w}"); // Overpass takes south, west, north, east
        assert_eq!(urlencode("a b[\"]"), "a%20b%5B%22%5D");
    }

    #[test]
    fn candidates_group_by_name_and_kind_with_their_extent() {
        let body = br#"{"elements":[
          {"type":"way","id":1,"bounds":{"minlat":5,"minlon":-6,"maxlat":5.1,"maxlon":-5.9},"tags":{"name":"Sassandra","waterway":"river"}},
          {"type":"way","id":2,"bounds":{"minlat":5.1,"minlon":-5.9,"maxlat":5.3,"maxlon":-5.7},"tags":{"name":"Sassandra","waterway":"river"}},
          {"type":"way","id":3,"bounds":{"minlat":4,"minlon":-6,"maxlat":4.1,"maxlon":-5.9},"tags":{"name":"Sassandra","waterway":"canal"}}]}"#;
        let c = parse_candidates(body).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].kind, "river");
        assert_eq!(c[0].way_ids, vec![1, 2]);
        assert_eq!(c[0].bbox, [-6.0, 5.0, -5.7, 5.3]);
        let f = ReplayFetch::new();
        let ov = Overpass::new(&f);
        let f2 = ReplayFetch::new()
            .with_response(&ov.url(&Overpass::search_query("Sassandra")), body.to_vec());
        assert_eq!(Overpass::new(&f2).search_rivers("Sassandra").unwrap(), c);
    }

    /// A server that fails with an HTTP status, then (for the other hosts) replays.
    struct Down<'a> {
        host: &'a str,
        status: &'a str,
        then: ReplayFetch,
    }
    impl Fetch for Down<'_> {
        fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            if url.contains(self.host) {
                return Err(FetchError::Failed {
                    url: url.into(),
                    reason: format!("http status: {}", self.status),
                });
            }
            self.then.get(url)
        }
        fn get_range(&self, url: &str, s: u64, l: u64) -> Result<Vec<u8>, FetchError> {
            self.then.get_range(url, s, l)
        }
    }

    const NET: &[u8] = br#"{"elements":[{"type":"way","id":1,"nodes":[10,11],"geometry":[{"lat":5,"lon":-6},{"lat":5,"lon":-5.99}],"tags":{"waterway":"river"}}]}"#;

    #[test]
    fn a_failing_server_is_asked_once_more_then_the_mirror_answers_and_says_so() {
        let f = Down {
            host: "overpass-api.de",
            status: "502",
            then: ReplayFetch::new()
                .with_response_containing("overpass.kumi.systems", NET.to_vec()),
        };
        let ov = Overpass::new(&f);
        let ways = ov.waterways([-6.1, 4.9, -5.9, 5.1]).unwrap();
        assert_eq!(ways.len(), 1);
        assert_eq!(ov.answered_by().as_deref(), Some(MIRRORS[0]));
        let notes = ov.notes();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0].starts_with("overpass-api.de failed") && notes[0].contains("502"),
            "{notes:?}"
        );
        assert_eq!(notes[1], "overpass.kumi.systems answered");
    }

    #[test]
    fn a_busy_page_counts_as_a_failure_and_when_all_fail_every_server_is_named() {
        let f = ReplayFetch::new()
            .with_response_containing("a.example", b"<html>too busy</html>".to_vec())
            .with_response_containing("b.example", NET.to_vec());
        let ov = Overpass::with_endpoint(
            &f,
            "https://a.example/api/interpreter, https://b.example/api/interpreter",
        );
        ov.waterways([-6.1, 4.9, -5.9, 5.1]).unwrap();
        assert_eq!(f.requests().len(), 3, "a twice, then b");
        assert_eq!(
            ov.answered_by().as_deref(),
            Some("https://b.example/api/interpreter")
        );
        let down = Down {
            host: "example",
            status: "504",
            then: ReplayFetch::new(),
        };
        let e = Overpass::with_endpoint(&down, "https://a.example/x,https://b.example/x")
            .waterways([0.0, 0.0, 1.0, 1.0])
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("a.example, b.example") && e.contains("no Overpass server answered"),
            "{e}"
        );
        // A request the server rejects for good is not sent to the mirrors.
        let gone = ReplayFetch::new();
        let ov = Overpass::new(&gone);
        assert!(ov.waterways([0.0, 0.0, 1.0, 1.0]).is_err());
        assert_eq!(gone.requests().len(), 1);
    }

    #[test]
    fn the_setting_takes_one_server_or_a_list_and_names_hosts() {
        assert_eq!(default_endpoints().split(',').count(), 1 + MIRRORS.len());
        assert_eq!(
            host("https://overpass-api.de/api/interpreter"),
            "overpass-api.de"
        );
        let f = ReplayFetch::new();
        assert!(
            Overpass::with_endpoint(&f, "")
                .url("x")
                .starts_with(DEFAULT_ENDPOINT)
        );
    }

    #[test]
    fn a_busy_server_page_is_a_clear_error_and_bad_ways_are_skipped() {
        let e = parse_ways(b"<html>busy").unwrap_err();
        assert!(e.to_string().contains("not JSON"), "{e}");
        let body = br#"{"elements":[
          {"type":"way","id":1,"nodes":[10,11],"geometry":[{"lat":5,"lon":-6},{"lat":5,"lon":-5.99}],"tags":{"waterway":"river"}},
          {"type":"way","id":2,"nodes":[10,11,12],"geometry":[{"lat":5,"lon":-6}]},
          {"type":"node","id":3}]}"#;
        let w = parse_ways(body).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].id, 1);
    }
}
