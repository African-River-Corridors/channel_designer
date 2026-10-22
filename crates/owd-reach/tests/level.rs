//! The default water level on the recorded lower Sassandra: five SWORD reaches from the subset of the
//! v17c release, their Hydrocron time series, and a window of the Copernicus GLO-30 DEM
//! (`tools/make_level_fixture.py`).

use owd_format::design::Session;
use owd_format::design::crs::Utm;
use owd_format::design::provenance::Mark;
use owd_reach::ReplayFetch;
use owd_reach::level::{
    Datum, LevelInputs, LevelParams, Observation, Source, default_level, level_from_sources,
    never_rising, parse_hydrocron, percentile, record_level,
};
use owd_reach::sword::Reach;

macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

const SUBSET: &[u8] = include_bytes!("../../../examples/reach/sword_v17c_shp_subset.zip");
const DEM: &[u8] = include_bytes!("../../../examples/reach/glo30_sassandra_lower.tif");
const REACHES: [(&str, &str); 5] = [
    (
        "14560100021",
        include_str!("../../../examples/reach/hydrocron_14560100021.json"),
    ),
    (
        "14560100033",
        include_str!("../../../examples/reach/hydrocron_14560100033.json"),
    ),
    (
        "14560100041",
        include_str!("../../../examples/reach/hydrocron_14560100041.json"),
    ),
    (
        "14560100051",
        include_str!("../../../examples/reach/hydrocron_14560100051.json"),
    ),
    (
        "14560100061",
        include_str!("../../../examples/reach/hydrocron_14560100061.json"),
    ),
];
const RECORD: &str = "https://zenodo.example/api/records/sword";
const ZIP: &str = "https://zenodo.example/sword.zip";
const HYDROCRON: &str = "https://hydrocron.example/timeseries";
const UTM: Utm = Utm {
    zone: 29,
    south: false,
};

fn fetch() -> ReplayFetch {
    let record = format!(
        r#"{{"files":[{{"key":"SWORD_v17c_shp.zip","size":{},"links":{{"self":"{ZIP}"}}}}]}}"#,
        SUBSET.len()
    );
    let mut f = ReplayFetch::new()
        .with_response(RECORD, record)
        .with_response(ZIP, SUBSET.to_vec());
    for (id, body) in REACHES {
        f = f.with_response_containing(&format!("feature_id={id}&"), body);
    }
    f
}

/// The river centreline: the five reaches' lines end to end, from the mouth upstream, in UTM 29N.
fn centreline() -> Vec<[f64; 2]> {
    let reaches = owd_reach::sword::reaches_in(&fetch(), RECORD, [-6.4, 4.9, -6.0, 5.2]).unwrap();
    let mut line: Vec<[f64; 2]> = Vec::new();
    for r in &reaches {
        let mut l = r.line.clone();
        if let Some(last) = line.last() {
            let d = |p: [f64; 2]| (p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2);
            if d(l[l.len() - 1]) < d(l[0]) {
                l.reverse();
            }
        } else if (l[0][0] - -6.09).abs() > (l[l.len() - 1][0] - -6.09).abs() {
            l.reverse();
        }
        line.extend(l);
    }
    UTM.forward_all(&line).unwrap()
}

fn dem() -> impl Fn([f64; 2]) -> f64 {
    let d = owd_format::geotiff::read_bytes(DEM).unwrap();
    assert_eq!(d.epsg, Some(4326));
    move |p: [f64; 2]| d.sample_bilinear(p[0], p[1])
}

fn level(params: &LevelParams, datum: Datum) -> owd_reach::level::LevelProfile {
    let d = dem();
    level_from_sources(
        &fetch(),
        RECORD,
        HYDROCRON,
        &centreline(),
        UTM,
        ("2023-07-21", "2026-10-01"),
        Some(&d),
        params,
        datum,
    )
    .unwrap()
}

