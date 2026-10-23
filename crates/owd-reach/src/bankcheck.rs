//! Checks and repairs on a pair of bank lines, whatever made them (spec 0013, ticket T1).
//!
//! The banks are read at stations along a reference line: the seed centreline (the route, or the last
//! centreline), smoothed over about one river width so that one inlet cannot drag it. Along the reference's left normal, each station
//! has an offset to the left bank and one to the right bank. The checks, in order:
//!
//! 1. **Own side.** The left bank lies left of the right bank along the normal; where it does not,
//!    one bank has crossed to the other shore, and both are repaired there. A normal that meets no
//!    bank is `missing`.
//! 2. **Lateral jump.** Per bank, the offset may move outward by at most `jump_factor` station steps
//!    from one station to the next. A station past the limit starts a run of bad stations that ends
//!    where the bank comes back within the limit of the offset before the run. Only outward jumps are
//!    cut (inlets, tributary mouths, spikes onto land): a jump inward narrows the river, and narrowing
//!    is left to the width check, which flags it and never widens it. A run longer than
//!    `max_run_m`, or one that touches either end of the banks, is a real change of the river, not an
//!    excursion: it is kept.
//! 3. **Excursion.** Per bank, an offset outward of the running median of the bank's own offsets
//!    over 250 m either side by more than a quarter of the local median width, and at least 30 m, is
//!    cut: a bay, a tributary mouth, or a cut across an island into another
//!    channel, rising too gradually for the jump limit.
//! 4. **Width.** The width must lie between `width_low` and `width_high` times the running median of
//!    the good widths within `width_window_m`. A wide station is repaired on the bank that left its
//!    own running median the most. A narrow one is flagged and kept: a real throat must never be
//!    widened away (and where a `narrow` stretch already covers it, it adds nothing).
//!
//! A repair replaces the offset by a straight interpolation between the nearest good stations of that
//! bank. The banks are rebuilt from the offsets, loops are cut out, and the centreline is rebuilt as
//! the mid-line of the final banks. Every flagged station joins a low-confidence stretch of kind
//! `repaired`, which says which checks failed. A crossing of the two banks left after the repairs is
//! flagged `crossing` and named by the commands: loud, never silent, but it does not stop a whole
//! reach for one bad place.
//!
//! Defaults (provisional until calibrated against a surveyed reach): jump limit 2 station steps per
//! station, width 0.5 to 2 times the running median over 500 m either side, reference smoothed over
//! one median river width either side, runs longer than 1 km kept.

use crate::banks::{Stretch, StretchKind};
use crate::error::ReachError;
use owd_format::design::midline::{
    SegIndex, TANGENT_HALF_M, arc_normals, bank_offsets, median_width, midline, resample,
};

type P2 = [f64; 2];

/// The settings of the checks. Every value here is a provisional default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckParams {
    /// The distance between stations, in metres.
    pub station_m: f64,
    /// The largest change of a bank's offset from one station to the next, in station steps.
    pub jump_factor: f64,
    /// A run of jumped stations longer than this, or one that runs to the end of the banks, is a real
    /// change of the river and is kept.
    pub max_run_m: f64,
    /// The half-length of the running median of the width, in metres.
    pub width_window_m: f64,
    pub width_low: f64,
    pub width_high: f64,
    /// An excursion stands outward of the running median by more than this share of the median width,
    /// and at least `excursion_min_m`.
    pub excursion_share: f64,
    pub excursion_min_m: f64,
    /// The half-length of the running median an excursion stands out of, in metres.
    pub excursion_window_m: f64,
    /// The most a repaired bank's offset may change from one station to the next, in metres (the edge
    /// fit's step limit); a repair too short to keep to it is widened until it does. `None`: no limit.
    pub max_step_m: Option<f64>,
    /// The half-length of the moving average that makes the reference line, in median river widths
    /// (at least two stations, at most 2 km).
    pub reference_widths: f64,
    /// The half-length used when no width can be measured, in metres.
    pub reference_half_m: f64,
}

impl CheckParams {
    /// The defaults at a station step.
    pub fn at_step(station_m: f64) -> CheckParams {
        CheckParams {
            station_m,
            jump_factor: 2.0,
            max_run_m: 1_000.0,
            width_window_m: 500.0,
            width_low: 0.5,
            width_high: 2.0,
            excursion_share: 0.25,
            excursion_min_m: 30.0,
            excursion_window_m: 250.0,
            max_step_m: None,
            reference_widths: 1.0,
            reference_half_m: 250.0,
        }
    }

    /// The largest change of offset between stations, in metres.
    pub fn max_jump_m(&self) -> f64 {
        self.jump_factor * self.station_m
    }
}

