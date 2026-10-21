//! A windowed reader for big tiled GeoTIFFs over [`Fetch`] range requests (ADR 0010: read by window, so
//! a 10 degree JRC tile never sits whole in memory, in the browser least of all).
//!
//! It reads the header and the directory, then only the tiles a window needs. What it reads: classic
//! little-endian TIFF, tiled, one 8-bit or 16-bit band, uncompressed, LZW or DEFLATE, with no predictor or the horizontal
//! one; geographic or projected north-up georeferencing from `ModelPixelScale` and `ModelTiepoint`.
//! Anything else is refused with a message that names it. The directory may sit at the end of the
//! file, as it does in the JRC tiles.

use crate::error::ReachError;
use crate::fetch::Fetch;

/// A tiled 8-bit raster, opened but not read.
#[derive(Debug, Clone)]
pub struct TiledTiff {
    url: String,
    pub width: usize,
    pub height: usize,
    tile_w: usize,
    tile_h: usize,
    bits: u16,
    compression: u16,
    predictor: u16,
    /// The EPSG code of the file's CRS (projected or geographic), if it names one.
    pub epsg: Option<u32>,
    offsets: Vec<u64>,
    counts: Vec<u64>,
    /// Top-left corner of the top-left pixel, in the file's CRS units.
    pub origin: [f64; 2],
    /// Pixel size in CRS units, both positive (north-up).
    pub pixel: [f64; 2],
}

fn bad(url: &str, what: &str) -> ReachError {
    ReachError::Parse(format!("{url}: {what}"))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn f64_at(b: &[u8], i: usize) -> f64 {
    f64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"))
}

/// One directory entry.
struct Entry {
    typ: u16,
    count: usize,
    /// The 4 value bytes: the value itself, or the offset of the values.
    raw: [u8; 4],
}

impl Entry {
    fn size(&self) -> usize {
        match self.typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => 0,
        }
    }

    fn inline(&self) -> bool {
        self.size() * self.count <= 4
    }

    fn offset(&self) -> u64 {
        u64::from(u32::from_le_bytes(self.raw))
    }

    /// The values as unsigned integers.
    fn ints(&self, fetch: &dyn Fetch, url: &str) -> Result<Vec<u64>, ReachError> {
        let n = self.size() * self.count;
        let data = if self.inline() {
            self.raw[..n].to_vec()
        } else {
            fetch.get_range(url, self.offset(), n as u64)?
        };
        if data.len() < n {
            return Err(bad(url, "the directory points past the end of the file"));
        }
        Ok(match self.typ {
            3 => (0..self.count)
                .map(|i| u64::from(u16_at(&data, 2 * i)))
                .collect(),
            4 => (0..self.count)
                .map(|i| u64::from(u32_at(&data, 4 * i)))
                .collect(),
            1 => data.iter().map(|b| u64::from(*b)).collect(),
            _ => return Err(bad(url, "a tag has a type this reader does not use")),
        })
    }

    fn doubles(&self, fetch: &dyn Fetch, url: &str) -> Result<Vec<f64>, ReachError> {
        if self.typ != 12 {
            return Err(bad(url, "a georeferencing tag is not DOUBLE"));
        }
        let n = 8 * self.count;
        let data = fetch.get_range(url, self.offset(), n as u64)?;
        if data.len() < n {
            return Err(bad(url, "the directory points past the end of the file"));
        }
        Ok((0..self.count).map(|i| f64_at(&data, 8 * i)).collect())
    }
}

