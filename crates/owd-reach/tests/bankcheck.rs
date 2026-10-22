//! Checks and repairs on bank lines of known shape (spec 0013, ticket T1): a straight channel 200 m
//! wide, in project metres, with one fault at a time.
//!
//! Label: property (every expected value follows from the geometry built here)

use owd_format::design::midline::{bank_offsets, midline_offsets};
use owd_reach::bankcheck::{CheckParams, Checks, check_and_repair, check_banks, lines_cross};
use owd_reach::banks::StretchKind;

macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

type P = [f64; 2];

/// A bank along y = `y`, a vertex every 10 m from x = 0 to 5 km, with `f` adding to y.
fn bank(y: f64, f: impl Fn(f64) -> f64) -> Vec<P> {
    (0..=500)
        .map(|i| {
            let x = i as f64 * 10.0;
            [x, y + f(x)]
        })
        .collect()
}

fn seed() -> Vec<P> {
    vec![[0.0, 0.0], [5_000.0, 0.0]]
}

fn params() -> CheckParams {
    CheckParams::at_step(20.0)
}

/// The width at each station of the result away from `skip` (a range of x), with the stations' x.
fn widths_away(c: &owd_reach::bankcheck::Checked, skip: std::ops::Range<f64>) -> Vec<(f64, f64)> {
    c.centreline
        .iter()
        .zip(&c.widths)
        .filter(|(p, _)| !skip.contains(&p[0]) && p[0] > 200.0 && p[0] < 4_800.0)
        .map(|(p, w)| (p[0], *w))
        .collect()
}

t! {
fn an_inlet_on_one_bank_is_cut_flagged_jump_and_the_width_elsewhere_is_kept() {
    // A creek mouth 60 m deep, 100 m along the bank, at x = 2000 to 2100.
    let inlet = |x: f64| if (2_000.0..=2_100.0).contains(&x) { 60.0 } else { 0.0 };
    let (left, right) = (bank(100.0, inlet), bank(-100.0, |_| 0.0));
    let c = check_and_repair(&left, &right, &seed(), &params()).unwrap();
    // The inlet is gone: no left bank vertex is more than a metre outside the channel.
    assert!(c.left.iter().all(|p| p[1] <= 101.0), "{:?}", c.left.iter().filter(|p| p[1] > 101.0).collect::<Vec<_>>());
    // One stretch, kind repaired, for a jump, over the inlet (each end within two stations).
    assert_eq!(c.stretches.len(), 1, "{:?}", c.stretches);
    let s = &c.stretches[0];
    let StretchKind::Repaired(k) = s.kind else { panic!("{s:?}") };
    assert!(k.jump, "{k:?}");
    assert!((s.from_m - 2_000.0).abs() <= 40.0 && (s.to_m - 2_100.0).abs() <= 40.0, "{s:?}");
    // Away from it the width is 200 m within one 10 m pixel.
    for (x, w) in widths_away(&c, 1_800.0..2_300.0) {
        assert!((w - 200.0).abs() <= 10.0, "width {w} at x = {x}");
    }
    // The centreline is the banks' mid-line.
    for o in midline_offsets(&c.centreline, &c.left, &c.right).into_iter().flatten() {
        assert!(o < 1.0, "{o}");
    }
}
}

t! {
fn banks_on_one_side_are_repaired_and_flagged_side() {
    // At x = 3000 to 3040 the left bank dives across the river to y = -120, beyond the right bank.
    let dive = |x: f64| if (3_000.0..=3_040.0).contains(&x) { -220.0 } else { 0.0 };
    let (left, right) = (bank(100.0, dive), bank(-100.0, |_| 0.0));
    assert!(lines_cross(&left, &right).is_some());
    let before = check_banks(&left, &right, &seed(), &params());
    assert!(before.iter().any(|(_, c)| c.side), "{before:?}");
    let c = check_and_repair(&left, &right, &seed(), &params()).unwrap();
    assert!(lines_cross(&c.left, &c.right).is_none());
    // No station of the result has both banks on one side.
    for (i, o) in bank_offsets(&c.centreline, &c.left, &c.right).iter().enumerate() {
        if let (Some(l), Some(r)) = o {
            assert!(*l > 0.0 && *r < 0.0, "station {i}: {l} {r}");
        }
    }
    let s: Vec<Checks> = c.stretches.iter().filter_map(|s| match s.kind {
        StretchKind::Repaired(k) => Some(k),
        _ => None,
    }).collect();
    assert!(s.iter().any(|k| k.side), "{:?}", c.stretches);
    // Clean banks fail nothing.
    assert!(check_banks(&bank(100.0, |_| 0.0), &bank(-100.0, |_| 0.0), &seed(), &params()).is_empty());
}
}

