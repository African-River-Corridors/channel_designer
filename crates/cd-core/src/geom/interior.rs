//! shapely `representative_point` for polygons: GEOS `InteriorPointArea`.
//!
//! A horizontal scan line at the middle of the polygon's extent, moved to halfway between the
//! nearest vertex ordinates above and below the centre so that it avoids every vertex; the point
//! is the midpoint of the widest section of that line inside the polygon. For several polygons,
//! the one with the widest section wins (the first on a tie).

use super::{Polygon, Pt};

fn avg(a: f64, b: f64) -> f64 {
    (a + b) / 2.0
}

fn rings(p: &Polygon) -> impl Iterator<Item = &Vec<Pt>> {
    core::iter::once(&p.exterior).chain(&p.holes)
}

/// `ScanLineYOrdinateFinder::getScanLineY`.
fn scan_line_y(p: &Polygon) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in &p.exterior {
        lo = lo.min(v[1]);
        hi = hi.max(v[1]);
    }
    let centre = avg(lo, hi);
    for r in rings(p) {
        for v in r {
            let y = v[1];
            if y <= centre {
                if y > lo {
                    lo = y;
                }
            } else if y < hi {
                hi = y;
            }
        }
    }
    avg(hi, lo)
}

/// `InteriorPointPolygon::process`: the point and the width of its section.
fn polygon_point(p: &Polygon) -> (Pt, f64) {
    let y = scan_line_y(p);
    let mut crossings: Vec<f64> = Vec::new();
    for r in rings(p) {
        let (lo, hi) = r
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                (l.min(v[1]), h.max(v[1]))
            });
        if y < lo || y > hi {
            continue;
        }
        for w in r.windows(2) {
            let (p0, p1) = (w[0], w[1]);
            if (p0[1] > y && p1[1] > y) || (p0[1] < y && p1[1] < y) {
                continue;
            }
            // `isEdgeCrossingCounted`
            if p0[1] == p1[1] || (p0[1] == y && p1[1] < y) || (p1[1] == y && p0[1] < y) {
                continue;
            }
            let x = if p0[0] == p1[0] {
                p0[0]
            } else {
                let m = (p1[1] - p0[1]) / (p1[0] - p0[0]);
                p0[0] + ((y - p0[1]) / m)
            };
            crossings.push(x);
        }
    }
    let mut point = p.exterior.first().copied().unwrap_or([f64::NAN; 2]);
    let mut width = 0.0;
    crossings.sort_by(f64::total_cmp);
    for pair in crossings.chunks(2) {
        if let [x1, x2] = *pair
            && x2 - x1 > width
        {
            width = x2 - x1;
            point = [avg(x1, x2), y];
        }
    }
    (point, width)
}

/// shapely `geom.representative_point()` for one or more polygons; `None` when there are none.
pub fn representative_point(polys: &[Polygon]) -> Option<Pt> {
    let mut best: Option<(Pt, f64)> = None;
    for p in polys.iter().filter(|p| !p.exterior.is_empty()) {
        let (pt, w) = polygon_point(p);
        if best.is_none_or(|(_, bw)| w > bw) {
            best = Some((pt, w));
        }
    }
    best.map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (scan line of an L-shaped polygon)
    use super::*;

    #[test]
    fn the_point_of_an_l_shape_is_on_its_widest_section() {
        // y spans 0..10 with a vertex at the centre, 5. The nearest ordinate at or below the
        // centre is 5 and the nearest above it is 10, so the scan line is y = 7.5, where the L
        // is 0..5 wide.
        let l = Polygon {
            exterior: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 5.0],
                [5.0, 5.0],
                [5.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ],
            holes: vec![],
        };
        assert_eq!(representative_point(&[l]), Some([2.5, 7.5]));
    }
}