impl TiledTiff {
    /// Read the header and directory of the file at `url`.
    pub fn open(fetch: &dyn Fetch, url: &str) -> Result<TiledTiff, ReachError> {
        let head = fetch.get_range(url, 0, 8)?;
        if head.len() < 8 || &head[..2] != b"II" {
            return Err(bad(url, "not a little-endian TIFF"));
        }
        match u16_at(&head, 2) {
            42 => {}
            43 => return Err(bad(url, "BigTIFF is not read")),
            _ => return Err(bad(url, "not a TIFF")),
        }
        let ifd = u64::from(u32_at(&head, 4));
        // The directory is short (tens of entries); the value arrays are fetched one by one.
        let dir = fetch.get_range(url, ifd, 2 + 12 * 64 + 4)?;
        if dir.len() < 2 {
            return Err(bad(url, "no directory at the offset in the header"));
        }
        let n = usize::from(u16_at(&dir, 0));
        if dir.len() < 2 + 12 * n {
            return Err(bad(url, "the directory is cut short"));
        }
        let mut entries: Vec<(u16, Entry)> = Vec::new();
        for i in 0..n {
            let at = 2 + 12 * i;
            entries.push((
                u16_at(&dir, at),
                Entry {
                    typ: u16_at(&dir, at + 2),
                    count: u32_at(&dir, at + 4) as usize,
                    raw: dir[at + 8..at + 12].try_into().expect("4 bytes"),
                },
            ));
        }
        let get = |tag: u16| entries.iter().find(|(t, _)| *t == tag).map(|(_, e)| e);
        let one = |tag: u16, default: Option<u64>| -> Result<u64, ReachError> {
            match get(tag) {
                Some(e) => e
                    .ints(fetch, url)?
                    .first()
                    .copied()
                    .ok_or_else(|| bad(url, "an empty tag")),
                None => default.ok_or_else(|| bad(url, &format!("tag {tag} is missing"))),
            }
        };
        let width = one(256, None)? as usize;
        let height = one(257, None)? as usize;
        let bits = one(258, Some(8))? as u16;
        if (bits != 8 && bits != 16) || one(277, Some(1))? != 1 {
            return Err(bad(url, "only one 8-bit or 16-bit band is read"));
        }
        if one(339, Some(1))? != 1 {
            return Err(bad(url, "only unsigned integer samples are read"));
        }
        let compression = one(259, Some(1))? as u16;
        if !matches!(compression, 1 | 5 | 8 | 32946) {
            return Err(bad(
                url,
                &format!("compression {compression} is not read (only none, LZW and DEFLATE)"),
            ));
        }
        let predictor = one(317, Some(1))? as u16;
        if predictor != 1 && predictor != 2 {
            return Err(bad(url, &format!("predictor {predictor} is not read")));
        }
        let (Some(tw), Some(th)) = (get(322), get(323)) else {
            return Err(bad(url, "not tiled (only tiled files are read by window)"));
        };
        let tile_w = tw.ints(fetch, url)?[0] as usize;
        let tile_h = th.ints(fetch, url)?[0] as usize;
        let offsets = get(324)
            .ok_or_else(|| bad(url, "no TileOffsets"))?
            .ints(fetch, url)?;
        let counts = get(325)
            .ok_or_else(|| bad(url, "no TileByteCounts"))?
            .ints(fetch, url)?;
        let across = width.div_ceil(tile_w);
        let down = height.div_ceil(tile_h);
        if offsets.len() != across * down || counts.len() != offsets.len() {
            return Err(bad(url, "the tile tables do not match the image size"));
        }
        let scale = get(33550)
            .ok_or_else(|| bad(url, "no georeferencing (ModelPixelScale)"))?
            .doubles(fetch, url)?;
        let tie = get(33922)
            .ok_or_else(|| bad(url, "no georeferencing (ModelTiepoint)"))?
            .doubles(fetch, url)?;
        if scale.len() < 2 || tie.len() < 6 {
            return Err(bad(url, "the georeferencing tags are too short"));
        }
        // Raster type: GeoKey 1025, 1 = PixelIsArea, 2 = PixelIsPoint.
        let mut point = false;
        let mut epsg = None;
        if let Some(k) = get(34735) {
            let keys = k.ints(fetch, url)?;
            for key in keys.chunks(4).skip(1) {
                if key.len() == 4 && key[0] == 1025 && key[3] == 2 {
                    point = true;
                }
                // ProjectedCSTypeGeoKey, or GeographicTypeGeoKey; 32767 means user-defined.
                if key.len() == 4 && (key[0] == 3072 || key[0] == 2048) && key[3] != 32767 {
                    epsg = Some(key[3] as u32);
                }
            }
        }
        let (px, py) = (scale[0], scale[1]);
        // The tie point maps raster (i, j) to (x, y): the corner of pixel (0, 0) for PixelIsArea.
        let mut ox = tie[3] - tie[0] * px;
        let mut oy = tie[4] + tie[1] * py;
        if point {
            ox -= px / 2.0;
            oy += py / 2.0;
        }
        Ok(TiledTiff {
            url: url.to_string(),
            width,
            height,
            tile_w,
            tile_h,
            bits,
            compression,
            predictor,
            epsg,
            offsets,
            counts,
            origin: [ox, oy],
            pixel: [px, py],
        })
    }