t! {
fn a_real_throat_is_flagged_narrow_and_never_widened() {
    // Both banks close in to 80 m wide over x = 2500 to 2800, with 100 m tapers.
    let close = |x: f64| -> f64 {
        let d = if x < 2_400.0 || x > 2_900.0 { 0.0 }
            else if x < 2_500.0 { (x - 2_400.0) / 100.0 }
            else if x <= 2_800.0 { 1.0 }
            else { (2_900.0 - x) / 100.0 };
        60.0 * d
    };
    let (left, right) = (bank(100.0, |x| -close(x)), bank(-100.0, close));
    let c = check_and_repair(&left, &right, &seed(), &params()).unwrap();
    let at_throat: Vec<f64> = c.centreline.iter().zip(&c.widths)
        .filter(|(p, _)| (2_550.0..2_750.0).contains(&p[0])).map(|(_, w)| *w).collect();
    assert!(!at_throat.is_empty());
    assert!(at_throat.iter().all(|w| (w - 80.0).abs() <= 1.0), "{at_throat:?}");
    assert!(c.stretches.iter().any(|s| matches!(s.kind, StretchKind::Repaired(k) if k.width_narrow && !k.jump)), "{:?}", c.stretches);
}
}

t! {
fn after_the_checks_no_bank_is_on_the_wrong_side_and_a_crossing_left_is_flagged() {
    // Property over random channels with random spikes and dives.
    let mut x: u64 = 3;
    let mut next = || {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((x >> 33) as f64) / (1u64 << 31) as f64
    };
    for case in 0..25 {
        let w = 80.0 + next() * 400.0;
        let (amp, ph) = (next() * 150.0, next() * 6.0);
        let faults: Vec<(f64, f64, f64)> = (0..4).map(|_| (200.0 + next() * 4_600.0, 20.0 + next() * 100.0, (next() - 0.4) * 3.0 * w)).collect();
        let fault = |x: f64, k: usize| faults.iter().skip(k).step_by(2).map(|(at, len, d)| if (x - at).abs() <= len / 2.0 { *d } else { 0.0 }).sum::<f64>();
        let wave = |x: f64| amp * (x / 700.0 + ph).sin();
        let left = bank(w / 2.0, |x| wave(x) + fault(x, 0));
        let right = bank(-w / 2.0, |x| wave(x) - fault(x, 1));
        let seed: Vec<P> = (0..=50).map(|i| [i as f64 * 100.0, wave(i as f64 * 100.0)]).collect();
        let c = check_and_repair(&left, &right, &seed, &params()).unwrap();
        // A crossing left is flagged, never passed in silence.
        let flagged = c.stretches.iter().any(|s| matches!(s.kind, StretchKind::Repaired(k) if k.crossing));
        assert_eq!(lines_cross(&c.left, &c.right).is_some(), flagged, "case {case}");
        if flagged {
            continue;
        }
        for (i, o) in bank_offsets(&c.centreline, &c.left, &c.right).iter().enumerate() {
            if let (Some(l), Some(r)) = o {
                assert!(*l > 0.0 && *r < 0.0, "case {case}, station {i}: {l} {r}");
            }
        }
    }
}
}

t! {
fn a_repair_keeps_to_the_step_limit_and_flags_what_it_took_in() {
    // 5 m stations, as after the edge fit. The left bank steps out 10 m at x = 3000 (within the jump
    // limit, a real change) and dives across the river from x = 2990 to 3010. Repaired over the dive
    // alone, the bank would change 10 m in 6 stations (1.7 m each): over the fit's 1.5 m limit.
    let bank5 = |y: f64, f: &dyn Fn(f64) -> f64| -> Vec<P> {
        (0..=1000).map(|i| {
            let x = i as f64 * 5.0;
            [x, y + f(x)]
        }).collect()
    };
    let left = bank5(100.0, &|x| if (2_990.0..=3_010.0).contains(&x) { -220.0 } else if x > 3_010.0 { 10.0 } else { 0.0 });
    let right = bank5(-100.0, &|_| 0.0);
    let p = CheckParams { max_step_m: Some(1.5), ..CheckParams::at_step(5.0) };
    let c = check_and_repair(&left, &right, &seed(), &p).unwrap();
    let near: Vec<&P> = c.left.iter().filter(|q| (2_850.0..3_150.0).contains(&q[0])).collect();
    for w in near.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], (w[1][1] - w[0][1]).abs());
        assert!(dy <= 1.5 * dx / 5.0 + 0.05, "{:?} to {:?}: {dy:.2} m in {dx:.1} m", w[0], w[1]);
    }
    // The stretch covers what the repair moved, which is more than the dive alone.
    let span = |c: &owd_reach::bankcheck::Checked| -> f64 {
        c.stretches.iter().filter(|s| matches!(s.kind, StretchKind::Repaired(k) if k.side)).map(|s| s.to_m - s.from_m).sum()
    };
    let limited = span(&c);
    // Without the limit the repair is the dive alone, and steeper.
    let c = check_and_repair(&left, &right, &seed(), &CheckParams::at_step(5.0)).unwrap();
    assert!(limited > span(&c), "{limited} against {}", span(&c));
    let steep = c.left.windows(2).any(|w| (2_850.0..3_150.0).contains(&w[0][0]) && (w[1][1] - w[0][1]).abs() > 1.5 * (w[1][0] - w[0][0]) / 5.0 + 0.05);
    assert!(steep, "the case must need the limit");
}
}