/// Which checks a station failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Checks {
    /// A bank on the wrong side of the river.
    pub side: bool,
    /// The normal met no bank.
    pub missing: bool,
    /// A bank jumped outward past the limit (a spike, an inlet, a tributary mouth): repaired.
    pub jump: bool,
    /// Wider than its neighbours allow: repaired.
    pub width_wide: bool,
    /// Narrower than its neighbours: flagged, the banks kept.
    pub width_narrow: bool,
    /// A bank standing outward of its neighbours over a short run (a bay, a tributary mouth, a cut
    /// across an island into another channel), rising too gradually to be a jump: repaired.
    pub excursion: bool,
    /// The banks still cross here after every round of repairs: not repaired, and said loudly.
    pub crossing: bool,
}

impl Checks {
    pub fn any(self) -> bool {
        self.side
            || self.missing
            || self.jump
            || self.width_wide
            || self.width_narrow
            || self.excursion
            || self.crossing
    }

    fn or(self, o: Checks) -> Checks {
        Checks {
            side: self.side || o.side,
            missing: self.missing || o.missing,
            jump: self.jump || o.jump,
            width_wide: self.width_wide || o.width_wide,
            width_narrow: self.width_narrow || o.width_narrow,
            excursion: self.excursion || o.excursion,
            crossing: self.crossing || o.crossing,
        }
    }

    /// The names, as the document writes them.
    pub fn names(self) -> Vec<&'static str> {
        let mut v = Vec::new();
        for (on, name) in [
            (self.side, "side"),
            (self.missing, "missing"),
            (self.jump, "jump"),
            (self.width_wide, "width-wide"),
            (self.width_narrow, "width-narrow"),
            (self.excursion, "excursion"),
            (self.crossing, "crossing"),
        ] {
            if on {
                v.push(name);
            }
        }
        v
    }
}

/// The banks after the checks.
#[derive(Debug, Clone)]
pub struct Checked {
    pub left: Vec<P2>,
    pub right: Vec<P2>,
    /// The mid-line of `left` and `right`.
    pub centreline: Vec<P2>,
    /// Width at each station of the centreline, in metres (0 where a bank is not met).
    pub widths: Vec<f64>,
    /// The flagged stretches, by chainage along `centreline`.
    pub stretches: Vec<Stretch>,
    /// Stations of the reference line that failed a check, with the checks.
    pub failed: Vec<(f64, Checks)>,
}

/// A moving average over `k` stations either side, the window shrunk evenly near the ends, so the end
/// points stay where they are and the line keeps its length.
fn moving_average(pts: &[P2], k: usize) -> Vec<P2> {
    let n = pts.len();
    let mut pre = vec![[0.0, 0.0]; n + 1];
    for i in 0..n {
        pre[i + 1] = [pre[i][0] + pts[i][0], pre[i][1] + pts[i][1]];
    }
    (0..n)
        .map(|i| {
            let h = k.min(i).min(n - 1 - i);
            let (a, b) = (i - h, i + h);
            let m = (b - a + 1) as f64;
            [
                (pre[b + 1][0] - pre[a][0]) / m,
                (pre[b + 1][1] - pre[a][1]) / m,
            ]
        })
        .collect()
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    })
}

/// Stations of one bank that jumped outward, away from the river (`None` is already bad). `out` is
/// the sign of outward: +1 for the left bank, -1 for the right. A jump inward narrows the river: it
/// is never cut here (the width check flags it), so a real throat is never widened away.
fn jumps(off: &[Option<f64>], out: f64, p: &CheckParams) -> Vec<bool> {
    let lim = p.max_jump_m();
    let max_run = (p.max_run_m / p.station_m).ceil() as usize;
    let mut bad = vec![false; off.len()];
    // Start from the first station that agrees with the running median of its neighbours, so an
    // outlier at the start (a hooked bank end) does not make everything after it look like a jump.
    // The stations before it are not judged: like a run that reaches the end, there is nothing
    // beyond them to judge them against.
    let kw = (p.width_window_m / p.station_m).round() as usize;
    let med = |i: usize| {
        let (a, b) = (i.saturating_sub(kw), (i + kw).min(off.len() - 1));
        let mut v: Vec<f64> = off[a..=b].iter().flatten().copied().collect();
        median(&mut v)
    };
    let Some(start) = (0..off.len())
        .find(|i| off[*i].is_some_and(|o| med(*i).is_some_and(|m| (o - m).abs() <= lim)))
    else {
        return bad;
    };
    let mut last_good: Option<(usize, f64)> = None;
    let mut i = start;
    while i < off.len() {
        let Some(o) = off[i] else {
            i += 1;
            continue;
        };
        match last_good {
            Some((a, g)) if out * (o - g) > lim * (i - a) as f64 => {
                // A run: until a station comes back within the limit of the offset before it.
                let mut j = i;
                while j < off.len() && off[j].is_none_or(|v| out * (v - g) > lim) {
                    j += 1;
                }
                if j - i > max_run || j == off.len() {
                    // A real change of the river: keep it, from its first station.
                    last_good = Some((i, o));
                    i += 1;
                    continue;
                }
                for b in bad.iter_mut().take(j).skip(i) {
                    *b = true;
                }
                i = j;
            }
            _ => {
                last_good = Some((i, o));
                i += 1;
            }
        }
    }
    bad
}

