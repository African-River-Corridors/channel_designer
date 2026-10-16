//! The optimise stage on the `optimise_pools` example (a straight 3 km design line across the
//! synthetic river, given in UTM 30N and shifted into the zone): the weirs, pools and lattice option
//! equal the cascade search and `optimise_pools` called directly, and equal what the frozen Python
//! printed for the example. The lattice option is kept with its pools and checks, and the volumes
//! stage measures it like any other option, under the pool levels.
//!
//! Label: snapshot (the Python reference for the example); the direct-call checks are properties.

use cd_core::alignment::radius_check::min_radius_on_chord;
use cd_core::geom::{Polygon, Pt};
use cd_core::optimise::cascade::{
    CascadeConfig, SectionOptions, build_level_table, cascade_levels, earthworks_reach_cost,
    search_cascade,
};
use cd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pools,
};
use cd_core::synthetic::{RiverParams, synthetic_river};
use cd_core::volumes::sections::HalfWidth;
use cd_format::design::{AlignMethod, DesignError, Role, Session, Stage, StageStatus};
use cd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn params() -> RiverParams {
    RiverParams {
        length_m: 3000.0,
        wavelength_m: 2400.0,
        amplitude_m: 120.0,
        hill_at: [1500.0, -60.0],
        hill_height_m: 10.0,
        hill_radius_m: 80.0,
        ..RiverParams::default()
    }
}

fn terrain(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let r = synthetic_river(params()).unwrap();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
        .collect()
}

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    }
}

/// The example's settings, as the example sets them.
fn example_settings() -> Value {
    json!({
        "candidate_sites_m": [500.0, 1000.0, 1500.0, 2000.0, 2500.0],
        "lifts_m": [0.5, 1.0],
        "terminal_min_level_m": 0.5,
        "min_spacing_m": 1000.0,
        "dry_weight": 1.5,
        "structure_fixed": 1.0e5,
        "structure_per_lift": 2.0e5,
        "cut_model": "b",
        "maxw_m": 120.0,
        "rake_half_m": 80.0,
        "rake_step_m": 5.0,
        "ds_step_m": 10.0,
        "pitch_m": 20.0,
        "n_heading": 24,
        "radius_multiples": [1.0, 2.0, 4.0],
        "corridor_half_m": 180.0,
        "cell_m": 5.0,
    })
}

fn design() -> Session {
    design_with(true)
}