    fn tile(&self, fetch: &dyn Fetch, tx: usize, ty: usize) -> Result<Vec<u8>, ReachError> {
        let across = self.width.div_ceil(self.tile_w);
        let i = ty * across + tx;
        let (off, len) = (self.offsets[i], self.counts[i]);
        let bps = usize::from(self.bits / 8);
        let want = self.tile_w * self.tile_h * bps;
        if len == 0 {
            return Ok(vec![0; want]);
        }
        let raw = fetch.get_range(&self.url, off, len)?;
        if (raw.len() as u64) < len {
            return Err(bad(&self.url, "a tile is cut short"));
        }
        let mut px = match self.compression {
            5 => weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
                .decode(&raw)
                .map_err(|e| bad(&self.url, &format!("a tile does not decode: {e}")))?,
            8 | 32946 => miniz_oxide::inflate::decompress_to_vec_zlib(&raw)
                .map_err(|e| bad(&self.url, &format!("a tile does not inflate: {e:?}")))?,
            _ => raw,
        };
        if px.len() < want {
            return Err(bad(
                &self.url,
                "a tile decodes to fewer pixels than its size",
            ));
        }
        px.truncate(want);
        if self.predictor == 2 {
            for row in px.chunks_mut(self.tile_w * bps) {
                if bps == 1 {
                    for x in 1..row.len() {
                        row[x] = row[x].wrapping_add(row[x - 1]);
                    }
                } else {
                    for x in 1..self.tile_w {
                        let prev = u16::from_le_bytes([row[2 * x - 2], row[2 * x - 1]]);
                        let cur = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]);
                        let v = cur.wrapping_add(prev).to_le_bytes();
                        row[2 * x] = v[0];
                        row[2 * x + 1] = v[1];
                    }
                }
            }
        }
        Ok(px)
    }

    /// The pixels of a window of an 8-bit file: `cols` by `rows` from pixel (`col0`, `row0`). Pixels
    /// outside the image are 0. Only the tiles the window touches are fetched.
    pub fn read_window(
        &self,
        fetch: &dyn Fetch,
        col0: i64,
        row0: i64,
        cols: usize,
        rows: usize,
    ) -> Result<Vec<u8>, ReachError> {
        if self.bits != 8 {
            return Err(bad(
                &self.url,
                "this file has 16-bit pixels: read it with read_window16",
            ));
        }
        self.window_bytes(fetch, col0, row0, cols, rows)
    }

    /// The same for a 16-bit file.
    pub fn read_window16(
        &self,
        fetch: &dyn Fetch,
        col0: i64,
        row0: i64,
        cols: usize,
        rows: usize,
    ) -> Result<Vec<u16>, ReachError> {
        if self.bits != 16 {
            return Err(bad(
                &self.url,
                "this file has 8-bit pixels: read it with read_window",
            ));
        }
        let b = self.window_bytes(fetch, col0, row0, cols, rows)?;
        Ok(b.as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes(*p))
            .collect())
    }

    fn window_bytes(
        &self,
        fetch: &dyn Fetch,
        col0: i64,
        row0: i64,
        cols: usize,
        rows: usize,
    ) -> Result<Vec<u8>, ReachError> {
        let bps = usize::from(self.bits / 8);
        let mut out = vec![0u8; cols * rows * bps];
        let (c1, r1) = (col0 + cols as i64, row0 + rows as i64);
        if c1 <= 0 || r1 <= 0 || col0 >= self.width as i64 || row0 >= self.height as i64 {
            return Ok(out);
        }
        let tx0 = (col0.max(0) as usize) / self.tile_w;
        let ty0 = (row0.max(0) as usize) / self.tile_h;
        let tx1 = ((c1.min(self.width as i64) - 1).max(0) as usize) / self.tile_w;
        let ty1 = ((r1.min(self.height as i64) - 1).max(0) as usize) / self.tile_h;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let px = self.tile(fetch, tx, ty)?;
                let (tc, tr) = ((tx * self.tile_w) as i64, (ty * self.tile_h) as i64);
                for y in 0..self.tile_h as i64 {
                    let gy = tr + y;
                    if gy < row0 || gy >= r1 || gy >= self.height as i64 {
                        continue;
                    }
                    let xa = tc.max(col0);
                    let xb = (tc + self.tile_w as i64).min(c1).min(self.width as i64);
                    if xa >= xb {
                        continue;
                    }
                    let src = ((y as usize) * self.tile_w + (xa - tc) as usize) * bps;
                    let dst = (((gy - row0) as usize) * cols + (xa - col0) as usize) * bps;
                    let n = (xb - xa) as usize * bps;
                    out[dst..dst + n].copy_from_slice(&px[src..src + n]);
                }
            }
        }
        Ok(out)
    }
}

