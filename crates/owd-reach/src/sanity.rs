//! The reach's sanity checks (issue #360): banks and a centreline that do not match the route are
//! broken, whatever the per-station checks found.
//!
//! - **Length.** Each bank and the centreline must be from [`LENGTH_RATIO_MIN`] to
//!   [`LENGTH_RATIO_MAX`] times the route's length. A bank follows the river, so its length is
//!   close to the route's: on a river of width `w` its length differs from the centreline's by
//!   about `w/2` per radian of turn, which is a few per cent on a meandering reach. A bank that
//!   wanders round other water (flooded pits, a lake) comes out many times the route. Banks are
//!   often shorter than the route: they are trimmed where the mask cannot see the water (at a
//!   mouth, at the window's edge), so the lower limit is looser.
//! - **Crossing.** The left and right banks must not cross each other.
//!
//! A screening tolerance, not a rule of any Rule Set: it says the banks need a look, not that the
//! river is unnavigable.

use crate::bankcheck::lines_cross;

/// The longest a bank or the centreline may be, as a multiple of the route's length.
/// Provisional: 1.5 leaves room for the few per cent of a bank on a meander, and catches the
/// banks of the issue (about 10 times the route).
pub const LENGTH_RATIO_MAX: f64 = 1.5;

/// The shortest a bank or the centreline may be, as a multiple of the route's length.
/// Provisional: under half, the mask saw less than half of the route. The recorded estuary
/// example's centreline is 0.65 of its route, trimmed at the mouth, and passes.
pub const LENGTH_RATIO_MIN: f64 = 0.5;

type P2 = [f64; 2];

/// One failed sanity check.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// `left_bank_length`, `right_bank_length`, `centreline_length` or `banks_cross`.
    pub check: &'static str,
    pub message: String,
}

/// The length of a line in metres (projected coordinates).
fn length(line: &[P2]) -> f64 {
    line.windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// The sanity checks of banks and a centreline (projected, metres) against the route's length.
/// Empty when every check passes.
pub fn check(route_length_m: f64, left: &[P2], right: &[P2], centreline: &[P2]) -> Vec<Warning> {
    let mut out = Vec::new();
    for (check, name, line) in [
        ("left_bank_length", "the left bank", left),
        ("right_bank_length", "the right bank", right),
        ("centreline_length", "the centreline", centreline),
    ] {
        let len = length(line);
        let ratio = len / route_length_m;
        if !(ratio.is_finite() && (LENGTH_RATIO_MIN..=LENGTH_RATIO_MAX).contains(&ratio)) {
            out.push(Warning {
                check,
                message: format!(
                    "{name} is {:.1} km for a route of {:.1} km ({ratio:.2} times; the limits are {} and {} times)",
                    len / 1000.0,
                    route_length_m / 1000.0,
                    LENGTH_RATIO_MIN,
                    LENGTH_RATIO_MAX
                ),
            });
        }
    }
    if let Some(p) = lines_cross(left, right) {
        out.push(Warning {
            check: "banks_cross",
            message: format!(
                "the left and right banks cross (first at {:.0}, {:.0})",
                p[0], p[1]
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    //! Label: property (straight and meandering synthetic banks against their route).
    use super::*;

    fn straight(y: f64, len: f64) -> Vec<P2> {
        (0..=100).map(|i| [len * i as f64 / 100.0, y]).collect()
    }

    #[test]
    fn banks_that_follow_the_route_pass() {
        let w = check(
            10_000.0,
            &straight(20.0, 10_000.0),
            &straight(-20.0, 10_000.0),
            &straight(0.0, 10_000.0),
        );
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn banks_on_a_meander_pass() {
        // A 40 m wide sine meander: amplitude 300 m, wavelength 1500 m, over 10 km.
        let line = |off: f64| -> Vec<P2> {
            (0..=2000)
                .map(|i| {
                    let x = 5.0 * i as f64;
                    let k = core::f64::consts::TAU / 1500.0;
                    let y = 300.0 * (k * x).sin();
                    let (dx, dy): (f64, f64) = (1.0, 300.0 * k * (k * x).cos());
                    let n = dx.hypot(dy);
                    [x - off * dy / n, y + off * dx / n]
                })
                .collect()
        };
        let c = line(0.0);
        let w = check(length(&c), &line(20.0), &line(-20.0), &c);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn a_bank_ten_times_the_route_fails() {
        // The left bank zigzags round other water: ten times the route.
        let zig: Vec<P2> = (0..=200)
            .map(|i| [50.0 * i as f64, if i % 2 == 0 { 20.0 } else { 270.0 }])
            .collect();
        let w = check(
            10_000.0,
            &zig,
            &straight(-20.0, 10_000.0),
            &straight(0.0, 10_000.0),
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].check, "left_bank_length");
    }

    #[test]
    fn a_centreline_of_173_km_for_a_10_km_route_fails() {
        let c = straight(0.0, 173_000.0);
        let w = check(
            10_000.0,
            &straight(20.0, 10_000.0),
            &straight(-20.0, 10_000.0),
            &c,
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].check, "centreline_length");
        assert!(w[0].message.contains("173.0 km"), "{}", w[0].message);
    }

    #[test]
    fn a_short_bank_fails_too() {
        let w = check(
            10_000.0,
            &straight(20.0, 4_900.0),
            &straight(-20.0, 10_000.0),
            &straight(0.0, 10_000.0),
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].check, "left_bank_length");
    }

    #[test]
    fn a_bank_trimmed_to_two_thirds_of_the_route_passes() {
        let w = check(
            10_000.0,
            &straight(20.0, 6_500.0),
            &straight(-20.0, 6_500.0),
            &straight(0.0, 6_500.0),
        );
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn banks_that_cross_fail() {
        let left: Vec<P2> = (0..=100)
            .map(|i| [100.0 * i as f64, if i < 50 { 20.0 } else { -20.0 }])
            .collect();
        let right: Vec<P2> = (0..=100)
            .map(|i| [100.0 * i as f64, if i < 50 { -20.0 } else { 20.0 }])
            .collect();
        let w = check(10_000.0, &left, &right, &straight(0.0, 10_000.0));
        assert!(w.iter().any(|w| w.check == "banks_cross"), "{w:?}");
    }

    #[test]
    fn no_route_length_fails_every_length_check() {
        let w = check(
            0.0,
            &straight(20.0, 10_000.0),
            &straight(-20.0, 10_000.0),
            &straight(0.0, 10_000.0),
        );
        assert_eq!(w.len(), 3, "{w:?}");
    }
}
