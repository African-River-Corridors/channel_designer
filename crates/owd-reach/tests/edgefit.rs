//! The edge fit on an image with a known edge, and on a recorded Sentinel-2 window.
//!
//! The synthetic scene is a straight river along an east-west line, 120 m wide, in a 10 m grid: water
//! and land have fixed colours, and a pixel the edge crosses is a mix of the two by the share of water
//! in it, so the true edge is known to well under a pixel. The mask banks start 12 m inside it.
//!
//! Label: property (the true edge is the line the image was made from)

use owd_format::design::crs::Utm;
use owd_reach::banks::{BankParams, MaskBanks, StretchKind};
use owd_reach::edgefit::{FitParams, fit_banks};
use owd_reach::sentinel2::Imagery;

macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

const X0: f64 = 500_000.0;
const Y1: f64 = 600_000.0;
const COLS: usize = 300;
const ROWS: usize = 100;
/// The river's middle line and half width, in metres north of the image's south edge (Y1 - 1000).
const YC: f64 = Y1 - 500.0;
const HALF: f64 = 60.0;

const WATER: [f32; 4] = [0.060, 0.050, 0.020, 0.010];
const LAND: [f32; 4] = [0.050, 0.040, 0.350, 0.200];

/// A deterministic wobble in [-1, 1] for a pixel.
fn wobble(i: usize, k: usize) -> f32 {
    let mut x = (i as u64 * 2654435761 + k as u64 * 40503) as u32;
    x ^= x >> 13;
    x = x.wrapping_mul(0x5bd1e995);
    x ^= x >> 15;
    (x % 2000) as f32 / 1000.0 - 1.0
}

/// The share of a pixel (rows from the top, 10 m) that is water: the overlap of its y range with the river.
fn water_share(row: usize) -> f64 {
    let (top, bottom) = (Y1 - row as f64 * 10.0, Y1 - (row + 1) as f64 * 10.0);
    let (lo, hi) = ((YC - HALF).max(bottom), (YC + HALF).min(top));
    ((hi - lo) / 10.0).clamp(0.0, 1.0)
}

fn scene(cloud: Option<std::ops::Range<usize>>) -> Imagery {
    let mut bands: [Vec<f32>; 4] = Default::default();
    let mut valid = Vec::new();
    for r in 0..ROWS {
        let f = water_share(r) as f32;
        for c in 0..COLS {
            let i = r * COLS + c;
            for k in 0..4 {
                let v = f * WATER[k] + (1.0 - f) * LAND[k];
                bands[k].push((v * (1.0 + 0.04 * wobble(i, k))).max(0.001));
            }
            valid.push(!cloud.as_ref().is_some_and(|x| x.contains(&c)));
        }
    }
    Imagery::from_bands(
        Utm {
            zone: 29,
            south: false,
        },
        X0,
        Y1,
        10.0,
        COLS,
        ROWS,
        bands,
        valid,
    )
}

/// Mask banks 12 m inside the true edge, stations every 20 m from x = 200 m to 2800 m.
fn mask_banks() -> MaskBanks {
    let xs: Vec<f64> = (0..=130).map(|i| X0 + 200.0 + 20.0 * i as f64).collect();
    let left: Vec<[f64; 2]> = xs.iter().map(|x| [*x, YC + HALF - 12.0]).collect();
    let right: Vec<[f64; 2]> = xs.iter().map(|x| [*x, YC - HALF + 12.0]).collect();
    let centreline: Vec<[f64; 2]> = xs.iter().map(|x| [*x, YC]).collect();
    let mut polygon = left.clone();
    polygon.extend(right.iter().rev());
    polygon.push(left[0]);
    MaskBanks {
        utm: Utm {
            zone: 29,
            south: false,
        },
        params: BankParams::default(),
        widths: vec![2.0 * (HALF - 12.0); xs.len()],
        centreline,
        left,
        right,
        polygon,
        stretches: Vec::new(),
        pixel_m: 30.0,
        source: "jrc-gsw",
    }
}

