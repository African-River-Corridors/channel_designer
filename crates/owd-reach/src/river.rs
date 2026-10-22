//! A whole river from a search hit (spec 0012, T14): read the waterways of one OSM relation or way
//! over Overpass, find the main stem's mouth and upstream end, and route between them. The user then
//! cuts a reach out of it with a start, an end and via points.
//!
//! OSM draws a waterway in the direction of flow. The **mouth** is a way end that starts no other
//! way of the river; the **upstream end** is a way start that ends no other. Of all such pairs, the
//! one with the longest route between them is the main stem, so a tributary or a side channel does
//! not win, and of two delta mouths the one farther from the source does.

use crate::error::ReachError;
use crate::geo::LonLat;
use crate::overpass::{Overpass, Way, parse_ways};
use crate::provenance::provenance_record;
use crate::reach::{Planned, ReachSpec, record_reach};
use crate::route::{Graph, Route};
use crate::search::OsmRef;
use owd_format::design::Session;
use owd_format::design::provenance::Mark;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The waterway kinds a river's main stem is made of.
const KINDS: &[&str] = &["river", "stream", "canal"];
/// How many times a lone way's search box may grow while the river still reaches its edge.
const GROW_ROUNDS: usize = 3;

/// A whole river: its ways, and the main stem from the mouth to the upstream end.
#[derive(Debug, Clone, PartialEq)]
pub struct WholeRiver {
    pub osm: OsmRef,
    pub name: Option<String>,
    pub mouth: LonLat,
    pub upstream: LonLat,
    /// From the mouth to the upstream end.
    pub route: Route,
    /// Every way of the river, for cutting a reach out of it. OSM geometry: never written into the
    /// document.
    pub ways: Vec<Way>,
}

/// Escape text for an exact Overpass tag match inside double quotes.
fn quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The query for a relation's members (with their roles) and their ways.
pub fn relation_query(id: i64) -> String {
    format!("[out:json][timeout:120];rel({id});out body;way(r);out geom;")
}

/// The query for the names and extents of some ways.
pub fn ways_tags_query(ids: &[i64]) -> String {
    let list: Vec<String> = ids.iter().map(i64::to_string).collect();
    format!(
        "[out:json][timeout:60];way(id:{});out tags bb;",
        list.join(",")
    )
}

/// The query for the waterways called `name` in a bounding box `[west, south, east, north]`.
pub fn named_ways_query(name: &str, bbox: [f64; 4]) -> String {
    format!(
        "[out:json][timeout:120];way[\"waterway\"~\"^(river|stream|canal)$\"][\"name\"=\"{}\"]({},{},{},{});out geom;",
        quoted(name),
        bbox[1],
        bbox[0],
        bbox[3],
        bbox[2]
    )
}

fn json_of(body: &[u8]) -> Result<Value, ReachError> {
    serde_json::from_slice(body).map_err(|e| {
        ReachError::Parse(format!(
            "the Overpass answer is not JSON ({e}); the server may be busy"
        ))
    })
}

