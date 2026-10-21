//! `owd water-level` in process, with every answer recorded: the SWORD subset, the Hydrocron series of
//! five lower Sassandra reaches and a GLO-30 window (`tools/make_level_fixture.py`).
//!
//! Label: property

use owd_cli::level::{Endpoints, LevelFlags, water_level_with};
use owd_format::design::crs::Utm;
use owd_format::design::{Role, Session};
use owd_reach::ReplayFetch;
use std::path::{Path, PathBuf};

const RECORD: &str = "https://zenodo.example/api/records/sword";
const ZIP: &str = "https://zenodo.example/sword.zip";
const HYDROCRON: &str = "https://hydrocron.example/timeseries";
const REACHES: [&str; 5] = [
    "14560100021",
    "14560100033",
    "14560100041",
    "14560100051",
    "14560100061",
];

fn ex(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/reach")
        .join(name)
}

fn fetch() -> ReplayFetch {
    let zip = std::fs::read(ex("sword_v17c_shp_subset.zip")).unwrap();
    let record = format!(
        r#"{{"files":[{{"key":"SWORD_v17c_shp.zip","size":{},"links":{{"self":"{ZIP}"}}}}]}}"#,
        zip.len()
    );
    let mut f = ReplayFetch::new()
        .with_response(RECORD, record)
        .with_response(ZIP, zip);
    for id in REACHES {
        f = f.with_response_containing(
            &format!("feature_id={id}&"),
            std::fs::read(ex(&format!("hydrocron_{id}.json"))).unwrap(),
        );
    }
    f
}

/// A document with the reaches' line as its river centreline and the GLO-30 window as its DEM.
fn study(dir: &Path) -> PathBuf {
    let reaches = owd_reach::sword::reaches_in(&fetch(), RECORD, [-6.4, 4.9, -6.0, 5.2]).unwrap();
    let mut line: Vec<[f64; 2]> = Vec::new();
    for r in &reaches {
        let mut l = r.line.clone();
        match line.last() {
            Some(last) => {
                let d = |p: [f64; 2]| (p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2);
                if d(l[l.len() - 1]) < d(l[0]) {
                    l.reverse();
                }
            }
            None if l[0][0] < l[l.len() - 1][0] => l.reverse(),
            None => {}
        }
        line.extend(l);
    }
    let utm = Utm {
        zone: 29,
        south: false,
    }
    .forward_all(&line)
    .unwrap();
    let gj = format!(
        r#"{{"type":"LineString","coordinates":{},"crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32629"}}}}}}"#,
        serde_json::to_string(&utm).unwrap()
    );
    let mut s = Session::new("Lower Sassandra", "2026-10-21");
    s.import(&[(Role::Centreline, gj.as_str())]).unwrap();
    let bytes = std::fs::read(ex("glo30_sassandra_lower.tif")).unwrap();
    std::fs::write(dir.join("dem.tif"), &bytes).unwrap();
    let dem = owd_format::geotiff::read_bytes(&bytes).unwrap();
    s.set_dem("dem.tif", &bytes, &dem);
    let file = dir.join("study.owd");
    std::fs::write(&file, s.to_text()).unwrap();
    file
}

fn flags(file: &Path) -> LevelFlags {
    LevelFlags {
        doc: file.to_str().unwrap().to_string(),
        percentile: None,
        min_swot_width: None,
        from: None,
        to: Some("2026-10-01".into()),
        dem_datum: None,
        output: None,
        json: true,
    }
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-level-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const AT: Endpoints = Endpoints {
    sword_record: RECORD,
    hydrocron: HYDROCRON,
};

#[test]
fn the_default_level_goes_into_the_document_as_its_water_level_profile() {
    let dir = work_dir("default");
    let file = study(&dir);
    if let Err(e) = water_level_with(flags(&file), &fetch(), &AT) {
        panic!("{}", e.message);
    }
    let s = Session::open(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let (profile, from) = s.water_profile().unwrap();
    assert_eq!(from, "input");
    assert!(profile.len() >= 5, "{profile:?}");
    let p = s.provenance("water_level").unwrap();
    assert_eq!(p["source"], "swot-hydrocron");
    assert_eq!(p["mark"], "default");
    assert_eq!(p["settings"]["datum"], "EGM2008");
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    // Another percentile: the level is the user's.
    let mut f = flags(&file);
    f.percentile = Some(50.0);
    water_level_with(f, &fetch(), &AT)
        .map_err(|e| e.message)
        .unwrap();
    let s = Session::open(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(s.provenance("water_level").unwrap()["mark"], "user");
    // Bad arguments say what is wrong.
    let mut f = flags(&file);
    f.percentile = Some(120.0);
    let Err(e) = water_level_with(f, &fetch(), &AT) else {
        panic!("120th percentile accepted")
    };
    assert!(e.message.contains("--percentile"), "{}", e.message);
    let mut f = flags(&file);
    f.dem_datum = Some("navd88".into());
    let Err(e) = water_level_with(f, &fetch(), &AT) else {
        panic!("unknown datum accepted")
    };
    assert!(e.message.contains("--dem-datum"), "{}", e.message);
}