t! {
fn the_10th_percentile_of_a_recorded_reach_matches_a_hand_calculation() {
    // Label: sourced - hand calculation from the recorded Hydrocron answer of reach 14560100033.
    // Kept: reach quality 0 or 1 and a value, 49 observations. Sorted, they start
    // -0.0481, -0.004, 0.0857, 0.0933, 0.1776, 0.2627, ...
    // Rank 0.10 * (49 - 1) = 4.8, so the level is 0.1776 + 0.8 * (0.2627 - 0.1776) = 0.24568 m.
    let obs = parse_hydrocron(REACHES[1].1.as_bytes()).unwrap();
    let good: Vec<f64> = obs.iter().filter(|o| o.quality <= 1).map(|o| o.wse_m).collect();
    assert_eq!(good.len(), 49);
    assert!((percentile(&good, 10.0).unwrap() - 0.24568).abs() < 1e-9);
    // And the level profile carries it, at the middle of the reach.
    let l = level(&LevelParams::default(), Datum::Egm2008);
    let swot: Vec<&owd_reach::level::Stretch> =
        l.stretches.iter().filter(|s| matches!(s.source, Source::Swot { .. })).collect();
    assert_eq!(swot.len(), 5, "{:?}", l.stretches);
    let s = swot.iter().find(|s| matches!(&s.source, Source::Swot { reach_id, .. } if reach_id == "14560100033")).unwrap();
    let mid = (s.from_m + s.to_m) / 2.0;
    // It sits above reach 041's level upstream (0.1977 m), so the profile lowers it to that and the
    // record keeps the 10th percentile it was (#280).
    let a = l.adjusted.iter().find(|a| (a.chainage_m - mid).abs() < 1e-9).unwrap();
    assert!((a.was_m - 0.24568).abs() < 1e-9, "{a:?}");
    assert_eq!((a.source, a.direction()), ("swot", "lowered"));
    let p = l.points.iter().find(|p| (p[0] - mid).abs() < 1e-9).unwrap();
    assert_eq!(p[1], a.now_m);
    // Label: sourced - SWORD v17c's own `wseobs_p10` for reach 14560100061 is 10.87828 m (its dbf,
    // examples/reach/sword_v17c_shp_subset.zip): an independent 10th percentile of SWOT levels.
    let up = l.points.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    assert!((up - 10.87828).abs() < 0.05, "{up}");
}
}

t! {
fn the_flow_runs_toward_the_mouth_and_the_level_goes_into_the_document_as_its_profile() {
    let l = level(&LevelParams::default(), Datum::Egm2008);
    // The centreline starts at the mouth: downstream is toward smaller chainage.
    assert!(!l.downstream_is_increasing);
    assert_eq!(l.direction_from, "SWORD distance to the outlet");
    let (first, last) = l.swot_dates.clone().unwrap();
    assert!(first.as_str() >= "2023-07-21" && last.as_str() <= "2026-10-01", "{first} {last}");
    let mut s = Session::new("Level test", "2026-10-21");
    let c = centreline();
    let gj = format!(
        r#"{{"type":"LineString","coordinates":{},"crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32629"}}}}}}"#,
        serde_json::to_string(&c).unwrap()
    );
    s.import(&[(owd_format::design::Role::Centreline, gj.as_str())]).unwrap();
    record_level(&mut s, &l, Mark::Default, Some("DEM notice"), "2026-10-21").unwrap();
    // The stages read the input profile exactly as if it had been typed in.
    let (profile, from) = s.water_profile().unwrap();
    assert_eq!(from, "input");
    assert_eq!(profile, l.points);
    let at = s.water_levels_along(&c, &[0.0, 5_000.0]).unwrap();
    assert!(at.iter().all(|v| v.is_finite()), "{at:?}");
    let p = s.provenance("water_level").unwrap();
    assert_eq!(p["source"], "swot-hydrocron");
    assert_eq!(p["mark"], "default");
    assert_eq!(p["provisional"], true);
    assert_eq!(p["settings"]["percentile"], 10.0);
    assert_eq!(p["settings"]["datum"], "EGM2008");
    assert!(p["notice"].as_str().unwrap().contains("SWORD v17c"));
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    // Another percentile is the user's choice.
    let q = LevelParams { percentile: 50.0, ..LevelParams::default() };
    record_level(&mut s, &level(&q, Datum::Egm2008), Mark::User, None, "2026-10-21").unwrap();
    assert_eq!(s.provenance("water_level").unwrap()["mark"], "user");
}
}