/// Widen each run of bad stations of a bank until the straight line between the good stations either
/// side changes by at most `max_step` per station (spec 0013, T4: the repairs keep to the edge fit's
/// step limit). A run that reaches an end cannot be widened that way and is left.
fn widen_runs(off: &[Option<f64>], bad: &mut [bool], max_step: f64) {
    let n = bad.len();
    let good = |i: usize, bad: &[bool]| !bad[i] && off[i].is_some();
    let mut i = 0;
    while i < n {
        if good(i, bad) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && !good(j, bad) {
            j += 1;
        }
        // The run is i..j; its good neighbours are i - 1 and j.
        let (mut a, mut b) = (i, j);
        let mut grow_left = true;
        while a > 0 && b < n {
            let (va, vb) = (off[a - 1].unwrap_or(0.0), off[b].unwrap_or(0.0));
            if (vb - va).abs() <= max_step * (b - a + 1) as f64 {
                break;
            }
            // Take in the next good station on one side, then the other.
            if grow_left {
                a -= 1;
                while a > 0 && !good(a - 1, bad) {
                    a -= 1;
                }
            } else {
                b += 1;
                while b < n && !good(b, bad) {
                    b += 1;
                }
            }
            grow_left = !grow_left;
        }
        for k in bad.iter_mut().take(b).skip(a) {
            *k = true;
        }
        i = b.max(j);
    }
}

/// Fill the bad offsets of a bank by straight interpolation between its nearest good stations (the
/// ends hold the nearest good value). `None` when no station is good.
fn fill(off: &[Option<f64>], bad: &[bool]) -> Option<Vec<f64>> {
    let good: Vec<usize> = (0..off.len())
        .filter(|i| !bad[*i] && off[*i].is_some())
        .collect();
    let (&first, &last) = (good.first()?, good.last()?);
    let mut out = vec![0.0; off.len()];
    let mut k = 0;
    for (i, o) in out.iter_mut().enumerate() {
        if i <= first {
            *o = off[first]?;
        } else if i >= last {
            *o = off[last]?;
        } else {
            while good[k + 1] < i {
                k += 1;
            }
            let (a, b) = (good[k], good[k + 1]);
            let (va, vb) = (off[a]?, off[b]?);
            *o = if i == a {
                va
            } else {
                va + (vb - va) * (i - a) as f64 / (b - a) as f64
            };
        }
    }
    Some(out)
}

