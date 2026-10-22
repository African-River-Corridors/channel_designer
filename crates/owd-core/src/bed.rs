//! The bed model (spec 0012, ticket T7): the bed of a river with no survey, from a depth profile.
//!
//! Inside the water, `bed = water level - depth x shape`. The depth is read along the river
//! centreline from a table of `(chainage, depth)` points: linear between points and held flat
//! beyond the first and the last, as the water level is (ADR 0006). The shape is a parabola across
//! the water: 1 at the centreline and 0 at the bank on each side. So the bed is deepest at the
//! centreline and meets the water level at the banks, and it is never above the water.
//!
//! Outside the water the bed model says nothing ([`BedModel::bed_elevation`] gives `None`) and the
//! caller reads the DEM, as before. No file, clock or network access.
//!
//! Depth here is assumed, not measured: [`DEPTH_ASSUMED_LABEL`] goes on every output that used it.

use crate::alignment::PyError;
use crate::alignment::navigable::Chained;
use crate::geom::{Polygon, Pt, linear};
use std::collections::HashMap;

/// Said on every output that used a bed model.
pub const DEPTH_ASSUMED_LABEL: &str = "depth assumed, not measured";

/// The spacing of the cross-sections read along the centreline (m).
pub const BED_STATION_M: f64 = 10.0;

/// How far from the centreline a bank is looked for (m). Water wider than this on one side counts
/// as this wide: the bed is a parabola over this width, which is far wider than any reach screened.
pub const BED_REACH_M: f64 = 5000.0;

/// The size of a cell of the edge index (m).
const CELL_M: f64 = 500.0;

/// The edges of the water's rings, in a grid, so a station reads the banks near it and not every
/// edge of a river hundreds of kilometres long.
struct EdgeIndex {
    edges: Vec<[Pt; 2]>,
    cells: HashMap<(i64, i64), Vec<u32>>,
}

