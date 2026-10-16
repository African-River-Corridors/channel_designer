//! Is a candidate a loop artefact, or a legitimately winding channel? (`alignment/loops.py`)
//!
//! Self-intersection is the primary test; cumulative turn catches a spiral that never quite
//! crosses; a length ratio is a loose backstop. The thresholds are user choices, not a standard.

use crate::geom::{Pt, simple};
use crate::numpy::{np_mod, np_sum};
use ovcd_core::pyfmt::fixed;

/// A path that crosses itself is a loop.
pub const REJECT_SELF_INTERSECTING: bool = true;
/// Cumulative absolute heading change, degrees (three full turns).
pub const TURN_LIMIT_DEG: f64 = 1080.0;
/// Loose backstop on path length over window length.
pub const LEN_RATIO_BACKSTOP: f64 = 3.0;

/// Total absolute heading change along a polyline, in degrees.
pub fn cumulative_turn_deg(coords: &[Pt]) -> f64 {
    if coords.len() < 3 {
        return 0.0;
    }
    let d: Vec<Pt> = coords
        .windows(2)
        .map(|w| [w[1][0] - w[0][0], w[1][1] - w[0][1]])
        .filter(|d| libm::hypot(d[0], d[1]) > 1e-9)
        .collect();
    if d.len() < 2 {
        return 0.0;
    }
    let head: Vec<f64> = d.iter().map(|d| libm::atan2(d[1], d[0])).collect();
    let pi = core::f64::consts::PI;
    let abs_dh: Vec<f64> = head
        .windows(2)
        .map(|w| (np_mod(w[1] - w[0] + pi, 2.0 * pi) - pi).abs())
        .collect();
    np_sum(&abs_dh) * 180.0 / pi
}

/// True if the polyline crosses itself.
pub fn is_self_intersecting(coords: &[Pt]) -> bool {
    if coords.len() < 3 {
        return false;
    }
    !simple::line_is_simple(coords)
}

/// What `loop_verdict` measured.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopMetrics {
    pub path_len_m: f64,
    pub window_len_m: f64,
    pub len_ratio: f64,
    pub turn_deg: f64,
    pub self_intersecting: bool,
}

/// `(reject, reason, metrics)`. `reason` is `None` when the candidate is acceptable.
pub fn loop_verdict(coords: &[Pt], window_len_m: f64) -> (bool, Option<String>, LoopMetrics) {
    let plen = if coords.len() > 1 {
        let seg: Vec<f64> = coords
            .windows(2)
            .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .collect();
        np_sum(&seg)
    } else {
        0.0
    };
    let ratio = if window_len_m != 0.0 {
        plen / window_len_m
    } else {
        f64::INFINITY
    };
    let turn = cumulative_turn_deg(coords);
    let selfx = REJECT_SELF_INTERSECTING && is_self_intersecting(coords);
    let m = LoopMetrics {
        path_len_m: plen,
        window_len_m,
        len_ratio: ratio,
        turn_deg: turn,
        self_intersecting: selfx,
    };
    if selfx {
        return (true, Some("self-intersecting".into()), m);
    }
    if turn > TURN_LIMIT_DEG {
        let reason = format!(
            "cumulative turn {} deg > {}",
            fixed(turn, 0),
            fixed(TURN_LIMIT_DEG, 0)
        );
        return (true, Some(reason), m);
    }
    if ratio > LEN_RATIO_BACKSTOP {
        let reason = format!(
            "length ratio {} > {} backstop",
            fixed(ratio, 2),
            fixed(LEN_RATIO_BACKSTOP, 1)
        );
        return (true, Some(reason), m);
    }
    (false, None, m)
}

/// Boolean convenience wrapper over [`loop_verdict`].
pub fn is_loop_artefact(coords: &[Pt], window_len_m: f64) -> bool {
    loop_verdict(coords, window_len_m).0
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn arc(radius: f64, sweep_deg: f64, n: usize) -> Vec<Pt> {
        (0..n)
            .map(|i| {
                let t = sweep_deg.to_radians() * i as f64 / (n - 1) as f64;
                [radius * libm::cos(t), radius * libm::sin(t)]
            })
            .collect()
    }

    #[test]
    fn a_half_circle_turns_180_degrees_and_is_accepted() {
        let c = arc(500.0, 180.0, 120);
        assert!((cumulative_turn_deg(&c) - 180.0 * 118.0 / 119.0).abs() < 1e-6);
        let (reject, reason, _) = loop_verdict(&c, 1500.0);
        assert!(!reject && reason.is_none());
    }

    #[test]
    fn a_full_circle_crosses_itself() {
        let (reject, reason, m) = loop_verdict(&arc(300.0, 361.0, 120), 1000.0);
        assert!(reject && m.self_intersecting);
        assert_eq!(reason.as_deref(), Some("self-intersecting"));
    }

    #[test]
    fn a_window_far_shorter_than_its_path_trips_the_ratio_backstop() {
        let straight: Vec<Pt> = (0..50).map(|i| [i as f64 * 1000.0 / 49.0, 0.0]).collect();
        let (reject, reason, _) = loop_verdict(&straight, 250.0);
        assert!(reject);
        assert_eq!(reason.as_deref(), Some("length ratio 4.00 > 3.0 backstop"));
    }
}
