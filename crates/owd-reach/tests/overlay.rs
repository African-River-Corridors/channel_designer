//! The QA overlay (spec 0013, ticket T5) on a synthetic scene window: lines land where they run,
//! the KMZ places the image by its four corners, and a crop keeps what lies between two chainages.
//!
//! Label: property

use owd_format::design::crs::Utm;
use owd_reach::ReplayFetch;
use owd_reach::overlay::{CENTRE, Flag, LEFT, Lines, RIGHT, Shot, kmz, true_colour};
use owd_reach::sentinel2::Imagery;
use owd_reach::zip::RemoteZip;

const UTM: Utm = Utm {
    zone: 29,
    south: false,
};
const X0: f64 = 770_000.0;
const Y1: f64 = 627_000.0;
const COLS: usize = 200;
const ROWS: usize = 100;

/// A scene with a river along rows 40 to 60 (water dark, land green); `nodata` columns empty.
fn scene(nodata: std::ops::Range<usize>) -> (Imagery, Vec<f32>) {
    let mut bands: [Vec<f32>; 4] = Default::default();
    let mut blue = Vec::new();
    for r in 0..ROWS {
        for c in 0..COLS {
            let water = (40..60).contains(&r);
            let v: [f32; 5] = if nodata.contains(&c) {
                [0.0; 5]
            } else if water {
                [0.05, 0.04, 0.02, 0.01, 0.06]
            } else {
                [0.08, 0.05, 0.30, 0.20, 0.03]
            };
            for k in 0..4 {
                bands[k].push(v[k]);
            }
            blue.push(v[4]);
        }
    }
    let valid = vec![true; COLS * ROWS];
    (
        Imagery::from_bands(UTM, X0, Y1, 10.0, COLS, ROWS, bands, valid),
        blue,
    )
}

fn lines() -> Lines {
    let along = |y: f64| -> Vec<[f64; 2]> {
        (0..=20)
            .map(|i| [X0 + 100.0 + i as f64 * 90.0, y])
            .collect()
    };
    Lines {
        left: along(Y1 - 400.0),
        right: along(Y1 - 600.0),
        centre: along(Y1 - 500.0),
        flags: vec![Flag {
            from_m: 300.0,
            to_m: 600.0,
            kind: "repaired".into(),
        }],
    }
}

fn pixel(p: &owd_reach::overlay::Picture, x: f64, y: f64) -> [u8; 4] {
    let (c, r) = (((x - p.x0) / p.px) as usize, ((p.y1 - y) / p.px) as usize);
    let i = (r * p.width + c) * 4;
    [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]]
}

#[test]
fn the_lines_are_drawn_where_they_run_and_the_picture_is_the_window() {
    let (im, blue) = scene(0..0);
    let mut p = true_colour(&[Shot {
        imagery: &im,
        blue: &blue,
    }])
    .unwrap()
    .upscale(2);
    assert_eq!((p.width, p.height), (COLS * 2, ROWS * 2));
    let before = pixel(&p, X0 + 1_000.0, Y1 - 400.0);
    let far_before = pixel(&p, X0 + 1_000.0, Y1 - 900.0);
    p.draw(&lines());
    assert_eq!(pixel(&p, X0 + 1_000.0, Y1 - 400.0), LEFT, "was {before:?}");
    assert_eq!(pixel(&p, X0 + 1_000.0, Y1 - 600.0), RIGHT);
    // Outside the flagged stretch the centreline is cream; inside, it is still drawn over the band.
    assert_eq!(pixel(&p, X0 + 1_500.0, Y1 - 500.0), CENTRE);
    // Off the lines the image is untouched.
    assert_eq!(pixel(&p, X0 + 1_000.0, Y1 - 900.0), far_before);
    // The legend goes under the map and widens it if it must.
    let l = p.with_legend(&["Contains modified Copernicus Sentinel data 2025".to_string()]);
    assert_eq!(l.map_rows, p.map_rows);
    assert!(l.height > p.height && l.width >= p.width);
    let png = l.png().unwrap();
    assert_eq!(&png[1..4], b"PNG");
}

