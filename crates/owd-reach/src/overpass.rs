//! OSM waterways through Overpass: a name search and the waterway graph for a bounding box. The
//! only network access is the `Fetch` it is given. OSM is ODbL: the geometry read here is used to
//! route, and only way ids reach the document.

use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::geo::LonLat;
use serde_json::Value;
use std::collections::BTreeMap;

/// The public Overpass interpreter.
pub const DEFAULT_ENDPOINT: &str = "https://overpass-api.de/api/interpreter";

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

/// Overpass queries over a `Fetch`.
pub struct Overpass<'a> {
    fetch: &'a dyn Fetch,
    endpoint: String,
}

impl<'a> Overpass<'a> {
    pub fn new(fetch: &'a dyn Fetch) -> Self {
        Self::with_endpoint(fetch, DEFAULT_ENDPOINT)
    }

    /// Use another Overpass server (there are public mirrors).
    pub fn with_endpoint(fetch: &'a dyn Fetch, endpoint: &str) -> Self {
        Overpass {
            fetch,
            endpoint: endpoint.to_string(),
        }
    }

    /// The request URL of a query.
    pub fn url(&self, query: &str) -> String {
        format!("{}?data={}", self.endpoint, urlencode(query))
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
        let body = self.fetch.get(&self.url(&Self::search_query(name)))?;
        parse_candidates(&body)
    }

    /// Every waterway in a bounding box.
    pub fn waterways(&self, bbox: [f64; 4]) -> Result<Vec<Way>, ReachError> {
        let body = self.fetch.get(&self.url(&Self::waterways_query(bbox)))?;
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
