//! The custom vessel in the session and the file (spec 0015 T1), and file format 0.4.
//!
//! Label: property (a custom vessel with a class's figures against the class; a format-0.3 study
//! against itself after the upgrade; a deep draught's dredge against the class's).

use owd_core::geom::Pt;
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_format::design::{AlignMethod, DesignError, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn imported() -> Session {
    let r = synthetic_river(RiverParams::default()).unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let centre: Vec<Pt> = r.centreline.iter().map(shift).collect();
    let mut s = Session::new("River", "2026-10-22");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
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
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s
}

/// CEMT-IV's design figures as a custom vessel.
fn like_iv() -> CustomVessel {
    CustomVessel {
        name: "My barge".into(),
        loa_m: 85.0,
        beam_m: 9.5,
        draught_m: 2.5,
        formation: Formation::Motor,
        n_abreast: None,
        n_in_line: None,
        gb_class: None,
        air_draught_m: Some(7.0),
        deadweight_t: Some(1350.0),
    }
}

fn sizing(s: &Session) -> &Value {
    &s.document()["stages"]["sizing"]
}

#[test]
fn a_custom_vessel_is_kept_in_the_settings_and_sizes_with_its_label() {
    let mut s = imported();
    let warnings = s.set_custom_vessel(&like_iv()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let st = &sizing(&s)["settings"];
    assert!(st["vessel_standard_id"].is_null());
    assert_eq!(st["custom_vessel"]["name"], "My barge");
    assert_eq!(st["custom_vessel"]["loa_m"], 85.0);
    assert_eq!(st["custom_vessel"]["formation"], "motor");
    assert_eq!(st["custom_vessel"]["air_draught_m"], 7.0);
    assert_eq!(st["custom_vessel"]["deadweight_t"], 1350.0);
    s.run_stage(Stage::Sizing).unwrap();
    let r = &sizing(&s)["results"];
    assert!(r["vessel_standard_id"].is_null(), "{r}");
    assert_eq!(r["vessel_label"], "Custom: My barge, 85 × 9.5 × 2.5 m");
    assert_eq!(
        s.vessel_label().as_deref(),
        Some("Custom: My barge, 85 × 9.5 × 2.5 m")
    );

    // The same channel as the class with the same figures.
    let mut c = imported();
    c.set_vessel("CEMT-IV", Some(2.5)).unwrap();
    c.run_stage(Stage::Sizing).unwrap();
    for k in [
        "bottom_width_straight_m",
        "min_bend_radius_m",
        "design_depth_m",
        "bend_widening_at_rmin_m",
    ] {
        assert_eq!(r[k], sizing(&c)["results"][k], "{k}");
    }
    assert_eq!(c.vessel_label().as_deref(), Some("CEMT-IV"));
    assert!(sizing(&c)["results"].get("vessel_label").is_none());
}

#[test]
fn a_refused_custom_vessel_changes_nothing_and_names_the_field() {
    let mut s = imported();
    let before = s.document().clone();
    let Err(DesignError::Input(m)) = s.set_custom_vessel(&CustomVessel {
        beam_m: 0.0,
        ..like_iv()
    }) else {
        panic!()
    };
    assert!(m.contains("beam_m"), "{m}");
    assert_eq!(s.document(), &before);
}

#[test]
fn a_warning_is_returned_and_kept_with_the_results() {
    let mut s = imported();
    let w = s
        .set_custom_vessel(&CustomVessel {
            loa_m: 60.0,
            beam_m: 29.0,
            ..like_iv()
        })
        .unwrap();
    assert!(w.iter().any(|w| w.contains("length-to-beam")), "{w:?}");
    s.run_stage(Stage::Sizing).unwrap();
    let r = &sizing(&s)["results"]["warnings"];
    assert!(
        r.as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("length-to-beam")),
        "{r}"
    );
}

#[test]
fn a_class_replaces_a_custom_vessel_and_a_custom_vessel_replaces_a_class() {
    let mut s = imported();
    s.set_vessel("CEMT-IV", None).unwrap();
    s.set_custom_vessel(&like_iv()).unwrap();
    assert!(sizing(&s)["settings"]["vessel_standard_id"].is_null());
    s.set_vessel("CEMT-Va", None).unwrap();
    assert_eq!(sizing(&s)["settings"]["vessel_standard_id"], "CEMT-Va");
    assert!(sizing(&s)["settings"].get("custom_vessel").is_none());
}

#[test]
fn a_custom_vessel_study_saves_as_format_0_4_and_reopens_the_same() {
    let mut s = imported();
    s.set_custom_vessel(&CustomVessel {
        formation: Formation::Convoy,
        n_abreast: Some(2),
        n_in_line: Some(2),
        gb_class: Some("III".into()),
        ..like_iv()
    })
    .unwrap();
    s.run_stale().unwrap();
    let text = s.to_text();
    assert!(
        text.contains(r#""format_version": "0.4""#),
        "{}",
        &text[..200]
    );
    let back = Session::open(&text).unwrap();
    assert_eq!(back.to_text(), text);
    assert_eq!(back.stage_status(Stage::Sizing), StageStatus::Fresh);
    assert_eq!(back.custom_vessel().unwrap().n_abreast, Some(2));
    assert_eq!(
        back.custom_vessel().unwrap().gb_class.as_deref(),
        Some("III")
    );
}

#[test]
fn changing_the_custom_vessel_makes_sizing_stale() {
    let mut s = imported();
    s.set_custom_vessel(&like_iv()).unwrap();
    s.run_stale().unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    s.set_custom_vessel(&CustomVessel {
        beam_m: 11.0,
        ..like_iv()
    })
    .unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
}

#[test]
fn a_format_0_3_study_opens_upgrades_and_gives_the_same_results() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web/river-example.owd");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        text.contains(r#""format_version": 3,"#),
        "the fixture is format 0.3"
    );
    let s = Session::open(&text).unwrap();
    assert_eq!(s.document()["format_version"], "0.4");
    for stage in [Stage::Sizing, Stage::Alignment, Stage::Volumes] {
        assert_eq!(s.stage_status(stage), StageStatus::Fresh, "{stage:?}");
    }
    // Rerun from the upgraded file: the same results.
    let before = s.document().clone();
    let mut again = s.clone();
    again.run_stage(Stage::Sizing).unwrap();
    // The numbers (the fixture's `source` text is from the catalogue before spec 0015 T0).
    let (a, b) = (
        &again.document()["stages"]["sizing"]["results"],
        &before["stages"]["sizing"]["results"],
    );
    for k in [
        "loa_m",
        "beam_m",
        "draught_m",
        "bottom_width_straight_m",
        "min_bend_radius_m",
        "design_depth_m",
        "bend_widening_at_rmin_m",
        "bottom_width_at_rmin_m",
    ] {
        assert_eq!(a[k], b[k], "{k}");
    }
    assert!(Session::open(&s.to_text()).is_ok());
}

