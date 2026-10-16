//! A pure-Rust GeoTIFF DEM reader (ADR 0003: no GDAL). Spec 0001, ticket 79.
//!
//! It reads the first image of a GeoTIFF into a [`Dem`]: size, pixels, geotransform, nodata and the
//! EPSG code. Decoding of strips and tiles, and of the DEFLATE and LZW compressions with the
//! horizontal and floating-point predictors, is done by the `tiff` crate (image-rs, pure Rust, MIT).
//! The GeoTIFF layer (geotransform, raster type, EPSG, nodata) is read here from the tags.
//!
//! What is read: one band of 8, 16 or 32 bit integers, or 32 or 64 bit floats; uncompressed, DEFLATE
//! or LZW; north-up georeferencing from `ModelPixelScale` + `ModelTiepoint` or from
//! `ModelTransformation` with no rotation. `PixelIsPoint` rasters (as the Copernicus tiles are) are
//! shifted half a pixel, as GDAL shifts them, so every geotransform here is the one rasterio reports.
//!
//! What is refused, with a message that names it: any other compression, more than one band, a
//! rotated or sheared transform, ground control points, a file with no georeferencing, 64 bit
//! integers, and a damaged file. A refusal never returns a raster.

use std::io::Cursor;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeoTiffError {
    /// The file could not be opened or read.
    Io(String),
    /// The bytes are not a TIFF, or the TIFF is damaged.
    Corrupt(String),
    /// A valid GeoTIFF the reader does not handle. The text names what is missing.
    Unsupported(String),
}

impl core::fmt::Display for GeoTiffError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GeoTiffError::Io(m) => write!(f, "cannot read the GeoTIFF: {m}"),
            GeoTiffError::Corrupt(m) => write!(f, "the GeoTIFF is damaged or not a TIFF: {m}"),
            GeoTiffError::Unsupported(m) => write!(f, "unsupported GeoTIFF: {m}"),
        }
    }
}

impl std::error::Error for GeoTiffError {}

fn unsupported<T>(message: impl Into<String>) -> Result<T, GeoTiffError> {
    Err(GeoTiffError::Unsupported(message.into()))
}

/// A DEM in memory. `data` is row-major, north row first, as the file stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Dem {
    pub width: usize,
    pub height: usize,
    /// Every pixel as `f64`, nodata values kept (see [`Dem::value`]).
    pub data: Vec<f64>,
    /// `[a, b, c, d, e, f]`: `x = a*col + b*row + c`, `y = d*col + e*row + f`, in the file's CRS, at
    /// the outer corner of pixel (0, 0). `b` and `d` are zero (a rotated file is refused).
    pub geotransform: [f64; 6],
    /// The nodata value, if the file gives one.
    pub nodata: Option<f64>,
    /// The EPSG code of the CRS, when the file names one (not for a user-defined CRS).
    pub epsg: Option<u32>,
}

impl Dem {
    /// The pixel at `(row, col)` with nodata as NaN.
    pub fn value(&self, row: usize, col: usize) -> f64 {
        let v = self.data[row * self.width + col];
        match self.nodata {
            Some(nd) if v == nd => f64::NAN,
            _ => v,
        }
    }

    /// Bilinear height at a world point. Pixel values sit at pixel centres; the answer is NaN
    /// outside the outermost pixel centres and wherever a neighbour is nodata. This is the formula of
    /// the Python `volumes.raster.bilinear` over the whole raster.
    pub fn sample_bilinear(&self, x: f64, y: f64) -> f64 {
        let [a, _, c, _, e, f] = self.geotransform;
        let col = (x - c) / a - 0.5;
        let row = (y - f) / e - 0.5;
        let (c0, r0) = (col.floor(), row.floor());
        let ok = c0 >= 0.0
            && r0 >= 0.0
            && c0 < (self.width as f64 - 1.0)
            && r0 < (self.height as f64 - 1.0);
        if !ok {
            return f64::NAN;
        }
        let (fc, fr) = (col - c0, row - r0);
        let (cc, rr) = (c0 as usize, r0 as usize);
        self.value(rr, cc) * (1.0 - fc) * (1.0 - fr)
            + self.value(rr, cc + 1) * fc * (1.0 - fr)
            + self.value(rr + 1, cc) * (1.0 - fc) * fr
            + self.value(rr + 1, cc + 1) * fc * fr
    }