/// The main-stem ways of a relation answer (`relation_query`): members with role `main_stream`,
/// or no role; every member way if none has those roles. Also the relation's name.
pub fn relation_ways(body: &[u8]) -> Result<(Option<String>, Vec<Way>), ReachError> {
    let v = json_of(body)?;
    let rel = v["elements"]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["type"] == "relation"))
        .ok_or_else(|| ReachError::NoWaterway("OSM has no such river relation".into()))?;
    let name = rel["tags"]["name"].as_str().map(str::to_string);
    let roles: HashMap<i64, String> = rel["members"]
        .as_array()
        .map(|m| {
            m.iter()
                .filter(|m| m["type"] == "way")
                .filter_map(|m| Some((m["ref"].as_i64()?, m["role"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let ways: Vec<Way> = parse_ways(body)?
        .into_iter()
        .filter(|w| KINDS.contains(&w.kind.as_str()) && roles.contains_key(&w.id))
        .collect();
    let stem: Vec<Way> = ways
        .iter()
        .filter(|w| matches!(roles[&w.id].as_str(), "main_stream" | ""))
        .cloned()
        .collect();
    Ok((name, if stem.is_empty() { ways } else { stem }))
}

/// The ways joined to the seeds through shared nodes, and whether one of them reaches the box edge.
fn component(ways: Vec<Way>, seeds: &[i64], bbox: [f64; 4]) -> (Vec<Way>, bool) {
    let mut by_node: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, w) in ways.iter().enumerate() {
        for n in &w.nodes {
            by_node.entry(*n).or_default().push(i);
        }
    }
    let mut keep: BTreeSet<usize> = ways
        .iter()
        .enumerate()
        .filter(|(_, w)| seeds.contains(&w.id))
        .map(|(i, _)| i)
        .collect();
    let mut todo: Vec<usize> = keep.iter().copied().collect();
    while let Some(i) = todo.pop() {
        for n in &ways[i].nodes {
            for j in &by_node[n] {
                if keep.insert(*j) {
                    todo.push(*j);
                }
            }
        }
    }
    let eps = 1e-3;
    let edge = keep.iter().any(|i| {
        ways[*i].geom.iter().any(|p| {
            p[0] <= bbox[0] + eps
                || p[1] <= bbox[1] + eps
                || p[0] >= bbox[2] - eps
                || p[1] >= bbox[3] - eps
        })
    });
    let out = ways
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep.contains(i))
        .map(|(_, w)| w)
        .collect();
    (out, edge)
}

/// The main stem of a river's ways: `(mouth, upstream end, route from one to the other)`.
pub fn main_stem(ways: &[Way]) -> Result<(LonLat, LonLat, Route), ReachError> {
    if ways.is_empty() {
        return Err(ReachError::NoWaterway(
            "the river has no waterway ways in OSM".into(),
        ));
    }
    let mut pos: BTreeMap<i64, LonLat> = BTreeMap::new();
    let mut starts: BTreeSet<i64> = BTreeSet::new();
    let mut ends: BTreeSet<i64> = BTreeSet::new();
    for w in ways {
        let (a, b) = (w.nodes[0], w.nodes[w.nodes.len() - 1]);
        pos.insert(a, w.geom[0]);
        pos.insert(b, w.geom[w.geom.len() - 1]);
        starts.insert(a);
        ends.insert(b);
    }
    let mut mouths: Vec<i64> = ends.difference(&starts).copied().collect();
    let mut sources: Vec<i64> = starts.difference(&ends).copied().collect();
    // A river drawn as a loop, or against the flow in places, may have none: then any loose end.
    if mouths.is_empty() || sources.is_empty() {
        let mut uses: HashMap<i64, usize> = HashMap::new();
        for w in ways {
            for n in &w.nodes {
                *uses.entry(*n).or_default() += 1;
            }
        }
        let loose: Vec<i64> = pos.keys().copied().filter(|n| uses[n] == 1).collect();
        if mouths.is_empty() {
            mouths = loose.clone();
        }
        if sources.is_empty() {
            sources = loose;
        }
    }
    let graph = Graph::new(ways.to_vec());
    let mut best: Option<(LonLat, LonLat, Route)> = None;
    for m in &mouths {
        for s in &sources {
            if m == s {
                continue;
            }
            let Ok(r) = graph.route(&[pos[m], pos[s]]) else {
                continue;
            };
            if best.as_ref().is_none_or(|b| r.length_m > b.2.length_m) {
                best = Some((pos[m], pos[s], r));
            }
        }
    }
    best.ok_or_else(|| {
        ReachError::NoRoute("the river's ways do not join a mouth to an upstream end".into())
    })
}

/// Read a whole river from OSM: a relation's main stem, or a lone way grown to the ways with its
/// name that join it (`also` are more ways of the same river the search listed). Two or more
/// Overpass requests.
pub fn whole_river(ov: &Overpass, osm: OsmRef, also: &[OsmRef]) -> Result<WholeRiver, ReachError> {
    let (name, ways) = match osm {
        OsmRef::Relation(id) => relation_ways(&ov.get(&relation_query(id))?)?,
        OsmRef::Way(id) => {
            let mut seeds = vec![id];
            seeds.extend(also.iter().filter_map(|o| match o {
                OsmRef::Way(w) => Some(*w),
                OsmRef::Relation(_) => None,
            }));
            lone_way_river(ov, &seeds)?
        }
    };
    let (mouth, upstream, route) = main_stem(&ways)?;
    Ok(WholeRiver {
        osm,
        name,
        mouth,
        upstream,
        route,
        ways,
    })
}

fn lone_way_river(ov: &Overpass, seeds: &[i64]) -> Result<(Option<String>, Vec<Way>), ReachError> {
    let tags = json_of(&ov.get(&ways_tags_query(seeds))?)?;
    let found: Vec<&Value> = tags["elements"]
        .as_array()
        .map(|a| a.iter().filter(|e| e["type"] == "way").collect())
        .unwrap_or_default();
    let first = found
        .first()
        .ok_or_else(|| ReachError::NoWaterway("OSM has no such waterway way".into()))?;
    let Some(name) = first["tags"]["name"].as_str().map(str::to_string) else {
        // No name to follow: the way alone.
        let ways = ov.get(&format!(
            "[out:json][timeout:60];way(id:{});out geom;",
            seeds[0]
        ))?;
        return Ok((None, parse_ways(&ways)?));
    };
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for e in &found {
        let b = &e["bounds"];
        let g = |k: &str| b[k].as_f64();
        if let (Some(w), Some(s), Some(ea), Some(n)) =
            (g("minlon"), g("minlat"), g("maxlon"), g("maxlat"))
        {
            bbox = [
                bbox[0].min(w),
                bbox[1].min(s),
                bbox[2].max(ea),
                bbox[3].max(n),
            ];
        }
    }
    if bbox[0] > bbox[2] {
        return Err(ReachError::Parse(
            "the Overpass answer has no way extents".into(),
        ));
    }
    let mut pad = 0.25_f64.max((bbox[2] - bbox[0]).max(bbox[3] - bbox[1]));
    let mut ways = Vec::new();
    for _ in 0..GROW_ROUNDS {
        let b = [bbox[0] - pad, bbox[1] - pad, bbox[2] + pad, bbox[3] + pad];
        let all = parse_ways(&ov.get(&named_ways_query(&name, b))?)?;
        let (comp, at_edge) = component(all, seeds, b);
        ways = comp;
        if !at_edge {
            break;
        }
        pad *= 2.0;
    }
    Ok((Some(name), ways))
}

impl WholeRiver {
    /// The whole river as a reach: from the mouth to the upstream end.
    pub fn planned(&self) -> Planned {
        Planned {
            spec: ReachSpec {
                river: self.name.clone(),
                start: self.mouth,
                end: self.upstream,
                via: Vec::new(),
                trace: None,
            },
            route: self.route.clone(),
            candidates: Vec::new(),
        }
    }

    /// A reach cut out of the river: the shortest route along its own ways from `start`, through
    /// `via`, to `end`.
    pub fn cut(&self, start: LonLat, end: LonLat, via: &[LonLat]) -> Result<Planned, ReachError> {
        let mut points = vec![start];
        points.extend_from_slice(via);
        points.push(end);
        let route = Graph::new(self.ways.clone()).route(&points)?;
        Ok(Planned {
            spec: ReachSpec {
                river: self.name.clone(),
                start,
                end,
                via: via.to_vec(),
                trace: None,
            },
            route,
            candidates: Vec::new(),
        })
    }
}

/// Record a reach taken from a whole river: as [`record_reach`], and the route's provenance also
/// names the OSM river and, when the user chose it from the river search, that search's source id
/// (`osm-photon` or `osm-nominatim`) and date.
/// The mouth and upstream end are two OSM node positions, kept as the reach's start and end like a
/// click: an insubstantial extract.
pub fn record_river_reach(
    session: &mut Session,
    planned: &Planned,
    osm: OsmRef,
    searched: Option<&str>,
    date: &str,
) -> Result<(), ReachError> {
    record_reach(session, planned, date)?;
    let mut rec =
        provenance_record("osm-overpass", date, Mark::Default, false).map_err(ReachError::Input)?;
    let mut settings = json!({"osm_id": osm.to_string()});
    if let Some(source) = searched {
        settings["river_search"] = json!({"source": source, "date": date});
    }
    rec["settings"] = settings;
    session
        .set_provenance("route", rec)
        .map_err(|e| ReachError::Input(e.to_string()))
}

#[cfg(test)]
mod tests {
    //! Label: sourced - `examples/reach/overpass_ankobra.json`, recorded from overpass-api.de on
    //! 2026-10-22 (relation 3328713, © OpenStreetMap contributors); and synthetic networks drawn here.
    use super::*;
    use crate::fetch::ReplayFetch;
    use crate::geo::line_length_m;

    const ANKOBRA: &[u8] = include_bytes!("../../../examples/reach/overpass_ankobra.json");

    fn way(id: i64, nodes: &[(i64, f64, f64)]) -> Way {
        Way {
            id,
            nodes: nodes.iter().map(|n| n.0).collect(),
            geom: nodes.iter().map(|n| [n.1, n.2]).collect(),
            name: Some("Alpha".into()),
            kind: "river".into(),
        }
    }

    #[test]
    fn the_ankobra_runs_from_the_sea_to_its_far_end_over_every_member_way() {
        let f = ReplayFetch::new().with_response_containing("rel%283328713%29", ANKOBRA);
        let r = whole_river(&Overpass::new(&f), OsmRef::Relation(3328713), &[]).unwrap();
        assert_eq!(r.name.as_deref(), Some("Ankobra River"));
        // The sea end: node 2145888185 at the coast; the far end: node 5043693652.
        assert_eq!(r.mouth, [-2.2688538, 4.8983929]);
        assert_eq!(r.upstream, [-2.1349229, 5.7294889]);
        // One chain of 12 ways: the route is every way, end to end.
        assert_eq!(r.ways.len(), 12);
        assert_eq!(r.route.way_ids().len(), 12);
        let sum: f64 = r.ways.iter().map(|w| line_length_m(&w.geom)).sum();
        assert!(
            (r.route.length_m - sum).abs() < 1e-6 * sum,
            "{} {sum}",
            r.route.length_m
        );
        assert!(r.route.length_m > 100_000.0, "{}", r.route.length_m);
        assert_eq!(f.requests().len(), 1);
    }

    #[test]
    fn a_tributary_and_a_short_delta_arm_do_not_win() {
        // Main stem 1 -> 2 -> 3 -> 4 (mouth A, far) and a short arm 3 -> 5 (mouth B, near);
        // a tributary 6 -> 2 joins from a source near the mouth.
        let ways = vec![
            way(1, &[(1, 0.0, 0.0), (2, 0.0, -0.1)]),
            way(2, &[(2, 0.0, -0.1), (3, 0.0, -0.2)]),
            way(3, &[(3, 0.0, -0.2), (4, 0.0, -0.5)]),
            way(4, &[(3, 0.0, -0.2), (5, 0.05, -0.25)]),
            way(5, &[(6, 0.02, -0.12), (2, 0.0, -0.1)]),
        ];
        let (mouth, up, route) = main_stem(&ways).unwrap();
        assert_eq!(mouth, [0.0, -0.5]);
        assert_eq!(up, [0.0, 0.0]);
        assert_eq!(route.way_ids(), vec![3, 2, 1]);
    }

    #[test]
    fn a_lone_way_grows_to_the_named_ways_it_joins_and_no_further() {
        let tags = br#"{"elements":[{"type":"way","id":11,"bounds":{"minlat":1.0,"minlon":2.0,"maxlat":1.1,"maxlon":2.1},"tags":{"name":"Alpha","waterway":"river"}}]}"#;
        let g = |id: i64, a: (i64, f64, f64), b: (i64, f64, f64)| {
            format!(
                r#"{{"type":"way","id":{id},"nodes":[{},{}],"geometry":[{{"lat":{},"lon":{}}},{{"lat":{},"lon":{}}}],"tags":{{"name":"Alpha","waterway":"river"}}}}"#,
                a.0, b.0, a.2, a.1, b.2, b.1
            )
        };
        // 10 -> 11 -> 12 joined; 13 has the name but stands apart.
        let named = format!(
            r#"{{"elements":[{},{},{},{}]}}"#,
            g(10, (100, 2.0, 1.2), (101, 2.0, 1.1)),
            g(11, (101, 2.0, 1.1), (102, 2.05, 1.0)),
            g(12, (102, 2.05, 1.0), (103, 2.1, 0.95)),
            g(13, (200, 2.2, 1.0), (201, 2.25, 0.9)),
        );
        let f = ReplayFetch::new()
            .with_response_containing("out%20tags%20bb", tags.to_vec())
            .with_response_containing("out%20geom", named.into_bytes());
        let r = whole_river(&Overpass::new(&f), OsmRef::Way(11), &[]).unwrap();
        let mut ids: Vec<i64> = r.ways.iter().map(|w| w.id).collect();
        ids.sort();
        assert_eq!(ids, vec![10, 11, 12]);
        assert_eq!(r.mouth, [2.1, 0.95]);
        assert_eq!(r.upstream, [2.0, 1.2]);
        // Well inside the box: one named-ways request.
        assert_eq!(f.requests().len(), 2);
    }

    #[test]
    fn a_cut_follows_the_river_and_the_record_names_the_river_and_the_search() {
        let f = ReplayFetch::new().with_response_containing("rel%283328713%29", ANKOBRA);
        let r = whole_river(&Overpass::new(&f), OsmRef::Relation(3328713), &[]).unwrap();
        let mid = r.route.line()[r.route.line().len() / 2];
        let cut = r.cut(r.mouth, mid, &[]).unwrap();
        assert!(cut.route.length_m < r.route.length_m);
        assert!(cut.route.length_m > 0.3 * r.route.length_m);
        let mut d = Session::new("Ankobra River", "2026-10-22");
        record_river_reach(&mut d, &cut, r.osm, Some("osm-photon"), "2026-10-22").unwrap();
        let p = &d.document()["provenance"]["route"];
        assert_eq!(p["source"], "osm-overpass");
        assert_eq!(p["settings"]["osm_id"], "relation/3328713");
        assert_eq!(p["settings"]["river_search"]["source"], "osm-photon");
        assert_eq!(d.document()["reach"]["river"], "Ankobra River");
    }

    #[test]
    fn queries_escape_the_name_and_a_missing_relation_is_a_clear_error() {
        let q = named_ways_query("A \"B\"", [1.0, 2.0, 3.0, 4.0]);
        assert!(q.contains("[\"name\"=\"A \\\"B\\\"\"](2,1,4,3)"), "{q}");
        assert_eq!(
            ways_tags_query(&[1, 2]),
            "[out:json][timeout:60];way(id:1,2);out tags bb;"
        );
        let e = relation_ways(br#"{"elements":[]}"#).unwrap_err();
        assert!(e.to_string().contains("no such river"), "{e}");
    }
}
