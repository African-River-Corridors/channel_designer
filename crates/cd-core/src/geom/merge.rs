//! shapely `linemerge`: GEOS `LineMerger` (undirected), with its order of output.
//!
//! Nodes live in a map ordered by (x, y) (`NodeMap`, `CoordinateLessThan`). Each node's out-edges
//! are sorted by quadrant and then by orientation (`DirectedEdge::compareDirection`). Strings start
//! at every node whose degree is not 2, in node order, then at the nodes of isolated loops. A
//! string is reversed when more of its edges run against their line than with it.

use super::{Pt, orientation};
use std::collections::BTreeMap;

/// A coordinate as a map key: (x, y) in GEOS `CoordinateLessThan` order (`total_cmp` agrees with
/// `<` for the finite, non-negative-zero-distinguishing values a line holds).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key(Pt);

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // GEOS compares with `<`, so -0.0 and 0.0 are one node.
        let c = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(core::cmp::Ordering::Equal);
        c(self.0[0], other.0[0]).then(c(self.0[1], other.0[1]))
    }
}

struct DirEdge {
    to: usize,
    edge: usize,
    forward: bool,
    p0: Pt,
    p1: Pt,
    quadrant: i32,
}

/// GEOS `Quadrant::quadrant(dx, dy)`: 0 NE, 1 NW, 2 SW, 3 SE.
fn quadrant(dx: f64, dy: f64) -> i32 {
    if dx >= 0.0 {
        if dy >= 0.0 { 0 } else { 3 }
    } else if dy >= 0.0 {
        1
    } else {
        2
    }
}

/// GEOS `DirectedEdge::compareDirection`.
fn compare_direction(a: &DirEdge, b: &DirEdge) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    if a.quadrant > b.quadrant {
        return Greater;
    }
    if a.quadrant < b.quadrant {
        return Less;
    }
    match orientation(b.p0, b.p1, a.p1) {
        1 => Greater,
        -1 => Less,
        _ => Equal,
    }
}

/// shapely `linemerge(lines)`: the merged lines, in GEOS's order. Empty lines and lines whose
/// points are all equal are dropped.
pub fn line_merge(lines: &[Vec<Pt>]) -> Vec<Vec<Pt>> {
    let mut node_of: BTreeMap<Key, usize> = BTreeMap::new();
    let mut node_out: Vec<Vec<usize>> = Vec::new();
    let mut des: Vec<DirEdge> = Vec::new();
    let mut edge_lines: Vec<&Vec<Pt>> = Vec::new();
    let mut node = |p: Pt, node_out: &mut Vec<Vec<usize>>| {
        *node_of.entry(Key(p)).or_insert_with(|| {
            node_out.push(Vec::new());
            node_out.len() - 1
        })
    };
    for line in lines {
        let mut coords: Vec<Pt> = Vec::with_capacity(line.len());
        for &p in line {
            if coords.last() != Some(&p) {
                coords.push(p);
            }
        }
        if coords.len() <= 1 {
            continue;
        }
        let n = coords.len();
        let (a, b) = (
            node(coords[0], &mut node_out),
            node(coords[n - 1], &mut node_out),
        );
        let e = edge_lines.len();
        edge_lines.push(line);
        for (from, to, p0, p1, forward) in [
            (a, b, coords[0], coords[1], true),
            (b, a, coords[n - 1], coords[n - 2], false),
        ] {
            node_out[from].push(des.len());
            des.push(DirEdge {
                to,
                edge: e,
                forward,
                p0,
                p1,
                quadrant: quadrant(p1[0] - p0[0], p1[1] - p0[1]),
            });
        }
    }
    for out in &mut node_out {
        // A stable sort, as `std::sort` of two-element stars always is.
        out.sort_by(|&x, &y| compare_direction(&des[x], &des[y]));
    }
    let sym = |d: usize| d ^ 1;
    let mut edge_marked = vec![false; edge_lines.len()];
    let mut node_marked = vec![false; node_out.len()];
    let mut strings: Vec<Vec<usize>> = Vec::new();
    let next = |d: usize| -> Option<usize> {
        let to = des[d].to;
        let out = &node_out[to];
        if out.len() != 2 {
            return None;
        }
        Some(if out[0] == sym(d) { out[1] } else { out[0] })
    };
    let build_from = |n: usize, edge_marked: &mut Vec<bool>, strings: &mut Vec<Vec<usize>>| {
        for &start in &node_out[n] {
            if edge_marked[des[start].edge] {
                continue;
            }
            let mut s = Vec::new();
            let mut cur = start;
            loop {
                s.push(cur);
                edge_marked[des[cur].edge] = true;
                match next(cur) {
                    Some(nx) if nx != start => cur = nx,
                    _ => break,
                }
            }
            strings.push(s);
        }
    };
    let order: Vec<usize> = node_of.values().copied().collect();
    for &n in &order {
        if node_out[n].len() != 2 {
            build_from(n, &mut edge_marked, &mut strings);
            node_marked[n] = true;
        }
    }
    for &n in &order {
        if !node_marked[n] {
            build_from(n, &mut edge_marked, &mut strings);
            node_marked[n] = true;
        }
    }
    strings
        .into_iter()
        .map(|s| {
            let mut coords: Vec<Pt> = Vec::new();
            let mut forward = 0;
            for &d in &s {
                if des[d].forward {
                    forward += 1;
                }
                let line = edge_lines[des[d].edge];
                let mut push = |p: Pt| {
                    if coords.last() != Some(&p) {
                        coords.push(p);
                    }
                };
                if des[d].forward {
                    line.iter().for_each(|&p| push(p));
                } else {
                    line.iter().rev().for_each(|&p| push(p));
                }
            }
            if s.len() - forward > forward {
                coords.reverse();
            }
            coords
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn a_shuffled_chain_merges_into_one_line() {
        let a = vec![[0.0, 0.0], [10.0, 0.0]];
        let b = vec![[10.0, 0.0], [20.0, 5.0]];
        let c = vec![[20.0, 5.0], [30.0, 5.0]];
        let merged = line_merge(&[c, a, b]);
        assert_eq!(
            merged,
            vec![vec![[0.0, 0.0], [10.0, 0.0], [20.0, 5.0], [30.0, 5.0]]]
        );
    }

    #[test]
    fn lines_meeting_at_a_t_stay_apart() {
        let merged = line_merge(&[
            vec![[0.0, 0.0], [10.0, 0.0]],
            vec![[10.0, 0.0], [20.0, 5.0]],
            vec![[10.0, 0.0], [10.0, -9.0]],
        ]);
        assert_eq!(merged.len(), 3);
    }
}