t! {
fn where_the_reaches_are_too_narrow_the_dem_gives_a_level_that_never_rises_downstream() {
    // Label: property. With the width limit raised to 320 m, reaches 033 (300 m), 051 (296 m) and 061
    // (216 m) take the DEM; 021 and 041 keep SWOT.
    let p = LevelParams { min_swot_width_m: 320.0, ..LevelParams::default() };
    let l = level(&p, Datum::Egm2008);
    let dem: Vec<&owd_reach::level::Stretch> = l.stretches.iter().filter(|s| s.source == Source::Dem).collect();
    assert!(!dem.is_empty());
    assert!(dem.iter().any(|s| s.why.as_deref().unwrap().contains("216 m wide")), "{dem:?}");
    for s in &dem {
        // Downstream is toward smaller chainage: going up the chainage the level never falls.
        let pts: Vec<[f64; 2]> = l.points.iter().copied().filter(|q| q[0] >= s.from_m && q[0] <= s.to_m).collect();
        assert!(pts.len() > 3, "{s:?}");
        assert!(pts.windows(2).all(|w| w[1][1] >= w[0][1] - 1e-12), "{pts:?}");
    }
    let swot = l.stretches.iter().filter(|s| matches!(s.source, Source::Swot { .. })).count();
    assert_eq!(swot, 2);
    // A level the DEM alone gives records as the DEM's, dated the day it was made, with a notice.
    let all_dem = level(&LevelParams { min_swot_width_m: 5_000.0, ..LevelParams::default() }, Datum::Egm2008);
    assert!(all_dem.swot_dates.is_none());
    let mut s = Session::new("DEM only", "2026-10-21");
    for (notice, source) in [(None, "dem"), (Some("Copernicus notice"), "cop-dem-glo30")] {
        record_level(&mut s, &all_dem, Mark::Default, notice, "2026-10-21").unwrap();
        let p = s.provenance("water_level").unwrap();
        assert_eq!(p["source"], source);
        assert_eq!(p["date"], "2026-10-21");
        assert!(!p["notice"].as_str().unwrap().is_empty());
        assert!(s.validate().is_empty(), "{:?}", s.validate());
    }
}
}

t! {
fn never_rising_holds_for_any_profile() {
    // Label: property. A deterministic spread of noisy profiles: the result never rises, keeps the
    // count, and keeps the mean (pooling replaces values by their block means).
    for seed in 0..200u64 {
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let v: Vec<f64> = (0..40)
            .map(|i| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                10.0 - 0.1 * f64::from(i) + ((x >> 33) as f64 / f64::from(u32::MAX) - 0.5) * 4.0
            })
            .collect();
        let r = never_rising(&v);
        assert_eq!(r.len(), v.len());
        assert!(r.windows(2).all(|w| w[1] <= w[0] + 1e-12));
        let (a, b): (f64, f64) = (v.iter().sum(), r.iter().sum());
        assert!((a - b).abs() < 1e-9);
    }
}
}

t! {
fn the_datum_conversion_on_a_known_point() {
    // Label: sourced - the SWOT River Single-Pass product gives `wse` as the height above the geoid
    // model it carries (`geoid_hght`, EGM2008, height above the WGS84 ellipsoid). The recorded pass of
    // 15 October 2023 over reach 14560100033: wse 1.127 m, geoid_hght 27.6044704 m.
    let obs = parse_hydrocron(REACHES[1].1.as_bytes()).unwrap();
    let o: &Observation = obs.iter().find(|o| o.time == "2023-10-15T17:02:52Z").unwrap();
    assert_eq!(Datum::Egm2008.from_swot(o), 1.127);
    assert!((Datum::Ellipsoid.from_swot(o) - 28.7314704).abs() < 1e-9);
    // Label: sourced - EGM2008 geoid height at the reach's centre (4.99990817 N, 6.1244852 W) from
    // NGA's 2.5 minute EGM2008 grid (PROJ `us_nga_egm08_25.tif`): 27.479 m. SWOT's geoid agrees to
    // 0.13 m (it is a finer model of the same geoid).
    assert!((o.geoid_m - 27.479).abs() < 0.2);
    // The level follows: ellipsoidal heights are the EGM2008 ones plus about 27.6 m here.
    let (a, b) = (level(&LevelParams::default(), Datum::Egm2008), level(&LevelParams::default(), Datum::Ellipsoid));
    let lift: Vec<f64> = a.points.iter().zip(&b.points).filter(|(p, _)| p[1].is_finite()).map(|(p, q)| q[1] - p[1]).collect();
    assert!(lift.iter().any(|d| (d - 27.6).abs() < 0.05), "{lift:?}");
}
}

t! {
fn a_reach_hydrocron_does_not_know_gives_no_observations() {
    let body = br#"{"error": "400: Results with the specified Feature ID 1 were not found"}"#;
    assert!(parse_hydrocron(body).unwrap().is_empty());
    assert!(parse_hydrocron(b"{}").is_err());
}
}

