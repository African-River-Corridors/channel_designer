//! Routing on the OSM waterway graph (spec 0012): a shortest route through a start, via points
//! and an end, and a rough trace matched to the waterways it follows. Waterways join where they
//! share an OSM node, so a route crosses from one river to another and follows either branch at a
//! split (put a via point on the branch you want). A stretch of a trace with no waterway near it
//! stays as drawn.

use crate::error::ReachError;
use crate::geo::{LonLat, haversine_m, nearest_on_segment, resample};
use crate::overpass::Way;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// How far a click may be from a waterway and still snap to it. Provisional.
pub const SNAP_MAX_M: f64 = 1_000.0;
/// How far a traced point may be from a waterway to count as following it. Provisional.
pub const TRACE_TOLERANCE_M: f64 = 150.0;
/// The spacing of the points a trace is read at.
pub const TRACE_STEP_M: f64 = 50.0;

/// A position on the graph: a point on one segment of one way.
#[derive(Debug, Clone, Copy)]
pub struct Snap {
    way: usize,
    seg: usize,
    t: f64,
    pub pos: LonLat,
    pub dist_m: f64,
}

/// One piece of a route.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    /// Along waterways, by OSM way id. The line is for display and the next steps; it is OSM
    /// geometry, and is never written into the document.
    Waterway {
        way_ids: Vec<i64>,
        line: Vec<LonLat>,
    },
    /// Where no waterway was near: the user's own drawing.
    AsDrawn { line: Vec<LonLat> },
}

/// A route: its parts in order, and its length.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub parts: Vec<Part>,
    pub length_m: f64,
}

impl Route {
    /// Every way id along the route, in order, once per run.
    pub fn way_ids(&self) -> Vec<i64> {
        let mut out: Vec<i64> = Vec::new();
        for p in &self.parts {
            if let Part::Waterway { way_ids, .. } = p {
                for id in way_ids {
                    if out.last() != Some(id) {
                        out.push(*id);
                    }
                }
            }
        }
        out
    }

    /// The number of as-drawn stretches.
    pub fn as_drawn_count(&self) -> usize {
        self.parts
            .iter()
            .filter(|p| matches!(p, Part::AsDrawn { .. }))
            .count()
    }

    /// The whole route as one line.
    pub fn line(&self) -> Vec<LonLat> {
        let mut out: Vec<LonLat> = Vec::new();
        for p in &self.parts {
            let l = match p {
                Part::Waterway { line, .. } | Part::AsDrawn { line } => line,
            };
            extend_line(&mut out, l);
        }
        out
    }
}

fn extend_line(out: &mut Vec<LonLat>, more: &[LonLat]) {
    for p in more {
        if out.last() != Some(p) {
            out.push(*p);
        }
    }
}

/// A float that sorts like its value (distances are never negative or NaN).
#[derive(PartialEq, Clone, Copy)]
struct Dist(f64);
impl Eq for Dist {}
impl PartialOrd for Dist {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Dist {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// The waterway graph of a set of ways.
pub struct Graph {
    ways: Vec<Way>,
    node_ix: HashMap<i64, usize>,
    pos: Vec<LonLat>,
    /// `(to node, length in metres, way index)`.
    adj: Vec<Vec<(usize, f64, usize)>>,
}

struct Found {
    length_m: f64,
    way_ids: Vec<i64>,
    line: Vec<LonLat>,
}

impl Graph {
    pub fn new(ways: Vec<Way>) -> Graph {
        let mut node_ix: HashMap<i64, usize> = HashMap::new();
        let mut pos: Vec<LonLat> = Vec::new();
        for w in &ways {
            for (n, p) in w.nodes.iter().zip(&w.geom) {
                node_ix.entry(*n).or_insert_with(|| {
                    pos.push(*p);
                    pos.len() - 1
                });
            }
        }
        let mut adj = vec![Vec::new(); pos.len()];
        for (wi, w) in ways.iter().enumerate() {
            for k in 0..w.nodes.len() - 1 {
                let (a, b) = (node_ix[&w.nodes[k]], node_ix[&w.nodes[k + 1]]);
                let len = haversine_m(w.geom[k], w.geom[k + 1]);
                adj[a].push((b, len, wi));
                adj[b].push((a, len, wi));
            }
        }
        Graph {
            ways,
            node_ix,
            pos,
            adj,
        }
    }