fn seg_cross(a: P2, b: P2, c: P2, d: P2) -> bool {
    let o = |p: P2, q: P2, r: P2| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// The longest loop cut from a centreline, in stations: 2000, 10 km at the 5 m step. The worst
/// tangle of the whole Sassandra run was 6.7 km (issue #373).
pub const CENTRELINE_LOOP_STATIONS: usize = 2000;

/// Cut the loops out of a line: where segment `i` crosses a later segment `j`, the vertices between
/// them go and the crossing point joins the two. Only crossings within `window` segments are sought
/// (an offset line loops locally, at a bend tighter than its offset).
pub fn cut_loops(line: &[P2], window: usize) -> Vec<P2> {
    let mut out: Vec<P2> = Vec::with_capacity(line.len());
    let mut i = 0;
    while i < line.len() {
        out.push(line[i]);
        if i + 1 >= line.len() {
            break;
        }
        let (a, b) = (line[i], line[i + 1]);
        let mut jumped = false;
        for j in (i + 2..(i + window).min(line.len() - 1)).rev() {
            let (c, d) = (line[j], line[j + 1]);
            if seg_cross(a, b, c, d) {
                let e = [b[0] - a[0], b[1] - a[1]];
                let f = [d[0] - c[0], d[1] - c[1]];
                let den = e[0] * f[1] - e[1] * f[0];
                let t = ((c[0] - a[0]) * f[1] - (c[1] - a[1]) * f[0]) / den;
                out.push([a[0] + t * e[0], a[1] + t * e[1]]);
                i = j + 1;
                jumped = true;
                break;
            }
        }
        if !jumped {
            i += 1;
        }
    }
    out
}

/// Replace each run of bad stations of a bank by the repaired points, between the bank's own points
/// at the good stations either side; the rest of the bank keeps its vertices. `raw` are the offsets as
/// measured, `fixed` after the repair.
fn splice_runs(
    orig: &[P2],
    reference: &[P2],
    nrm: &[P2],
    raw: &[Option<f64>],
    fixed: &[f64],
    bad: &[bool],
) -> Vec<P2> {
    let at = |i: usize, o: f64| {
        [
            reference[i][0] + nrm[i][0] * o,
            reference[i][1] + nrm[i][1] * o,
        ]
    };
    // The nearest vertex to `p` at or after `start`, among the next few thousand: the walk goes along
    // the bank, so a search from where the last splice ended keeps it linear, and a bank that passes
    // near itself further on cannot be matched there.
    let nearest = |p: P2, start: usize| {
        let end = (start + 4_000).min(orig.len());
        (start.min(end.saturating_sub(1))..end)
            .min_by(|a, b| {
                let da = (orig[*a][0] - p[0]).powi(2) + (orig[*a][1] - p[1]).powi(2);
                let db = (orig[*b][0] - p[0]).powi(2) + (orig[*b][1] - p[1]).powi(2);
                da.total_cmp(&db)
            })
            .unwrap_or(start)
    };
    let n = bad.len();
    let mut out: Vec<P2> = Vec::with_capacity(orig.len());
    let mut from = 0; // next vertex of `orig` not yet copied
    let mut i = 0;
    while i < n {
        if !bad[i] {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && bad[j] {
            j += 1;
        }
        // Good stations either side of the run i..j, and their points on the original bank.
        let ia = if i == 0 {
            from
        } else {
            nearest(at(i - 1, raw[i - 1].unwrap_or(fixed[i - 1])), from)
        };
        let ib = if j == n {
            orig.len() - 1
        } else {
            nearest(at(j, raw[j].unwrap_or(fixed[j])), ia)
        };
        if ib < ia {
            // The bank does not run with the reference here: leave this run as it is. Its stations
            // are flagged all the same.
            i = j;
            continue;
        }
        out.extend_from_slice(&orig[from..=ia]);
        out.extend((i..j).map(|k| at(k, fixed[k])));
        from = if j == n { orig.len() } else { ib };
        i = j;
    }
    if from < orig.len() {
        out.extend_from_slice(&orig[from..]);
    }
    // A loop of an offset line at a tight bend can span a few hundred stations.
    cut_loops(&out, 512)
}

/// The part of a bank that lies alongside the centreline: leading and trailing vertices that lie past
/// the centreline's first or last point, along its direction there, by more than the river's width at
/// that end (at least 100 m), are dropped. A bank may follow a shore a little past the end (a lagoon,
/// a mouth); one that runs off farther is a cast that found other water. At least two are kept.
fn trim_to_centreline(bank: &[P2], centre: &[P2], ends: (f64, f64)) -> Vec<P2> {
    if bank.len() < 3 || centre.len() < 2 {
        return bank.to_vec();
    }
    let cs = owd_format::design::midline::arc(centre);
    let len = cs[cs.len() - 1];
    let beyond = |p: P2, end: P2, next: P2| {
        // Past the end along the centreline's direction there.
        let d = [end[0] - next[0], end[1] - next[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
        ((p[0] - end[0]) * d[0] + (p[1] - end[1]) * d[1]) / l
    };
    let (c0, c1) = (centre[0], centre[1]);
    let (z0, z1) = (centre[centre.len() - 1], centre[centre.len() - 2]);
    let _ = len;
    let (lim0, lim1) = (ends.0.max(100.0), ends.1.max(100.0));
    let mut a = 0;
    while a + 2 < bank.len() && beyond(bank[a], c0, c1) > lim0 {
        a += 1;
    }
    let mut b = bank.len() - 1;
    while b > a + 1 && beyond(bank[b], z0, z1) > lim1 {
        b -= 1;
    }
    bank[a..=b].to_vec()
}

/// Whether two lines cross each other.
pub fn lines_cross(a: &[P2], b: &[P2]) -> Option<P2> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let idx = SegIndex::new(b, 50.0);
    let mut buf = Vec::new();
    for w in a.windows(2) {
        let len = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if len == 0.0 {
            continue;
        }
        let d = [(w[1][0] - w[0][0]) / len, (w[1][1] - w[0][1]) / len];
        // A crossing is a hit along the segment's own direction within its length.
        let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
        if let Some(t) = idx.nearest_hit(mid, d, len / 2.0, &mut buf) {
            return Some([mid[0] + d[0] * t, mid[1] + d[1] * t]);
        }
    }
    None
}

type Assessed = (
    Vec<P2>,
    Vec<(Option<f64>, Option<f64>)>,
    Vec<Checks>,
    Vec<bool>,
    Vec<bool>,
);

/// Check the banks against the reference made from `seed` (any rough centreline): per station of the
/// reference, its chainage, the two offsets and the checks failed. Nothing is changed.
fn assess(left: &[P2], right: &[P2], seed: &[P2], p: &CheckParams) -> Assessed {
    // The reference comes from the seed, not from the mid-line of the banks under test: banks that
    // cross back and forth would give a mid-line that does too.
    let m0 = resample(seed, p.station_m);
    // Smoothed over about one river width: enough that one inlet cannot drag it and that it does not
    // curl in a wide river, little enough that it stays on the water of a narrow, winding one.
    let half = median_width(&m0, left, right).map_or(p.reference_half_m, |w| {
        (w * p.reference_widths).clamp(2.0 * p.station_m, 2_000.0)
    });
    let k = (half / p.station_m).round() as usize;
    let reference = resample(&moving_average(&m0, k), p.station_m);
    let off = bank_offsets(&reference, left, right);
    let n = reference.len();
    let mut checks = vec![Checks::default(); n];
    let (mut bad_l, mut bad_r) = (vec![false; n], vec![false; n]);
    // 1. Own side, and a normal that meets no bank.
    let mut lo: Vec<Option<f64>> = Vec::with_capacity(n);
    let mut ro: Vec<Option<f64>> = Vec::with_capacity(n);
    // A normal past the end of a bank meets nothing: that is where the bank ends, not a fault. Only
    // stations between the first and the last that meet both banks are checked.
    let both = |i: usize| off[i].0.is_some() && off[i].1.is_some();
    let first = (0..n).find(|i| both(*i)).unwrap_or(n);
    let last = (0..n).rev().find(|i| both(*i)).unwrap_or(0);
    // The banks are out of order where the left one lies at or right of the right one along the
    // normal: one has crossed to the other shore. A reference that strays off a narrow river puts
    // both banks on one side of it, in order: that is not a fault, and repairing it would widen the
    // river.
    for (i, (l, r)) in off.iter().enumerate() {
        if i < first || i > last {
            lo.push(*l);
            ro.push(*r);
            continue;
        }
        if l.is_none() || r.is_none() {
            checks[i].missing = true;
        }
        let swapped = matches!((l, r), (Some(a), Some(b)) if a <= b);
        if swapped {
            checks[i].side = true;
        }
        bad_l[i] = l.is_none() || swapped;
        bad_r[i] = r.is_none() || swapped;
        lo.push(l.filter(|_| !swapped));
        ro.push(r.filter(|_| !swapped));
    }
    // 2. Lateral jumps, per bank.
    for (o, bad, out) in [(&mut lo, &mut bad_l, 1.0), (&mut ro, &mut bad_r, -1.0)] {
        let j = jumps(o, out, p);
        for i in 0..n {
            if j[i] {
                checks[i].jump = true;
                bad[i] = true;
                o[i] = None;
            }
        }
    }
    // 2b. Excursions: a bank outward of the running median of its own offsets (over
    // `excursion_window_m` either side) by more than a share of the local median width.
    let ww: Vec<Option<f64>> = (0..n).map(|i| Some(lo[i]? - ro[i]?)).collect();
    let kw = (p.width_window_m / p.station_m).round() as usize;
    let local_width: Vec<Option<f64>> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(kw), (i + kw).min(n - 1));
            let mut v: Vec<f64> = ww[a..=b].iter().flatten().copied().collect();
            median(&mut v)
        })
        .collect();
    let ke = (p.excursion_window_m / p.station_m).round() as usize;
    for (o, bad, out) in [(&mut lo, &mut bad_l, 1.0), (&mut ro, &mut bad_r, -1.0)] {
        let med: Vec<Option<f64>> = (0..n)
            .map(|i| {
                let (a, b) = (i.saturating_sub(ke), (i + ke).min(n - 1));
                let mut v: Vec<f64> = o[a..=b].iter().flatten().copied().collect();
                median(&mut v)
            })
            .collect();
        for i in 0..n {
            if let (Some(v), Some(m), Some(w)) = (o[i], med[i], local_width[i])
                && out * (v - m) > (p.excursion_share * w).max(p.excursion_min_m)
            {
                checks[i].excursion = true;
                bad[i] = true;
                o[i] = None;
            }
        }
    }
    // 3. Width, against the running median of the good widths.
    let w: Vec<Option<f64>> = (0..n).map(|i| Some(lo[i]? - ro[i]?)).collect();
    let kw = (p.width_window_m / p.station_m).round() as usize;
    let running = |v: &[Option<f64>], i: usize| {
        let (a, b) = (i.saturating_sub(kw), (i + kw).min(n - 1));
        let mut s: Vec<f64> = v[a..=b].iter().flatten().copied().collect();
        median(&mut s)
    };
    for i in 0..n {
        let (Some(wi), Some(med)) = (w[i], running(&w, i)) else {
            continue;
        };
        if wi > p.width_high * med {
            checks[i].width_wide = true;
            let dl = running(&lo, i).map_or(0.0, |m| (lo[i].unwrap_or(m) - m).abs());
            let dr = running(&ro, i).map_or(0.0, |m| (ro[i].unwrap_or(m) - m).abs());
            if dl >= dr {
                bad_l[i] = true;
            } else {
                bad_r[i] = true;
            }
        } else if wi < p.width_low * med {
            checks[i].width_narrow = true;
        }
    }
    (reference, off, checks, bad_l, bad_r)
}