    /// [`Dem::sample_bilinear`] for many points: the shape of the package's terrain sampler
    /// `terrain(xs, ys) -> z`, so `&|xs, ys| dem.sample_many(xs, ys)` can stand in for one. The
    /// Python `RasterTerrain` reads a window of the file per call (the bounds of the points plus
    /// 8 m); where the raster volume integrator needs that windowing it is wired over this reader.
    pub fn sample_many(&self, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| self.sample_bilinear(x, y))
            .collect()
    }

    /// `(xmin, ymin, xmax, ymax)` of the raster's outer edges.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let [a, _, c, _, e, f] = self.geotransform;
        let (x1, y1) = (c + a * self.width as f64, f + e * self.height as f64);
        (c.min(x1), f.min(y1), c.max(x1), f.max(y1))
    }
}

/// Read a GeoTIFF file.
pub fn read_file(path: &std::path::Path) -> Result<Dem, GeoTiffError> {
    let bytes =
        std::fs::read(path).map_err(|e| GeoTiffError::Io(format!("{}: {e}", path.display())))?;
    read_bytes(&bytes)
}

fn corrupt(e: tiff::TiffError) -> GeoTiffError {
    match e {
        tiff::TiffError::UnsupportedError(u) => GeoTiffError::Unsupported(u.to_string()),
        other => GeoTiffError::Corrupt(other.to_string()),
    }
}

/// The name of a TIFF compression code, for the message of a refusal.
fn compression_name(code: u16) -> String {
    let name = match code {
        2 => "CCITT Huffman",
        3 => "CCITT Group 3 fax",
        4 => "CCITT Group 4 fax",
        6 | 7 => "JPEG",
        32773 => "PackBits",
        34887 => "LERC",
        34925 => "LZMA",
        50000 => "ZSTD",
        50001 => "WebP",
        _ => "an unknown compression",
    };
    format!("{name} (TIFF compression code {code})")
}

fn geo_tag(d: &mut Decoder<Cursor<&[u8]>>, tag: Tag) -> Result<Option<Vec<f64>>, GeoTiffError> {
    match d.find_tag(tag).map_err(corrupt)? {
        Some(v) => v.into_f64_vec().map(Some).map_err(corrupt),
        None => Ok(None),
    }
}

/// `(epsg, pixel_is_point)` from the GeoKey directory.
fn geo_keys(d: &mut Decoder<Cursor<&[u8]>>) -> Result<(Option<u32>, bool), GeoTiffError> {
    let Some(v) = d.find_tag(Tag::GeoKeyDirectoryTag).map_err(corrupt)? else {
        return Ok((None, false));
    };
    let dir = v.into_u16_vec().map_err(corrupt)?;
    if dir.len() < 4 {
        return Err(GeoTiffError::Corrupt(
            "the GeoKey directory is truncated".into(),
        ));
    }
    let n = dir[3] as usize;
    if dir.len() < 4 + 4 * n {
        return Err(GeoTiffError::Corrupt(
            "the GeoKey directory is truncated".into(),
        ));
    }
    let (mut epsg, mut point) = (None, false);
    for k in 0..n {
        let e = &dir[4 + 4 * k..8 + 4 * k];
        let (key, location, value) = (e[0], e[1], e[3]);
        if location != 0 {
            continue; // the value lives in another tag; none of the keys read here does
        }
        match key {
            // ProjectedCSTypeGeoKey, then GeographicTypeGeoKey; 32767 is "user defined".
            3072 if value != 32767 => epsg = Some(u32::from(value)),
            2048 if value != 32767 && epsg.is_none() => epsg = Some(u32::from(value)),
            // GTRasterTypeGeoKey: 1 PixelIsArea, 2 PixelIsPoint.
            1025 => point = value == 2,
            _ => {}
        }
    }
    Ok((epsg, point))
}

