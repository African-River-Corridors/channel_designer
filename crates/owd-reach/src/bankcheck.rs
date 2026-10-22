//! Checks and repairs on a pair of bank lines, whatever made them (spec 0013, ticket T1).
//!
//! The banks are read at stations along a reference line: the seed centreline (the route, or the last
//! centreline), smoothed over about one river width so that one inlet cannot drag it. Along the reference's left normal, each station
//! has an offset to the left bank and one to the right bank. The checks, in order:
//!
//! 1. **Own side.** The left bank lies at a positive offset, the right bank at a negative one. A normal
//!    that meets no bank is `missing`.
//! 2. **Lateral jump.** Per bank, the offset may move outward by at most `jump_factor` station steps
//!    from one station to the next. A station past the limit starts a run of bad stations that ends
//!    where the bank comes back within the limit of the offset before the run. Only outward jumps are
//!    cut (inlets, tributary mouths, spikes onto land): a jump inward narrows the river, and narrowing
//!    is left to the width check, which flags it and never widens it. A run longer than
//!    `max_run_m`, or one that runs to the end of the banks, is a real change of the river, not an
//!    excursion: it is kept.
//! 3. **Width.** The width must lie between `width_low` and `width_high` times the running median of
//!    the good widths within `width_window_m`. A wide station is repaired on the bank that left its
//!    own running median the most. A narrow one is flagged and kept: a real throat must never be
//!    widened away (and where a `narrow` stretch already covers it, it adds nothing).
//!
//! A repair replaces the offset by a straight interpolation between the nearest good stations of that
//! bank. The banks are rebuilt from the offsets, loops are cut out, and the centreline is rebuilt as
//! the mid-line of the final banks. Every flagged station joins a low-confidence stretch of kind
//! `repaired`, which says which checks failed. A crossing of the two banks left after the repairs is
//! an error: the check fails loudly, never silently.
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
}

impl Checks {
    pub fn any(self) -> bool {
        self.side || self.missing || self.jump || self.width_wide || self.width_narrow
    }

    fn or(self, o: Checks) -> Checks {
        Checks {
            side: self.side || o.side,
            missing: self.missing || o.missing,
            jump: self.jump || o.jump,
            width_wide: self.width_wide || o.width_wide,
            width_narrow: self.width_narrow || o.width_narrow,
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
    // Stations before it that stand outward of that median are cut too.
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
    for i in 0..start {
        if let (Some(o), Some(m)) = (off[i], med(i)) {
            bad[i] = out * (o - m) > lim;
        }
    }
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
    let nearest = |p: P2| {
        (0..orig.len())
            .min_by(|a, b| {
                let da = (orig[*a][0] - p[0]).powi(2) + (orig[*a][1] - p[1]).powi(2);
                let db = (orig[*b][0] - p[0]).powi(2) + (orig[*b][1] - p[1]).powi(2);
                da.total_cmp(&db)
            })
            .unwrap_or(0)
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
            0
        } else {
            nearest(at(i - 1, raw[i - 1].unwrap_or(fixed[i - 1])))
        };
        let ib = if j == n {
            orig.len() - 1
        } else {
            nearest(at(j, raw[j].unwrap_or(fixed[j])))
        };
        if ia < from || ib < ia {
            // The bank does not run with the reference here: rebuild it all from the offsets.
            let pts: Vec<P2> = (0..n).map(|k| at(k, fixed[k])).collect();
            return cut_loops(&pts, 64);
        }
        out.extend_from_slice(&orig[from..=ia]);
        out.extend((i..j).map(|k| at(k, fixed[k])));
        from = if j == n { orig.len() } else { ib };
        i = j;
    }
    if from < orig.len() {
        out.extend_from_slice(&orig[from..]);
    }
    cut_loops(&out, 64)
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
    for (i, (l, r)) in off.iter().enumerate() {
        let lok = l.filter(|v| *v > 0.0);
        let rok = r.filter(|v| *v < 0.0);
        if i < first || i > last {
            lo.push(lok);
            ro.push(rok);
            continue;
        }
        if l.is_none() || r.is_none() {
            checks[i].missing = true;
        }
        if (l.is_some() && lok.is_none()) || (r.is_some() && rok.is_none()) {
            checks[i].side = true;
        }
        bad_l[i] = lok.is_none();
        bad_r[i] = rok.is_none();
        lo.push(lok);
        ro.push(rok);
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

/// Check the banks, repair them, and rebuild the centreline as their mid-line. `seed` is any rough
/// centreline (the last one, or the route).
pub fn check_and_repair(
    left: &[P2],
    right: &[P2],
    seed: &[P2],
    p: &CheckParams,
) -> Result<Checked, ReachError> {
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
    let lf = fill(&lo, &bad_l).ok_or_else(fail)?;
    let rf = fill(&ro, &bad_r).ok_or_else(fail)?;
    let rebuild = |orig: &[P2], raw: &[Option<f64>], f: &[f64], bad: &[bool]| -> Vec<P2> {
        splice_runs(orig, &reference, &nrm, raw, f, bad)
    };
    let left2 = rebuild(left, &lo, &lf, &bad_l);
    let right2 = rebuild(right, &ro, &rf, &bad_r);
    if let Some(x) = lines_cross(&left2, &right2) {
        return Err(ReachError::Input(format!(
            "the banks still cross at ({:.1}, {:.1}) after the repairs: check them by eye (owd qa-overlay)",
            x[0], x[1]
        )));
    }
    let centreline = midline(&left2, &right2, &reference, p.station_m);
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
    for (i, c) in checks.iter().enumerate() {
        if !c.any() {
            continue;
        }
        let at = chainage_on(&centreline, &cs, reference[i]);
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
    Ok(Checked {
        left: left2,
        right: right2,
        centreline,
        widths,
        stretches,
        failed,
    })
}

fn width_near(cs: &[f64], widths: &[f64], at: f64) -> f64 {
    let i = cs
        .partition_point(|v| *v < at)
        .min(widths.len().saturating_sub(1));
    widths.get(i).copied().unwrap_or(0.0)
}

/// The chainage of `p` along `line` (arc lengths `s`): of its nearest point.
pub fn chainage_on(line: &[P2], s: &[f64], p: P2) -> f64 {
    let mut best = (f64::MAX, 0.0);
    for (i, w) in line.windows(2).enumerate() {
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
            best = (dd, s[i] + t * (s[i + 1] - s[i]));
        }
    }
    best.1
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
    let at = |c: f64| chainage_on(new, &sn, owd_format::design::midline::point_at(old, &so, c));
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

/// All the stretches, in chainage order. A stretch flagged only as narrower than its neighbours adds
/// nothing where a `narrow` stretch overlaps or touches it.
pub fn merge(mut a: Vec<Stretch>, b: Vec<Stretch>) -> Vec<Stretch> {
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