/// The failures of a pair of banks, without repairing them: chainage along the reference and checks.
pub fn check_banks(left: &[P2], right: &[P2], seed: &[P2], p: &CheckParams) -> Vec<(f64, Checks)> {
    let (reference, _, checks, _, _) = assess(left, right, seed, p);
    let s = owd_format::design::midline::arc(&reference);
    checks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.any())
        .map(|(i, c)| (s[i], *c))
        .collect()
}

/// The most rounds of checks and repairs.
pub const MAX_ROUNDS: usize = 3;

fn repairs(c: &Checked) -> bool {
    c.failed
        .iter()
        .any(|(_, k)| k.side || k.missing || k.jump || k.width_wide || k.excursion)
}

/// Check the banks, repair them, and rebuild the centreline as their mid-line. `seed` is any rough
/// centreline (the last one, or the route). Each round reads the repaired banks again, along the same
/// seed, so a fault a repair uncovered the next one finds; the rounds stop when one repairs nothing, at most [`MAX_ROUNDS`]. A crossing left after the last round is flagged (`crossing`),
/// never passed in silence. The stretches are every round's.
pub fn check_and_repair(
    left: &[P2],
    right: &[P2],
    seed: &[P2],
    p: &CheckParams,
) -> Result<Checked, ReachError> {
    let (mut c, mut crossed) = check_once(left, right, seed, p)?;
    for _ in 1..MAX_ROUNDS {
        if !repairs(&c) && !crossed {
            break;
        }
        // Every round reads the banks along the seed, never along its own last centreline: a round
        // that went wrong must not make the next one's reference worse.
        let (mut next, x) = check_once(&c.left, &c.right, seed, p)?;
        let earlier = rechain(&c.stretches, &c.centreline, &next.centreline);
        next.stretches = merge(earlier, next.stretches);
        c = next;
        crossed = x;
    }
    if crossed {
        flag_crossings(&mut c, p);
    }
    Ok(c)
}

