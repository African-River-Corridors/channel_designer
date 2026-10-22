//! The water grid (spec 0013, ticket T2): a straight river 200 m wide on synthetic 10 m scenes, with
//! clouds where each test puts them.
//!
//! Label: property (the water and land colours are made up; the index and the threshold decide)

use owd_format::design::crs::Utm;
use owd_reach::sentinel2::Imagery;
use owd_reach::water::{
    DEFAULT_MAX_SCENES, DEFAULT_MNDWI_THRESHOLD, Origin, SceneWindow, State, compose, mndwi,
};

macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

const UTM: Utm = Utm {
    zone: 29,
    south: false,
};
const X0: f64 = 700_000.0;
const Y1: f64 = 600_600.0;
const COLS: usize = 300;
const ROWS: usize = 120;
/// The river: y from 600_200 to 600_400 (rows 20 to 40), all along x.
const RIVER: std::ops::Range<usize> = 20..40;

/// Green, red, NIR, SWIR of water and land (MNDWI +0.71 and -0.60).
const WATER: [f32; 4] = [0.060, 0.050, 0.020, 0.010];
const LAND: [f32; 4] = [0.050, 0.040, 0.350, 0.200];

/// A scene; `cloud(col, row)` says where it cannot see.
fn scene(cloud: impl Fn(usize, usize) -> bool) -> Imagery {
    let mut bands: [Vec<f32>; 4] = Default::default();
    let mut valid = Vec::new();
    for r in 0..ROWS {
        for c in 0..COLS {
            let v = if RIVER.contains(&r) { WATER } else { LAND };
            for k in 0..4 {
                bands[k].push(v[k]);
            }
            valid.push(!cloud(c, r));
        }
    }
    Imagery::from_bands(UTM, X0, Y1, 10.0, COLS, ROWS, bands, valid)
}

/// The route: along the river's middle.
fn route() -> Vec<[f64; 2]> {
    vec![[X0, Y1 - 300.0], [X0 + 3_000.0, Y1 - 300.0]]
}

fn pt(c: usize, r: usize) -> [f64; 2] {
    [X0 + (c as f64 + 0.5) * 10.0, Y1 - (r as f64 + 0.5) * 10.0]
}

fn win<'a>(id: &'a str, cloud_pct: f64, im: &'a Imagery) -> SceneWindow<'a> {
    SceneWindow {
        id,
        date: "2025-01-26",
        cloud_pct,
        imagery: im,
    }
}

t! {
fn clear_pixels_are_classified_by_the_index() {
    let im = scene(|_, _| false);
    let idx = mndwi(&im);
    assert!((idx[30 * COLS + 5] - 0.714_285_7).abs() < 1e-6, "{}", idx[30 * COLS + 5]);
    assert!((idx[5] + 0.6).abs() < 1e-6, "{}", idx[5]);
    let g = compose(&[win("a", 1.0, &im)], &route(), 250.0, DEFAULT_MNDWI_THRESHOLD, DEFAULT_MAX_SCENES, None).unwrap();
    for r in 0..ROWS {
        let want = if RIVER.contains(&r) { State::Water } else { State::Land };
        assert_eq!(g.at(pt(150, r)), (want, Origin::Scene(0)), "row {r}");
    }
    assert_eq!(g.corridor_unknown, 0.0);
}
}

t! {
fn a_cloud_in_the_best_scene_is_filled_from_the_next_scene_not_from_jrc() {
    // Scene a sees all the river but x = 1000 to 1500 m; scene b is cloudy elsewhere but sees there.
    let a = scene(|c, _| (100..150).contains(&c));
    let b = scene(|c, _| !(80..170).contains(&c));
    let jrc = |_: [f64; 2]| Some(false);
    let g = compose(&[win("b", 1.0, &b), win("a", 9.0, &a)], &route(), 250.0, 0.0, 3, Some(&jrc)).unwrap();
    // a sees more of the corridor, so it comes first despite its higher overall cloud.
    assert_eq!(g.scenes[0].id, "a");
    assert_eq!(g.scenes[1].id, "b");
    assert_eq!(g.at(pt(120, 30)), (State::Water, Origin::Scene(1)));
    assert_eq!(g.at(pt(120, 5)), (State::Land, Origin::Scene(1)));
    assert_eq!(g.at(pt(20, 30)), (State::Water, Origin::Scene(0)));
    assert_eq!(g.corridor_jrc, 0.0);
    assert_eq!(g.corridor_unknown, 0.0);
}
}