t! {
fn a_known_edge_is_found_within_one_pixel_from_a_seed_12_m_inside_it() {
    let fit = fit_banks(&mask_banks(), &scene(None), &FitParams::default()).unwrap();
    // Away from the two ends (the matched filter needs room each side).
    let mid = |v: &[[f64; 2]]| v.iter().filter(|p| p[0] > X0 + 400.0 && p[0] < X0 + 2600.0).map(|p| p[1]).collect::<Vec<_>>();
    for y in mid(&fit.left) {
        assert!((y - (YC + HALF)).abs() <= 10.0, "left bank at {} m, true edge {}", y - YC, HALF);
    }
    for y in mid(&fit.right) {
        assert!((y - (YC - HALF)).abs() <= 10.0, "right bank at {} m, true edge {}", y - YC, -HALF);
    }
    // The shift stays within the limit, and the banks moved outward (a positive shift is outward).
    let p = FitParams::default();
    assert!(fit.left_shift.iter().chain(&fit.right_shift).all(|d| d.abs() <= p.max_shift_m + 1e-9));
    let mean: f64 = fit.left_shift[40..fit.left_shift.len() - 40].iter().sum::<f64>() / (fit.left_shift.len() - 80) as f64;
    assert!(mean > 8.0 && mean < 16.0, "mean shift {mean}");
    // The slope limit holds: 1.5 m per 5 m station.
    for w in fit.left_shift.windows(2) {
        assert!((w[1] - w[0]).abs() <= p.max_step_m + 1e-6);
    }
    // 120 m wide river is not low-confidence width (under 30 m).
    assert!(fit.stretches.iter().all(|s| s.kind != StretchKind::Narrow), "{:?}", fit.stretches);
    // The polygon closes and the centreline sits on the river's middle.
    assert_eq!(fit.polygon.first(), fit.polygon.last());
    let c = fit.centreline[fit.centreline.len() / 2];
    assert!((c[1] - YC).abs() < 6.0, "{c:?}");
}
}

t! {
fn a_cloudy_stretch_keeps_the_mask_bank_and_is_flagged_and_only_that_stretch() {
    // Cloud over image columns 100 to 140, which is x = 1000 to 1400 m.
    let fit = fit_banks(&mask_banks(), &scene(Some(100..140)), &FitParams::default()).unwrap();
    assert_eq!(fit.stretches.len(), 1, "{:?}", fit.stretches);
    let s = &fit.stretches[0];
    assert_eq!(s.kind, StretchKind::NoEdge);
    // The banks run from x = 200 m: chainage 800 to 1200 m is the cloud; the flag may extend a few
    // stations each side (the confidence is a running median over 5 stations and the pins ramp).
    assert!(s.from_m > 650.0 && s.from_m < 850.0, "{s:?}");
    assert!(s.to_m > 1150.0 && s.to_m < 1350.0, "{s:?}");
    // Inside the cloud the banks are the mask banks exactly: the shift there is 0 where the stretch is kept.
    let kept: Vec<usize> = (0..fit.left_kept.len()).filter(|i| fit.left_kept[*i]).collect();
    assert!(!kept.is_empty());
    for i in &kept {
        assert!(fit.left_shift[*i].abs() < 1e-6, "kept station {i} shifted {}", fit.left_shift[*i]);
    }
    for p in fit.left.iter().filter(|p| p[0] > X0 + 1100.0 && p[0] < X0 + 1300.0) {
        assert!((p[1] - (YC + HALF - 12.0)).abs() < 1e-6, "left bank moved in the cloud: {p:?}");
    }
    // Clear water either side of the cloud was fitted to the edge.
    for p in fit.left.iter().filter(|p| p[0] > X0 + 400.0 && p[0] < X0 + 700.0) {
        assert!((p[1] - (YC + HALF)).abs() <= 10.0, "{p:?}");
    }
}
}

