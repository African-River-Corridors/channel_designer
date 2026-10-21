//! GEOS `math::DD`: double-double arithmetic (Dekker split, no fused multiply-add), as far as
//! `CGAlgorithmsDD` uses it. Each step is written as GEOS writes it, so the rounding is the same.

use super::Pt;

/// A double-double: the value `hi + lo`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dd {
    hi: f64,
    lo: f64,
}

/// 2^27 + 1, for IEEE double (GEOS `DD::SPLIT`).
const SPLIT: f64 = 134_217_729.0;

impl Dd {
    pub fn new(x: f64) -> Self {
        Dd { hi: x, lo: 0.0 }
    }

    /// GEOS `DD::selfAdd(yhi, ylo)`.
    pub fn plus(self, y: Dd) -> Dd {
        let (hi, lo, yhi, ylo) = (self.hi, self.lo, y.hi, y.lo);
        let s_ = hi + yhi;
        let t_ = lo + ylo;
        let mut e = s_ - hi;
        let f = t_ - lo;
        let mut s = s_ - e;
        let mut t = t_ - f;
        s = (yhi - e) + (hi - s);
        t = (ylo - f) + (lo - t);
        e = s + t_;
        let h_ = s_ + e;
        let h = e + (s_ - h_);
        e = t + h;
        let zhi = h_ + e;
        let zlo = e + (h_ - zhi);
        Dd { hi: zhi, lo: zlo }
    }

    /// GEOS `DD::selfSubtract`: adds `-1 * y`.
    pub fn minus(self, y: Dd) -> Dd {
        self.plus(Dd {
            hi: -y.hi,
            lo: -y.lo,
        })
    }

    /// GEOS `DD::selfMultiply(yhi, ylo)`.
    pub fn times(self, y: Dd) -> Dd {
        let (hi, lo, yhi, ylo) = (self.hi, self.lo, y.hi, y.lo);
        let mut c_ = SPLIT * hi;
        let mut hx = c_ - hi;
        let mut c = SPLIT * yhi;
        hx = c_ - hx;
        let tx = hi - hx;
        let mut hy = c - yhi;
        c_ = hi * yhi;
        hy = c - hy;
        let ty = yhi - hy;
        c = ((((hx * hy - c_) + hx * ty) + tx * hy) + tx * ty) + (hi * ylo + lo * yhi);
        let zhi = c_ + c;
        hx = c_ - zhi;
        let zlo = c + hx;
        Dd { hi: zhi, lo: zlo }
    }

    /// GEOS `DD::selfDivide(yhi, ylo)`.
    pub fn over(self, y: Dd) -> Dd {
        let (hi, lo, yhi, ylo) = (self.hi, self.lo, y.hi, y.lo);
        let c_ = hi / yhi;
        let mut c = SPLIT * c_;
        let mut hc = c - c_;
        let mut u = SPLIT * yhi;
        hc = c - hc;
        let tc = c_ - hc;
        let mut hy = u - yhi;
        let u_ = c_ * yhi;
        hy = u - hy;
        let ty = yhi - hy;
        u = (((hc * hy - u_) + hc * ty) + tc * hy) + tc * ty;
        c = ((((hi - u_) - u) + lo) - c_ * ylo) / yhi;
        u = c_ + c;
        Dd {
            hi: u,
            lo: (c_ - u) + c,
        }
    }

    /// GEOS `DD::doubleValue`.
    pub fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    /// The sign: +1, -1 or 0 (GEOS `OrientationDD`: compares with zero, `hi` first).
    pub fn signum(self) -> i32 {
        if self.hi > 0.0 || (self.hi == 0.0 && self.lo > 0.0) {
            1
        } else if self.hi < 0.0 || (self.hi == 0.0 && self.lo < 0.0) {
            -1
        } else {
            0
        }
    }
}

/// GEOS `CGAlgorithmsDD::intersection`: where the lines through `p1-p2` and `q1-q2` cross, or
/// `None` if they are parallel (the quotient is not finite).
pub fn line_intersection(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> Option<Pt> {
    let (q1x, q1y, q2x, q2y) = (
        Dd::new(q1[0]),
        Dd::new(q1[1]),
        Dd::new(q2[0]),
        Dd::new(q2[1]),
    );
    let (p1x, p1y, p2x, p2y) = (
        Dd::new(p1[0]),
        Dd::new(p1[1]),
        Dd::new(p2[0]),
        Dd::new(p2[1]),
    );
    let px = p1y.minus(p2y);
    let py = p2x.minus(p1x);
    let pw = p1x.times(p2y).minus(p2x.times(p1y));
    let qx = q1y.minus(q2y);
    let qy = q2x.minus(q1x);
    let qw = q1x.times(q2y).minus(q2x.times(q1y));
    let x = py.times(qw).minus(qy.times(pw));
    let y = qx.times(pw).minus(px.times(qw));
    let w = px.times(qy).minus(qx.times(py));
    let xi = x.over(w).to_f64();
    let yi = y.over(w).to_f64();
    if xi.is_finite() && yi.is_finite() {
        Some([xi, yi])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn products_carry_the_rounding_error() {
        let a = Dd::new(1.0 + f64::EPSILON);
        let p = a.times(a);
        assert_eq!(p.hi, 1.0 + 2.0 * f64::EPSILON);
        assert_eq!(p.lo, f64::EPSILON * f64::EPSILON);
    }

    #[test]
    fn parallel_lines_have_no_crossing() {
        assert_eq!(
            line_intersection([0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]),
            None
        );
        assert_eq!(
            line_intersection([0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]),
            Some([1.0, 1.0])
        );
    }
}
