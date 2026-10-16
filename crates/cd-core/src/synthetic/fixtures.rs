//! Known-answer fixtures for water-aware alignment (spec 0005). Each builds a water polygon, the
//! seed leg(s) and any fixed piece, and its doc comment gives the answer a correct method must
//! reach, derived by hand. All are in metres, in a local plane frame.

use crate::geom::{Polygon, Pt};

/// Water, seed legs (in order) and fixed pieces (must-follow segments) for one fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterFixture {
    pub water: Polygon,
    pub legs: Vec<Vec<Pt>>,
    pub fixed: Vec<Vec<Pt>>,
}

impl WaterFixture {
    /// The same fixture turned by `angle_rad` about the origin and then moved by `offset`.
    /// Known answers carry over, turned and moved the same way (property tests use this).
    pub fn transformed(&self, angle_rad: f64, offset: Pt) -> WaterFixture {
        let (s, c) = (libm::sin(angle_rad), libm::cos(angle_rad));
        let f = |p: &Pt| {
            [
                c * p[0] - s * p[1] + offset[0],
                s * p[0] + c * p[1] + offset[1],
            ]
        };
        let line = |l: &Vec<Pt>| l.iter().map(f).collect::<Vec<Pt>>();
        WaterFixture {
            water: Polygon {
                exterior: line(&self.water.exterior),
                holes: self.water.holes.iter().map(line).collect(),
            },
            legs: self.legs.iter().map(line).collect(),
            fixed: self.fixed.iter().map(line).collect(),
        }
    }
}

fn closed(mut ring: Vec<Pt>) -> Polygon {
    ring.push(ring[0]);
    Polygon {
        exterior: ring,
        holes: vec![],
    }
}

/// A small deterministic generator (splitmix64), so fixtures need no random-number crate.
pub(crate) struct SplitMix(u64);

impl SplitMix {
    pub(crate) fn new(seed: u64) -> Self {
        SplitMix(seed)
    }

