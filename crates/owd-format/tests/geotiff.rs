//! The pure-Rust GeoTIFF reader, held to what rasterio (GDAL) reads from the same files.
//!
//! `tests/fixtures/dem/*.tif` are written by `tools/make_dem_fixtures.py`, and `expected.json` holds
//! what rasterio reports for each: size, geotransform, nodata, EPSG code, a SHA-256 of every pixel
//! (as little-endian float64, nodata kept), the first and last rows, and bilinear heights at probe
//! points. The `refuse_*` files must be refused with a message that names what is missing.
//!
//! No Python needed: it reads the committed files.
//!
//! Label: snapshot. `expected.json` is what rasterio 1.5.2 / GDAL 3.12.2 printed. Never hand-edit it.

use owd_format::geotiff::{Dem, GeoTiffError, read_file};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dem")
}

fn expected() -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir().join("expected.json")).unwrap()).unwrap()
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) if s == "float:nan" => f64::NAN,
        _ => v.as_f64().unwrap(),
    }
}

fn pixel_hash(dem: &Dem) -> String {
    let mut bytes = Vec::with_capacity(dem.data.len() * 8);
    for &v in &dem.data {
        let v = if v.is_nan() {
            f64::from_bits(0x7ff8_0000_0000_0000)
        } else {
            v
        };
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn same(a: f64, b: f64, tol: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= tol * b.abs().max(1.0)
}

#[test]
fn every_fixture_reads_as_rasterio_reads_it() {
    let exp = expected();
    let fixtures = exp.as_object().unwrap();
    assert!(fixtures.len() >= 12, "the fixture set shrank");
    let mut worst_height_diff: f64 = 0.0;
    for (name, e) in fixtures {
        let dem = read_file(&dir().join(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(
            dem.width as u64,
            e["width"].as_u64().unwrap(),
            "{name} width"
        );
        assert_eq!(
            dem.height as u64,
            e["height"].as_u64().unwrap(),
            "{name} height"
        );
        for (i, g) in e["geotransform"].as_array().unwrap().iter().enumerate() {
            assert!(
                same(dem.geotransform[i], num(g), 1e-12),
                "{name} geotransform[{i}]"
            );
        }
        assert_eq!(
            dem.nodata
                .map(f64::to_bits)
                .map(|b| if f64::from_bits(b).is_nan() { 0 } else { b }),
            e["nodata"].as_ref_or_null().map(|v| {
                let n = num(v);
                if n.is_nan() { 0 } else { n.to_bits() }
            }),
            "{name} nodata"
        );
        assert_eq!(dem.epsg.map(u64::from), e["epsg"].as_u64(), "{name} EPSG");
        assert_eq!(
            pixel_hash(&dem),
            e["pixels_sha256"].as_str().unwrap(),
            "{name}: pixels differ"
        );
        let w = dem.width;
        for (k, v) in e["first_row"].as_array().unwrap().iter().enumerate() {
            assert!(
                same(dem.data[k], num(v), 0.0) || dem.data[k] == num(v),
                "{name} first row {k}"
            );
        }
        for (k, v) in e["last_row"].as_array().unwrap().iter().enumerate() {
            let got = dem.data[(dem.height - 1) * w + k];
            assert!(
                got == num(v) || (got.is_nan() && num(v).is_nan()),
                "{name} last row {k}"
            );
        }
        for p in e["probes"].as_array().unwrap() {
            let (x, y, want) = (num(&p["x"]), num(&p["y"]), num(&p["z"]));
            let got = dem.sample_bilinear(x, y);
            assert!(
                (got.is_nan() && want.is_nan()) || (got - want).abs() <= 1e-6,
                "{name}: height at ({x}, {y}) is {got}, rasterio gives {want}"
            );
            if got.is_finite() {
                worst_height_diff = worst_height_diff.max((got - want).abs());
            }
        }
    }
    println!("largest height difference from rasterio: {worst_height_diff:e} m");
}

trait AsRefOrNull {
    fn as_ref_or_null(&self) -> Option<&Value>;
}

impl AsRefOrNull for Value {
    fn as_ref_or_null(&self) -> Option<&Value> {
        if self.is_null() { None } else { Some(self) }
    }
}

fn refusal(name: &str) -> String {
    match read_file(&dir().join(name)) {
        Err(e) => e.to_string(),
        Ok(d) => panic!(
            "{name} was read as a {} x {} raster, expected a refusal",
            d.width, d.height
        ),
    }
}

#[test]
fn an_unsupported_geotiff_is_refused_with_a_message_that_names_what_is_missing() {
    let zstd = refusal("refuse_zstd.tif");
    assert!(zstd.contains("ZSTD") && zstd.contains("50000"), "{zstd}");
    assert!(
        zstd.contains("DEFLATE") && zstd.contains("LZW"),
        "it should say what is handled: {zstd}"
    );
    assert!(refusal("refuse_packbits.tif").contains("PackBits"));
    assert!(refusal("refuse_two_bands.tif").contains("2 bands"));
    assert!(refusal("refuse_rotated.tif").contains("rotated"));
    assert!(refusal("refuse_no_georef.tif").contains("no georeferencing"));
}

#[test]
fn a_file_that_is_not_a_tiff_or_is_cut_short_is_refused_not_misread() {
    assert!(matches!(
        owd_format::geotiff::read_bytes(b"not a tiff at all"),
        Err(GeoTiffError::Corrupt(_))
    ));
    assert!(matches!(
        owd_format::geotiff::read_bytes(&[]),
        Err(GeoTiffError::Corrupt(_))
    ));
    assert!(matches!(
        read_file(&dir().join("no_such_file.tif")),
        Err(GeoTiffError::Io(_))
    ));
    // A file cut anywhere is either refused or still holds exactly the pixels of the whole file
    // (the cut fell in trailing metadata): it is never a different raster.
    for name in [
        "f32_tiled_deflate_pred3.tif",
        "i16_tiled_lzw.tif",
        "f32_strip_none.tif",
    ] {
        let good = std::fs::read(dir().join(name)).unwrap();
        let whole = owd_format::geotiff::read_bytes(&good).unwrap();
        let mut refused = 0;
        for k in 1..60 {
            let cut = good.len() * k / 60;
            match owd_format::geotiff::read_bytes(&good[..cut]) {
                Err(_) => refused += 1,
                Ok(d) => assert_eq!(
                    d, whole,
                    "{name} cut at {cut} was read as a different raster"
                ),
            }
        }
        assert!(
            refused >= 30,
            "{name}: only {refused} of 59 cuts were refused"
        );
    }
}

#[test]
fn the_copernicus_clip_is_read_with_its_half_pixel_shift() {
    let dem = read_file(&dir().join("glo30_n05_w003_clip.tif")).unwrap();
    assert_eq!((dem.width, dem.height, dem.epsg), (80, 80, Some(4326)));
    // GLO-30 is 1 arc second; the clip's outer corner is half a pixel beyond its first centre.
    let px = 1.0 / 3600.0;
    assert!((dem.geotransform[0] - px).abs() < 1e-15 && (dem.geotransform[4] + px).abs() < 1e-15);
    let (x0, y1, ..) = (dem.geotransform[2], dem.geotransform[5], 0);
    let centre = x0 + 0.5 * px;
    assert!(
        (centre * 3600.0 - (centre * 3600.0).round()).abs() < 1e-6,
        "the first centre is on a whole arc second"
    );
    assert!(y1 > 5.0 && y1 < 6.0);
    assert!(
        dem.data.iter().all(|v| (0.0..2000.0).contains(v)),
        "heights are land heights in metres"
    );
}

#[test]
fn the_build_has_no_native_gdal_dependency() {
    // The lock file lists every crate the workspace builds. None may be GDAL or a binding to it
    // (ADR 0003); the GeoTIFF reader is the `tiff` crate and the code in `src/geotiff.rs`.
    let lock =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .unwrap();
    let names: Vec<&str> = lock
        .lines()
        .filter_map(|l| l.strip_prefix("name = \""))
        .map(|l| l.trim_end_matches('"'))
        .collect();
    assert!(
        names.contains(&"tiff"),
        "the reader's crate is in the build"
    );
    for n in &names {
        let l = n.to_lowercase();
        assert!(
            !(l.starts_with("gdal")
                || l.contains("libgdal")
                || l.starts_with("geos")
                || l == "proj-sys"
                || l == "proj"),
            "a native geodata library is in the build: {n}"
        );
    }
}
