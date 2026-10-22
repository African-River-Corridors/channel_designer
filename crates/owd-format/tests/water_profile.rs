//! The water level as a profile along the river centreline (ADR 0006), on the synthetic river.
//!
//! - Two design lines through the same place see the same water.
//! - Pool levels from the optimise stage are the profile the volumes read; an old document with a
//!   start level and a slope converts to a two-point profile and its stages show as stale.
//!
//! Label: property.

use owd_core::geom::{Pt, linear};
use owd_core::synthetic::{RiverParams, SyntheticRiver, synthetic_river};
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

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

/// The quickstart river in a UTM 30N frame with the example's own origin, so no last bit moves.
fn quickstart(river: &SyntheticRiver, design_line_sampling: bool) -> Session {
    let mut s = Session::new("Quickstart", "2026-10-16");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(river.centreline)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([river.polygon.exterior])
            ),
        ),
    ])
    .unwrap();
    if design_line_sampling {
        s.sample_water_at_design_line_chainage();
    }
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    // The example's water level as a profile: the river's own level at each vertex of its centreline.
    let profile: Vec<[f64; 2]> = (0..river.centreline.len())
        .map(|i| {
            let c = linear::length(&river.centreline[..=i]);
            [c, river.water_level(c)]
        })
        .collect();
    s.set_water_profile(&profile).unwrap();
    s.set_dem("d.tif", b"d", &dem());
    s
}

#[test]
fn two_design_lines_through_the_same_place_see_the_same_water() {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let mut s = quickstart(&river, false);
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    // Line A is the river itself. Line B starts 1 km on and is shifted 15 m sideways.
    let a = river.centreline.clone();
    let b: Vec<Pt> = river
        .centreline
        .iter()
        .filter(|p| p[0] >= 1000.0)
        .map(|p| [p[0], p[1] + 15.0])
        .collect();
    // The river's point at x = 3000, as a chainage on each line.
    let on = |line: &[Pt]| {
        let c = linear::project(
            line,
            [
                3000.0,
                linear::interpolate(&a, linear::project(&a, [3000.0, 0.0])).unwrap()[1],
            ],
        );
        s.water_levels_along(line, &[c]).unwrap()[0]
    };
    let (la, lb) = (on(&a), on(&b));
    assert!((la - lb).abs() < 1e-3, "A sees {la} m, B sees {lb} m");
    // Chainage along each line differs, so a profile on each line's own chainage could not agree.
    let (ca, cb) = (
        linear::project(&a, [3000.0, 0.0]),
        linear::project(&b, [3000.0, 0.0]),
    );
    assert!((ca - cb).abs() > 500.0, "{ca} {cb}");
    assert!((la - river.water_level(linear::project(&a, [3000.0, 0.0]))).abs() < 1e-3);
}

#[test]
fn a_profile_steps_at_a_weir_and_holds_flat_beyond_its_ends() {
    use owd_format::design::water::level_at;
    let p = [[0.0, 1.0], [100.0, 1.0], [100.0, 2.0], [300.0, 4.0]];
    assert_eq!(level_at(&p, -50.0), 1.0);
    assert_eq!(level_at(&p, 50.0), 1.0);
    assert_eq!(level_at(&p, 100.0), 2.0, "at a step the later level holds");
    assert_eq!(level_at(&p, 200.0), 3.0);
    assert_eq!(level_at(&p, 900.0), 4.0);
    assert_eq!(
        level_at(&[[5.0, 7.0]], 123.0),
        7.0,
        "one point is a constant"
    );
}

#[test]
fn an_old_document_with_a_start_level_and_slope_converts_and_its_stages_are_stale() {
    let river = synthetic_river(RiverParams::default()).unwrap();
    let mut s = quickstart(&river, false);
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| river.terrain(x, y))
            .collect()
    };
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    // The document as the first #61-#63 builds wrote it: no profile; the level in the stage settings.
    let mut old = s.document().clone();
    old["inputs"].as_object_mut().unwrap().remove("water_level");
    old["stages"]["volumes"]["settings"]["water_level_start_m"] = json!(3.0);
    old["stages"]["volumes"]["settings"]["water_surface_slope"] = json!(0.001);
    let opened = Session::open(&serde_json::to_string(&old).unwrap()).unwrap();
    let len = linear::length(&river.centreline);
    let got = opened.input_water_profile().unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], [0.0, 3.0]);
    assert!((got[1][0] - len).abs() < 1e-9 && (got[1][1] - (3.0 - 0.001 * len)).abs() < 1e-9);
    let settings = &opened.document()["stages"]["volumes"]["settings"];
    assert!(
        settings.get("water_level_start_m").is_none()
            && settings.get("water_surface_slope").is_none()
    );
    // Its results came from the old model: stale, never silently fresh.
    assert_eq!(opened.stage_status(Stage::Volumes), StageStatus::Stale);
    // A document that already has a profile is left as it is.
    assert_eq!(
        Session::open(&s.to_text()).unwrap().input_water_profile(),
        s.input_water_profile()
    );
}
