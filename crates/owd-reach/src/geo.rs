//! Small geodesy for routing: distances on the sphere and projection onto a segment in a local
//! plane. Positions are `[lon, lat]` in degrees (WGS84).

pub type LonLat = [f64; 2];

const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Great-circle distance in metres.
pub fn haversine_m(a: LonLat, b: LonLat) -> f64 {
    let (la1, la2) = (a[1].to_radians(), b[1].to_radians());
    let dlat = la2 - la1;
    let dlon = (b[0] - a[0]).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

/// Metres east and north of `origin`, on a plane good for a few kilometres.
pub fn to_local(origin: LonLat, p: LonLat) -> [f64; 2] {
    let k = EARTH_RADIUS_M.to_radians();
    [
        (p[0] - origin[0]) * k * origin[1].to_radians().cos(),
        (p[1] - origin[1]) * k,
    ]
}

/// Where `p` is nearest on the segment `a`..`b`.
#[derive(Debug, Clone, Copy)]
pub struct SegmentHit {
    /// 0 at `a`, 1 at `b`.
    pub t: f64,
    pub pos: LonLat,
    pub dist_m: f64,
}

pub fn nearest_on_segment(p: LonLat, a: LonLat, b: LonLat) -> SegmentHit {
    let la = to_local(p, a);
    let lb = to_local(p, b);
    let d = [lb[0] - la[0], lb[1] - la[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    // The point is the origin of the local plane.
    let t = if len2 == 0.0 {
        0.0
    } else {
        ((-la[0] * d[0] - la[1] * d[1]) / len2).clamp(0.0, 1.0)
    };
    let q = [la[0] + t * d[0], la[1] + t * d[1]];
    SegmentHit {
        t,
        pos: [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
        dist_m: (q[0] * q[0] + q[1] * q[1]).sqrt(),
    }
}

/// Length of a line in metres.
pub fn line_length_m(line: &[LonLat]) -> f64 {
    line.windows(2).map(|w| haversine_m(w[0], w[1])).sum()
}

/// Points along a line every `step_m` metres, with both ends kept.
pub fn resample(line: &[LonLat], step_m: f64) -> Vec<LonLat> {
    let Some(&first) = line.first() else {
        return Vec::new();
    };
    let mut out = vec![first];
    let mut carry = 0.0; // distance already covered towards the next sample
    for w in line.windows(2) {
        let seg = haversine_m(w[0], w[1]);
        let mut at = step_m - carry;
        while at < seg {
            let f = at / seg;
            out.push([
                w[0][0] + f * (w[1][0] - w[0][0]),
                w[0][1] + f * (w[1][1] - w[0][1]),
            ]);
            at += step_m;
        }
        carry = seg - (at - step_m);
    }
    let last = *line.last().expect("a line with a first point has a last");
    if out.last() != Some(&last) {
        out.push(last);
    }
    out
}

/// `[west, south, east, north]` of some points.
pub fn bbox(points: &[LonLat]) -> Option<[f64; 4]> {
    let mut it = points.iter();
    let f = it.next()?;
    let mut b = [f[0], f[1], f[0], f[1]];
    for p in it {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    //! Label: sourced - one degree of latitude is 6371008.8 m * pi / 180 = 111195 m (mean Earth radius, IUGG).
    use super::*;

    #[test]
    fn a_degree_of_latitude_is_111195_m() {
        assert!((haversine_m([0.0, 0.0], [0.0, 1.0]) - 111_195.0).abs() < 1.0);
    }

    #[test]
    fn the_nearest_point_is_clamped_to_the_segment() {
        let a = [0.0, 0.0];
        let b = [0.01, 0.0];
        let mid = nearest_on_segment([0.005, 0.001], a, b);
        assert!((mid.t - 0.5).abs() < 1e-6);
        assert!((mid.dist_m - 111.2).abs() < 0.2);
        assert_eq!(nearest_on_segment([-0.5, 0.0], a, b).t, 0.0);
        assert_eq!(nearest_on_segment([0.5, 0.0], a, b).t, 1.0);
    }

    #[test]
    fn resampling_keeps_both_ends_and_spaces_the_points_along_the_line() {
        let line = [[0.0, 0.0], [0.0, 0.01], [0.01, 0.01]]; // about 1112 m + 1108 m
        let s = resample(&line, 100.0);
        assert_eq!(s[0], line[0]);
        assert_eq!(*s.last().unwrap(), line[2]);
        // 2220 m at 100 m: samples at 0 to 2200 m, then the end. Straight stretches are spaced exactly.
        assert_eq!(s.len(), 24);
        assert!((haversine_m(s[3], s[4]) - 100.0).abs() < 0.5);
        assert!((haversine_m(s[18], s[19]) - 100.0).abs() < 0.5);
    }
}