t! {
fn a_throat_under_three_pixels_wide_after_the_fit_is_flagged_narrow_and_only_the_throat() {
    // A 100 m river with a 24 m throat in image columns 120 to 160 (x = 1200 to 1600 m).
    let half = |x: f64| if x > X0 + 1200.0 && x < X0 + 1600.0 { 12.0 } else { 50.0 };
    let mut b = mask_banks();
    for p in &mut b.left { p[1] = YC + half(p[0]) - if half(p[0]) < 20.0 { 0.0 } else { 2.0 }; }
    for p in &mut b.right { p[1] = YC - half(p[0]) + if half(p[0]) < 20.0 { 0.0 } else { 2.0 }; }
    let mut poly = b.left.clone();
    poly.extend(b.right.iter().rev());
    poly.push(b.left[0]);
    b.polygon = poly;
    let mut im = scene(None);
    for r in 0..ROWS {
        let (top, bottom) = (Y1 - r as f64 * 10.0, Y1 - (r + 1) as f64 * 10.0);
        for c in 0..COLS {
            let h = half(X0 + 10.0 * c as f64 + 5.0);
            let f = (((YC + h).min(top) - (YC - h).max(bottom)) / 10.0).clamp(0.0, 1.0) as f32;
            let i = r * COLS + c;
            for k in 0..4 {
                im.bands[k][i] = (f * WATER[k] + (1.0 - f) * LAND[k]) * (1.0 + 0.04 * wobble(i, k));
            }
        }
    }
    let fit = fit_banks(&b, &im, &FitParams::default()).unwrap();
    assert_eq!(fit.stretches.len(), 1, "{:?}", fit.stretches);
    let s = &fit.stretches[0];
    assert_eq!(s.kind, StretchKind::Narrow);
    // The banks run from x = 200 m: the throat is chainage 1000 to 1400 m, blurred by the filter and the pins.
    assert!(s.from_m > 850.0 && s.from_m < 1150.0, "{s:?}");
    assert!(s.to_m > 1250.0 && s.to_m < 1550.0, "{s:?}");
    assert!(s.min_width_m < 30.0, "{s:?}");
}
}

t! {
fn an_edge_beyond_the_largest_shift_moves_the_bank_by_that_shift_and_is_flagged() {
    // Mask banks 45 m inside the true edge: past the 30 m limit, and past the 40 m search. The water
    // runs on across the whole search, so each bank moves out by exactly 30 m and says the edge lies
    // farther.
    let mut b = mask_banks();
    for p in &mut b.left { p[1] = YC + HALF - 45.0; }
    for p in &mut b.right { p[1] = YC - HALF + 45.0; }
    let mut poly = b.left.clone();
    poly.extend(b.right.iter().rev());
    poly.push(b.left[0]);
    b.polygon = poly;
    let p = FitParams::default();
    let fit = fit_banks(&b, &scene(None), &p).unwrap();
    let n = fit.left_shift.len();
    for d in fit.left_shift[40..n - 40].iter().chain(&fit.right_shift[40..n - 40]) {
        assert!((d - p.max_shift_m).abs() < 1e-6, "shift {d}");
    }
    assert!(fit.left_shift.iter().chain(&fit.right_shift).all(|d| d.abs() <= p.max_shift_m + 1e-9));
    for q in fit.left.iter().filter(|q| q[0] > X0 + 600.0 && q[0] < X0 + 2400.0) {
        assert!((q[1] - (YC + HALF - 15.0)).abs() < 1e-3, "{q:?}");
    }
    assert!(fit.left_beyond[40..n - 40].iter().all(|f| *f));
    assert_eq!(fit.stretches.len(), 1, "{:?}", fit.stretches);
    assert_eq!(fit.stretches[0].kind, StretchKind::AtShiftLimit);
}
}

t! {
fn banks_in_another_utm_zone_than_the_scene_are_refused() {
    let mut b = mask_banks();
    b.utm = Utm { zone: 30, south: false };
    let e = fit_banks(&b, &scene(None), &FitParams::default()).unwrap_err().to_string();
    assert!(e.contains("one zone"), "{e}");
}
}

mod recorded {
    //! The recorded window: the lower Sassandra near Sassandra town, JRC mask banks, then Sentinel-2.
    use super::*;
    use owd_format::design::Session;
    use owd_reach::ReplayFetch;
    use owd_reach::banks::{banks_from_jrc, record_banks};
    use owd_reach::edgefit::{SceneQuery, fit_to_scene, record_edge_fit};
    use owd_reach::jrc::Tile;

    const JRC: &[u8] =
        include_bytes!("../../../examples/reach/jrc_occurrence_sassandra_estuary.tif");
    const STAC: &str = include_str!("../../../examples/reach/stac_search_sassandra.json");

