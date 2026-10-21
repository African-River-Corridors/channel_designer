//! A uniform grid over the edges of a set of polygons, for fast point-in-polygon tests, segment
//! hits and boundary distances on large polygons (navigable water of a long reach).
//! Results equal the brute-force tests; only the speed differs.

use super::linear::point_segment_distance;
use super::{Polygon, Pt};

/// Polygon edges bucketed into square cells.
#[derive(Debug, Clone)]
pub struct EdgeGrid {
    edges: Vec<[Pt; 2]>,
    origin: Pt,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl EdgeGrid {
    pub fn new(polys: &[Polygon]) -> Self {
        let edges: Vec<[Pt; 2]> = polys
            .iter()
            .flat_map(|p| core::iter::once(&p.exterior).chain(&p.holes))
            .flat_map(|r| r.windows(2).map(|w| [w[0], w[1]]))
            .filter(|e| e[0] != e[1])
            .collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for e in &edges {
            for p in e {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        if edges.is_empty() {
            return EdgeGrid {
                edges,
                origin: [0.0; 2],
                cell: 1.0,
                nx: 0,
                ny: 0,
                cells: vec![],
            };
        }
        let (w, h) = ((hi[0] - lo[0]).max(1e-9), (hi[1] - lo[1]).max(1e-9));
        let cell = ((w * h / edges.len() as f64).sqrt() * 2.0)
            .max(w.max(h) / 512.0)
            .max(1e-6);
        let nx = ((w / cell).floor() as usize + 1).min(1024);
        let ny = ((h / cell).floor() as usize + 1).min(1024);
        let cell = (w / nx as f64).max(h / ny as f64) * (1.0 + 1e-12);
        let mut g = EdgeGrid {
            edges,
            origin: lo,
            cell,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
        };
        for (i, e) in g.edges.iter().enumerate() {
            let (x0, y0) = g.idx([e[0][0].min(e[1][0]), e[0][1].min(e[1][1])]);
            let (x1, y1) = g.idx([e[0][0].max(e[1][0]), e[0][1].max(e[1][1])]);
            for iy in y0..=y1 {
                for ix in x0..=x1 {
                    g.cells[iy * nx + ix].push(i as u32);
                }
            }
        }
        g
    }

    fn idx(&self, p: Pt) -> (usize, usize) {
        let fx = ((p[0] - self.origin[0]) / self.cell).floor();
        let fy = ((p[1] - self.origin[1]) / self.cell).floor();
        (
            fx.clamp(0.0, (self.nx - 1) as f64) as usize,
            fy.clamp(0.0, (self.ny - 1) as f64) as usize,
        )
    }

    /// The ids of the edges in the cells that overlap the box `lo..hi`, each once.
    fn edges_in(&self, lo: Pt, hi: Pt) -> Vec<u32> {
        if self.nx == 0 {
            return vec![];
        }
        let (x0, y0) = self.idx(lo);
        let (x1, y1) = self.idx(hi);
        let mut ids: Vec<u32> = Vec::new();
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                ids.extend(&self.cells[iy * self.nx + ix]);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn outside_box(&self, p: Pt) -> bool {
        self.nx == 0
            || p[0] < self.origin[0]
            || p[1] < self.origin[1]
            || p[0] > self.origin[0] + self.cell * self.nx as f64
            || p[1] > self.origin[1] + self.cell * self.ny as f64
    }

    /// Whether `p` is inside the polygons (even-odd crossing count of a ray towards +x). A point
    /// exactly on an edge may go either way; callers that care use `near_boundary`.
    pub fn contains(&self, p: Pt) -> bool {
        if self.outside_box(p) {
            return false;
        }
        let hi = [self.origin[0] + self.cell * self.nx as f64, p[1]];
        let mut inside = false;
        for i in self.edges_in(p, hi) {
            let [a, b] = self.edges[i as usize];
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// Whether `p` lies within `tol` of an edge.
    pub fn near_boundary(&self, p: Pt, tol: f64) -> bool {
        self.edges_in([p[0] - tol, p[1] - tol], [p[0] + tol, p[1] + tol])
            .into_iter()
            .any(|i| {
                let [a, b] = self.edges[i as usize];
                point_segment_distance(p, a, b) <= tol
            })
    }

    /// Inside, or within `tol` of the boundary.
    pub fn covers(&self, p: Pt, tol: f64) -> bool {
        self.contains(p) || self.near_boundary(p, tol)
    }

    /// The parameters `t` in [0, 1] at which segment `p -> q` crosses an edge.
    pub fn hits(&self, p: Pt, q: Pt) -> Vec<f64> {
        let lo = [p[0].min(q[0]), p[1].min(q[1])];
        let hi = [p[0].max(q[0]), p[1].max(q[1])];
        let d = [q[0] - p[0], q[1] - p[1]];
        let mut out = Vec::new();
        for i in self.edges_in(lo, hi) {
            let [a, b] = self.edges[i as usize];
            let e = [b[0] - a[0], b[1] - a[1]];
            let den = d[0] * e[1] - d[1] * e[0];
            if den == 0.0 {
                continue;
            }
            let ap = [a[0] - p[0], a[1] - p[1]];
            let t = (ap[0] * e[1] - ap[1] * e[0]) / den;
            let u = (ap[0] * d[1] - ap[1] * d[0]) / den;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                out.push(t);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::geom::overlay::contains_xy;
    use crate::synthetic::{RiverParams, synthetic_river};

    #[test]
    fn the_grid_agrees_with_the_brute_force_containment_test() {
        let river = synthetic_river(RiverParams::default()).unwrap();
        let polys = core::slice::from_ref(&river.polygon);
        let g = EdgeGrid::new(polys);
        let mut rng = crate::synthetic::fixtures::SplitMix::new(3);
        let mut inside = 0;
        for _ in 0..4000 {
            let p = [
                3000.0 + 3100.0 * rng.signed_unit(),
                300.0 * rng.signed_unit(),
            ];
            let want = contains_xy(polys, p);
            assert_eq!(g.contains(p), want, "{p:?}");
            inside += want as usize;
        }
        assert!(inside > 100);
        assert!(!g.contains([1e6, 0.0]) && !g.contains([-1e6, 0.0]));
    }

    #[test]
    fn hits_and_near_boundary_find_a_square_edge() {
        let sq = Polygon {
            exterior: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ],
            holes: vec![],
        };
        let g = EdgeGrid::new(&[sq]);
        let t = g.hits([5.0, 5.0], [15.0, 5.0]);
        assert_eq!(t, vec![0.5]);
        assert!(g.near_boundary([10.05, 5.0], 0.1) && !g.near_boundary([10.5, 5.0], 0.1));
        assert!(g.covers([10.05, 5.0], 0.1) && g.contains([5.0, 5.0]));
    }
}