    pub fn way_count(&self) -> usize {
        self.ways.len()
    }

    /// The nearest point on any waterway within `max_m` of `p`.
    pub fn snap(&self, p: LonLat, max_m: f64) -> Option<Snap> {
        let mut best: Option<Snap> = None;
        for (wi, w) in self.ways.iter().enumerate() {
            for k in 0..w.geom.len() - 1 {
                let h = nearest_on_segment(p, w.geom[k], w.geom[k + 1]);
                if h.dist_m <= max_m && best.is_none_or(|b| h.dist_m < b.dist_m) {
                    best = Some(Snap {
                        way: wi,
                        seg: k,
                        t: h.t,
                        pos: h.pos,
                        dist_m: h.dist_m,
                    });
                }
            }
        }
        best
    }

    fn seg_len(&self, s: &Snap) -> f64 {
        let g = &self.ways[s.way].geom;
        haversine_m(g[s.seg], g[s.seg + 1])
    }

    /// The nodes at the two ends of a snap's segment, and the distance to each.
    fn ends(&self, s: &Snap) -> [(usize, f64); 2] {
        let w = &self.ways[s.way];
        let l = self.seg_len(s);
        [
            (self.node_ix[&w.nodes[s.seg]], s.t * l),
            (self.node_ix[&w.nodes[s.seg + 1]], (1.0 - s.t) * l),
        ]
    }

    /// The shortest way from one snap to another. `cap_m` stops the search at that distance.
    fn shortest(&self, from: &Snap, to: &Snap, cap_m: Option<f64>) -> Option<Found> {
        let same_seg = from.way == to.way && from.seg == to.seg;
        let direct = same_seg.then(|| (from.t - to.t).abs() * self.seg_len(from));
        let mut dist = vec![f64::INFINITY; self.pos.len()];
        let mut prev: Vec<Option<(usize, usize)>> = vec![None; self.pos.len()];
        let mut heap = BinaryHeap::new();
        for (n, d) in self.ends(from) {
            if d < dist[n] {
                dist[n] = d;
                heap.push((Reverse(Dist(d)), n));
            }
        }
        let targets = self.ends(to);
        let mut settled = [false; 2];
        while let Some((Reverse(Dist(d)), n)) = heap.pop() {
            if d > dist[n] {
                continue;
            }
            if cap_m.is_some_and(|c| d > c) {
                break;
            }
            for (i, (tn, _)) in targets.iter().enumerate() {
                if *tn == n {
                    settled[i] = true;
                }
            }
            if settled[0] && settled[1] {
                break;
            }
            for &(m, len, wi) in &self.adj[n] {
                let nd = d + len;
                if nd < dist[m] {
                    dist[m] = nd;
                    prev[m] = Some((n, wi));
                    heap.push((Reverse(Dist(nd)), m));
                }
            }
        }
        let via_graph = targets
            .iter()
            .map(|&(n, d)| (dist[n] + d, n))
            .filter(|(t, _)| t.is_finite())
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some(dd) = direct
            && via_graph.is_none_or(|(g, _)| dd <= g)
        {
            return Some(Found {
                length_m: dd,
                way_ids: vec![self.ways[from.way].id],
                line: vec![from.pos, to.pos],
            });
        }
        let (length_m, end_node) = via_graph?;
        if cap_m.is_some_and(|c| length_m > c + 1.0) {
            return None;
        }
        // Walk back from the end node to a seed node.
        let mut nodes = vec![end_node];
        let mut edge_ways: Vec<usize> = Vec::new();
        let mut at = end_node;
        while let Some((p, wi)) = prev[at] {
            nodes.push(p);
            edge_ways.push(wi);
            at = p;
        }
        nodes.reverse();
        edge_ways.reverse();
        let mut way_ids = vec![self.ways[from.way].id];
        way_ids.extend(edge_ways.iter().map(|&wi| self.ways[wi].id));
        way_ids.push(self.ways[to.way].id);
        way_ids.dedup();
        let mut line = vec![from.pos];
        line.extend(nodes.iter().map(|&n| self.pos[n]));
        line.push(to.pos);
        line.dedup();
        Some(Found {
            length_m,
            way_ids,
            line,
        })
    }

