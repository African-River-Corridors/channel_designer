//! The centreline of a river from its two banks (spec 0013): their mid-line. One function, used by the
//! bank generator, the edge fit and the bank editor, so the three never disagree.
//!
//! It starts from a seed line. Then, three times: at stations along the line, cast the normal both
//! ways to the first crossing with each bank on its own side, and move the station to the midpoint of the two
//! crossings. A light smoothing goes between the passes, not after the last one, so the line stored is
//! a mid-line. A station whose normal meets one bank or neither keeps its place.
//!
//! The geometry is planar, in project metres. Square roots go through `libm`, so native and wasm give
//! the same bits.
//!
//! Label: property

type Pt = [f64; 2];

/// The passes of the mid-line.
pub const MIDLINE_PASSES: usize = 3;

/// The farthest a station looks for a bank along its normal, each way, in metres.
pub const MAX_RAY_M: f64 = 5_000.0;

fn dist(a: Pt, b: Pt) -> f64 {
    libm::hypot(b[0] - a[0], b[1] - a[1])
}

/// Arc length at each vertex.
pub fn arc(line: &[Pt]) -> Vec<f64> {
    let mut s = Vec::with_capacity(line.len());
    s.push(0.0);
    for w in line.windows(2) {
        s.push(s[s.len() - 1] + dist(w[0], w[1]));
    }
    s
}

/// The point at arc length `at` (clamped), given the arc lengths `s` of `line`.
pub fn point_at(line: &[Pt], s: &[f64], at: f64) -> Pt {
    if line.len() == 1 {
        return line[0];
    }
    let at = at.clamp(0.0, s[s.len() - 1]);
    let i = s
        .partition_point(|v| *v <= at)
        .saturating_sub(1)
        .min(line.len() - 2);
    let seg = s[i + 1] - s[i];
    let f = if seg > 0.0 { (at - s[i]) / seg } else { 0.0 };
    [
        line[i][0] + f * (line[i + 1][0] - line[i][0]),
        line[i][1] + f * (line[i + 1][1] - line[i][1]),
    ]
}

/// Evenly spaced stations along a line, about `step` apart, both ends included.
pub fn resample(line: &[Pt], step: f64) -> Vec<Pt> {
    if line.len() < 2 {
        return line.to_vec();
    }
    let s = arc(line);
    let len = s[s.len() - 1];
    let n = ((len / step.max(1e-6)).round() as usize).max(1);
    (0..=n)
        .map(|k| point_at(line, &s, len * k as f64 / n as f64))
        .collect()
}

/// The half-length of line, each side of a station, whose direction gives the station's normal. Long
/// enough that the normals of a line still being moved, or of rough banks, do not swing.
pub const TANGENT_HALF_M: f64 = 100.0;

/// Unit left normals of a line, from the chord between the points `half_m` of arc either side.
pub fn arc_normals(line: &[Pt], half_m: f64) -> Vec<Pt> {
    let s = arc(line);
    line.iter()
        .zip(&s)
        .map(|(_, at)| {
            let (a, b) = (
                point_at(line, &s, at - half_m),
                point_at(line, &s, at + half_m),
            );
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = libm::hypot(dx, dy).max(1e-12);
            [-dy / l, dx / l]
        })
        .collect()
}