/// How `write_test_tiff` compresses its tiles.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TestCodec {
    None,
    Lzw,
    /// DEFLATE with the horizontal predictor, as Sentinel-2 COGs are written.
    DeflatePredictor,
}

/// Build a small tiled, georeferenced 8-bit TIFF in memory, laid out like a JRC tile (data first, the
/// directory last). For tests and fixtures only.
#[doc(hidden)]
pub fn write_test_tiff(
    width: usize,
    height: usize,
    tile: usize,
    pixels: &[u8],
    lzw: bool,
    origin: [f64; 2],
    pixel: [f64; 2],
) -> Vec<u8> {
    let codec = if lzw { TestCodec::Lzw } else { TestCodec::None };
    build_test_tiff(width, height, tile, 8, pixels, codec, origin, pixel, 4326)
}

/// The same for a 16-bit band, with a CRS (`epsg`: 4326 geographic, or a UTM code).
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn write_test_tiff16(
    width: usize,
    height: usize,
    tile: usize,
    pixels: &[u16],
    codec: TestCodec,
    origin: [f64; 2],
    pixel: [f64; 2],
    epsg: u16,
) -> Vec<u8> {
    let bytes: Vec<u8> = pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
    build_test_tiff(width, height, tile, 16, &bytes, codec, origin, pixel, epsg)
}