    fn s2_fetch() -> ReplayFetch {
        let base = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/29/N/RF/2023/12/S2B_29NRF_20231223_0_L2A";
        let mut f = ReplayFetch::new()
            .with_response_containing("earth-search.aws.element84.com/v1/search", STAC)
            .with_response(
                &Tile {
                    west: -10,
                    north: 10,
                }
                .url(),
                JRC.to_vec(),
            );
        let bands: [(&str, &[u8]); 5] = [
            (
                "B03",
                include_bytes!("../../../examples/reach/s2_sassandra_B03.tif"),
            ),
            (
                "B04",
                include_bytes!("../../../examples/reach/s2_sassandra_B04.tif"),
            ),
            (
                "B08",
                include_bytes!("../../../examples/reach/s2_sassandra_B08.tif"),
            ),
            (
                "B11",
                include_bytes!("../../../examples/reach/s2_sassandra_B11.tif"),
            ),
            (
                "SCL",
                include_bytes!("../../../examples/reach/s2_sassandra_SCL.tif"),
            ),
        ];
        // The window was cut from the 23 December 2023 scene. The other scenes in the search answer have
        // no recorded bands: asking for them fails the test.
        for (name, bytes) in bands {
            f = f.with_response(&format!("{base}/{name}.tif"), bytes.to_vec());
        }
        f
    }