/// Unit left normals of a line, from the direction over `k` stations either side.
pub fn normals(line: &[Pt], k: usize) -> Vec<Pt> {
    let n = line.len();
    (0..n)
        .map(|i| {
            let (a, b) = (line[i.saturating_sub(k)], line[(i + k).min(n - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = libm::hypot(dx, dy).max(1e-12);
            [-dy / l, dx / l]
        })
        .collect()
}

/// A grid of the segments of a line, so a ray tests only the segments near it.
pub struct SegIndex<'a> {
    line: &'a [Pt],
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl<'a> SegIndex<'a> {
    pub fn new(line: &'a [Pt], cell: f64) -> SegIndex<'a> {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in line {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if line.is_empty() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let nx = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let ny = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut cells = vec![Vec::new(); nx * ny];
        for (i, w) in line.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let cx = |x: f64| (((x - lo[0]) / cell).floor() as usize).min(nx - 1);
            let cy = |y: f64| (((y - lo[1]) / cell).floor() as usize).min(ny - 1);
            for gy in cy(a[1].min(b[1]))..=cy(a[1].max(b[1])) {
                for gx in cx(a[0].min(b[0]))..=cx(a[0].max(b[0])) {
                    cells[gy * nx + gx].push(i as u32);
                }
            }
        }
        SegIndex {
            line,
            x0: lo[0],
            y0: lo[1],
            cell,
            nx,
            ny,
            cells,
        }
    }

    /// The segment ids in the cells a ray from `p` along `d` passes, up to `max_t` each way.
    fn candidates(&self, p: Pt, d: Pt, max_t: f64, out: &mut Vec<u32>) {
        out.clear();
        let steps = (max_t / (self.cell * 0.5)).ceil() as i64;
        let mut last = usize::MAX;
        for k in -steps..=steps {
            let t = k as f64 * self.cell * 0.5;
            let (x, y) = (p[0] + d[0] * t, p[1] + d[1] * t);
            let (fx, fy) = ((x - self.x0) / self.cell, (y - self.y0) / self.cell);
            // The cell and its neighbours: a ray between two sample points can clip a corner.
            for (ox, oy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (gx, gy) = (fx.floor() as i64 + ox, fy.floor() as i64 + oy);
                if gx < 0 || gy < 0 || gx >= self.nx as i64 || gy >= self.ny as i64 {
                    continue;
                }
                let c = gy as usize * self.nx + gx as usize;
                if c == last && (ox, oy) == (0, 0) {
                    continue;
                }
                out.extend_from_slice(&self.cells[c]);
            }
            last = (fy.floor().max(0.0) as usize) * self.nx + fx.floor().max(0.0) as usize;
        }
        out.sort_unstable();
        out.dedup();
    }

    /// The signed distances along `d` (a unit vector) from `p` to the first crossing with the line
    /// ahead (positive) and behind (negative), within `max_t` either way.
    pub fn hits(&self, p: Pt, d: Pt, max_t: f64, buf: &mut Vec<u32>) -> (Option<f64>, Option<f64>) {
        self.candidates(p, d, max_t, buf);
        let (mut ahead, mut behind): (Option<f64>, Option<f64>) = (None, None);
        for &i in buf.iter() {
            let (a, b) = (self.line[i as usize], self.line[i as usize + 1]);
            let e = [b[0] - a[0], b[1] - a[1]];
            let den = d[0] * e[1] - d[1] * e[0];
            if den.abs() < 1e-12 {
                continue;
            }
            let w = [a[0] - p[0], a[1] - p[1]];
            let t = (w[0] * e[1] - w[1] * e[0]) / den;
            let u = (w[0] * d[1] - w[1] * d[0]) / den;
            if !(-1e-9..=1.0 + 1e-9).contains(&u) || t.abs() > max_t {
                continue;
            }
            if t >= 0.0 {
                if ahead.is_none_or(|v| t < v) {
                    ahead = Some(t);
                }
            } else if behind.is_none_or(|v| t > v) {
                behind = Some(t);
            }
        }
        (ahead, behind)
    }

    /// The signed distance to the nearest crossing either way.
    pub fn nearest_hit(&self, p: Pt, d: Pt, max_t: f64, buf: &mut Vec<u32>) -> Option<f64> {
        match self.hits(p, d, max_t, buf) {
            (Some(a), Some(b)) => Some(if a <= -b { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}

/// The offsets of the two banks from a station along its left normal `n`: the first crossing of the
/// left bank on the left, and of the right bank on the right. A bank met only on the wrong side gives
/// its nearest crossing there (a negative left or a positive right offset).
fn side_hits(
    li: &SegIndex,
    ri: &SegIndex,
    p: Pt,
    n: Pt,
    buf: &mut Vec<u32>,
) -> (Option<f64>, Option<f64>) {
    let (la, lb) = li.hits(p, n, MAX_RAY_M, buf);
    let (ra, rb) = ri.hits(p, n, MAX_RAY_M, buf);
    (la.or(lb), rb.or(ra))
}

/// Per station of `centre`: the signed distances along its left normal to the first crossing of the
/// left bank on the left and of the right bank on the right (see `side_hits`; `None` where the normal
/// meets the bank on neither side within [`MAX_RAY_M`]).
pub fn bank_offsets(centre: &[Pt], left: &[Pt], right: &[Pt]) -> Vec<(Option<f64>, Option<f64>)> {
    let cell = cell_for(left, right);
    let (li, ri) = (SegIndex::new(left, cell), SegIndex::new(right, cell));
    let nrm = arc_normals(centre, tangent_half(centre, &li, &ri));
    let mut buf = Vec::new();
    centre
        .iter()
        .zip(&nrm)
        .map(|(p, n)| side_hits(&li, &ri, *p, *n, &mut buf))
        .collect()
}

/// How far each station of `centre` is from the midpoint of the banks along its normal: half the
/// sum of the two offsets (`None` where a bank is not met).
pub fn midline_offsets(centre: &[Pt], left: &[Pt], right: &[Pt]) -> Vec<Option<f64>> {
    bank_offsets(centre, left, right)
        .into_iter()
        .map(|o| match o {
            (Some(l), Some(r)) => Some(((l + r) / 2.0).abs()),
            _ => None,
        })
        .collect()
}

/// A grid cell about the size of a typical segment, at least 25 m.
fn cell_for(left: &[Pt], right: &[Pt]) -> f64 {
    let (a, b) = (arc(left), arc(right));
    let len = a.last().copied().unwrap_or(0.0) + b.last().copied().unwrap_or(0.0);
    let n = (left.len() + right.len()).max(2) as f64;
    (4.0 * len / n).clamp(25.0, 500.0)
}

/// The mid-line of two banks, at stations about `station_m` apart, from a seed line (for example the
/// last centreline, or the two banks paired by their share of length).
pub fn midline(left: &[Pt], right: &[Pt], seed: &[Pt], station_m: f64) -> Vec<Pt> {
    if left.len() < 2 || right.len() < 2 || seed.len() < 2 {
        return seed.to_vec();
    }
    let cell = cell_for(left, right);
    let (li, ri) = (SegIndex::new(left, cell), SegIndex::new(right, cell));
    let mut line = resample(seed, station_m);
    let mut buf = Vec::new();
    // Normals over half the river's width at least, so a wide river does not swing them.
    let half = tangent_half(&line, &li, &ri);
    for pass in 0..MIDLINE_PASSES {
        let nrm = arc_normals(&line, half);
        line = line
            .iter()
            .zip(&nrm)
            .map(|(p, n)| match side_hits(&li, &ri, *p, *n, &mut buf) {
                (Some(a), Some(b)) => {
                    let t = (a + b) / 2.0;
                    [p[0] + n[0] * t, p[1] + n[1] * t]
                }
                _ => *p,
            })
            .collect();
        if pass + 1 < MIDLINE_PASSES {
            line = resample(&smooth(&line), station_m);
        }
    }
    line
}

/// The median width across a line's stations, by the first crossings either side (`None` with none).
pub fn median_width(line: &[Pt], left: &[Pt], right: &[Pt]) -> Option<f64> {
    let cell = cell_for(left, right);
    let (li, ri) = (SegIndex::new(left, cell), SegIndex::new(right, cell));
    median_width_on(line, &li, &ri)
}

fn median_width_on(line: &[Pt], li: &SegIndex, ri: &SegIndex) -> Option<f64> {
    let nrm = arc_normals(line, TANGENT_HALF_M);
    let mut buf = Vec::new();
    let mut w: Vec<f64> = line
        .iter()
        .zip(&nrm)
        .filter_map(|(p, n)| match side_hits(li, ri, *p, *n, &mut buf) {
            (Some(l), Some(r)) if l > r => Some(l - r),
            _ => None,
        })
        .collect();
    if w.is_empty() {
        return None;
    }
    w.sort_by(f64::total_cmp);
    Some(w[w.len() / 2])
}

/// The half-length of line that gives a normal: [`TANGENT_HALF_M`], or half the median width if
/// that is more, up to 1 km.
fn tangent_half(line: &[Pt], li: &SegIndex, ri: &SegIndex) -> f64 {
    median_width_on(line, li, ri)
        .map_or(TANGENT_HALF_M, |w| (w / 2.0).clamp(TANGENT_HALF_M, 1_000.0))
}

/// One pass of a 1-2-1 smoothing; the ends stay.
fn smooth(line: &[Pt]) -> Vec<Pt> {
    let n = line.len();
    (0..n)
        .map(|i| {
            if i == 0 || i + 1 == n {
                line[i]
            } else {
                [
                    (line[i - 1][0] + 2.0 * line[i][0] + line[i + 1][0]) / 4.0,
                    (line[i - 1][1] + 2.0 * line[i][1] + line[i + 1][1]) / 4.0,
                ]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wavy(off: f64, n: usize) -> Vec<Pt> {
        (0..=n)
            .map(|i| {
                let x = i as f64 * 10.0;
                [x, off + 30.0 * libm::sin(x / 400.0)]
            })
            .collect()
    }

    #[test]
    fn the_mid_line_of_smooth_banks_is_their_middle_within_a_metre() {
        // A meandering channel 100 m wide (in y), and a seed 40 m off to one side.
        let (left, right) = (wavy(100.0, 300), wavy(0.0, 300));
        let seed: Vec<Pt> = (0..=300).map(|i| [i as f64 * 10.0, 90.0]).collect();
        let m = midline(&left, &right, &seed, 20.0);
        let off = midline_offsets(&m, &left, &right);
        // Away from the ends, where the normal can miss a bank, every station is on the mid-line.
        for (i, o) in off.iter().enumerate().skip(3).take(off.len() - 6) {
            let o = o.unwrap_or_else(|| panic!("station {i} met no bank"));
            assert!(o < 1.0, "station {i} is {o} m off the mid-line");
        }
        // And between the banks.
        for (p, (l, r)) in m.iter().zip(bank_offsets(&m, &left, &right)) {
            if let (Some(l), Some(r)) = (l, r) {
                assert!(l > 0.0 && r < 0.0, "{p:?}: {l} {r}");
            }
        }
    }

    #[test]
    fn a_mid_line_lies_between_banks_of_any_shape_that_do_not_cross() {
        // Property over random smooth banks: the mid-line stays between them.
        let mut x: u64 = 11;
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) as f64) / (1u64 << 31) as f64
        };
        for _ in 0..40 {
            let (a, ph, w) = (next() * 60.0, next() * 6.0, 60.0 + next() * 300.0);
            let bank = |off: f64, k: f64| -> Vec<Pt> {
                (0..=200)
                    .map(|i| {
                        let x = i as f64 * 15.0;
                        [
                            x,
                            off + a * libm::sin(x / 500.0 + ph) + k * libm::sin(x / 90.0),
                        ]
                    })
                    .collect()
            };
            let (left, right) = (bank(w, 3.0), bank(0.0, -2.0));
            let seed: Vec<Pt> = (0..=200).map(|i| [i as f64 * 15.0, w * 0.2]).collect();
            let m = midline(&left, &right, &seed, 20.0);
            for (l, r) in bank_offsets(&m, &left, &right).into_iter().flatten_pairs() {
                assert!(l > 0.0 && r < 0.0, "width {w}: {l} {r}");
            }
        }
    }

    trait FlattenPairs {
        fn flatten_pairs(self) -> Vec<(f64, f64)>;
    }
    impl<I: Iterator<Item = (Option<f64>, Option<f64>)>> FlattenPairs for I {
        fn flatten_pairs(self) -> Vec<(f64, f64)> {
            self.filter_map(|(a, b)| Some((a?, b?))).collect()
        }
    }

    #[test]
    fn a_normal_that_meets_one_bank_keeps_its_station() {
        let left: Vec<Pt> = (0..=10).map(|i| [i as f64 * 10.0, 50.0]).collect();
        let right: Vec<Pt> = (0..=10).map(|i| [i as f64 * 10.0, 0.0]).collect();
        // A seed that runs on past the banks' end.
        let seed: Vec<Pt> = vec![[0.0, 25.0], [300.0, 25.0]];
        let m = midline(&left, &right, &seed, 20.0);
        assert_eq!(*m.last().unwrap(), [300.0, 25.0]);
        assert!((m[2][1] - 25.0).abs() < 1e-9);
    }
}