/// A straight 10 km river in UTM 29N flowing toward smaller chainage: a SWORD reach at 0-3 km whose
/// SWOT level is `down`, one at 7-10 km whose level is `up`, and the DEM reading `dem` everywhere.
fn straight_river(down: f64, up: f64, dem: Option<f64>) -> owd_reach::level::LevelProfile {
    let (x0, y0) = (400_000.0, 550_000.0);
    let centre: Vec<[f64; 2]> = (0..=100).map(|k| [x0 + 100.0 * f64::from(k), y0]).collect();
    let line = |a: f64, b: f64| -> Vec<[f64; 2]> {
        (0..=10)
            .map(|k| {
                UTM.inverse(x0 + a + (b - a) * f64::from(k) / 10.0, y0)
                    .unwrap()
            })
            .collect()
    };
    let reach = |id: &str, a: f64, b: f64, dist_out_m: f64| Reach {
        id: id.into(),
        width_m: 400.0,
        dist_out_m,
        kind: 1,
        name: "Test".into(),
        line: line(a, b),
    };
    let reaches = [
        reach("down", 0.0, 3_000.0, 1_500.0),
        reach("up", 7_000.0, 10_000.0, 8_500.0),
    ];
    let observations = |id: &str| -> Result<Vec<Observation>, owd_reach::error::ReachError> {
        let wse_m = if id == "down" { down } else { up };
        Ok((0..3)
            .map(|k| Observation {
                time: format!("2024-01-0{}T00:00:00Z", k + 1),
                wse_m,
                geoid_m: 27.5,
                quality: 0,
            })
            .collect())
    };
    let d = move |_: [f64; 2]| dem.unwrap_or(f64::NAN);
    let l = default_level(
        &centre,
        UTM,
        &reaches,
        &LevelInputs {
            observations: &observations,
            dem: dem.is_some().then_some(&d as &dyn Fn([f64; 2]) -> f64),
        },
        &LevelParams::default(),
        Datum::Egm2008,
    )
    .unwrap();
    assert!(!l.downstream_is_increasing);
    // Going up the chainage (upstream) the level never falls: it never rises downstream.
    assert!(
        l.points.windows(2).all(|w| w[1][1] >= w[0][1]),
        "{:?}",
        l.points
    );
    l
}

/// The points of `l` between the two reaches of [`straight_river`]: the DEM-filled stretch.
fn filled(l: &owd_reach::level::LevelProfile) -> Vec<[f64; 2]> {
    l.points
        .iter()
        .copied()
        .filter(|p| p[0] > 3_000.0 + 1.0 && p[0] < 7_000.0 - 1.0)
        .collect()
}

t! {
fn a_dem_stretch_downstream_of_a_lower_swot_level_is_lowered_to_it() {
    // Label: sourced - hand calculation (#280). SWOT gives 0.5 m downstream and 1.0 m upstream; the
    // DEM between them reads 3.0 m, 2 m above the SWOT level just upstream of it. The level never
    // rises downstream and SWOT outranks the DEM, so every DEM point there is lowered to 1.0 m, and
    // each change is recorded.
    let l = straight_river(0.5, 1.0, Some(3.0));
    assert!(l.stretches.iter().any(|s| s.source == Source::Dem), "{:?}", l.stretches);
    let swot_up = l.points.iter().find(|p| (p[0] - 8_500.0).abs() < 1.0).unwrap();
    assert!((swot_up[1] - 1.0).abs() < 1e-12, "{swot_up:?}");
    let filled = filled(&l);
    assert!(filled.len() > 30, "{filled:?}");
    assert!(filled.iter().all(|p| p[1] == 1.0), "{filled:?}");
    // Each change is in the profile's record, and in the water-level provenance.
    assert_eq!(l.adjusted.len(), filled.len());
    assert!(l.adjusted.iter().all(|a| a.was_m == 3.0 && a.now_m == 1.0 && a.source == "dem" && a.direction() == "lowered"));
    let mut s = Session::new("Join test", "2026-10-22");
    record_level(&mut s, &l, Mark::Default, None, "2026-10-22").unwrap();
    let p = s.provenance("water_level").unwrap();
    let rec = p["settings"]["adjusted"].as_array().unwrap();
    assert_eq!(rec.len(), filled.len());
    assert_eq!(rec[0]["was_m"], 3.0);
    assert_eq!(rec[0]["now_m"], 1.0);
    assert_eq!(rec[0]["source"], "dem");
    assert_eq!(rec[0]["direction"], "lowered");
    assert!(rec[0]["why"].as_str().unwrap().contains("never rises downstream"));
    assert!(s.validate().is_empty(), "{:?}", s.validate());
}
}