fn length(line: &[P]) -> f64 {
    line.windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum()
}

t! {
    /// Issue #373, offline, on the whole-river Sentinel-2 banks of a run of 2026-10-22 (every 4th
    /// vertex), whose centreline came out 178.7 km for a 144 km route: an 11 km bank gap across a
    /// bend, and mid-line loops at bends and junctions. Without the gap rule and the loop cut these
    /// banks give a 215 km centreline for a 134 km seed; with them, about the seed's length.
    fn the_recorded_whole_river_gives_a_centreline_about_as_long_as_its_seed() {
        let fx: serde_json::Value =
            serde_json::from_str(include_str!("data/midline-tangle.json")).unwrap();
        let pts = |k: &str| -> Vec<P> {
            fx[k]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
                .collect()
        };
        let (left, right, seed) = (pts("left"), pts("right"), pts("seed"));
        let p = CheckParams {
            max_step_m: Some(1.5),
            ..CheckParams::at_step(5.0)
        };
        let c = check_and_repair(&left, &right, &seed, &p).unwrap();
        let ratio = length(&c.centreline) / length(&seed);
        assert!(ratio < 1.1, "the centreline is {ratio} times the seed");
    }
}

mod merged_stretches {
    //! #361: the stretches from two sources (the mask or the edge fit, and the bank checks) are
    //! merged into disjoint stretches, so there are never more stretches than stations.
    //!
    //! Label: property
    use owd_reach::bankcheck::{Checks, merge};
    use owd_reach::banks::{Stretch, StretchKind};

    /// The stretch of the stations `a..=b`, 10 m apart, each covering half a step either side.
    fn st(a: usize, b: usize, kind: StretchKind) -> Stretch {
        Stretch {
            from_m: (a as f64 * 10.0 - 5.0).max(0.0),
            to_m: b as f64 * 10.0 + 5.0,
            kind,
            min_width_m: 20.0 + a as f64,
        }
    }

    fn jump() -> StretchKind {
        StretchKind::Repaired(Checks {
            jump: true,
            ..Checks::default()
        })
    }

    #[test]
    fn overlapping_stretches_merge_and_never_outnumber_the_stations() {
        // Every station flagged several times over by the two sources.
        let stations = 50;
        let a: Vec<Stretch> = (0..stations)
            .map(|i| st(i, i, StretchKind::Narrow))
            .collect();
        let b: Vec<Stretch> = (0..stations)
            .step_by(2)
            .map(|i| st(i, (i + 3).min(stations - 1), jump()))
            .collect();
        let m = merge(a, b);
        assert!(
            m.len() <= stations,
            "{} stretches for {stations} stations",
            m.len()
        );
        for w in m.windows(2) {
            assert!(w[0].to_m < w[1].from_m, "{:?} overlaps {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn a_merged_stretch_keeps_the_most_serious_kind_and_the_narrowest_width() {
        let m = merge(
            vec![
                st(0, 4, StretchKind::Narrow),
                st(20, 22, StretchKind::Narrow),
            ],
            vec![st(3, 8, StretchKind::NoEdge)],
        );
        assert_eq!(m.len(), 2, "{m:?}");
        assert_eq!(m[0].kind, StretchKind::NoEdge);
        assert_eq!((m[0].from_m, m[0].to_m), (0.0, 85.0));
        assert_eq!(m[0].min_width_m, 20.0);
        assert_eq!(m[1].kind, StretchKind::Narrow);
        // Two repairs merge their checks.
        let width = StretchKind::Repaired(Checks {
            width_wide: true,
            ..Checks::default()
        });
        let m = merge(vec![st(0, 2, jump())], vec![st(2, 5, width)]);
        assert_eq!(m.len(), 1);
        assert_eq!(
            m[0].kind,
            StretchKind::Repaired(Checks {
                jump: true,
                width_wide: true,
                ..Checks::default()
            })
        );
    }
}