    /// A value in [-1, 1).
    pub(crate) fn signed_unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// **Straight canal.** Water is the rectangle `x` in `[-width, length + width]`, `|y| <= width/2`.
/// One seed leg runs from `(0, 0)` to `(length, 0)` with a vertex every 100 m; interior vertices
/// are moved sideways by up to `jitter_m` (deterministic from `seed`), the ends are not.
///
/// Known answer: the fairway fit returns the straight line `y = 0` from `(0, 0)` to `(length, 0)`.
pub fn straight_canal(length_m: f64, width_m: f64, jitter_m: f64, seed: u64) -> WaterFixture {
    let h = width_m / 2.0;
    let water = closed(vec![
        [-width_m, -h],
        [length_m + width_m, -h],
        [length_m + width_m, h],
        [-width_m, h],
    ]);
    let n = (length_m / 100.0).ceil().max(1.0) as usize;
    let mut rng = SplitMix::new(seed);
    let leg = (0..=n)
        .map(|i| {
            let x = length_m * i as f64 / n as f64;
            let y = if i == 0 || i == n {
                0.0
            } else {
                jitter_m * rng.signed_unit()
            };
            [x, y]
        })
        .collect();
    WaterFixture {
        water,
        legs: vec![leg],
        fixed: vec![],
    }
}

/// Where the narrowing reach's water starts and ends narrowing (m along `x`).
pub const NARROWING_TAPER_IN: (f64, f64) = (900.0, 1000.0);
/// Where it widens again.
pub const NARROWING_TAPER_OUT: (f64, f64) = (1500.0, 1600.0);
/// The narrowing reach's length (m).
pub const NARROWING_LENGTH_M: f64 = 3000.0;

/// **Narrowing reach.** A straight reach along `y = 0`, 3,000 m long, symmetric about the line.
/// The water is `wide_m` wide, narrows linearly to `narrow_m` between x = 900 and 1,000 m, stays
/// `narrow_m` to x = 1,500 m, and widens linearly back to `wide_m` by x = 1,600 m. It extends
/// `wide_m` beyond each end. The seed leg is the centre line `(0, 0)` to `(3000, 0)`.
///
/// Known answer: for a straight channel of bottom width `W`, with `narrow_m < W < wide_m`, the
/// water is narrower than `W` exactly on `x` in `[900 + 100 k, 1600 - 100 k]` with
/// `k = (wide_m - W) / (wide_m - narrow_m)`, and nowhere else. For 60 m, 20 m and W = 30.4 m
/// (PIANC-B, CEMT IV): `k = 0.74`, so the stretch is 974 m to 1,526 m.
pub fn narrowing_reach(wide_m: f64, narrow_m: f64) -> WaterFixture {
    let (w, n) = (wide_m / 2.0, narrow_m / 2.0);
    let (a0, a1) = NARROWING_TAPER_IN;
    let (b0, b1) = NARROWING_TAPER_OUT;
    let end = NARROWING_LENGTH_M + wide_m;
    let water = closed(vec![
        [-wide_m, -w],
        [a0, -w],
        [a1, -n],
        [b0, -n],
        [b1, -w],
        [end, -w],
        [end, w],
        [b1, w],
        [b0, n],
        [a1, n],
        [a0, w],
        [-wide_m, w],
    ]);
    WaterFixture {
        water,
        legs: vec![vec![[0.0, 0.0], [NARROWING_LENGTH_M, 0.0]]],
        fixed: vec![],
    }
}

/// **Open 90° basin.** Two legs meet at a right angle at the origin: `(0, -leg_m)` to `(0, 0)`
/// heading north, then `(0, 0)` to `(leg_m, 0)` heading east. The water is an L of open water:
/// the outer banks are 200 m outside each leg (beyond the corner too), and the only bank on the
/// inside of the turn is a land corner at `(c, -c)`, `c = inner_corner_m`, with land at
/// `x >= c, y <= -c`.
///
/// Known answer: a fillet of radius `R` tangent to both legs has its centre at `(R, -R)`. In
/// navigable water shrunk by `b` (half the bottom width plus clearance) its arc clears the land
/// corner by `b` while `R - (R - c)·√2 >= b`, so the largest fillet the basin allows is
/// `R_max = (1 + √2)(√2·c - b)`, provided `R_max <= leg_m`. For `c = 200` m and `b = 15.2` m:
/// `R_max = 646.1` m (to 0.1 m). A polygonal round join in the navigable water moves this by
/// under 0.1 m.
pub fn open_basin_90(leg_m: f64, inner_corner_m: f64) -> WaterFixture {
    let (c, out) = (inner_corner_m, 200.0);
    let water = closed(vec![
        [-out, -leg_m - out],
        [c, -leg_m - out],
        [c, -c],
        [leg_m + out, -c],
        [leg_m + out, out],
        [-out, out],
    ]);
    WaterFixture {
        water,
        legs: vec![
            vec![[0.0, -leg_m], [0.0, 0.0]],
            vec![[0.0, 0.0], [leg_m, 0.0]],
        ],
        fixed: vec![],
    }
}

/// The largest fillet radius the open basin allows (see [`open_basin_90`]).
pub fn open_basin_max_radius(inner_corner_m: f64, shrink_m: f64) -> f64 {
    (1.0 + core::f64::consts::SQRT_2) * (core::f64::consts::SQRT_2 * inner_corner_m - shrink_m)
}

/// **Fixed tail.** Open water, the rectangle `x` in `[-300, 3300]`, `y` in `[-300, 400]`. The seed
/// leg runs east from `(0, 0)` to `(2000, 0)`. The fixed segment (a must-follow piece) runs east
/// from `(2400, 100)` to `(3000, 100)`.
///
/// Known answer: the merged line reaches `(2400, 100)` heading due east (heading error under the
/// method's tolerance), and from there on is the fixed segment, unchanged.
pub fn fixed_tail() -> WaterFixture {
    WaterFixture {
        water: closed(vec![
            [-300.0, -300.0],
            [3300.0, -300.0],
            [3300.0, 400.0],
            [-300.0, 400.0],
        ]),
        legs: vec![vec![[0.0, 0.0], [2000.0, 0.0]]],
        fixed: vec![vec![[2400.0, 100.0], [3000.0, 100.0]]],
    }
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (the known answers in each fixture's doc comment)
    use super::*;
    use crate::geom::overlay::contains_xy;
    use crate::geom::valid::is_valid;

    fn inside(f: &WaterFixture, p: Pt) -> bool {
        contains_xy(core::slice::from_ref(&f.water), p)
    }

    #[test]
    fn the_straight_canal_builds_with_its_leg_in_the_water_and_fixed_ends() {
        let f = straight_canal(2000.0, 100.0, 20.0, 7);
        assert!(is_valid(core::slice::from_ref(&f.water)));
        let leg = &f.legs[0];
        assert_eq!((leg[0], *leg.last().unwrap()), ([0.0, 0.0], [2000.0, 0.0]));
        assert!(leg.iter().all(|p| inside(&f, *p) && p[1].abs() <= 20.0));
        assert!(leg.iter().any(|p| p[1] != 0.0));
        assert_eq!(f, straight_canal(2000.0, 100.0, 20.0, 7));
        assert_ne!(f, straight_canal(2000.0, 100.0, 20.0, 8));
    }

    #[test]
    fn the_narrowing_reach_is_as_wide_as_its_doc_says() {
        let f = narrowing_reach(60.0, 20.0);
        assert!(is_valid(core::slice::from_ref(&f.water)));
        for (x, half) in [(500.0, 30.0), (974.0, 15.2), (1200.0, 10.0), (1526.0, 15.2)] {
            assert!(inside(&f, [x, half - 0.01]), "{x}");
            assert!(!inside(&f, [x, half + 0.01]), "{x}");
        }
    }

    #[test]
    fn the_open_basin_has_its_land_corner_and_its_radius_formula() {
        let f = open_basin_90(1500.0, 200.0);
        assert!(is_valid(core::slice::from_ref(&f.water)));
        assert!(!inside(&f, [201.0, -201.0]) && inside(&f, [199.0, -201.0]));
        let r = open_basin_max_radius(200.0, 15.2);
        assert!((r - 646.1).abs() < 0.05, "{r}");
        // At R_max the arc's nearest point (on the diagonal) is b from the land corner.
        let mid = [
            r - r / core::f64::consts::SQRT_2,
            -r + r / core::f64::consts::SQRT_2,
        ];
        let d = crate::geom::dist(mid, [200.0, -200.0]);
        assert!((d - 15.2).abs() < 1e-9, "{d}");
    }

    #[test]
    fn the_fixed_tail_builds_with_its_pieces_in_open_water() {
        let f = fixed_tail();
        assert!(is_valid(core::slice::from_ref(&f.water)));
        assert!(f.legs[0].iter().chain(&f.fixed[0]).all(|p| inside(&f, *p)));
    }

    #[test]
    fn a_turned_and_moved_fixture_keeps_its_shape() {
        let f = narrowing_reach(60.0, 20.0);
        let g = f.transformed(0.7, [1000.0, -50.0]);
        let back = g.transformed(-0.7, [0.0, 0.0]);
        let (s, c) = (libm::sin(-0.7), libm::cos(-0.7));
        let off = [c * 1000.0 - s * -50.0, s * 1000.0 + c * -50.0];
        for (p, q) in f.water.exterior.iter().zip(&back.water.exterior) {
            assert!((p[0] - (q[0] - off[0])).abs() < 1e-9 && (p[1] - (q[1] - off[1])).abs() < 1e-9);
        }
    }
}
