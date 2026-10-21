//! Bank lines from a water mask of known shape: a straight river 12 pixels wide with a 2-pixel throat.
//! The raster is a synthetic occurrence window (0 or 100 %) at the JRC pixel size, 0.00025 degrees, at
//! 5 degrees north, where a pixel is 27.8 m north-south and 27.7 m east-west.
//!
//! Label: property (the expected widths are worked out from the pixel size above)

use owd_format::design::Session;
use owd_reach::ReachError;
use owd_reach::banks::{BankParams, StretchKind, banks_from_mask, record_banks};
use owd_reach::jrc::Occurrence;

macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

const STEP: f64 = 0.00025;
const PX_EW: f64 = 111_320.0 * 0.996_194_7 * STEP; // 27.72 m: one pixel east-west at 5 deg N
const WEST: f64 = -6.1;
const NORTH: f64 = 5.04;
const THROAT: std::ops::Range<usize> = 250..290;

/// River along lat 5.0: rows 154..166 are water (12 pixels), but only rows 159..161 in the throat.
fn river() -> Occurrence {
    Occurrence::from_fn(WEST, NORTH, [STEP, STEP], 600, 320, |c, r| {
        let in_throat = THROAT.contains(&c);
        let water = if in_throat {
            (159..161).contains(&r)
        } else {
            (154..166).contains(&r)
        };
        if water { 100 } else { 0 }
    })
}

/// A route along the river, 33 m north of its middle, from col 20 to col 580.
fn route() -> Vec<[f64; 2]> {
    (0..=56)
        .map(|i| [WEST + (20.0 + 10.0 * i as f64) * STEP, 5.0003])
        .collect()
}

t! {
fn a_mask_of_known_width_gives_that_width_within_one_pixel() {
    let b = banks_from_mask(&route(), &river(), &BankParams::default()).unwrap();
    // 12 pixels of water, less half a pixel: the 75 % edge sits a quarter pixel inside each end.
    let wide: Vec<f64> = b
        .centreline
        .iter()
        .zip(&b.widths)
        .enumerate()
        .filter(|(i, _)| *i > 40 && *i < b.widths.len() - 40 && (*i < 250 || *i > 400)) // away from the throat (stations 318 to 374)
        .map(|(_, (_, w))| *w)
        .collect();
    assert!(wide.len() > 100);
    for w in &wide {
        assert!((w - 11.5 * PX_EW).abs() < PX_EW, "width {w} m, expected about {} m", 11.5 * PX_EW);
    }
    // The left bank is north of the right bank (the route runs east, so left is north).
    assert!(b.left.iter().zip(&b.right).all(|(l, r)| l[1] > r[1]));
    // The corrected centreline lies on the river's middle, not on the route 33 m north of it.
    let mid = b.centreline[b.centreline.len() / 10];
    let mid_ll = owd_format::design::crs::Utm::for_point(-6.05, 5.0).inverse(mid[0], mid[1]).unwrap();
    assert!((mid_ll[1] - 5.0).abs() * 111_320.0 < 10.0, "{mid_ll:?}");
    // The polygon closes and is the two banks.
    assert_eq!(b.polygon.first(), b.polygon.last());
    assert_eq!(b.polygon.len(), b.left.len() + b.right.len() + 1);
}
}

t! {
fn the_narrow_throat_is_flagged_and_only_the_throat() {
    let b = banks_from_mask(&route(), &river(), &BankParams::default()).unwrap();
    assert_eq!(b.stretches.len(), 1, "{:?}", b.stretches);
    let s = &b.stretches[0];
    assert_eq!(s.kind, StretchKind::Narrow);
    // The throat is 40 pixels, 1109 m, starting 250 - 20 = 230 pixels (6.38 km) from the route start;
    // the stations (20 m) and the bank search (up to a few pixels) blur each end by under 100 m.
    let from = (250.0 - 20.0) * PX_EW;
    let to = (290.0 - 20.0) * PX_EW;
    assert!((s.from_m - from).abs() < 100.0, "from {} expected {}", s.from_m, from);
    assert!((s.to_m - to).abs() < 100.0, "to {} expected {}", s.to_m, to);
    // 2 pixels of water, less half a pixel: 1.5 pixels, about 42 m, well under 90 m.
    assert!(s.min_width_m < 60.0 && s.min_width_m > 25.0, "{}", s.min_width_m);
}
}