fn design_with(water: bool) -> Session {
    let r = synthetic_river(params()).unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let line = [shift(&[0.0, 0.0]), shift(&[3000.0, 0.0])];
    let mut s = Session::new("Pools", "2026-10-16");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(line)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_sizing("PIANC-B", "CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.set_dem("dem.tif", b"dem", &dem());
    if water {
        s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    }
    s
}

fn pts(v: &Value) -> Vec<Pt> {
    v["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

#[test]
fn the_sizing_gives_the_examples_depth_and_r_min_and_a_half_width_near_its_15_m() {
    let s = design();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(r["design_depth_m"], 3.25);
    // The example hard-codes 15 m; the sizing gives 15.2 m. The stage uses the sizing's.
    assert!((r["bottom_width_straight_m"].as_f64().unwrap() / 2.0 - 15.2).abs() < 1e-9);
    assert_eq!(r["min_bend_radius_m"], 255.0);
}

#[test]
fn weirs_pools_and_the_lattice_option_equal_the_core_called_directly() {
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    let res = s.document()["stages"]["optimise"]["results"].clone();
    let hb = s.document()["stages"]["sizing"]["results"]["bottom_width_straight_m"]
        .as_f64()
        .unwrap()
        / 2.0;
    let baseline = pts(&res["baseline"]);
    assert!((baseline[0][0] - SHIFT[0]).abs() < 1e-9 && baseline.len() >= 2);

    // The core, called directly, as the example calls it.
    let r = synthetic_river(params()).unwrap();
    let poly = [Polygon {
        exterior: r.polygon.exterior.iter().map(shift).collect(),
        holes: vec![],
    }];
    let mut cfg = CascadeConfig::new(
        vec![500.0, 1000.0, 1500.0, 2000.0, 2500.0],
        vec![0.5, 1.0],
        cd_core::geom::linear::length(&baseline),
        0.5,
    );
    cfg.min_spacing_m = 1000.0;
    let cfg = cfg.validated().unwrap();
    let table = build_level_table(
        &baseline,
        &terrain,
        &poly,
        &cascade_levels(&cfg),
        3.25,
        &HalfWidth::Fixed(hb),
        Some("b"),
        Some(25.0),
        Some(120.0),
        None,
        SectionOptions::default(),
    )
    .unwrap();
    let reach = earthworks_reach_cost(&table, 1.0, 1.5);
    let structure = |lift: f64| 1.0e5 + 2.0e5 * lift;
    let search = search_cascade(&cfg, &reach, &structure).unwrap();
    let best = search.best().unwrap();
    let pools = best.pools();
    assert_eq!(
        res["pools"],
        json!(
            pools
                .iter()
                .map(|&(a, b, l)| json!([a, b, l]))
                .collect::<Vec<_>>()
        )
    );
    assert_eq!(
        res["cascade"]["total_cost"].as_f64().unwrap(),
        best.total_cost
    );

    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(hb));
    spec.side_slope_h_per_v = 3.0;
    spec.dry_weight = 1.5;
    spec.rake_half_m = 80.0;
    spec.rake_step_m = 5.0;
    spec.ds_step_m = 10.0;
    let mut lat = LatticeConfig::new(255.0);
    lat.pitch_m = 20.0;
    lat.n_heading = 24;
    lat.radius_multiples = vec![1.0, 2.0, 4.0];
    let lat = lat.validated().unwrap();
    let direct = optimise_pools(
        &baseline,
        &terrain,
        &poly,
        &pools,
        3.25,
        &spec,
        &lat,
        None,
        PoolOptions {
            corridor_half_m: 180.0,
            cell_m: 5.0,
        },
        true,
    )
    .unwrap();
    let opt = s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert_eq!(pts(&opt["line"]), direct.line);
    assert_eq!(opt["details"]["n_replaced"], direct.n_replaced());
    let measured = min_radius_on_chord(&direct.line, 255.0 / 3.0).unwrap();
    assert_eq!(
        opt["checks"]["measured_min_radius_m"].as_f64().unwrap(),
        measured
    );
    assert_eq!(opt["checks"]["radius_check_chord_m"], 255.0 / 3.0);
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
}

#[test]
fn volumes_measure_the_lattice_option_like_any_other_under_the_pool_levels() {
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::NotRun);
    s.run_volumes(&json!({"cut_model": "b", "maxw_m": 120.0}), Some(&terrain))
        .unwrap();
    let rows = s.document()["stages"]["volumes"]["results"]["options"]
        .as_array()
        .unwrap()
        .clone();
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["rolling-circle", "lattice"]);
    assert!(rows.iter().all(|r| r["water_source"] == "optimise"));
    // The profile is the pool levels: a step of the weir's lift where the weir stands.
    let (profile, source) = s.water_profile().unwrap();
    assert_eq!(source, "optimise");
    let weir = &s.document()["stages"]["optimise"]["results"]["cascade"]["weirs"][0];
    let lift = weir["lift_m"].as_f64().unwrap();
    let steps: Vec<&[[f64; 2]]> = profile
        .windows(2)
        .filter(|w| w[0][0] == w[1][0] && w[0][1] != w[1][1])
        .collect();
    assert!(!steps.is_empty(), "{profile:?}");
    assert!((steps[0][1][1] - steps[0][0][1] - lift).abs() < 1e-9);
    assert!(rows.iter().all(|r| r["dredge_m3"].as_f64().unwrap() > 0.0));
    // The lattice moves dredge to dry cut, as the example shows: more dredge, less dry cut.
    assert!(rows[1]["excav_m3"].as_f64().unwrap() < rows[0]["excav_m3"].as_f64().unwrap());
    // A new optimise run leaves the volumes stale.
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    s.run_optimise(&json!({"dry_weight": 3.0}), Some(&terrain))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
}

#[test]
fn the_stage_says_what_it_needs() {
    let mut s = design_with(false);
    let e = s
        .run_optimise(&Value::Null, Some(&terrain))
        .unwrap_err()
        .to_string();
    assert!(e.contains("needs a water level"), "{e}");
    let mut s = design();
    let e = s
        .run_optimise(&example_settings(), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("dem.tif") && e.contains("missing"), "{e}");
    let mut bare = Session::new("x", "2026-10-16");
    bare.set_sizing("PIANC-B", "CEMT_IV", None).unwrap();
    bare.run_stage(Stage::Sizing).unwrap();
    assert!(matches!(
        bare.run_optimise(&example_settings(), Some(&terrain)),
        Err(DesignError::Input(m)) if m.contains("needs a DEM")
    ));
}
