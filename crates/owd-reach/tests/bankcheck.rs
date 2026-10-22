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
fn after_the_checks_no_bank_is_on_the_wrong_side_and_none_crosses() {
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
        let c = match check_and_repair(&left, &right, &seed, &params()) {
            Ok(c) => c,
            // A crossing left is an error, never a silent pass: that is allowed, a silent cross is not.
            Err(e) => { assert!(e.to_string().contains("still cross"), "case {case}: {e}"); continue; }
        };
        assert!(lines_cross(&c.left, &c.right).is_none(), "case {case}");
        for (i, o) in bank_offsets(&c.centreline, &c.left, &c.right).iter().enumerate() {
            if let (Some(l), Some(r)) = o {
                assert!(*l > 0.0 && *r < 0.0, "case {case}, station {i}: {l} {r}");
            }
        }
    }
}
}