t! {
fn a_river_with_no_water_at_the_threshold_is_refused_with_a_hint() {
    let dry = Occurrence::from_fn(WEST, NORTH, [STEP, STEP], 600, 320, |_, r| if (154..166).contains(&r) { 60 } else { 0 });
    let e = banks_from_mask(&route(), &dry, &BankParams::default()).unwrap_err();
    assert!(matches!(e, ReachError::NoWater(_)));
    assert!(e.to_string().contains("75%") && e.to_string().contains("Lower the threshold"), "{e}");
    // The same river at a lower threshold (a provisional default the user may change) works.
    let p = BankParams { threshold_pct: 50.0, ..BankParams::default() };
    assert!(banks_from_mask(&route(), &dry, &p).is_ok());
}
}

t! {
fn a_one_pixel_line_of_land_across_the_river_does_not_hide_it() {
    // A one-pixel line of land across the river at col 100 (a mask artefact), and a bank that steps.
    let r = Occurrence::from_fn(WEST, NORTH, [STEP, STEP], 600, 320, |c, row| {
        if (154..166).contains(&row) && c != 100 { 100 } else { 0 }
    });
    let b = banks_from_mask(&route(), &r, &BankParams::default()).unwrap();
    assert!(b.stretches.is_empty(), "{:?}", b.stretches);
}
}

t! {
fn the_document_gets_the_inputs_marked_default_with_the_jrc_notice_and_provisional_settings() {
    let b = banks_from_mask(&route(), &river(), &BankParams::default()).unwrap();
    let mut s = Session::new("Banks test", "2026-10-21");
    record_banks(&mut s, &b).unwrap();
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
        let p = s.provenance(input).unwrap();
        assert_eq!(p["mark"], "default");
        assert_eq!(p["source"], "jrc-gsw");
        assert_eq!(p["provisional"], true);
        assert_eq!(p["settings"]["mask_threshold_pct"], 75.0);
        assert_eq!(p["settings"]["provisional"], serde_json::json!(["mask_threshold_pct", "min_trusted_width_m"]));
    }
    assert!(s.notices()[0].contains("Pekel et al. 2016") && s.notices()[0].contains("EC JRC/Google"));
    assert_eq!(s.low_confidence_width().unwrap()["stretches"].as_array().unwrap().len(), 1);
    assert_eq!(s.low_confidence_width().unwrap()["min_width_m"], 90.0);
    // Round trip.
    let back = Session::open(&s.to_text()).unwrap();
    assert!(back.validate().is_empty());
}
}

const SASSANDRA: &[u8] =
    include_bytes!("../../../examples/reach/jrc_occurrence_sassandra_estuary.tif");

t! {
fn a_recorded_window_of_the_lower_sassandra_gives_banks_and_flags_the_thin_stretch() {
    use owd_reach::banks::banks_from_jrc;
    use owd_reach::jrc::Tile;
    // A real window of JRC occurrence (examples/reach/jrc_occurrence_sassandra_estuary.tif), standing in
    // for the tile `10W_10N`. The route runs up the river arm from the lagoon: wide water, then a thin
    // streak 2 to 4 pixels wide, then none at 75 % (the river is there, but seen as water less often).
    let f = owd_reach::ReplayFetch::new().with_response(&Tile { west: -10, north: 10 }.url(), SASSANDRA.to_vec());
    let route = [[-6.0715, 4.966], [-6.0715, 4.9703], [-6.0705, 4.974], [-6.0697, 4.977], [-6.0695, 4.98], [-6.07, 4.983]];
    let b = banks_from_jrc(&f, &route, &BankParams::default()).unwrap();
    // Wide water at the start, none flagged there.
    assert!(b.widths[..40].iter().all(|w| *w >= 90.0), "{:?}", &b.widths[..40]);
    // One thin stretch (narrow, under 90 m), then the end of the water at 75 %.
    let kinds: Vec<StretchKind> = b.stretches.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [StretchKind::Narrow, StretchKind::NoWater, StretchKind::NoWater], "{:?}", b.stretches);
    assert!(b.stretches[0].min_width_m < 90.0 && b.stretches[0].min_width_m > 0.0);
    assert!(b.stretches.windows(2).all(|w| w[0].to_m <= w[1].from_m));
    // Banks stay within the largest search distance of the water; the polygon closes.
    assert_eq!(b.polygon.first(), b.polygon.last());
    assert!(b.left.iter().zip(&b.right).all(|(l, r)| ((l[0] - r[0]).powi(2) + (l[1] - r[1]).powi(2)).sqrt() <= 3_000.0 + 1.0));
    // Read by window: the directory and the few tiles of the window, nothing else.
    assert!(f.requests().len() <= 12, "{} requests", f.requests().len());
}
}
