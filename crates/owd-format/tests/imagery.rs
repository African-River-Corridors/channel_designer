#![cfg(feature = "imagery")]
//! Label: property
//! Plan backdrops other than shaded relief (spec 0004, ticket 6), through `drawing::sheets_from_text_with`,
//! `geotiff::read_rgb_bytes` and `design::terrain::rgb_sampler`: an orthophoto in WGS84 and one in UTM
//! place the same image within one pixel, and fetched EOxCloudless 2016 prints its attribution and year
//! on every sheet that shows it, or falls back to shaded relief with a message. The tiles are
//! synthetic and no test uses the network.
use otd::raster::{Grid, jpeg_rgb};
use otd_imagery::{ATTRIBUTION, FetchError, Source, TILE_PX, Tile, TileFetch};
use owd_format::design::crs::Utm;
use owd_format::design::terrain::rgb_sampler;
use owd_format::drawing::{self, Background, DrawOptions, Sheet, SheetKind};
use owd_format::geotiff::{self, Rgb};
use std::path::PathBuf;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

fn web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

fn example() -> (String, Vec<u8>) {
    (
        std::fs::read_to_string(web().join("river-example.owd")).unwrap(),
        std::fs::read(web().join("river-dem.tif")).unwrap(),
    )
}

/// The example's zone (EPSG 32630).
fn zone() -> Utm {
    match owd_format::design::crs::Crs::from_epsg(32630).unwrap() {
        owd_format::design::crs::Crs::Utm(u) => u,
        _ => unreachable!(),
    }
}

/// The test pattern, by UTM point: red on a 200 m square, white elsewhere.
fn pattern(e: f64, n: f64) -> [u8; 3] {
    if (501_000.0..501_200.0).contains(&e) && (599_900.0..600_100.0).contains(&n) {
        [220, 30, 30]
    } else {
        [255, 255, 255]
    }
}

/// An 8 bit RGB GeoTIFF, north up, over `[x0, y0, x1, y1]` in its CRS, each pixel the pattern at its
/// centre (in UTM; for WGS84 through the zone's forward projection).
fn orthophoto(epsg: u16, bbox: [f64; 4], cols: u32, rows: u32) -> Vec<u8> {
    let [x0, y0, x1, y1] = bbox;
    let (dx, dy) = ((x1 - x0) / f64::from(cols), (y1 - y0) / f64::from(rows));
    let mut centres = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            centres.push([
                x0 + (f64::from(c) + 0.5) * dx,
                y1 - (f64::from(r) + 0.5) * dy,
            ]);
        }
    }
    let utm = if epsg == 4326 {
        zone().forward_all(&centres).unwrap()
    } else {
        centres
    };
    let data: Vec<u8> = utm.iter().flat_map(|p| pattern(p[0], p[1])).collect();
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        let mut img = enc.new_image::<colortype::RGB8>(cols, rows).unwrap();
        let (geo, key) = if epsg == 4326 { (2, 2048) } else { (1, 3072) };
        let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, geo, 1025, 0, 1, 1, key, 0, 1, epsg];
        let e = img.encoder();
        e.write_tag(Tag::Unknown(33550), &[dx, dy, 0.0][..])
            .unwrap();
        e.write_tag(Tag::Unknown(33922), &[0.0, 0.0, 0.0, x0, y1, 0.0][..])
            .unwrap();
        e.write_tag(Tag::Unknown(34735), &keys[..]).unwrap();
        img.write_data(&data).unwrap();
    }
    out.into_inner()
}

/// The pixel box `[c0, r0, c1, r1]` of the red cells of a grid, sampled at 2.5 m cell centres over
/// the window: the cells a plan sheet at 1:5000 and 0.5 mm a pixel samples.
fn red_box(rgb: &Rgb, window: [f64; 4]) -> [usize; 4] {
    let s = rgb_sampler(rgb, Some(32630)).unwrap();
    let (cols, rows) = (
        ((window[2] - window[0]) / 2.5) as usize,
        ((window[3] - window[1]) / 2.5) as usize,
    );
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for r in 0..rows {
        for c in 0..cols {
            xs.push(window[0] + (c as f64 + 0.5) * 2.5);
            ys.push(window[3] - (r as f64 + 0.5) * 2.5);
        }
    }
    let px = s(&xs, &ys);
    let mut b = [usize::MAX, usize::MAX, 0, 0];
    for (i, p) in px.iter().enumerate() {
        if p.is_some_and(|q| q[0] > 150 && q[1] < 100) {
            let (c, r) = (i % cols, i / cols);
            b = [b[0].min(c), b[1].min(r), b[2].max(c), b[3].max(r)];
        }
    }
    assert!(b[0] <= b[2], "red cells found");
    b
}