/// Flag where the banks still cross: a `repaired` stretch with the check `crossing` around each
/// crossing point, by chainage on the centreline. Nothing is moved: the stretch says to look.
fn flag_crossings(c: &mut Checked, p: &CheckParams) {
    let cs = owd_format::design::midline::arc(&c.centreline);
    let mut points = Vec::new();
    let idx = SegIndex::new(&c.right, 50.0);
    let mut buf = Vec::new();
    for w in c.left.windows(2) {
        let len = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if len == 0.0 {
            continue;
        }
        let d = [(w[1][0] - w[0][0]) / len, (w[1][1] - w[0][1]) / len];
        let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
        if let Some(t) = idx.nearest_hit(mid, d, len / 2.0, &mut buf) {
            points.push([mid[0] + d[0] * t, mid[1] + d[1] * t]);
        }
    }
    let crossing = Checks {
        crossing: true,
        ..Checks::default()
    };
    let flags: Vec<Stretch> = points
        .iter()
        .map(|x| {
            let at = chainage_on(&c.centreline, &cs, *x);
            Stretch {
                from_m: (at - p.station_m).max(0.0),
                to_m: at + p.station_m,
                kind: StretchKind::Repaired(crossing),
                min_width_m: 0.0,
            }
        })
        .collect();
    for f in &flags {
        c.failed.push((f.from_m + p.station_m, crossing));
    }
    c.stretches = merge(std::mem::take(&mut c.stretches), flags);
}