t! {
fn a_dem_stretch_below_the_swot_level_downstream_is_raised_to_it() {
    // Label: sourced - hand calculation (#280). SWOT gives 1.0 m downstream and 2.0 m upstream; the
    // DEM between them reads 0.5 m, below the SWOT level just downstream of it. SWOT outranks the
    // DEM, so every DEM point there is raised to 1.0 m; the SWOT levels stay as observed.
    let l = straight_river(1.0, 2.0, Some(0.5));
    let filled = filled(&l);
    assert!(filled.len() > 30, "{filled:?}");
    assert!(filled.iter().all(|p| p[1] == 1.0), "{filled:?}");
    assert_eq!(l.adjusted.len(), filled.len());
    assert!(l.adjusted.iter().all(|a| a.was_m == 0.5 && a.now_m == 1.0 && a.source == "dem" && a.direction() == "raised"));
    let mut s = Session::new("Raise test", "2026-10-22");
    record_level(&mut s, &l, Mark::Default, None, "2026-10-22").unwrap();
    let rec = s.provenance("water_level").unwrap()["settings"]["adjusted"].clone();
    assert_eq!(rec[0]["direction"], "raised");
    assert_eq!(rec[0]["was_m"], 0.5);
    assert_eq!(rec[0]["now_m"], 1.0);
}
}

t! {
fn a_swot_level_above_the_swot_level_upstream_is_lowered_to_it() {
    // Label: sourced - hand calculation (#280). No DEM. SWOT gives 1.2 m downstream and 1.0 m
    // upstream: a 0.2 m rise toward the sea, within SWOT's noise in a tidal estuary. The level never
    // rises downstream, so the downstream level is lowered to 1.0 m and the record keeps 1.2 m.
    let l = straight_river(1.2, 1.0, None);
    assert_eq!(l.points.len(), 2, "{:?}", l.points);
    assert!(l.points.iter().all(|p| p[1] == 1.0), "{:?}", l.points);
    assert_eq!(l.adjusted.len(), 1);
    let a = &l.adjusted[0];
    assert!((a.chainage_m - 1_500.0).abs() < 1.0, "{a:?}");
    assert_eq!((a.was_m, a.now_m, a.source, a.direction()), (1.2, 1.0, "swot", "lowered"));
    let mut s = Session::new("SWOT test", "2026-10-22");
    record_level(&mut s, &l, Mark::Default, None, "2026-10-22").unwrap();
    let rec = s.provenance("water_level").unwrap()["settings"]["adjusted"].clone();
    assert_eq!(rec[0]["source"], "swot");
    assert_eq!(rec[0]["was_m"], 1.2);
    assert_eq!(rec[0]["now_m"], 1.0);
    assert!(rec[0]["why"].as_str().unwrap().contains("tidal estuary"));
}
}

t! {
fn the_recorded_sassandra_level_never_rises_toward_the_sea() {
    // Label: property (#280). On the recorded lower Sassandra (the sea at chainage 0), with the
    // shipped settings and with the narrow reaches taken from the DEM (so SWOT and DEM stretches
    // meet), every point of the profile is at or below the one upstream of it: going up the chainage
    // the level never falls. Every change is recorded, and none goes the wrong way.
    for p in [LevelParams::default(), LevelParams { min_swot_width_m: 320.0, ..LevelParams::default() }] {
        let l = level(&p, Datum::Egm2008);
        assert!(!l.downstream_is_increasing);
        assert!(l.points.windows(2).all(|w| w[1][1] >= w[0][1]), "{:?}", l.points);
        // With the shipped settings reach 033's SWOT level (0.246 m) sits above reach 041's upstream
        // (0.198 m); with the narrow reaches from the DEM, the DEM at 4.2 to 6.6 km sits above 041.
        let source = if p == LevelParams::default() { "swot" } else { "dem" };
        assert!(l.adjusted.iter().any(|a| a.source == source), "{:?}", l.adjusted);
        for a in &l.adjusted {
            let q = l.points.iter().find(|q| q[0] == a.chainage_m).unwrap();
            assert_eq!(q[1], a.now_m);
            assert!(a.source == "dem" || a.direction() == "lowered", "{a:?}");
        }
    }
}
}