#[test]
fn an_orthophoto_in_wgs84_and_one_in_utm_place_the_same_image_within_one_pixel() {
    let utm_box = [500_500.0, 599_500.0, 501_700.0, 600_500.0];
    let utm = geotiff::read_rgb_bytes(&orthophoto(32630, utm_box, 480, 400)).unwrap();
    // the same ground in WGS84: the box of the UTM corners, at about the same pixel size
    let ll = zone()
        .inverse_all(&[
            [utm_box[0], utm_box[1]],
            [utm_box[2], utm_box[3]],
            [utm_box[0], utm_box[3]],
            [utm_box[2], utm_box[1]],
        ])
        .unwrap();
    let lon = (
        ll.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
        ll.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
    );
    let lat = (
        ll.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        ll.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
    );
    let wgs =
        geotiff::read_rgb_bytes(&orthophoto(4326, [lon.0, lat.0, lon.1, lat.1], 480, 400)).unwrap();
    assert_eq!(wgs.epsg, Some(4326));
    let window = [500_800.0, 599_700.0, 501_400.0, 600_300.0];
    let (a, b) = (red_box(&utm, window), red_box(&wgs, window));
    for i in 0..4 {
        assert!(a[i].abs_diff(b[i]) <= 1, "UTM {a:?}, WGS84 {b:?}");
    }
    // and the red square is where the pattern puts it: 200 m is 80 cells of 2.5 m
    assert_eq!((a[2] - a[0] + 1, a[3] - a[1] + 1), (80, 80));
}

#[test]
fn an_orthophoto_draws_the_plan_over_it() {
    let (text, dem) = example();
    let rgb = geotiff::read_rgb_bytes(&orthophoto(
        32630,
        [499_500.0, 599_000.0, 503_500.0, 601_000.0],
        400,
        200,
    ))
    .unwrap();
    let o = DrawOptions {
        sheets: vec![SheetKind::Plan],
        ..DrawOptions::default()
    };
    let sheets =
        drawing::sheets_from_text_with(&text, Some(&dem), &o, &Background::Orthophoto(&rgb))
            .unwrap();
    for s in &sheets {
        assert!(
            s.extra.iter().any(|(n, _)| n.ends_with("-orthophoto.jpg")),
            "{}",
            s.number
        );
        assert!(
            notes(s)
                .iter()
                .any(|n| n.contains("OVER THE ORTHOPHOTO GIVEN"))
        );
        drawing::render(s).expect("otd accepts it");
    }
}

/// Synthetic tiles: one 256 x 256 green tile for every request, with a log.
struct Tiles {
    fail: bool,
    asked: std::cell::RefCell<usize>,
}

impl TileFetch for Tiles {
    fn fetch(&self, source: Source, _tile: Tile) -> Result<Vec<u8>, FetchError> {
        assert_eq!(source, Source::EoxCloudless2016, "the only source");
        *self.asked.borrow_mut() += 1;
        if self.fail {
            return Err(FetchError::Network("timed out".into()));
        }
        let n = TILE_PX as usize;
        Ok(jpeg_rgb(
            &Grid::new(n, n, [0.0, 0.0, 1.0, 1.0], vec![[40, 120, 60]; n * n]).unwrap(),
            90,
        )
        .unwrap())
    }
}

fn notes(s: &Sheet) -> Vec<String> {
    s.document["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

fn plan_over(tiles: &Tiles) -> Vec<Sheet> {
    let (text, dem) = example();
    let o = DrawOptions {
        sheets: vec![SheetKind::Plan],
        ..DrawOptions::default()
    };
    drawing::sheets_from_text_with(&text, Some(&dem), &o, &Background::Imagery(tiles)).unwrap()
}

#[test]
fn fetched_imagery_prints_its_attribution_and_its_year_on_every_sheet() {
    let tiles = Tiles {
        fail: false,
        asked: Default::default(),
    };
    let sheets = plan_over(&tiles);
    assert!(*tiles.asked.borrow() > 0);
    for s in &sheets {
        assert!(
            s.extra.iter().any(|(n, _)| n.ends_with("-imagery.jpg")),
            "{}",
            s.number
        );
        let n = notes(s);
        assert!(n.iter().any(|l| l == ATTRIBUTION), "{}: {n:?}", s.number);
        assert_eq!(
            ATTRIBUTION,
            "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)"
        );
        assert!(n.iter().any(|l| l.contains("EOXCLOUDLESS 2016") && l.contains("MAY HAVE MOVED SINCE 2016")), "{n:?}");
        assert!(
            n.iter().all(|l| l.chars().count() <= 120),
            "the standard's note length"
        );
        let out = drawing::render(s).expect("otd accepts it");
        assert!(
            out.warnings.iter().all(|w| !w.contains("could not")),
            "{:?}",
            out.warnings
        );
    }
}

#[test]
fn a_failed_fetch_falls_back_to_shaded_relief_with_a_message() {
    let tiles = Tiles {
        fail: true,
        asked: Default::default(),
    };
    let sheets = plan_over(&tiles);
    for s in &sheets {
        assert!(
            s.extra.iter().any(|(n, _)| n.ends_with("-relief.jpg")),
            "{}",
            s.number
        );
        assert!(
            !notes(s).iter().any(|l| l == ATTRIBUTION),
            "no attribution without the imagery"
        );
        assert!(notes(s).iter().any(|l| l.contains("COULD NOT BE FETCHED")));
        let out = drawing::render(s).expect("a failed fetch is not an error");
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("the imagery fetch failed: timed out")
                    && w.contains("shaded relief instead")),
            "{:?}",
            out.warnings
        );
    }
}