impl EdgeIndex {
    fn new(water: &[Polygon]) -> EdgeIndex {
        let mut edges = Vec::new();
        for poly in water {
            for ring in core::iter::once(&poly.exterior).chain(&poly.holes) {
                edges.extend(ring.windows(2).map(|w| [w[0], w[1]]));
            }
        }
        let cell = |v: f64| (v / CELL_M).floor() as i64;
        let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            let (x0, x1) = (e[0][0].min(e[1][0]), e[0][0].max(e[1][0]));
            let (y0, y1) = (e[0][1].min(e[1][1]), e[0][1].max(e[1][1]));
            if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
                continue;
            }
            for cx in cell(x0)..=cell(x1) {
                for cy in cell(y0)..=cell(y1) {
                    cells.entry((cx, cy)).or_default().push(i as u32);
                }
            }
        }
        EdgeIndex { edges, cells }
    }

    /// The distances from `p` to the water's edge along `+n` and `-n`, or `(0, 0)` when `p` is not
    /// in the water (an even number of edges lie ahead of it). Edges are read to `BED_REACH_M`.
    fn half_widths(&self, p: Pt, n: Pt) -> (f64, f64) {
        let cell = |v: f64| (v / CELL_M).floor() as i64;
        let (a, b) = (
            [p[0] - n[0] * BED_REACH_M, p[1] - n[1] * BED_REACH_M],
            [p[0] + n[0] * BED_REACH_M, p[1] + n[1] * BED_REACH_M],
        );
        let mut near: Vec<u32> = Vec::new();
        for cx in cell(a[0].min(b[0]))..=cell(a[0].max(b[0])) {
            for cy in cell(a[1].min(b[1]))..=cell(a[1].max(b[1])) {
                if let Some(v) = self.cells.get(&(cx, cy)) {
                    near.extend_from_slice(v);
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        let (mut pos, mut neg, mut ahead) = (BED_REACH_M, BED_REACH_M, 0usize);
        for &i in &near {
            let [e0, e1] = self.edges[i as usize];
            let d = [e1[0] - e0[0], e1[1] - e0[1]];
            let den = n[0] * d[1] - n[1] * d[0];
            if den == 0.0 {
                continue;
            }
            let ap = [e0[0] - p[0], e0[1] - p[1]];
            let u = (ap[0] * d[1] - ap[1] * d[0]) / den;
            let v = (ap[0] * n[1] - ap[1] * n[0]) / den;
            // Half-open, so a ray through a vertex counts one edge, not two.
            if (0.0..1.0).contains(&v) {
                if u > 0.0 {
                    ahead += 1;
                    pos = pos.min(u);
                } else if u < 0.0 {
                    neg = neg.min(-u);
                }
            }
        }
        if ahead % 2 == 1 {
            (pos, neg)
        } else {
            (0.0, 0.0)
        }
    }
}

/// The depth at `chainage_m` on a table of `(chainage, depth)` points sorted by chainage: linear
/// between points, flat beyond the ends. Two points at one chainage make a step: the later holds.
pub fn depth_at(table: &[[f64; 2]], chainage_m: f64) -> f64 {
    let (Some(first), Some(last)) = (table.first(), table.last()) else {
        return f64::NAN;
    };
    if chainage_m.is_nan() {
        return f64::NAN;
    }
    if chainage_m < first[0] {
        return first[1];
    }
    let i = table.partition_point(|p| p[0] <= chainage_m) - 1;
    if i + 1 >= table.len() {
        return last[1];
    }
    let (a, b) = (table[i], table[i + 1]);
    a[1] + (b[1] - a[1]) * (chainage_m - a[0]) / (b[0] - a[0])
}

/// The parabola across the water: 1 at the centreline (`offset_m` 0), 0 at the bank
/// (`|offset_m| >= half_width_m`), never negative.
pub fn parabola(offset_m: f64, half_width_m: f64) -> f64 {
    if half_width_m.is_nan() || half_width_m <= 0.0 {
        return 0.0;
    }
    let t = offset_m / half_width_m;
    (1.0 - t * t).max(0.0)
}

/// One cross-section of the water, read at a station of the centreline.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Section {
    chainage_m: f64,
    at: Pt,
    /// The unit normal, to the left of the direction of the centreline.
    normal: Pt,
    /// The distance from the centreline to the bank on the left and on the right.
    left_m: f64,
    right_m: f64,
}

/// A bed model on a river: the depth table, the centreline and the water it lies in.
#[derive(Debug, Clone, PartialEq)]
pub struct BedModel {
    table: Vec<[f64; 2]>,
    centreline: Vec<Pt>,
    sections: Vec<Section>,
    /// The centreline for projecting many points onto it fast (the same answers as a scan).
    index: linear::ProjectIndex,
}

impl BedModel {
    /// A bed model from `table` (`(chainage m, depth m)`, at least one point, chainages not
    /// decreasing, depths finite and not negative), on `centreline` and the `water` around it.
    pub fn new(
        table: &[[f64; 2]],
        centreline: &[Pt],
        water: &[Polygon],
    ) -> Result<BedModel, PyError> {
        if table.is_empty() {
            return Err(PyError::value("a bed model needs at least one depth."));
        }
        if table
            .iter()
            .any(|p| !(p[0].is_finite() && p[1].is_finite()))
        {
            return Err(PyError::value(
                "every point of the depth table must be a finite chainage and depth.",
            ));
        }
        if table.iter().any(|p| p[1] < 0.0) {
            return Err(PyError::value("a depth must not be negative."));
        }
        if table.windows(2).any(|w| w[1][0] < w[0][0]) {
            return Err(PyError::value(
                "the chainages of the depth table must not decrease.",
            ));
        }
        if centreline.len() < 2 {
            return Err(PyError::value(
                "the river centreline needs at least two points.",
            ));
        }
        let index = EdgeIndex::new(water);
        let c = Chained::new(centreline);
        let len = c.length();
        let n = (len / BED_STATION_M).ceil().max(1.0) as usize;
        let mut sections: Vec<Section> = (0..=n)
            .map(|i| {
                let s = (i as f64 * BED_STATION_M).min(len);
                let d = c.direction(s, BED_STATION_M);
                let normal = [-d[1], d[0]];
                let at = c.at(s);
                let (left_m, right_m) = index.half_widths(at, normal);
                Section {
                    chainage_m: s,
                    at,
                    normal,
                    left_m,
                    right_m,
                }
            })
            .collect();
        // A station on the edge of the water (the end of a river that ends at the polygon's cap)
        // reads no water. It takes the width of the nearest station that does.
        let wet: Vec<usize> = (0..sections.len())
            .filter(|&i| sections[i].left_m > 0.0 || sections[i].right_m > 0.0)
            .collect();
        for i in 0..sections.len() {
            if wet.contains(&i) {
                continue;
            }
            if let Some(&j) = wet.iter().min_by_key(|&&j| j.abs_diff(i)) {
                sections[i].left_m = sections[j].left_m;
                sections[i].right_m = sections[j].right_m;
            }
        }
        Ok(BedModel {
            table: table.to_vec(),
            centreline: centreline.to_vec(),
            sections,
            index: linear::ProjectIndex::new(centreline),
        })
    }

    /// The depth of the centreline at `chainage_m` (m).
    pub fn depth_m(&self, chainage_m: f64) -> f64 {
        depth_at(&self.table, chainage_m)
    }

    /// The half-widths of the water at `chainage_m`: to the left and to the right of the centreline.
    pub fn half_widths_m(&self, chainage_m: f64) -> (f64, f64) {
        let (a, b, t) = self.between(chainage_m);
        let mix = |x: f64, y: f64| x + (y - x) * t;
        (mix(a.left_m, b.left_m), mix(a.right_m, b.right_m))
    }

    fn between(&self, chainage_m: f64) -> (&Section, &Section, f64) {
        let s = &self.sections;
        let i = s
            .partition_point(|x| x.chainage_m <= chainage_m)
            .clamp(1, s.len() - 1);
        let (a, b) = (&s[i - 1], &s[i]);
        let span = b.chainage_m - a.chainage_m;
        let t = if span > 0.0 {
            ((chainage_m - a.chainage_m) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (a, b, t)
    }

    /// The depth of the bed below the water at the place `offset_m` to the left of the centreline
    /// at `chainage_m` (negative: to the right): `depth x shape`. Zero at the banks and beyond.
    pub fn depth_below_water_m(&self, chainage_m: f64, offset_m: f64) -> f64 {
        let (left, right) = self.half_widths_m(chainage_m);
        let half = if offset_m >= 0.0 { left } else { right };
        self.depth_m(chainage_m) * parabola(offset_m, half)
    }

    /// The depth below the water at `p`, or `None` when `p` is not in the water: farther from the
    /// centreline than the bank on its side. (Water the centreline's cross-sections do not reach,
    /// such as a bay, is read from the DEM.)
    pub fn depth_below_water_at(&self, p: Pt) -> Option<f64> {
        let s = self.index.project(p);
        let (a, b, t) = self.between(s);
        let normal = if t < 0.5 { a.normal } else { b.normal };
        let at = [
            a.at[0] + (b.at[0] - a.at[0]) * t,
            a.at[1] + (b.at[1] - a.at[1]) * t,
        ];
        let offset = (p[0] - at[0]) * normal[0] + (p[1] - at[1]) * normal[1];
        let (left, right) = self.half_widths_m(s);
        let half = if offset >= 0.0 { left } else { right };
        (offset.abs() <= half).then(|| self.depth_below_water_m(s, offset))
    }

    /// The bed elevation at `p` for a water level of `water_level_m` there, or `None` when `p` is
    /// not in the water (read the DEM). Never above `water_level_m`.
    pub fn bed_elevation(&self, p: Pt, water_level_m: f64) -> Option<f64> {
        self.depth_below_water_at(p).map(|d| water_level_m - d)
    }

    /// The river chainage of `p`, as the water level reads it.
    pub fn chainage_of(&self, p: Pt) -> f64 {
        self.index.project(p)
    }
}

#[cfg(test)]
mod tests {
    //! Label: property (the bed is never above the water and meets it at the banks), and
    //! sourced - hand calculation (a parabola across a straight channel).
    use super::*;
    use crate::synthetic::{RiverParams, synthetic_river};

    fn straight(width_m: f64) -> (Vec<Pt>, Vec<Polygon>) {
        let h = width_m / 2.0;
        (
            vec![[0.0, 0.0], [1000.0, 0.0]],
            vec![Polygon {
                exterior: vec![
                    [-50.0, -h],
                    [1050.0, -h],
                    [1050.0, h],
                    [-50.0, h],
                    [-50.0, -h],
                ],
                holes: vec![],
            }],
        )
    }

    #[test]
    fn the_depth_table_is_linear_between_points_and_flat_beyond_the_ends() {
        let t = [[100.0, 2.0], [300.0, 4.0]];
        assert_eq!(depth_at(&t, 0.0), 2.0);
        assert_eq!(depth_at(&t, 200.0), 3.0);
        assert_eq!(depth_at(&t, 900.0), 4.0);
        assert!(depth_at(&[], 1.0).is_nan());
        // A step takes its later depth.
        let step = [[0.0, 1.0], [50.0, 1.0], [50.0, 5.0], [100.0, 5.0]];
        assert_eq!(depth_at(&step, 50.0), 5.0);
        assert_eq!(depth_at(&step, 49.0), 1.0);
    }

    #[test]
    fn a_straight_channel_has_the_hand_calculated_parabola() {
        // 100 m wide, 3 m deep: at 25 m from the centre the shape is 1 - 0.25 = 0.75, so the bed
        // is 2.25 m below the water; at the bank it is at the water level.
        let (line, water) = straight(100.0);
        let bed = BedModel::new(&[[0.0, 3.0]], &line, &water).unwrap();
        let wl = 10.0;
        let at = |y: f64| bed.bed_elevation([500.0, y], wl).unwrap();
        assert!((at(0.0) - 7.0).abs() < 1e-9);
        assert!((at(25.0) - (10.0 - 2.25)).abs() < 1e-9);
        assert!((at(-25.0) - (10.0 - 2.25)).abs() < 1e-9);
        assert!((at(49.999999) - 10.0).abs() < 1e-5);
        assert_eq!(bed.bed_elevation([500.0, 60.0], wl), None);
        // The area across the channel is 2/3 x width x depth = 200 m2.
        let n = 10_000;
        let area: f64 = (0..n)
            .map(|i| {
                let y = -50.0 + 100.0 * (i as f64 + 0.5) / n as f64;
                (wl - at(y)) * (100.0 / n as f64)
            })
            .sum();
        assert!((area - 200.0).abs() < 0.01, "{area}");
    }

    #[test]
    fn the_depth_follows_the_table_along_the_river() {
        let (line, water) = straight(100.0);
        let bed = BedModel::new(&[[0.0, 2.0], [1000.0, 4.0]], &line, &water).unwrap();
        assert!((bed.bed_elevation([250.0, 0.0], 0.0).unwrap() + 2.5).abs() < 1e-9);
        assert!((bed.bed_elevation([750.0, 0.0], 0.0).unwrap() + 3.5).abs() < 1e-9);
    }

    #[test]
    fn the_bed_is_never_above_the_water_and_meets_it_at_the_banks() {
        let river = synthetic_river(RiverParams::default()).unwrap();
        let water = [river.polygon.clone()];
        let table = [[0.0, 1.5], [2500.0, 4.0], [4000.0, 0.5]];
        let bed = BedModel::new(&table, &river.centreline, &water).unwrap();
        let c = Chained::new(&river.centreline);
        let mut s = 0.0;
        let mut checked = 0;
        while s <= c.length() {
            let (l, r) = bed.half_widths_m(s);
            assert!(l > 0.0 && r > 0.0, "chainage {s}: {l} {r}");
            for k in 0..=40 {
                let f = -1.0 + 2.0 * k as f64 / 40.0;
                let off = if f >= 0.0 { f * l } else { f * r };
                let d = bed.depth_below_water_m(s, off);
                assert!(d >= 0.0, "bed above the water at {s} {off}: {d}");
                assert!(d <= bed.depth_m(s) + 1e-12);
                if (f.abs() - 1.0).abs() < 1e-12 {
                    assert!(d.abs() < 1e-9, "bed does not meet the water at the bank");
                }
                if f == 0.0 {
                    assert!((d - bed.depth_m(s)).abs() < 1e-12);
                }
                checked += 1;
            }
            // Through the polygon query too, at points inside the water.
            let p = c.at(s);
            if let Some(d) = bed.depth_below_water_at(p) {
                assert!((0.0..=bed.depth_m(s) + 1e-9).contains(&d));
                assert!(bed.bed_elevation(p, 7.0).unwrap() <= 7.0);
            }
            s += 37.0;
        }
        assert!(checked > 1000);
    }

    #[test]
    fn a_bad_table_is_refused() {
        let (line, water) = straight(100.0);
        for bad in [
            vec![],
            vec![[0.0, -1.0]],
            vec![[0.0, f64::NAN]],
            vec![[10.0, 1.0], [5.0, 1.0]],
        ] {
            assert!(BedModel::new(&bad, &line, &water).is_err(), "{bad:?}");
        }
        assert!(BedModel::new(&[[0.0, 1.0]], &line[..1], &water).is_err());
    }
}