fn geotransform(
    d: &mut Decoder<Cursor<&[u8]>>,
    pixel_is_point: bool,
) -> Result<[f64; 6], GeoTiffError> {
    let scale = geo_tag(d, Tag::ModelPixelScaleTag)?;
    let tie = geo_tag(d, Tag::ModelTiepointTag)?;
    let model = geo_tag(d, Tag::ModelTransformationTag)?;
    let mut gt = match (scale, tie, model) {
        (_, _, Some(m)) if m.len() == 16 => {
            if m[1] != 0.0 || m[4] != 0.0 {
                return unsupported(
                    "a rotated or sheared geotransform (ModelTransformation); only north-up rasters are read",
                );
            }
            [m[0], 0.0, m[3], 0.0, m[5], m[7]]
        }
        (Some(s), Some(t), None) if s.len() >= 2 && t.len() == 6 => {
            // One tiepoint (I, J, K, X, Y, Z) and the pixel scale; y runs down the rows.
            let (sx, sy) = (s[0], s[1]);
            [sx, 0.0, t[3] - t[0] * sx, 0.0, -sy, t[4] + t[1] * sy]
        }
        (_, Some(t), None) if t.len() > 6 => {
            return unsupported("more than one tiepoint (ground control points)");
        }
        _ => {
            return unsupported(
                "no georeferencing: the ModelPixelScale and ModelTiepoint tags (or ModelTransformation) are missing",
            );
        }
    };
    if pixel_is_point {
        // GDAL reads a PixelIsPoint raster with the origin moved half a pixel up and left.
        gt[2] -= gt[0] * 0.5;
        gt[5] -= gt[4] * 0.5;
    }
    Ok(gt)
}

/// Rewrite the ImageWidth entry of the first IFD, in a copy of the file. `None` if the entry cannot
/// be found or does not hold a value of that size inline.
fn with_image_width(bytes: &[u8], new_width: u32) -> Option<Vec<u8>> {
    let little = match bytes.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u64_at = |at: usize, n: usize| -> Option<u64> {
        let b = bytes.get(at..at + n)?;
        let mut v = 0u64;
        for i in 0..n {
            let byte = u64::from(if little { b[n - 1 - i] } else { b[i] });
            v = (v << 8) | byte;
        }
        Some(v)
    };
    let big = match u16_at(2)? {
        42 => false,
        43 => true,
        _ => return None,
    };
    let (ifd, count, entry_len, count_len) = if big {
        let ifd = u64_at(8, 8)? as usize;
        (ifd, u64_at(ifd, 8)? as usize, 20usize, 8usize)
    } else {
        let ifd = u64_at(4, 4)? as usize;
        (ifd, usize::from(u16_at(ifd)?), 12usize, 2usize)
    };
    for k in 0..count {
        let e = ifd + count_len + k * entry_len;
        if u16_at(e)? != 256 {
            continue;
        }
        let (field_type, n) = (u16_at(e + 2)?, u64_at(e + 4, if big { 8 } else { 4 })?);
        if n != 1 {
            return None;
        }
        let value_at = e + if big { 12 } else { 8 };
        let mut out = bytes.to_vec();
        match field_type {
            3 if new_width <= u32::from(u16::MAX) => {
                let w = new_width as u16;
                let b = if little {
                    w.to_le_bytes()
                } else {
                    w.to_be_bytes()
                };
                out[value_at..value_at + 2].copy_from_slice(&b);
            }
            4 => {
                let b = if little {
                    new_width.to_le_bytes()
                } else {
                    new_width.to_be_bytes()
                };
                out[value_at..value_at + 4].copy_from_slice(&b);
            }
            _ => return None,
        }
        return Some(out);
    }
    None
}