    /// The shortest route through `points` in order (start, via points, end), each snapped to a
    /// waterway within `SNAP_MAX_M`.
    pub fn route(&self, points: &[LonLat]) -> Result<Route, ReachError> {
        if points.len() < 2 {
            return Err(ReachError::Input("a route needs a start and an end".into()));
        }
        let mut snaps = Vec::new();
        for (i, p) in points.iter().enumerate() {
            let s = self.snap(*p, SNAP_MAX_M).ok_or_else(|| {
                ReachError::NoWaterway(format!(
                    "no waterway within {} m of point {} ({:.5}, {:.5}); click closer to the river",
                    SNAP_MAX_M as u32,
                    i + 1,
                    p[0],
                    p[1]
                ))
            })?;
            snaps.push(s);
        }
        let mut way_ids: Vec<i64> = Vec::new();
        let mut line: Vec<LonLat> = Vec::new();
        let mut length_m = 0.0;
        for (i, w) in snaps.windows(2).enumerate() {
            let f = self.shortest(&w[0], &w[1], None).ok_or_else(|| {
                ReachError::NoRoute(format!(
                    "the waterways do not join point {} to point {}: add a via point, or check the map",
                    i + 1,
                    i + 2
                ))
            })?;
            length_m += f.length_m;
            for id in f.way_ids {
                if way_ids.last() != Some(&id) {
                    way_ids.push(id);
                }
            }
            extend_line(&mut line, &f.line);
        }
        Ok(Route {
            parts: vec![Part::Waterway { way_ids, line }],
            length_m,
        })
    }

    /// Match a rough trace to the waterways it follows. A stretch with no waterway within
    /// `tolerance_m`, or none that joins its neighbours, stays as drawn.
    pub fn match_trace(&self, trace: &[LonLat], tolerance_m: f64) -> Result<Route, ReachError> {
        if trace.len() < 2 {
            return Err(ReachError::Input(
                "a trace needs at least two points".into(),
            ));
        }
        let samples = resample(trace, TRACE_STEP_M);
        let snaps: Vec<Option<Snap>> = samples.iter().map(|p| self.snap(*p, tolerance_m)).collect();
        let mut parts: Vec<Part> = Vec::new();
        let mut length_m = 0.0;
        for i in 1..samples.len() {
            let routed = match (&snaps[i - 1], &snaps[i]) {
                (Some(a), Some(b)) => {
                    let cap = 4.0 * haversine_m(a.pos, b.pos) + 500.0;
                    self.shortest(a, b, Some(cap))
                }
                _ => None,
            };
            match routed {
                Some(f) => {
                    length_m += f.length_m;
                    match parts.last_mut() {
                        Some(Part::Waterway { way_ids, line }) => {
                            for id in f.way_ids {
                                if way_ids.last() != Some(&id) {
                                    way_ids.push(id);
                                }
                            }
                            extend_line(line, &f.line);
                        }
                        _ => parts.push(Part::Waterway {
                            way_ids: f.way_ids,
                            line: f.line,
                        }),
                    }
                }
                None => {
                    let seg = [samples[i - 1], samples[i]];
                    length_m += haversine_m(seg[0], seg[1]);
                    match parts.last_mut() {
                        Some(Part::AsDrawn { line }) => extend_line(line, &seg),
                        _ => parts.push(Part::AsDrawn { line: seg.to_vec() }),
                    }
                }
            }
        }
        Ok(Route { parts, length_m })
    }
}