/// One round of checks and repairs, and whether the banks still cross after it.
fn check_once(
    left: &[P2],
    right: &[P2],
    seed: &[P2],
    p: &CheckParams,
) -> Result<(Checked, bool), ReachError> {
    if left.len() < 2 || right.len() < 2 || seed.len() < 2 {
        return Err(ReachError::Input("the banks are too short to check".into()));
    }
    let (reference, off, checks, bad_l, bad_r) = assess(left, right, seed, p);
    // The same normals as `bank_offsets` gave the offsets along.
    let nrm = arc_normals(
        &reference,
        median_width(&reference, left, right)
            .map_or(TANGENT_HALF_M, |w| (w / 2.0).clamp(TANGENT_HALF_M, 1_000.0)),
    );
    let lo: Vec<Option<f64>> = off.iter().map(|o| o.0).collect();
    let ro: Vec<Option<f64>> = off.iter().map(|o| o.1).collect();
    let fail = || {
        ReachError::NoWater(
            "the bank checks found no good station on one bank: the banks are not usable here"
                .into(),
        )
    };
    let (mut bad_l, mut bad_r) = (bad_l, bad_r);
    let mut checks = checks;
    if let Some(m) = p.max_step_m {
        widen_runs(&lo, &mut bad_l, m);
        widen_runs(&ro, &mut bad_r, m);
        // A station a repair took in to keep the step limit is part of that repair, and says so.
        let flagged: Vec<usize> = (0..checks.len()).filter(|i| checks[*i].any()).collect();
        for i in 0..checks.len() {
            if (bad_l[i] || bad_r[i])
                && !checks[i].any()
                && let Some(j) = flagged.iter().min_by_key(|j| j.abs_diff(i))
            {
                checks[i] = checks[*j];
            }
        }
    }
    let lf = fill(&lo, &bad_l).ok_or_else(fail)?;
    let rf = fill(&ro, &bad_r).ok_or_else(fail)?;
    let rebuild = |orig: &[P2], raw: &[Option<f64>], f: &[f64], bad: &[bool]| -> Vec<P2> {
        splice_runs(orig, &reference, &nrm, raw, f, bad)
    };
    let left2 = rebuild(left, &lo, &lf, &bad_l);
    let right2 = rebuild(right, &ro, &rf, &bad_r);
    let crossed = lines_cross(&left2, &right2).is_some();
    // Where the normals of a tight bend cross, or the banks are poor, the mid-line can loop back on
    // itself; a loop is no part of the river (issue #373).
    let centreline = cut_loops(
        &midline(&left2, &right2, &reference, p.station_m),
        CENTRELINE_LOOP_STATIONS,
    );
    let widths: Vec<f64> = bank_offsets(&centreline, &left2, &right2)
        .iter()
        .map(|o| match o {
            (Some(l), Some(r)) => (l - r).max(0.0),
            _ => 0.0,
        })
        .collect();
    // The centreline ends where the banks do: stations at either end whose normal does not meet both
    // banks are dropped.
    let a = widths.iter().position(|w| *w > 0.0).unwrap_or(0);
    let b = widths
        .iter()
        .rposition(|w| *w > 0.0)
        .unwrap_or(widths.len() - 1);
    let (centreline, widths) = if b > a {
        (centreline[a..=b].to_vec(), widths[a..=b].to_vec())
    } else {
        (centreline, widths)
    };
    // And the banks end where the centreline does: a bank running on past either end (a cast that
    // found other water at the first or last station) is met by no normal, so no check sees it.
    let ends = (
        widths.first().copied().unwrap_or(0.0),
        widths.last().copied().unwrap_or(0.0),
    );
    let (left2, right2) = (
        trim_to_centreline(&left2, &centreline, ends),
        trim_to_centreline(&right2, &centreline, ends),
    );
    let rs = owd_format::design::midline::arc(&reference);
    let failed: Vec<(f64, Checks)> = checks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.any())
        .map(|(i, c)| (rs[i], *c))
        .collect();
    // Stretches: runs of flagged reference stations, by chainage on the new centreline.
    let cs = owd_format::design::midline::arc(&centreline);
    let half = p.station_m / 2.0;
    let mut stretches: Vec<Stretch> = Vec::new();
    let mut prev: Option<usize> = None;
    // The search window, in segments, of the walk along the centreline.
    const WINDOW: usize = 200;
    let mut cursor = 0usize;
    for (i, c) in checks.iter().enumerate() {
        if !c.any() {
            // Keep the cursor with the stations, so the window follows them.
            cursor = (i * centreline.len()) / reference.len().max(1);
            continue;
        }
        // Near where the last flagged station was: the stations run along the centreline.
        let (at, seg) = chainage_near(
            &centreline,
            &cs,
            reference[i],
            cursor.saturating_sub(WINDOW)..cursor + 4 * WINDOW,
        );
        cursor = seg;
        let w = width_near(&cs, &widths, at);
        match stretches.last_mut() {
            Some(s) if prev == Some(i.wrapping_sub(1)) => {
                s.to_m = s.to_m.max(at + half);
                s.from_m = s.from_m.min(at - half).max(0.0);
                s.min_width_m = s.min_width_m.min(w);
                if let StretchKind::Repaired(k) = &mut s.kind {
                    *k = k.or(*c);
                }
            }
            _ => stretches.push(Stretch {
                from_m: (at - half).max(0.0),
                to_m: at + half,
                kind: StretchKind::Repaired(*c),
                min_width_m: w,
            }),
        }
        prev = Some(i);
    }
    Ok((
        Checked {
            left: left2,
            right: right2,
            centreline,
            widths,
            stretches,
            failed,
        },
        crossed,
    ))
}

fn width_near(cs: &[f64], widths: &[f64], at: f64) -> f64 {
    let i = cs
        .partition_point(|v| *v < at)
        .min(widths.len().saturating_sub(1));
    widths.get(i).copied().unwrap_or(0.0)
}