    t! {
    fn a_recorded_scene_moves_the_mask_banks_and_never_by_more_than_the_largest_shift() {
        let f = s2_fetch();
        // The route of the bank test (tests/banks.rs): up the river arm from the lagoon.
        let route = [[-6.0715, 4.966], [-6.0715, 4.9703], [-6.0705, 4.974], [-6.0697, 4.977], [-6.0695, 4.98], [-6.07, 4.983]];
        let banks = banks_from_jrc(&f, &route, &BankParams::default()).unwrap();
        let q = SceneQuery {
            endpoint: owd_reach::sentinel2::SEARCH_ENDPOINT.to_string(),
            from: "2023-12-01".into(),
            to: "2024-03-01".into(),
            months: vec![12, 1, 2],
            max_cloud_pct: 10.0,
        };
        let p = FitParams::default();
        let fitted = fit_to_scene(&f, &banks, &p, &q).unwrap();
        // The clearest dry-season scene in the answer, and only its bands were read.
        assert_eq!(fitted.scene.id, "S2B_29NRF_20231223_0_L2A");
        assert_eq!(fitted.scene.date, "2023-12-23");
        let fit = &fitted.fit;
        let shifts: Vec<f64> = fit.left_shift.iter().chain(&fit.right_shift).copied().collect();
        // Never more than the largest shift, as recorded and as drawn: every fitted vertex lies within
        // 30 m of its mask bank, outside the stretches the bank checks repaired (spec 0013).
        assert!(shifts.iter().all(|d| d.abs() <= p.max_shift_m + 1e-9));
        let near = |line: &[[f64; 2]], q: [f64; 2]| {
            line.windows(2).map(|w| {
                let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
                let l2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
                let t = (((q[0] - w[0][0]) * d[0] + (q[1] - w[0][1]) * d[1]) / l2).clamp(0.0, 1.0);
                ((q[0] - w[0][0] - t * d[0]).powi(2) + (q[1] - w[0][1] - t * d[1]).powi(2)).sqrt()
            }).fold(f64::MAX, f64::min)
        };
        // Spec 0013: outside the stretches the bank checks repaired, which say so. The window opens
        // into a lagoon, whose mouth is an outward excursion of the bank and is cut; the repaired
        // stretches stay under a third of the centreline.
        // The length the repairs moved (flag-only checks left out), counted once where stretches overlap.
        let mut moved_by: Vec<(f64, f64)> = fit.stretches.iter().filter(|s| matches!(s.kind, StretchKind::Repaired(k) if k.side || k.missing || k.jump || k.width_wide || k.excursion)).map(|s| (s.from_m, s.to_m)).collect();
        moved_by.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut repaired_m, mut reach) = (0.0, f64::MIN);
        for (a, b) in moved_by {
            if b > reach {
                repaired_m += b - a.max(reach);
                reach = b;
            }
        }
        let length = fit.centreline.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum::<f64>();
        assert!(repaired_m * 3.0 < length, "{repaired_m} m of {length} m repaired: {:?}", fit.stretches);
        let cs: Vec<f64> = fit.centreline.windows(2).scan(0.0, |a, w| {
            *a += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            Some(*a)
        }).collect();
        let cs: Vec<f64> = std::iter::once(0.0).chain(cs).collect();
        let repaired = |q: [f64; 2]| {
            let at = owd_reach::bankcheck::chainage_on(&fit.centreline, &cs, q);
            fit.stretches.iter().any(|s| matches!(s.kind, StretchKind::Repaired(_)) && at >= s.from_m - 10.0 && at <= s.to_m + 10.0)
        };
        for (fitted, mask) in [(&fit.left, &banks.left), (&fit.right, &banks.right)] {
            let kept: Vec<&[f64; 2]> = fitted.iter().filter(|q| !repaired(**q)).collect();
            let worst = kept.iter().map(|q| near(mask, **q)).fold(0.0, f64::max);
            assert!(worst <= p.max_shift_m + 0.01, "a fitted bank is {worst} m from its mask bank");
        }
        // And they moved: the mask's 75 % line lies inside the dry-season water, so the fit moves the
        // banks out, most of them by metres, not centimetres.
        let moved = shifts.iter().filter(|d| d.abs() > 2.0).count();
        assert!(moved * 2 > shifts.len(), "only {moved} of {} stations moved", shifts.len());
        let outward = shifts.iter().filter(|d| **d > 2.0).count();
        assert!(outward * 10 > moved * 9, "{outward} of {moved} moved outward");
        // Where the water runs on past 30 m the bank moves by the full 30 m and the stretch says so.
        assert!(fit.stretches.iter().any(|s| s.kind == StretchKind::AtShiftLimit), "{:?}", fit.stretches);
        assert!(fit.stretches.iter().all(|s| s.kind != StretchKind::NoEdge), "{:?}", fit.stretches);
        // Native and wasm give the same banks: the summed shift, to the ADR 0005 tolerance.
        // Label: snapshot (what this code printed for the recorded window).
        let total: f64 = shifts.iter().sum();
        assert!((total - SNAPSHOT_TOTAL_SHIFT_M).abs() < 0.01, "summed shift {total}");
        assert_eq!((fit.left.len(), fit.right.len()), SNAPSHOT_STATIONS);
        // The document: the four inputs, from the scene, with the Sentinel notice and the scene id.
        let mut s = Session::new("Edge fit test", "2026-10-21");
        record_banks(&mut s, &banks).unwrap();
        record_edge_fit(&mut s, &fitted).unwrap();
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
            let pr = s.provenance(input).unwrap();
            assert_eq!(pr["source"], "sentinel2-l2a");
            assert_eq!(pr["date"], "2023-12-23");
            assert_eq!(pr["mark"], "default");
            assert_eq!(pr["settings"]["scene_id"], "S2B_29NRF_20231223_0_L2A");
            let notice = pr["notice"].as_str().unwrap();
            assert!(notice.contains("Contains modified Copernicus Sentinel data 2023"), "{notice}");
            assert!(notice.contains("EC JRC/Google"), "{notice}");
        }
        let lc = s.low_confidence_width().unwrap();
        assert_eq!(lc["source"], "sentinel2-l2a");
        assert_eq!(lc["min_width_m"], 30.0);
        assert!(Session::open(&s.to_text()).unwrap().validate().is_empty());
    }
    }

    const SNAPSHOT_TOTAL_SHIFT_M: f64 = 20_358.091;
    const SNAPSHOT_STATIONS: (usize, usize) = (293, 434);
}

mod recorded_tiles {
    //! The recorded window across two Sentinel-2 tiles: 22 km of the Sassandra from 5.30 to 5.45 N,
    //! where tile 29NQF alone covers the south, 29NQG alone the north, and both the middle
    //! (`tools/make_s2_tiles_fixture.py`). Both scenes are of 23 December 2023.
    use super::*;
    use owd_format::design::Session;
    use owd_reach::ReplayFetch;
    use owd_reach::banks::{banks_from_jrc, record_banks};
    use owd_reach::edgefit::{SceneQuery, fit_to_scene, record_edge_fit};
    use owd_reach::jrc::Tile;
    use owd_reach::sentinel2::parse_scenes;