#[test]
fn a_second_scene_fills_where_the_first_has_no_data() {
    let (a, ba) = (scene(0..50).0, scene(0..50).1);
    let (b, bb) = scene(0..0);
    let one = true_colour(&[Shot {
        imagery: &a,
        blue: &ba,
    }])
    .unwrap();
    assert_eq!(pixel(&one, X0 + 100.0, Y1 - 100.0)[..3], [0, 0, 0]);
    let two = true_colour(&[
        Shot {
            imagery: &a,
            blue: &ba,
        },
        Shot {
            imagery: &b,
            blue: &bb,
        },
    ])
    .unwrap();
    assert_ne!(pixel(&two, X0 + 100.0, Y1 - 100.0)[..3], [0, 0, 0]);
}

#[test]
fn the_kmz_places_the_image_by_its_corners_and_carries_the_lines_and_notices() {
    let (im, blue) = scene(0..0);
    let mut p = true_colour(&[Shot {
        imagery: &im,
        blue: &blue,
    }])
    .unwrap();
    p.draw(&lines());
    let notices = vec![
        "Contains modified Copernicus Sentinel data 2025".to_string(),
        "Source: EC JRC/Google".to_string(),
    ];
    let bytes = kmz(&p, &lines(), "Test km 0 to 2", &notices).unwrap();
    let url = "file:///overlay.kmz";
    let f = ReplayFetch::new().with_response(url, bytes.clone());
    let z = RemoteZip::open(&f, url, bytes.len() as u64).unwrap();
    let kml = String::from_utf8(z.read(&f, z.member("doc.kml").unwrap()).unwrap()).unwrap();
    let png = z.read(&f, z.member("overlay.png").unwrap()).unwrap();
    let decoded = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    assert_eq!(
        (decoded.info().width, decoded.info().height),
        (COLS as u32, ROWS as u32)
    );
    // The quad: bottom left, bottom right, top right, top left of the window, in WGS84.
    let quad = kml
        .split("<coordinates>")
        .nth(1)
        .unwrap()
        .split("</coordinates>")
        .next()
        .unwrap();
    let got: Vec<[f64; 2]> = quad
        .split_whitespace()
        .map(|c| {
            let v: Vec<f64> = c.split(',').map(|x| x.parse().unwrap()).collect();
            [v[0], v[1]]
        })
        .collect();
    let (x1, y0) = (X0 + COLS as f64 * 10.0, Y1 - ROWS as f64 * 10.0);
    let want = UTM
        .inverse_all(&[[X0, y0], [x1, y0], [x1, Y1], [X0, Y1]])
        .unwrap();
    for (g, w) in got.iter().zip(&want) {
        assert!(
            (g[0] - w[0]).abs() < 1e-6 && (g[1] - w[1]).abs() < 1e-6,
            "{g:?} vs {w:?}"
        );
    }
    for n in &notices {
        assert!(kml.contains(n.as_str()), "{n}");
    }
    for name in [
        "Left bank",
        "Right bank",
        "Centreline",
        "repaired 300 to 600 m",
    ] {
        assert!(kml.contains(name), "{name}");
    }
}

#[test]
fn a_crop_keeps_the_lines_between_two_chainages() {
    let l = lines().crop(450.0, 1_350.0);
    let s = owd_format::design::midline::arc(&l.centre);
    assert!((s.last().unwrap() - 900.0).abs() < 1e-6);
    assert!((l.centre[0][0] - (X0 + 100.0 + 450.0)).abs() < 1e-6);
    assert!(
        l.left
            .iter()
            .all(|p| p[0] >= X0 + 550.0 - 1e-6 && p[0] <= X0 + 1_450.0 + 1e-6)
    );
    assert_eq!(
        l.flags,
        vec![Flag {
            from_m: 0.0,
            to_m: 150.0,
            kind: "repaired".into()
        }]
    );
}