/// Read a GeoTIFF held in memory.
pub fn read_bytes(bytes: &[u8]) -> Result<Dem, GeoTiffError> {
    let mut d = Decoder::new(Cursor::new(bytes)).map_err(corrupt)?;
    let (w, h) = d.dimensions().map_err(corrupt)?;

    // KNOWN FAULT in the `tiff` crate (0.11.3): an LZW tile on the right edge of the raster, whose
    // visible part is narrower than the tile, is refused with "no lzw end code found" although the
    // stream is sound (it decodes with `weezl` alone). The fault is in how the crate skips the
    // padding at the end of each row. So a tiled LZW file whose width is not a whole number of tiles
    // is decoded as the padded raster the file really holds, and cropped afterwards. Remove this when
    // the crate reads such tiles (the fixture `i16_tiled_lzw.tif` fails without it).
    let tile_w = d
        .find_tag_unsigned::<u32>(Tag::TileWidth)
        .map_err(corrupt)?;
    let lzw = d
        .find_tag_unsigned::<u16>(Tag::Compression)
        .map_err(corrupt)?
        == Some(5);
    let padded = match tile_w {
        Some(tw) if lzw && tw > 0 && w % tw != 0 => Some(w.div_ceil(tw) * tw),
        _ => None,
    };
    let patched: Option<Vec<u8>> = padded.and_then(|pw| with_image_width(bytes, pw));
    if padded.is_some() && patched.is_none() {
        return unsupported(
            "a tiled LZW file whose width is not a whole number of tiles and whose layout the reader cannot rewrite",
        );
    }
    let mut d = match &patched {
        Some(p) => Decoder::new(Cursor::new(p.as_slice())).map_err(corrupt)?,
        None => Decoder::new(Cursor::new(bytes)).map_err(corrupt)?,
    };

    let bands = d
        .find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
        .map_err(corrupt)?
        .unwrap_or(1);
    if bands != 1 {
        return unsupported(format!(
            "{bands} bands; the reader handles single-band DEMs"
        ));
    }
    let compression = d
        .find_tag_unsigned::<u16>(Tag::Compression)
        .map_err(corrupt)?
        .unwrap_or(1);
    if !matches!(compression, 1 | 5 | 8 | 32946) {
        return unsupported(format!(
            "compression {}; the reader handles uncompressed, DEFLATE and LZW (with the horizontal or floating-point predictor)",
            compression_name(compression)
        ));
    }

    let (epsg, pixel_is_point) = geo_keys(&mut d)?;
    let gt = geotransform(&mut d, pixel_is_point)?;
    let nodata = match d.find_tag(Tag::GdalNodata).map_err(corrupt)? {
        Some(v) => {
            let text = v.into_string().map_err(corrupt)?;
            let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            Some(text.parse::<f64>().map_err(|_| {
                GeoTiffError::Corrupt(format!("the GDAL_NODATA value `{text}` is not a number"))
            })?)
        }
        None => None,
    };

    let data: Vec<f64> = match d.read_image().map_err(corrupt)? {
        DecodingResult::U8(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::U16(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::U32(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::I8(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::I16(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::I32(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::F32(v) => v.into_iter().map(f64::from).collect(),
        DecodingResult::F64(v) => v,
        _ => {
            return unsupported(
                "this pixel type (64 bit integers and 16 bit floats are not read); DEMs are 8, 16 or 32 bit integers or 32 or 64 bit floats",
            );
        }
    };
    let (width, height) = (w as usize, h as usize);
    let data = match padded {
        Some(pw) => {
            let pw = pw as usize;
            if data.len() != pw * height {
                return Err(GeoTiffError::Corrupt(format!(
                    "the image holds {} values, expected {} ({pw} x {height})",
                    data.len(),
                    pw * height
                )));
            }
            data.chunks(pw)
                .flat_map(|row| row[..width].iter().copied())
                .collect()
        }
        None => data,
    };
    if data.len() != width * height {
        return Err(GeoTiffError::Corrupt(format!(
            "the image holds {} values, expected {} ({width} x {height})",
            data.len(),
            width * height
        )));
    }
    Ok(Dem {
        width,
        height,
        data,
        geotransform: gt,
        nodata,
        epsg,
    })
}