    const ROUTE: &str = include_str!("../../../examples/reach/sassandra_tiles_route.json");
    const JRC: &[u8] = include_bytes!("../../../examples/reach/jrc_occurrence_sassandra_tiles.tif");
    const STAC: &str = include_str!("../../../examples/reach/stac_search_sassandra_tiles.json");

    macro_rules! band {
        ($tile:literal, $band:literal) => {
            include_bytes!(concat!(
                "../../../examples/reach/s2tiles_",
                $tile,
                "_",
                $band,
                ".tif"
            ))
        };
    }

    fn fetch() -> ReplayFetch {
        let mut f = ReplayFetch::new()
            .with_response_containing("earth-search.aws.element84.com/v1/search", STAC)
            .with_response(
                &Tile {
                    west: -10,
                    north: 10,
                }
                .url(),
                JRC.to_vec(),
            );
        let files: [(&str, &str, &[u8]); 10] = [
            ("29NQF", "B03", band!("29NQF", "B03")),
            ("29NQF", "B04", band!("29NQF", "B04")),
            ("29NQF", "B08", band!("29NQF", "B08")),
            ("29NQF", "B11", band!("29NQF", "B11")),
            ("29NQF", "SCL", band!("29NQF", "SCL")),
            ("29NQG", "B03", band!("29NQG", "B03")),
            ("29NQG", "B04", band!("29NQG", "B04")),
            ("29NQG", "B08", band!("29NQG", "B08")),
            ("29NQG", "B11", band!("29NQG", "B11")),
            ("29NQG", "SCL", band!("29NQG", "SCL")),
        ];
        for sc in parse_scenes(STAC.as_bytes()).unwrap() {
            let tile = sc.id.split('_').nth(1).unwrap().to_string();
            for (url, band) in [
                (&sc.green, "B03"),
                (&sc.red, "B04"),
                (&sc.nir, "B08"),
                (&sc.swir, "B11"),
            ]
            .into_iter()
            .chain(sc.scl.as_ref().map(|u| (u, "SCL")))
            {
                let bytes = files
                    .iter()
                    .find(|(t, b, _)| *t == tile && *b == band)
                    .unwrap()
                    .2;
                f = f.with_response(url, bytes.to_vec());
            }
        }
        f
    }