/// The chainage of `p` along `line` (arc lengths `s`): of its nearest point.
pub fn chainage_on(line: &[P2], s: &[f64], p: P2) -> f64 {
    chainage_near(line, s, p, 0..line.len().saturating_sub(1)).0
}

/// The chainage of the nearest point of `line` to `p` among the segments `segs`, and that segment.
/// Searching only near where the point is expected keeps a walk along a long line linear.
pub fn chainage_near(line: &[P2], s: &[f64], p: P2, segs: std::ops::Range<usize>) -> (f64, usize) {
    let mut best = (f64::MAX, 0.0, segs.start);
    let end = segs.end.min(line.len().saturating_sub(1));
    for i in segs.start.min(end)..end {
        let w = [line[i], line[i + 1]];
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 == 0.0 {
            0.0
        } else {
            (((p[0] - w[0][0]) * d[0] + (p[1] - w[0][1]) * d[1]) / l2).clamp(0.0, 1.0)
        };
        let q = [w[0][0] + t * d[0], w[0][1] + t * d[1]];
        let dd = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
        if dd < best.0 {
            best = (dd, s[i] + t * (s[i + 1] - s[i]), i);
        }
    }
    (best.1, best.2)
}

/// Move stretches from chainage on `old` to chainage on `new` (each end through its point).
pub fn rechain(stretches: &[Stretch], old: &[P2], new: &[P2]) -> Vec<Stretch> {
    if old.len() < 2 || new.len() < 2 {
        return stretches.to_vec();
    }
    let (so, sn) = (
        owd_format::design::midline::arc(old),
        owd_format::design::midline::arc(new),
    );
    // Near the same share of the length: the two lines follow the same river.
    let (lo, ln) = (so[so.len() - 1].max(1e-9), sn[sn.len() - 1]);
    let at = |c: f64| {
        let guess = sn.partition_point(|v| *v < c / lo * ln);
        chainage_near(
            new,
            &sn,
            owd_format::design::midline::point_at(old, &so, c),
            guess.saturating_sub(300)..guess + 300,
        )
        .0
    };
    stretches
        .iter()
        .map(|s| {
            let (a, b) = (at(s.from_m), at(s.to_m));
            Stretch {
                from_m: a.min(b),
                to_m: a.max(b),
                ..*s
            }
        })
        .collect()
}

/// How serious a kind of stretch is, for a merged stretch: the most serious kind wins.
fn rank(k: StretchKind) -> u8 {
    match k {
        StretchKind::NoWater => 6,
        StretchKind::NoEdge => 5,
        StretchKind::JrcFallback => 4,
        StretchKind::AtShiftLimit => 3,
        StretchKind::Repaired(_) => 2,
        StretchKind::Narrow => 1,
    }
}

/// The kind of two overlapping stretches merged: the more serious, or both repairs' checks.
fn merged_kind(a: StretchKind, b: StretchKind) -> StretchKind {
    match (a, b) {
        (StretchKind::Repaired(x), StretchKind::Repaired(y)) => StretchKind::Repaired(x.or(y)),
        _ if rank(b) > rank(a) => b,
        _ => a,
    }
}

/// All the stretches, in chainage order, merged where they overlap or touch (#361): one place is
/// one stretch, of its most serious kind and its narrowest width, so there are never more
/// stretches than stations. A stretch flagged only as narrower than its neighbours adds nothing
/// where a `narrow` stretch overlaps or touches it.
pub fn merge(a: Vec<Stretch>, b: Vec<Stretch>) -> Vec<Stretch> {
    let mut out: Vec<Stretch> = Vec::new();
    for s in merge_unfused(a, b) {
        match out.last_mut() {
            Some(l) if s.from_m <= l.to_m => {
                l.to_m = l.to_m.max(s.to_m);
                l.min_width_m = l.min_width_m.min(s.min_width_m);
                l.kind = merged_kind(l.kind, s.kind);
            }
            _ => out.push(s),
        }
    }
    out
}

fn merge_unfused(mut a: Vec<Stretch>, b: Vec<Stretch>) -> Vec<Stretch> {
    let narrow_only = |s: &Stretch| matches!(s.kind, StretchKind::Repaired(c) if c == Checks { width_narrow: true, ..Checks::default() });
    // Overlapping, or next to it within the stretch's own length (the station where the width starts
    // to fall is often the one before the mask calls it narrow).
    let covered = |s: &Stretch, all: &[Stretch]| {
        let pad = s.to_m - s.from_m;
        all.iter().any(|o| {
            o.kind == StretchKind::Narrow && o.from_m < s.to_m + pad && s.from_m - pad < o.to_m
        })
    };
    let all: Vec<Stretch> = a.iter().chain(&b).cloned().collect();
    a.extend(b);
    a.retain(|s| !(narrow_only(s) && covered(s, &all)));
    a.sort_by(|x, y| x.from_m.total_cmp(&y.from_m));
    a
}