#[test]
fn a_format_from_a_newer_owd_or_an_old_number_is_refused_in_words() {
    let mut doc = imported().document().clone();
    doc["format_version"] = json!("0.5");
    let Err(DesignError::Input(m)) = Session::open(&doc.to_string()) else {
        panic!()
    };
    assert!(m.contains("newer OWD"), "{m}");
    doc["format_version"] = json!(2);
    let Err(DesignError::Input(m)) = Session::open(&doc.to_string()) else {
        panic!()
    };
    assert!(m.contains("Rebuild"), "{m}");
}

#[test]
fn the_schema_takes_format_0_4_and_a_custom_vessel() {
    let schema: Value = serde_json::from_str(Session::schema_text()).unwrap();
    assert_eq!(schema["properties"]["format_version"]["const"], "0.4");
    let mut s = imported();
    s.set_custom_vessel(&like_iv()).unwrap();
    s.run_stale().unwrap();
    assert_eq!(s.validate(), Vec::<String>::new());
}

#[test]
fn a_draught_deeper_than_the_river_sizes_and_shows_as_more_dredge() {
    let dredge = |custom: CustomVessel| -> f64 {
        let mut s = imported();
        s.set_custom_vessel(&custom).unwrap();
        s.run_stage(Stage::Sizing).unwrap();
        s.align(
            AlignMethod::RollingCircle,
            &json!({"use_water": false}),
            None,
        )
        .unwrap();
        let dem = Dem {
            width: 2,
            height: 2,
            data: vec![0.0; 4],
            geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
            nodata: Some(-9999.0),
            epsg: Some(32630),
        };
        s.set_dem("dem/river.tif", b"a stand-in", &dem);
        let r = synthetic_river(RiverParams::default()).unwrap();
        s.set_water_fall(r.water_level(0.0), 0.0).unwrap();
        let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
            xs.iter()
                .zip(ys)
                .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
                .collect()
        };
        s.run_volumes(&json!({"cut_model": "b"}), Some(&terrain))
            .unwrap();
        s.document()["stages"]["volumes"]["results"]["options"][0]["dredge_m3"]
            .as_f64()
            .unwrap()
    };
    let shallow = dredge(like_iv());
    let deep = dredge(CustomVessel {
        draught_m: 9.0,
        ..like_iv()
    });
    assert!(deep > shallow, "{deep} vs {shallow}");
}