t! {
fn under_cloud_in_every_scene_the_jrc_mask_fills_in_and_says_so() {
    let a = scene(|c, _| (100..150).contains(&c));
    let b = scene(|c, _| (100..150).contains(&c));
    // The JRC mask sees the river 100 m narrower (rows 25 to 35).
    let jrc = |p: [f64; 2]| Some((Y1 - 350.0..Y1 - 250.0).contains(&p[1]));
    let g = compose(&[win("a", 1.0, &a), win("b", 2.0, &b)], &route(), 250.0, 0.0, 3, Some(&jrc)).unwrap();
    assert_eq!(g.at(pt(120, 30)), (State::Water, Origin::Jrc));
    assert_eq!(g.at(pt(120, 22)), (State::Land, Origin::Jrc));
    assert!(g.corridor_jrc > 0.1 && g.corridor_jrc < 0.25, "{}", g.corridor_jrc);
    // Without a JRC fill the pixels stay unknown, and the share says how many.
    let g = compose(&[win("a", 1.0, &a), win("b", 2.0, &b)], &route(), 250.0, 0.0, 3, None).unwrap();
    assert_eq!(g.at(pt(120, 30)), (State::Unknown, Origin::None));
    assert!((g.corridor_unknown - 50.0 / 300.0).abs() < 0.02, "{}", g.corridor_unknown);
    // b adds nothing a did not see, so only a is listed.
    assert_eq!(g.scenes.len(), 1);
}
}

t! {
fn the_scene_that_sees_the_river_ranks_above_the_one_with_less_cloud_overall() {
    // a: 2 % cloud overall, all of it on the river at x = 0 to 900 m. b: 10 % cloud, all far off it.
    let a = scene(|c, r| c < 90 && RIVER.contains(&r));
    let b = scene(|_, r| r >= 108);
    let g = compose(&[win("a", 2.0, &a), win("b", 10.0, &b)], &route(), 250.0, 0.0, 1, None).unwrap();
    assert_eq!(g.scenes.len(), 1);
    assert_eq!(g.scenes[0].id, "b");
    assert_eq!(g.scenes[0].corridor_clear, 1.0);
    assert_eq!(g.corridor_unknown, 0.0);
}
}

t! {
fn scenes_on_different_windows_are_refused() {
    let a = scene(|_, _| false);
    let mut b = scene(|_, _| false);
    b.x0 += 20.0;
    let e = compose(&[win("a", 1.0, &a), win("b", 1.0, &b)], &route(), 250.0, 0.0, 3, None).unwrap_err();
    assert!(e.contains("same window"), "{e}");
}
}

#[cfg(not(target_arch = "wasm32"))]
mod recorded {
    //! Label: property
    use super::*;
    use owd_reach::ReplayFetch;
    use owd_reach::sentinel2::{parse_scenes, read_clear, read_imagery};

    const STAC: &str = include_str!("../../../examples/reach/stac_search_sassandra.json");

    #[test]
    fn a_recorded_scene_sees_the_lagoon_as_water_and_the_clear_mask_matches_the_imagery() {
        let scenes = parse_scenes(STAC.as_bytes()).unwrap();
        let s = scenes
            .iter()
            .find(|s| s.id == "S2B_29NRF_20231223_0_L2A")
            .unwrap();
        let mut f = ReplayFetch::new();
        for (url, file) in [
            (
                &s.green,
                &include_bytes!("../../../examples/reach/s2_sassandra_B03.tif")[..],
            ),
            (
                &s.red,
                &include_bytes!("../../../examples/reach/s2_sassandra_B04.tif")[..],
            ),
            (
                &s.nir,
                &include_bytes!("../../../examples/reach/s2_sassandra_B08.tif")[..],
            ),
            (
                &s.swir,
                &include_bytes!("../../../examples/reach/s2_sassandra_B11.tif")[..],
            ),
            (
                s.scl.as_ref().unwrap(),
                &include_bytes!("../../../examples/reach/s2_sassandra_SCL.tif")[..],
            ),
        ] {
            f = f.with_response(url, file.to_vec());
        }
        // The window of the edge-fit test: the river arm from the lagoon, UTM 29N.
        let u = UTM;
        let pts = u
            .forward_all(&[[-6.0735, 4.964], [-6.0675, 4.985]])
            .unwrap();
        let im = read_imagery(&f, s, pts[0], pts[1]).unwrap();
        let clear = read_clear(&f, s, pts[0], pts[1]).unwrap();
        // Every pixel the imagery calls clear, the classification alone does too (the imagery also
        // drops pixels with no data in a band).
        assert_eq!(clear.len(), im.valid.len());
        assert!(im.valid.iter().zip(&clear).all(|(v, c)| !*v || *c));
        assert!(clear.iter().filter(|c| **c).count() * 10 >= clear.len() * 9);
        let route = u
            .forward_all(&[[-6.0715, 4.966], [-6.0715, 4.9703]])
            .unwrap();
        let g = compose(&[win(&s.id, s.cloud_pct, &im)], &route, 50.0, 0.0, 3, None).unwrap();
        // The lagoon end of the route is open water; most of its corridor is water.
        let corr: Vec<usize> = (0..g.state.len()).filter(|i| g.corridor[*i]).collect();
        let water = corr.iter().filter(|i| g.state[**i] == State::Water).count();
        assert!(
            water * 10 >= corr.len() * 9,
            "{water} of {} corridor pixels water",
            corr.len()
        );
    }
}