    t! {
    fn a_reach_across_two_tiles_is_fitted_in_parts_each_on_a_scene_that_covers_it() {
        let f = fetch();
        let v: serde_json::Value = serde_json::from_str(ROUTE).unwrap();
        let route: Vec<[f64; 2]> = serde_json::from_value(v["route"].clone()).unwrap();
        // At the default 75 % the JRC mask sees almost no water on this reach (the region is cloudy
        // more often than not, so occurrence stays low): the worked example uses 30 %, and so does this.
        let mask = BankParams { threshold_pct: 30.0, ..BankParams::default() };
        let banks = banks_from_jrc(&f, &route, &mask).unwrap();
        let q = SceneQuery {
            endpoint: owd_reach::sentinel2::SEARCH_ENDPOINT.to_string(),
            from: "2023-12-01".into(),
            to: "2024-03-01".into(),
            months: vec![12, 1, 2],
            max_cloud_pct: 10.0,
        };
        // Parts of 6 km, so the 22 km reach has several, south to north.
        let p = FitParams { part_m: 6_000.0, ..FitParams::default() };
        let fitted = fit_to_scene(&f, &banks, &p, &q).unwrap();
        assert!(fitted.parts.len() >= 3, "{:?}", fitted.parts);
        let used: std::collections::BTreeSet<&str> = fitted
            .parts
            .iter()
            .filter(|x| x.note.is_none())
            .filter_map(|x| x.scene.as_ref())
            .map(|sc| sc.id.as_str())
            .collect();
        assert_eq!(
            used.into_iter().collect::<Vec<_>>(),
            ["S2B_29NQF_20231223_0_L2A", "S2B_29NQG_20231223_0_L2A"],
            "{:?}",
            fitted.parts
        );
        // Each part's scene covers it: the part's banks lie inside the scene's footprint.
        let ll = banks.utm.inverse_all(&banks.centreline).unwrap();
        let cs: Vec<f64> = std::iter::once(0.0)
            .chain(banks.centreline.windows(2).scan(0.0, |a, w| {
                *a += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
                Some(*a)
            }))
            .collect();
        for part in fitted.parts.iter().filter(|x| x.note.is_none()) {
            let b = part.scene.as_ref().unwrap().bbox.unwrap();
            for (q, c) in ll.iter().zip(&cs) {
                if *c >= part.from_m && *c <= part.to_m {
                    assert!(q[0] >= b[0] && q[0] <= b[2] && q[1] >= b[1] && q[1] <= b[3], "{part:?} {q:?}");
                }
            }
        }
        // The joins keep the limits: no shift over 30 m, and no change over 1.5 m per 5 m from one
        // station to the next, along each whole bank. The stations split the bank evenly, so they
        // are a little over 5 m apart, and the limit scales with the spacing (to the millimetre).
        for (shifts, bank) in [(&fitted.fit.left_shift, &banks.left), (&fitted.fit.right_shift, &banks.right)] {
            assert!(shifts.iter().all(|d| d.abs() <= p.max_shift_m + 1e-9));
            let len: f64 = bank.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum();
            let ds = len / (len / p.station_step_m).floor();
            let cap = (p.max_step_m * ds / p.station_step_m * 1000.0).floor() / 1000.0;
            assert_eq!(shifts.len() as f64, (len / p.station_step_m).floor() + 1.0);
            for w in shifts.windows(2) {
                assert!((w[1] - w[0]).abs() <= cap + 1e-9, "{w:?} over {cap}");
            }
        }
        // And the banks moved: the mask's 75 % line lies inside the dry-season water here too.
        let moved = fitted.fit.left_shift.iter().chain(&fitted.fit.right_shift).filter(|d| d.abs() > 2.0).count();
        assert!(moved * 2 > fitted.fit.left_shift.len() + fitted.fit.right_shift.len(), "only {moved} moved");
        // The provenance lists every scene used.
        let mut s = Session::new("Tiles", "2026-10-22");
        record_banks(&mut s, &banks).unwrap();
        record_edge_fit(&mut s, &fitted).unwrap();
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        let pr = s.provenance("left_bank").unwrap();
        let ids: Vec<&str> = pr["settings"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x["scene_id"].as_str())
            .collect();
        assert!(ids.contains(&"S2B_29NQF_20231223_0_L2A") && ids.contains(&"S2B_29NQG_20231223_0_L2A"), "{ids:?}");
        assert_eq!(pr["date"], "2023-12-23");
        assert_eq!(pr["notice"].as_str().unwrap().matches("Contains modified Copernicus Sentinel data 2023").count(), 1);
    }
    }
}

t! {
fn a_river_too_narrow_for_the_scene_is_not_fitted_and_says_why() {
    // #361: on a river about 40 m wide (four 10 m pixels) the fit made the banks worse. Below the
    // width it handles, it is not run: the mask banks are kept, and the reason says the width.
    use owd_reach::ReplayFetch;
    use owd_reach::edgefit::{MIN_EDGE_FIT_WIDTH_M, SceneQuery, edge_fit_skip_reason, fit_to_scene};
    let mut narrow = mask_banks();
    narrow.widths = vec![40.0; narrow.widths.len()];
    let why = edge_fit_skip_reason(&narrow).expect("a reason");
    assert!(why.contains("40 m") && why.contains(&format!("{MIN_EDGE_FIT_WIDTH_M} m")), "{why}");
    assert!(why.contains("mask banks are kept"), "{why}");
    assert!(edge_fit_skip_reason(&mask_banks()).is_none());
    // `fit_to_scene` refuses it with the same words, before any request.
    let f = ReplayFetch::new();
    let q = SceneQuery {
        endpoint: owd_reach::sentinel2::SEARCH_ENDPOINT.to_string(),
        from: "2023-12-01".into(),
        to: "2024-03-01".into(),
        months: vec![12, 1, 2],
        max_cloud_pct: 10.0,
    };
    let e = fit_to_scene(&f, &narrow, &FitParams::default(), &q).unwrap_err();
    assert!(e.to_string().contains(&why), "{e}");
    assert!(f.requests().is_empty());
}
}