#[allow(clippy::too_many_arguments)]
fn build_test_tiff(
    width: usize,
    height: usize,
    tile: usize,
    bits: u16,
    pixels: &[u8],
    codec: TestCodec,
    origin: [f64; 2],
    pixel: [f64; 2],
    epsg: u16,
) -> Vec<u8> {
    let bps = usize::from(bits / 8);
    assert_eq!(pixels.len(), width * height * bps);
    let across = width.div_ceil(tile);
    let down = height.div_ceil(tile);
    let mut out = vec![b'I', b'I', 42, 0, 0, 0, 0, 0];
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for ty in 0..down {
        for tx in 0..across {
            let mut block = vec![0u8; tile * tile * bps];
            for y in 0..tile {
                for x in 0..tile {
                    let (gx, gy) = (tx * tile + x, ty * tile + y);
                    if gx < width && gy < height {
                        let s = (gy * width + gx) * bps;
                        let d = (y * tile + x) * bps;
                        block[d..d + bps].copy_from_slice(&pixels[s..s + bps]);
                    }
                }
            }
            if codec == TestCodec::DeflatePredictor {
                // Horizontal differencing, last pixel first so each uses its untouched left neighbour.
                for row in block.chunks_mut(tile * bps) {
                    for x in (1..tile).rev() {
                        if bps == 1 {
                            row[x] = row[x].wrapping_sub(row[x - 1]);
                        } else {
                            let prev = u16::from_le_bytes([row[2 * x - 2], row[2 * x - 1]]);
                            let cur = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]);
                            let v = cur.wrapping_sub(prev).to_le_bytes();
                            row[2 * x] = v[0];
                            row[2 * x + 1] = v[1];
                        }
                    }
                }
            }
            let data = match codec {
                TestCodec::None => block,
                TestCodec::Lzw => {
                    weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
                        .encode(&block)
                        .expect("lzw encodes")
                }
                TestCodec::DeflatePredictor => {
                    miniz_oxide::deflate::compress_to_vec_zlib(&block, 6)
                }
            };
            offsets.push(out.len() as u32);
            counts.push(data.len() as u32);
            out.extend_from_slice(&data);
        }
    }
    let put_arr = |out: &mut Vec<u8>, bytes: Vec<u8>| -> u32 {
        let at = out.len() as u32;
        out.extend_from_slice(&bytes);
        at
    };
    let off_at = put_arr(
        &mut out,
        offsets.iter().flat_map(|v| v.to_le_bytes()).collect(),
    );
    let cnt_at = put_arr(
        &mut out,
        counts.iter().flat_map(|v| v.to_le_bytes()).collect(),
    );
    let scale_at = put_arr(
        &mut out,
        [pixel[0], pixel[1], 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
    );
    let tie_at = put_arr(
        &mut out,
        [0.0, 0.0, 0.0, origin[0], origin[1], 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
    );
    let key = if epsg == 4326 { 2048 } else { 3072 };
    let model = if epsg == 4326 { 2 } else { 1 };
    let keys: Vec<u16> = vec![
        1, 1, 0, 3, 1024, 0, 1, model, 1025, 0, 1, 1, key, 0, 1, epsg,
    ];
    let keys_at = put_arr(
        &mut out,
        keys.iter().flat_map(|v| v.to_le_bytes()).collect(),
    );
    let ifd_at = out.len() as u32;
    let short = |tag: u16, v: u16| (tag, 3u16, 1u32, u32::from(v));
    let long = |tag: u16, v: u32| (tag, 4u16, 1u32, v);
    let (comp, pred) = match codec {
        TestCodec::None => (1, 1),
        TestCodec::Lzw => (5, 1),
        TestCodec::DeflatePredictor => (8, 2),
    };
    let mut e = vec![
        long(256, width as u32),
        long(257, height as u32),
        short(258, bits),
        short(259, comp),
        short(262, 1),
        short(277, 1),
        short(284, 1),
        short(317, pred),
        short(322, tile as u16),
        short(323, tile as u16),
        (324, 4, offsets.len() as u32, off_at),
        (325, 4, counts.len() as u32, cnt_at),
        short(339, 1),
        (33550, 12, 3, scale_at),
        (33922, 12, 6, tie_at),
        (34735, 3, keys.len() as u32, keys_at),
    ];
    e.sort_by_key(|x| x.0);
    out.extend_from_slice(&(e.len() as u16).to_le_bytes());
    for (tag, typ, count, val) in e {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&val.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out[4..8].copy_from_slice(&ifd_at.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::fetch::ReplayFetch;

    fn ramp(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .map(|i| ((i * 7 + i / w * 3) % 251) as u8)
            .collect()
    }

    #[test]
    fn a_window_reads_back_exactly_for_both_compressions_and_across_tiles() {
        let (w, h) = (70, 45);
        let px = ramp(w, h);
        for lzw in [false, true] {
            let file = write_test_tiff(w, h, 16, &px, lzw, [10.0, 20.0], [0.5, 0.25]);
            let f = ReplayFetch::new().with_response("mem://t.tif", file);
            let t = TiledTiff::open(&f, "mem://t.tif").unwrap();
            assert_eq!((t.width, t.height), (w, h));
            assert_eq!(t.origin, [10.0, 20.0]);
            assert_eq!(t.pixel, [0.5, 0.25]);
            let got = t.read_window(&f, 13, 7, 40, 30).unwrap();
            for y in 0..30 {
                for x in 0..40 {
                    assert_eq!(
                        got[y * 40 + x],
                        px[(7 + y) * w + 13 + x],
                        "lzw={lzw} {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_16_bit_deflate_predictor_file_reads_back_exactly_and_names_its_crs() {
        let (w, h) = (70, 45);
        let px: Vec<u16> = (0..w * h)
            .map(|i| ((i * 37 + i / w * 11) % 20_000) as u16 + 60_000 * (i % 3 == 0) as u16 / 3)
            .collect();
        let file = write_test_tiff16(
            w,
            h,
            16,
            &px,
            TestCodec::DeflatePredictor,
            [500_000.0, 600_000.0],
            [10.0, 10.0],
            32629,
        );
        let f = ReplayFetch::new().with_response("mem://s2.tif", file);
        let t = TiledTiff::open(&f, "mem://s2.tif").unwrap();
        assert_eq!(t.epsg, Some(32629));
        let got = t.read_window16(&f, 13, 7, 40, 30).unwrap();
        for y in 0..30 {
            for x in 0..40 {
                assert_eq!(got[y * 40 + x], px[(7 + y) * w + 13 + x], "{x},{y}");
            }
        }
        assert!(t.read_window(&f, 0, 0, 2, 2).is_err());
    }

    #[test]
    fn a_window_fetches_only_the_tiles_it_touches() {
        let (w, h) = (256, 256);
        let file = write_test_tiff(w, h, 16, &ramp(w, h), true, [0.0, 0.0], [1.0, 1.0]);
        let f = ReplayFetch::new().with_response("mem://t.tif", file);
        let t = TiledTiff::open(&f, "mem://t.tif").unwrap();
        let before = f.requests().len();
        t.read_window(&f, 0, 0, 20, 20).unwrap(); // 2 x 2 tiles
        assert_eq!(f.requests().len() - before, 4);
    }

    #[test]
    fn pixels_outside_the_image_are_zero_and_unreadable_files_are_refused() {
        let (w, h) = (20, 20);
        let file = write_test_tiff(w, h, 16, &vec![9u8; w * h], false, [0.0, 0.0], [1.0, 1.0]);
        let f = ReplayFetch::new().with_response("mem://t.tif", file);
        let t = TiledTiff::open(&f, "mem://t.tif").unwrap();
        let g = t.read_window(&f, -2, -2, 4, 4).unwrap();
        assert_eq!(g[0], 0);
        assert_eq!(g[2 * 4 + 2], 9);
        assert!(
            t.read_window(&f, 100, 100, 3, 3)
                .unwrap()
                .iter()
                .all(|v| *v == 0)
        );
        let g = ReplayFetch::new().with_response("mem://x", b"MM\0*....".to_vec());
        assert!(
            TiledTiff::open(&g, "mem://x")
                .unwrap_err()
                .to_string()
                .contains("little-endian")
        );
        let g = ReplayFetch::new().with_response("mem://x", b"II+\0........".to_vec());
        assert!(
            TiledTiff::open(&g, "mem://x")
                .unwrap_err()
                .to_string()
                .contains("BigTIFF")
        );
    }
}
