//! Fillets: a circular arc of a given radius tangent to a line before and after a corner or a
//! bend (spec 0005: the push-out rung of the water-aware rolling circle and the leg joins).
//!
//! The line is kept up to chainage `s0` and from `s1`. Its tangent lines there meet at the
//! corner `V`. A fillet of radius `R` turns through the angle `θ` between them: its tangent points
//! lie `T = R tan(θ/2)` from `V` on each tangent line, and the replacement runs straight from the
//! line at `s0` to the first tangent point, round the arc, and straight on to the line at `s1`.
//! It joins the line tangentially at both ends.

use super::navigable::Chained;
use crate::geom::{Pt, dist};

/// The angle (radians, signed: positive to the left) from direction `a` to direction `b`.
pub fn turn_angle(a: Pt, b: Pt) -> f64 {
    libm::atan2(a[0] * b[1] - a[1] * b[0], a[0] * b[0] + a[1] * b[1])
}

/// A fillet replacement for `line` between chainages `s0` and `s1`, with the line's own
/// directions there (read across `dir_half` metres). `None` when the tangents are (nearly)
/// parallel, do not meet ahead of `s0` and behind `s1`, or the tangent points of radius `r` do not
/// fit between the window ends and the corner.
pub(crate) fn fillet(
    c: &Chained,
    s0: f64,
    s1: f64,
    r: f64,
    dir_half: f64,
    ds: f64,
) -> Option<Vec<Pt>> {
    let (a, b) = (c.at(s0), c.at(s1));
    // The kept line's own directions: just before s0 and just after s1.
    let (d_in, d_out) = (
        c.direction(s0 - dir_half, dir_half),
        c.direction(s1 + dir_half, dir_half),
    );
    fillet_between(a, d_in, b, d_out, r, ds)
}

/// The fillet of radius `r` between the ray from `a` along `d_in` and the ray into `b` along
/// `d_out` (both unit), sampled about every `ds` metres on the arc. See [`fillet`].
pub fn fillet_between(a: Pt, d_in: Pt, b: Pt, d_out: Pt, r: f64, ds: f64) -> Option<Vec<Pt>> {
    let th = turn_angle(d_in, d_out);
    if th.abs() < 1e-6 {
        return None;
    }
    // Corner: a + t d_in = b - u d_out.
    let den = d_in[0] * d_out[1] - d_in[1] * d_out[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let ab = [b[0] - a[0], b[1] - a[1]];
    let t = (ab[0] * d_out[1] - ab[1] * d_out[0]) / den;
    let u = (d_in[0] * ab[1] - d_in[1] * ab[0]) / den;
    if t <= 0.0 || u <= 0.0 {
        return None;
    }
    let v = [a[0] + t * d_in[0], a[1] + t * d_in[1]];
    let tl = r * libm::tan(th.abs() / 2.0);
    if tl > t || tl > u {
        return None;
    }
    let t1 = [v[0] - tl * d_in[0], v[1] - tl * d_in[1]];
    let t2 = [v[0] + tl * d_out[0], v[1] + tl * d_out[1]];
    let sgn = th.signum();
    let n_in = [-d_in[1] * sgn, d_in[0] * sgn];
    let centre = [t1[0] + r * n_in[0], t1[1] + r * n_in[1]];
    let a0 = libm::atan2(t1[1] - centre[1], t1[0] - centre[0]);
    let n = ((r * th.abs() / ds).ceil() as usize).max(2);
    let mut pts = vec![a];
    if dist(a, t1) > 1e-9 {
        pts.push(t1);
    }
    for k in 1..n {
        let ang = a0 + sgn * th.abs() * k as f64 / n as f64;
        pts.push([
            centre[0] + r * libm::cos(ang),
            centre[1] + r * libm::sin(ang),
        ]);
    }
    pts.push(t2);
    if dist(t2, b) > 1e-9 {
        pts.push(b);
    }
    Some(pts)
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (a right-angle corner: tangent length R, arc centre at
    //! (R, -R) from the corner)
    use super::*;
    use crate::alignment::radius_check::min_radius_on_chord;

    #[test]
    fn a_right_angle_fillet_has_its_centre_at_r_r_and_reads_its_radius() {
        let f = fillet_between(
            [0.0, -1000.0],
            [0.0, 1.0],
            [1000.0, 0.0],
            [1.0, 0.0],
            300.0,
            5.0,
        )
        .unwrap();
        // Tangent points (0, -300) and (300, 0); every arc point 300 m from (300, -300).
        assert!(dist(f[1], [0.0, -300.0]) < 1e-9);
        assert!(dist(*f.iter().rev().nth(1).unwrap(), [300.0, 0.0]) < 1e-9);
        for p in &f[2..f.len() - 2] {
            assert!((dist(*p, [300.0, -300.0]) - 300.0).abs() < 1e-9);
        }
        let r = min_radius_on_chord(&f, 100.0).unwrap();
        assert!((r - 300.0).abs() < 1.0, "{r}");
    }

    #[test]
    fn a_fillet_too_big_for_its_arms_is_refused() {
        assert!(
            fillet_between(
                [0.0, -200.0],
                [0.0, 1.0],
                [1000.0, 0.0],
                [1.0, 0.0],
                300.0,
                5.0
            )
            .is_none()
        );
        assert!(
            fillet_between([0.0, 0.0], [1.0, 0.0], [100.0, 0.0], [1.0, 0.0], 300.0, 5.0).is_none()
        );
    }
}
