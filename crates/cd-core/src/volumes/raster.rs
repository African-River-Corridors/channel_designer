//! Raster DEM volumes: the bilinear sampler and the raster volume integrator. Mirrors
//! `volumes/raster.py`.
//!
//! The raster is a north-up grid of float32 heights in memory ([`Raster`]); reading it from a
//! GeoTIFF is ticket 79's. Each step the Python takes from rasterio, GDAL or SciPy is written out
//! so that it picks the same cells:
//! - windows: rasterio 1.5 `from_bounds` (LAPACK back-substitution of the affine), `round_offsets`
//!   (`floor(x + 0.1)`), `round_lengths` (`floor(x + 0.5)`), `intersection`, `window_transform`;
//! - masks: GDAL 3.12 `GDALdllImageFilledPolygon` (pixel centres) and `GDALdllImageLineAllTouched`,
//!   after the inverse geotransform GDAL's GenImgProj transformer applies;
//! - SciPy `distance_transform_edt` (exact) and `median_filter` (`mode="nearest"`).

use crate::geom::{Polygon, Pt, linear, offset};
use crate::numpy::np_sum;
use crate::volumes::pools::Weir;

/// A north-up affine transform: `x = c + a * col`, `y = f + e * row` (rasterio `Affine` with
/// `b = d = 0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub c: f64,
    pub e: f64,
    pub f: f64,
}

/// A single-band raster of float32 heights, row-major, with an optional nodata value.
#[derive(Debug, Clone, PartialEq)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub transform: Affine,
    /// The heights, each a float32 widened to f64.
    pub data: Vec<f64>,
    pub nodata: Option<f64>,
}

/// A rasterio `Window`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    col_off: f64,
    row_off: f64,
    width: f64,
    height: f64,
}

/// rasterio `rowcol(..., op=float)` for a north-up transform: the LAPACK solve reduces to these.
fn rowcol(t: &Affine, x: f64, y: f64) -> (f64, f64) {
    ((y - t.f) / t.e, (x - t.c) / t.a)
}

/// rasterio `from_bounds(left, bottom, right, top, transform)`.
fn from_bounds(left: f64, bottom: f64, right: f64, top: f64, t: &Affine) -> Window {
    let pts = [(left, top), (right, top), (right, bottom), (left, bottom)];
    let rc: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| rowcol(t, x, y)).collect();
    let min =
        |v: &mut dyn Iterator<Item = f64>| v.fold(f64::INFINITY, |m, x| if x < m { x } else { m });
    let max = |v: &mut dyn Iterator<Item = f64>| {
        v.fold(f64::NEG_INFINITY, |m, x| if x > m { x } else { m })
    };
    let (r0, r1) = (
        min(&mut rc.iter().map(|p| p.0)),
        max(&mut rc.iter().map(|p| p.0)),
    );
    let (c0, c1) = (
        min(&mut rc.iter().map(|p| p.1)),
        max(&mut rc.iter().map(|p| p.1)),
    );
    Window {
        col_off: c0,
        row_off: r0,
        width: crate::py_max(c1 - c0, 0.0),
        height: crate::py_max(r1 - r0, 0.0),
    }
}

impl Window {
    fn rounded(self) -> Window {
        Window {
            col_off: (self.col_off + 0.1).floor(),
            row_off: (self.row_off + 0.1).floor(),
            width: (self.width + 0.5).floor(),
            height: (self.height + 0.5).floor(),
        }
    }

    /// rasterio `intersection`: `None` where it would raise `WindowError`.
    fn intersection(self, w: usize, h: usize) -> Option<Window> {
        let c0 = crate::py_max(self.col_off, 0.0);
        let r0 = crate::py_max(self.row_off, 0.0);
        let c1 = crate::py_min(self.col_off + self.width, w as f64);
        let r1 = crate::py_min(self.row_off + self.height, h as f64);
        (c1 - c0 > 0.0 && r1 - r0 > 0.0).then_some(Window {
            col_off: c0,
            row_off: r0,
            width: c1 - c0,
            height: r1 - r0,
        })
    }
}

/// rasterio `window_transform`: `translation(x - c, y - f) * transform`.
fn window_transform(t: &Affine, w: &Window) -> Affine {
    let x = w.col_off * t.a + w.row_off * 0.0 + t.c;
    let y = w.col_off * 0.0 + w.row_off * t.e + t.f;
    Affine {
        a: t.a,
        e: t.e,
        c: (x - t.c) + t.c,
        f: (y - t.f) + t.f,
    }
}

impl Raster {
    /// `ds.read(1, window=win)` as f64.
    fn read(&self, w: &Window) -> Grid {
        let (c0, r0) = (w.col_off as usize, w.row_off as usize);
        let (cols, rows) = (w.width as usize, w.height as usize);
        let mut v = Vec::with_capacity(cols * rows);
        for r in r0..r0 + rows {
            v.extend_from_slice(&self.data[r * self.width + c0..r * self.width + c0 + cols]);
        }
        Grid {
            w: cols,
            h: rows,
            v,
        }
    }

    /// `ds.read(1, window=win, out_shape=(oh, ow), resampling=Resampling.average)`: GDAL 3.12
    /// `GDALResampleChunk_AverageOrRMS` on float32 with no nodata. Each output cell averages the
    /// source cells it covers, the partly covered edge cells weighted by their covered fraction;
    /// values are scaled by the cell count before summing, and the mean is stored as float32. An
    /// exact 2 x 2 reduction takes GDAL's float32 fast path. `None` with nodata or NaN in the
    /// window (GDAL's masked path, which no case needs).
    fn read_average(&self, w: &Window, oh: usize, ow: usize) -> Option<Grid> {
        let src = self.read(w);
        if src.v.iter().any(|z| z.is_nan() || self.nodata == Some(*z)) {
            return None;
        }
        // Per output column or row: first and last-but-one source index, edge weights, total weight.
        let spans = |n_src: usize, n_dst: usize| -> Vec<(usize, usize, f64, f64, f64)> {
            let ratio = n_src as f64 / n_dst as f64;
            (0..n_dst)
                .map(|i| {
                    let off = i as f64 * ratio;
                    let mut a = (off + 1e-8) as usize;
                    let off2 = (i + 1) as f64 * ratio;
                    let mut b = (off2 - 1e-8).ceil() as usize;
                    if b == a {
                        b += 1;
                    }
                    if b > n_src {
                        b = n_src;
                    }
                    a = a.min(n_src);
                    let left = if b == a + 1 {
                        1.0
                    } else {
                        1.0 - (off - a as f64)
                    };
                    let right = 1.0 - (b as f64 - off2);
                    let mut total = left;
                    if a + 1 < b {
                        total += (b - a - 2) as f64;
                        total += right;
                    }
                    (a, b, left, right, total)
                })
                .collect()
        };
        let (xs, ys) = (spans(src.w, ow), spans(src.h, oh));
        let two = xs
            .iter()
            .zip(xs.iter().skip(1))
            .all(|(p, q)| p.1 - p.0 == 2 && q.0 == p.1)
            && xs.iter().all(|p| p.1 - p.0 == 2);
        let mut v = Vec::with_capacity(oh * ow);
        for &(ya, yb, bottom, top, ytotal) in &ys {
            for &(xa, xb, left, right, xtotal) in &xs {
                let at = |r: usize, c: usize| src.v[r * src.w + c];
                if two && yb == ya + 2 {
                    let q = 0.25f32;
                    let f = |r: usize, c: usize| at(r, c) as f32;
                    v.push(f64::from(
                        q * f(ya, xa)
                            + q * f(ya, xa + 1)
                            + q * f(ya + 1, xa)
                            + q * f(ya + 1, xa + 1),
                    ));
                    continue;
                }
                let mul = ((yb - ya) * (xb - xa)) as f64;
                let inv = 1.0 / mul;
                let mut total = 0.0;
                for (k, r) in (ya..yb).enumerate() {
                    let mut line = at(r, xa) * inv * left;
                    if xa + 1 < xb {
                        for c in xa + 1..xb - 1 {
                            line += at(r, c) * inv;
                        }
                        line += at(r, xb - 1) * inv * right;
                    }
                    let wy = if k == 0 {
                        bottom
                    } else if r + 1 == yb {
                        top
                    } else {
                        1.0
                    };
                    total += line * wy;
                }
                v.push(f64::from((mul * (total / (xtotal * ytotal))) as f32));
            }
        }
        Some(Grid { w: ow, h: oh, v })
    }
}

/// A row-major grid of values.
#[derive(Debug, Clone)]
struct Grid {
    w: usize,
    h: usize,
    v: Vec<f64>,
}

/// GDAL's inverse geotransform (`GDALInvGeoTransform`, no rotation) applied to a point.
fn to_pixel(t: &Affine, p: Pt) -> Pt {
    let (i0, i1) = (-t.c / t.a, 1.0 / t.a);
    let (i3, i5) = (-t.f / t.e, 1.0 / t.e);
    [i0 + p[0] * i1 + p[1] * 0.0, i3 + p[0] * 0.0 + p[1] * i5]
}

/// GDAL `GDALdllImageFilledPolygon`: the cells whose centre is inside the rings (even-odd).
fn polygon_mask(polys: &[Polygon], w: usize, h: usize, t: &Affine) -> Vec<bool> {
    let parts: Vec<Vec<Pt>> = polys
        .iter()
        .flat_map(|p| core::iter::once(&p.exterior).chain(&p.holes))
        .map(|r| r.iter().map(|&q| to_pixel(t, q)).collect())
        .collect();
    let mut mask = vec![false; w * h];
    let all: Vec<Pt> = parts.iter().flatten().copied().collect();
    if all.is_empty() {
        return mask;
    }
    let (mut dminy, mut dmaxy) = (all[0][1], all[0][1]);
    for q in &all[1..] {
        if q[1] < dminy {
            dminy = q[1];
        } else if q[1] > dmaxy {
            dmaxy = q[1];
        }
    }
    let miny = crate::py_max(0.0, dminy) as i64;
    let maxy = crate::py_min(dmaxy, h as f64 - 1.0) as i64;
    let maxx = w as i64 - 1;
    let mut burn = |y: i64, x0: i64, x1: i64| {
        let (a, b) = (x0.max(0), x1.min(w as i64 - 1));
        for x in a..=b {
            mask[y as usize * w + x as usize] = true;
        }
    };
    for y in miny..=maxy {
        let dy = y as f64 + 0.5;
        let mut ints: Vec<i64> = Vec::new();
        for part in &parts {
            let n = part.len();
            for i in 0..n {
                let (i1, i2) = if i == 0 { (n - 1, 0) } else { (i - 1, i) };
                let (mut dy1, mut dy2) = (part[i1][1], part[i2][1]);
                if (dy1 < dy && dy2 < dy) || (dy1 > dy && dy2 > dy) {
                    continue;
                }
                let (dx1, dx2);
                if dy1 < dy2 {
                    dx1 = part[i1][0];
                    dx2 = part[i2][0];
                } else if dy1 > dy2 {
                    core::mem::swap(&mut dy1, &mut dy2);
                    dx2 = part[i1][0];
                    dx1 = part[i2][0];
                } else {
                    // A horizontal segment: the bottom ones are filled on their own.
                    if part[i1][0] > part[i2][0] {
                        let hx1 = (part[i2][0] + 0.5).floor();
                        let hx2 = (part[i1][0] + 0.5).floor();
                        if hx1 > maxx as f64 || hx2 <= 0.0 {
                            continue;
                        }
                        let x1 = crate::py_max(hx1, 0.0) as i64;
                        let x2 = crate::py_min(hx2, w as f64) as i64;
                        burn(y, x1, x2 - 1);
                    }
                    continue;
                }
                if dy < dy2 && dy >= dy1 {
                    let x = (dy - dy1) * (dx2 - dx1) / (dy2 - dy1) + dx1;
                    ints.push((x.clamp(i32::MIN as f64, i32::MAX as f64) + 0.5).floor() as i64);
                }
            }
        }
        ints.sort_unstable();
        for pair in ints.chunks(2) {
            if let [a, b] = *pair
                && a <= maxx
                && b > 0
            {
                burn(y, a, b - 1);
            }
        }
    }
    mask
}

/// GDAL `GDALdllImageLineAllTouched`: every cell a line passes through.
fn line_mask_all_touched(line: &[Pt], w: usize, h: usize, t: &Affine) -> Vec<bool> {
    const EPS: f64 = 1e-4;
    let (nx, ny) = (w as f64, h as f64);
    let pts: Vec<Pt> = line.iter().map(|&q| to_pixel(t, q)).collect();
    let mut mask = vec![false; w * h];
    let mut put = |iy: i64, ix: i64| {
        if iy >= 0 && (iy as usize) < h && ix >= 0 && (ix as usize) < w {
            mask[iy as usize * w + ix as usize] = true;
        }
    };
    for s in pts.windows(2) {
        let (mut x, mut y, mut xe, mut ye) = (s[0][0], s[0][1], s[1][0], s[1][1]);
        if (y < 0.0 && ye < 0.0)
            || (y > ny && ye > ny)
            || (x < 0.0 && xe < 0.0)
            || (x > nx && xe > nx)
        {
            continue;
        }
        if x > xe {
            core::mem::swap(&mut x, &mut xe);
            core::mem::swap(&mut y, &mut ye);
        }
        if (x - xe).abs() < 0.01 {
            if ye < y {
                core::mem::swap(&mut y, &mut ye);
            }
            let ix = xe.floor() as i64;
            let iy = (y.floor() as i64).max(0);
            let iye = ((ye - EPS).floor() as i64).min(h as i64 - 1);
            if ix < 0 || ix >= w as i64 {
                continue;
            }
            for k in iy..=iye {
                put(k, ix);
            }
            continue;
        }
        if (y - ye).abs() < 0.01 {
            let ix = (x.floor() as i64).max(0);
            let iy = y.floor() as i64;
            let ixe = ((xe - EPS).floor() as i64).min(w as i64 - 1);
            if iy < 0 || iy >= h as i64 {
                continue;
            }
            for k in ix..=ixe {
                put(iy, k);
            }
            continue;
        }
        let slope = (ye - y) / (xe - x);
        if xe > nx {
            ye -= (xe - nx) * slope;
            xe = nx;
        }
        if x < 0.0 {
            y += (0.0 - x) * slope;
            x = 0.0;
        }
        if ye > y {
            if y < 0.0 {
                x += (0.0 - y) / slope;
                y = 0.0;
            }
            if ye >= ny {
                xe += (ye - ny) / slope;
                if xe > nx {
                    xe = nx;
                }
            }
        } else {
            if y >= ny {
                x += (ny - y) / slope;
                y = ny;
            }
            if ye < 0.0 {
                xe -= (ye - 0.0) / slope;
            }
        }
        while x >= 0.0 && x < xe {
            let ix = x.floor() as i64;
            let iy = y.floor() as i64;
            put(iy, ix);
            let mut step_x = (x + 1.0).floor() - x;
            let mut step_y = step_x * slope;
            if (y + step_y).floor() as i64 == iy {
            } else if slope < 0.0 {
                step_y = iy as f64 - y;
                if step_y > -0.000000001 {
                    step_y = -0.000000001;
                }
                step_x = step_y / slope;
            } else {
                step_y = (iy + 1) as f64 - y;
                if step_y < 0.000000001 {
                    step_y = 0.000000001;
                }
                step_x = step_y / slope;
            }
            x += step_x;
            y += step_y;
        }
    }
    mask
}

/// SciPy `distance_transform_edt(~features, sampling=px)`: the distance from each cell to the
/// nearest feature cell, `sqrt((dr * px)² + (dc * px)²)`, zero on the features.
fn distance_to(features: &[bool], w: usize, h: usize, px: f64) -> Vec<f64> {
    let feats: Vec<(i64, i64)> = (0..w * h)
        .filter(|&i| features[i])
        .map(|i| ((i / w) as i64, (i % w) as i64))
        .collect();
    (0..w * h)
        .map(|i| {
            if features[i] {
                return 0.0;
            }
            let (r, c) = ((i / w) as i64, (i % w) as i64);
            let mut best = (i64::MAX, 0i64, 0i64);
            for &(fr, fc) in &feats {
                let (dr, dc) = (fr - r, fc - c);
                let d2 = dr * dr + dc * dc;
                if d2 < best.0 {
                    best = (d2, dr, dc);
                }
            }
            let (dr, dc) = (best.1 as f64 * px, best.2 as f64 * px);
            (dr * dr + dc * dc).sqrt()
        })
        .collect()
}

/// SciPy `median_filter(z, size=size, mode="nearest")`: the median of each `size` × `size` block
/// (the upper median for an even count), edges extended.
fn median_filter(z: &[f64], w: usize, h: usize, size: usize) -> Vec<f64> {
    let lo = (size / 2) as i64;
    let rank = size * size / 2;
    let mut out = Vec::with_capacity(w * h);
    let mut buf = Vec::with_capacity(size * size);
    for r in 0..h as i64 {
        for c in 0..w as i64 {
            buf.clear();
            for dr in 0..size as i64 {
                for dc in 0..size as i64 {
                    let rr = (r + dr - lo).clamp(0, h as i64 - 1) as usize;
                    let cc = (c + dc - lo).clamp(0, w as i64 - 1) as usize;
                    buf.push(z[rr * w + cc]);
                }
            }
            buf.sort_by(f64::total_cmp);
            out.push(buf[rank]);
        }
    }
    out
}

/// `_despike`: a spatial Hampel filter. Cells further than `max(floor, k * 1.4826 * MAD)` from the
/// local median are replaced by it. Returns the cleaned heights and the spike mask.
// The arguments are `_despike`'s, one for one, plus the grid size.
#[allow(clippy::too_many_arguments)]
pub fn despike(
    z: &[f64],
    valid: &[bool],
    w: usize,
    h: usize,
    fill: f64,
    window: usize,
    k: f64,
    floor: f64,
) -> (Vec<f64>, Vec<bool>) {
    let zf: Vec<f64> = z
        .iter()
        .zip(valid)
        .map(|(&v, &ok)| if ok { v } else { fill })
        .collect();
    let med = median_filter(&zf, w, h, window);
    let dev: Vec<f64> = zf.iter().zip(&med).map(|(a, b)| (a - b).abs()).collect();
    let mad = median_filter(&dev, w, h, window);
    let mut out = z.to_vec();
    let mut spike = vec![false; z.len()];
    for i in 0..z.len() {
        let tol = crate::py_max(floor, k * 1.4826 * mad[i]);
        if valid[i] && dev[i] > tol {
            spike[i] = true;
            out[i] = med[i];
        }
    }
    (out, spike)
}

/// The number of 1 m cut-depth bands (`DEPTH_NBINS`).
pub const DEPTH_NBINS: usize = 40;

/// `_layer_volumes`: per band i, the sum over cells of `clip(cut - i, 0, 1)`, row by row.
pub fn layer_volumes(vals: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; DEPTH_NBINS];
    for (k, &v) in vals.iter().enumerate() {
        for (i, o) in out.iter_mut().enumerate() {
            let x = (v - i as f64).clamp(0.0, 1.0);
            if k == 0 { *o = x } else { *o += x }
        }
    }
    out
}

/// `bilinear(ds, xs, ys)`: pixel-centre bilinear heights, NaN outside the outermost centres and on
/// nodata, from one window read around the points.
pub fn bilinear(r: &Raster, xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let mut out = vec![f64::NAN; xs.len()];
    if xs.is_empty() {
        return out;
    }
    let mn = |v: &[f64]| {
        v.iter()
            .copied()
            .fold(f64::INFINITY, |m, x| if x < m { x } else { m })
    };
    let mx = |v: &[f64]| {
        v.iter()
            .copied()
            .fold(f64::NEG_INFINITY, |m, x| if x > m { x } else { m })
    };
    let (xmin, xmax, ymin, ymax) = (mn(xs) - 8.0, mx(xs) + 8.0, mn(ys) - 8.0, mx(ys) + 8.0);
    let Some(win) = from_bounds(xmin, ymin, xmax, ymax, &r.transform)
        .rounded()
        .intersection(r.width, r.height)
    else {
        return out;
    };
    let mut z = r.read(&win);
    if let Some(nd) = r.nodata {
        z.v.iter_mut()
            .filter(|v| **v == nd)
            .for_each(|v| *v = f64::NAN);
    }
    let tr = window_transform(&r.transform, &win);
    for (i, (&x, &y)) in xs.iter().zip(ys).enumerate() {
        let col = (x - tr.c) / tr.a - 0.5;
        let row = (y - tr.f) / tr.e - 0.5;
        let (c0, r0) = (col.floor(), row.floor());
        let (fc, fr) = (col - c0, row - r0);
        let (c0, r0) = (c0 as i64, r0 as i64);
        if c0 >= 0 && r0 >= 0 && c0 < z.w as i64 - 1 && r0 < z.h as i64 - 1 {
            let at = |rr: i64, cc: i64| z.v[rr as usize * z.w + cc as usize];
            out[i] = at(r0, c0) * (1.0 - fc) * (1.0 - fr)
                + at(r0, c0 + 1) * fc * (1.0 - fr)
                + at(r0 + 1, c0) * (1.0 - fc) * fr
                + at(r0 + 1, c0 + 1) * fc * fr;
        }
    }
    out
}

/// The inputs of the two volume integrators beyond the weirs, lines and raster, with the Python
/// defaults of `compute_pool_volumes` (`chunk_m` differs: 2000 m per pool, 100 m per chunk).
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeParams {
    pub navigable_depth_m: f64,
    pub channel_bottom_width_m: f64,
    pub side_slope_h_per_v: f64,
    pub overdepth_m: f64,
    pub despike: bool,
    pub despike_window_px: usize,
    pub despike_k: f64,
    pub despike_floor_m: f64,
    pub deep_cut_threshold_m: f64,
    pub chunk_m: f64,
    pub max_corridor_half_width_m: f64,
    /// `"pianc"` with both limits: the half bottom width from the navigation limits.
    pub pianc_limits: Option<(Vec<Pt>, Vec<Pt>)>,
    pub downstream_water_level_m: Option<f64>,
    pub read_decimate: usize,
}

impl VolumeParams {
    pub fn new(navigable_depth_m: f64, chunk_m: f64) -> Self {
        VolumeParams {
            navigable_depth_m,
            channel_bottom_width_m: 67.2,
            side_slope_h_per_v: 3.0,
            overdepth_m: 0.0,
            despike: true,
            despike_window_px: 5,
            despike_k: 4.0,
            despike_floor_m: 3.0,
            deep_cut_threshold_m: 20.0,
            chunk_m,
            max_corridor_half_width_m: 300.0,
            pianc_limits: None,
            downstream_water_level_m: None,
            read_decimate: 1,
        }
    }
}

/// `_pool_for_chainage`.
fn pool_for_chainage(weirs: &[Weir], s: f64) -> Option<&Weir> {
    let mut chosen = None;
    for w in weirs {
        if w.pool_start_m <= s && s < w.pool_end_m {
            return Some(w);
        }
        if w.pool_start_m <= s {
            chosen = Some(w);
        }
    }
    chosen
}

/// What one chunk of the corridor gives, cell by cell.
struct ChunkCells {
    cut: Vec<f64>,
    valid: Vec<bool>,
    in_river: Vec<bool>,
    in_corridor: Vec<bool>,
    spikes: Vec<bool>,
    offset: Vec<f64>,
    z_used: Vec<f64>,
    cell_area: f64,
}

/// The cells of one chunk `[c0, c1]` at grade `g`: `None` when the window is empty or the line
/// burns no cell (the Python `continue`s). `slope_cut = false` leaves the template uncut, for the
/// fairway modes, which cut on their own.
#[allow(clippy::too_many_arguments)]
fn chunk_cells(
    r: &Raster,
    centreline: &[Pt],
    river: &[Polygon],
    p: &VolumeParams,
    c0: f64,
    c1: f64,
    g: f64,
    hb: f64,
) -> Option<ChunkCells> {
    let seg = match linear::substring(centreline, c0, c1) {
        crate::geom::Geometry::LineString(l) => l,
        _ => return None,
    };
    let corridor =
        match offset::buffer_line(&seg, p.max_corridor_half_width_m, 16, offset::Cap::Flat).ok()? {
            crate::geom::Geometry::Polygon(q) => vec![q],
            crate::geom::Geometry::MultiPolygon(qs) => qs,
            _ => vec![],
        };
    let pts = corridor.iter().flat_map(|q| q.exterior.iter());
    let (mut l, mut b, mut rt, mut t) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for q in pts {
        l = l.min(q[0]);
        rt = rt.max(q[0]);
        b = b.min(q[1]);
        t = t.max(q[1]);
    }
    let win = from_bounds(l, b, rt, t, &r.transform)
        .rounded()
        .intersection(r.width, r.height)?;
    let dec = p.read_decimate.max(1);
    let px = r.transform.a.abs() * dec as f64;
    let (z, wt) = if dec > 1 {
        let oh = ((win.height / dec as f64).floor() as usize).max(1);
        let ow = ((win.width / dec as f64).floor() as usize).max(1);
        let z = r.read_average(&win, oh, ow)?;
        let t0 = window_transform(&r.transform, &win);
        let (sx, sy) = (win.width / ow as f64, win.height / oh as f64);
        (
            z,
            Affine {
                a: t0.a * sx,
                e: t0.e * sy,
                c: t0.c,
                f: t0.f,
            },
        )
    } else {
        (r.read(&win), window_transform(&r.transform, &win))
    };
    let (w, h) = (z.w, z.h);
    let in_corridor = polygon_mask(&corridor, w, h, &wt);
    let line = line_mask_all_touched(&seg, w, h, &wt);
    if !line.iter().any(|&x| x) {
        return None;
    }
    let offset = distance_to(&line, w, h, px);
    let valid: Vec<bool> = (0..w * h)
        .map(|i| in_corridor[i] && z.v[i].is_finite() && r.nodata.is_none_or(|nd| z.v[i] != nd))
        .collect();
    let (z_used, spikes) = if p.despike {
        despike(
            &z.v,
            &valid,
            w,
            h,
            g,
            p.despike_window_px,
            p.despike_k,
            p.despike_floor_m,
        )
    } else {
        (z.v.clone(), vec![false; w * h])
    };
    let in_river = polygon_mask(river, w, h, &wt);
    let cut = (0..w * h)
        .map(|i| {
            let template = g + crate::py_max(0.0, offset[i] - hb) / p.side_slope_h_per_v;
            let c = if valid[i] { z_used[i] - template } else { 0.0 };
            c.clamp(0.0, f64::INFINITY)
        })
        .collect();
    Some(ChunkCells {
        cut,
        valid,
        in_river,
        in_corridor,
        spikes,
        offset,
        z_used,
        cell_area: px * px,
    })
}

/// The local half bottom width at chainage `mid`.
fn half_bottom(centreline: &[Pt], p: &VolumeParams, mid: f64) -> f64 {
    let hb = p.channel_bottom_width_m / 2.0;
    match &p.pianc_limits {
        Some((left, right)) => {
            let pm = linear::interpolate(centreline, mid).unwrap_or([f64::NAN; 2]);
            let half_top = 0.5
                * (linear::point_line_distance(pm, left) + linear::point_line_distance(pm, right));
            crate::py_max(half_top - p.navigable_depth_m * p.side_slope_h_per_v, 0.0)
        }
        None => hb,
    }
}

fn masked_sum(v: &[f64], m: impl Fn(usize) -> bool) -> f64 {
    let picked: Vec<f64> = (0..v.len()).filter(|&i| m(i)).map(|i| v[i]).collect();
    if picked.is_empty() {
        0.0
    } else {
        np_sum(&picked)
    }
}

/// One pool's volumes (`PoolVolume`).
#[derive(Debug, Clone, PartialEq)]
pub struct PoolVolume {
    pub weir_index: usize,
    pub pool_water_level_m: f64,
    pub design_grade_m: f64,
    pub dredge_volume_m3: f64,
    pub excavation_volume_m3: f64,
    pub dredge_area_m2: f64,
    pub excavation_area_m2: f64,
    pub max_cut_m: f64,
    pub mean_cut_m: f64,
    pub despiked_cells: i64,
    pub deep_excavation_volume_m3: f64,
    pub deep_excavation_area_m2: f64,
    pub dredge_hist: Vec<f64>,
    pub excav_hist: Vec<f64>,
}

#[derive(Default)]
struct Accum {
    dredge: f64,
    excav: f64,
    deep: f64,
    darea: f64,
    earea: f64,
    deparea: f64,
    maxcut: f64,
    sumpos: f64,
    npos: i64,
    spikes: i64,
    dhist: Vec<f64>,
    ehist: Vec<f64>,
}

/// `compute_pool_volumes`: dredge and excavation per pool, in chunks of `chunk_m`.
pub fn compute_pool_volumes(
    weirs: &[Weir],
    centreline: &[Pt],
    river: &[Polygon],
    r: &Raster,
    p: &VolumeParams,
) -> Vec<PoolVolume> {
    if weirs.is_empty() {
        return vec![];
    }
    let reach0 = p.downstream_water_level_m;
    let start = if reach0.is_some() {
        0.0
    } else {
        weirs[0].pool_start_m
    };
    let end = weirs[weirs.len() - 1].pool_end_m;
    let mut edges = vec![start, end];
    let n = ((end - start) / p.chunk_m) as i64;
    edges.extend((1..=n).map(|i| start + i as f64 * p.chunk_m));
    edges.extend(weirs.iter().map(|w| w.pool_start_m));
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    edges.retain(|&e| start <= e && e <= end);
    let mut acc: Vec<(usize, Accum, f64)> = Vec::new();
    let grade = |wl: f64| wl - p.navigable_depth_m - p.overdepth_m;
    if let Some(d) = reach0 {
        acc.push((0, Accum::default(), grade(d)));
    }
    for w in weirs {
        acc.push((w.index, Accum::default(), grade(w.pool_water_level_m)));
    }
    let weir1 = weirs[0].pool_start_m;
    for e in edges.windows(2) {
        let (c0, c1) = (e[0], e[1]);
        if c1 - c0 < 1e-6 {
            continue;
        }
        let mid = (c0 + c1) / 2.0;
        let idx = if reach0.is_some() && mid < weir1 {
            0
        } else {
            match pool_for_chainage(weirs, mid) {
                Some(w) => w.index,
                None => continue,
            }
        };
        let Some(slot) = acc.iter().position(|a| a.0 == idx) else {
            continue;
        };
        let g = acc[slot].2;
        let hb = half_bottom(centreline, p, mid);
        let Some(cc) = chunk_cells(r, centreline, river, p, c0, c1, g, hb) else {
            continue;
        };
        let ca = cc.cell_area;
        let dredge = |i: usize| cc.valid[i] && cc.in_river[i] && cc.cut[i] > 0.0;
        let excav = |i: usize| cc.valid[i] && !cc.in_river[i] && cc.cut[i] > 0.0;
        let deep = |i: usize| excav(i) && cc.cut[i] > p.deep_cut_threshold_m;
        let count = |f: &dyn Fn(usize) -> bool| (0..cc.cut.len()).filter(|&i| f(i)).count() as f64;
        let a = &mut acc[slot].1;
        a.dredge += masked_sum(&cc.cut, dredge) * ca;
        a.excav += masked_sum(&cc.cut, excav) * ca;
        a.deep += masked_sum(&cc.cut, deep) * ca;
        a.darea += count(&dredge) * ca;
        a.earea += count(&excav) * ca;
        a.deparea += count(&deep) * ca;
        let cmax = cc
            .cut
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, |m, x| if x > m { x } else { m });
        a.maxcut = crate::py_max(a.maxcut, if cc.cut.is_empty() { 0.0 } else { cmax });
        a.sumpos += masked_sum(&cc.cut, |i| cc.cut[i] > 0.0);
        a.npos += cc.cut.iter().filter(|&&x| x > 0.0).count() as i64;
        if p.despike {
            a.spikes += (0..cc.cut.len())
                .filter(|&i| cc.spikes[i] && cc.in_corridor[i])
                .count() as i64;
        }
        for (hist, f) in [
            (&mut a.dhist, &dredge as &dyn Fn(usize) -> bool),
            (&mut a.ehist, &excav),
        ] {
            let vals: Vec<f64> = (0..cc.cut.len())
                .filter(|&i| f(i))
                .map(|i| cc.cut[i])
                .collect();
            if !vals.is_empty() {
                let lv = layer_volumes(&vals);
                if hist.is_empty() {
                    *hist = vec![0.0; DEPTH_NBINS];
                }
                for (h, v) in hist.iter_mut().zip(lv) {
                    *h += v * ca;
                }
            }
        }
    }
    acc.into_iter()
        .map(|(index, a, g)| PoolVolume {
            weir_index: index,
            pool_water_level_m: if index == 0 {
                reach0.unwrap_or(f64::NAN)
            } else {
                weirs
                    .iter()
                    .find(|w| w.index == index)
                    .map_or(f64::NAN, |w| w.pool_water_level_m)
            },
            design_grade_m: g,
            dredge_volume_m3: a.dredge,
            excavation_volume_m3: a.excav,
            dredge_area_m2: a.darea,
            excavation_area_m2: a.earea,
            max_cut_m: a.maxcut,
            mean_cut_m: if a.npos != 0 {
                a.sumpos / a.npos as f64
            } else {
                0.0
            },
            despiked_cells: a.spikes,
            deep_excavation_volume_m3: a.deep,
            deep_excavation_area_m2: a.deparea,
            dredge_hist: if a.dhist.is_empty() {
                vec![0.0; DEPTH_NBINS]
            } else {
                a.dhist
            },
            excav_hist: if a.ehist.is_empty() {
                vec![0.0; DEPTH_NBINS]
            } else {
                a.ehist
            },
        })
        .collect()
}

/// How `compute_chainage_volumes` carves the side slopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlopeMode {
    Always,
    DryBankOnly,
    FairwayOnly,
}

/// One chunk's record (`compute_chainage_volumes`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChainageRecord {
    pub s0_m: f64,
    pub s1_m: f64,
    pub grade_m: f64,
    pub wl_m: f64,
    pub pool: usize,
    pub len_m: f64,
    pub is_cut_through: bool,
    pub design_width_m: f64,
    pub dredge_m3: f64,
    pub excav_m3: f64,
    pub widening_m3: f64,
    pub cut_through_m3: f64,
    pub deep_m3: f64,
    pub river_width_m: f64,
}

/// `compute_chainage_volumes`: the dredge and excavation breakdown chunk by chunk.
pub fn compute_chainage_volumes(
    weirs: &[Weir],
    centreline: &[Pt],
    river: &[Polygon],
    r: &Raster,
    p: &VolumeParams,
    slope_mode: SlopeMode,
) -> Vec<ChainageRecord> {
    if weirs.is_empty() {
        return vec![];
    }
    let reach0 = p.downstream_water_level_m;
    let start = if reach0.is_some() {
        0.0
    } else {
        weirs[0].pool_start_m
    };
    let end = weirs[weirs.len() - 1].pool_end_m;
    let mut edges = vec![start, end];
    let mut s = start + p.chunk_m;
    while s < end {
        edges.push(s);
        s += p.chunk_m;
    }
    edges.extend(weirs.iter().map(|w| w.pool_start_m));
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    edges.retain(|&e| start <= e && e <= end);
    let weir1 = weirs[0].pool_start_m;
    let grade = |wl: f64| wl - p.navigable_depth_m - p.overdepth_m;
    let mut out = Vec::new();
    for e in edges.windows(2) {
        let (c0, c1) = (e[0], e[1]);
        if c1 - c0 < 1e-6 {
            continue;
        }
        let mid = (c0 + c1) / 2.0;
        let (idx, wl) = if let Some(d) = reach0.filter(|_| mid < weir1) {
            (0, d)
        } else {
            match pool_for_chainage(weirs, mid) {
                Some(w) => (w.index, w.pool_water_level_m),
                None => continue,
            }
        };
        let g = grade(wl);
        let hb = half_bottom(centreline, p, mid);
        let pm = linear::interpolate(centreline, mid).unwrap_or([f64::NAN; 2]);
        let is_cut = !crate::geom::overlay::contains_xy(river, pm);
        let mut rec = ChainageRecord {
            s0_m: c0,
            s1_m: c1,
            grade_m: g,
            wl_m: wl,
            pool: idx,
            len_m: c1 - c0,
            is_cut_through: is_cut,
            design_width_m: 2.0 * hb + 2.0 * p.side_slope_h_per_v * p.navigable_depth_m,
            dredge_m3: 0.0,
            excav_m3: 0.0,
            widening_m3: 0.0,
            cut_through_m3: 0.0,
            deep_m3: 0.0,
            river_width_m: 0.0,
        };
        let Some(cc) = chunk_cells(r, centreline, river, p, c0, c1, g, hb) else {
            out.push(rec);
            continue;
        };
        let n = cc.cut.len();
        let cut: Vec<f64> = match slope_mode {
            SlopeMode::Always => cc.cut.clone(),
            _ => {
                let fairway: Vec<bool> =
                    (0..n).map(|i| cc.valid[i] && cc.offset[i] <= hb).collect();
                let fcut: Vec<f64> = (0..n)
                    .map(|i| if fairway[i] { cc.z_used[i] - g } else { 0.0 })
                    .collect();
                let hits_dry = slope_mode == SlopeMode::DryBankOnly
                    && (0..n).any(|i| fcut[i] > 0.0 && !cc.in_river[i]);
                (0..n)
                    .map(|i| {
                        let mut c = if fairway[i] {
                            fcut[i].clamp(0.0, f64::INFINITY)
                        } else {
                            0.0
                        };
                        if hits_dry {
                            let slope_cell = cc.valid[i] && cc.offset[i] > hb && !cc.in_river[i];
                            let template =
                                g + crate::py_max(0.0, cc.offset[i] - hb) / p.side_slope_h_per_v;
                            let sc = if slope_cell {
                                cc.z_used[i] - template
                            } else {
                                0.0
                            };
                            c += sc.clamp(0.0, f64::INFINITY);
                        }
                        c
                    })
                    .collect()
            }
        };
        let ca = cc.cell_area;
        let dredge = |i: usize| cc.valid[i] && cc.in_river[i] && cut[i] > 0.0;
        let excav = |i: usize| cc.valid[i] && !cc.in_river[i] && cut[i] > 0.0;
        let ev = masked_sum(&cut, excav) * ca;
        rec.dredge_m3 = masked_sum(&cut, dredge) * ca;
        rec.excav_m3 = ev;
        rec.widening_m3 = if is_cut { 0.0 } else { ev };
        rec.cut_through_m3 = if is_cut { ev } else { 0.0 };
        rec.deep_m3 = masked_sum(&cut, |i| excav(i) && cut[i] > p.deep_cut_threshold_m) * ca;
        let wet = (0..n)
            .filter(|&i| cc.in_river[i] && cc.in_corridor[i])
            .count() as f64;
        rec.river_width_m = wet * ca / (c1 - c0);
        out.push(rec);
    }
    out
}

#[cfg(test)]
mod tests {
    //! Label: sourced - hand calculation (cells of a 4 x 4 raster)
    use super::*;

    fn unit() -> Affine {
        Affine {
            a: 1.0,
            c: 0.0,
            e: -1.0,
            f: 4.0,
        }
    }

    #[test]
    fn a_square_masks_the_cells_whose_centres_it_holds() {
        let sq = Polygon {
            exterior: vec![[0.6, 0.6], [2.6, 0.6], [2.6, 2.6], [0.6, 2.6], [0.6, 0.6]],
            holes: vec![],
        };
        let m = polygon_mask(&[sq], 4, 4, &unit());
        let on: Vec<usize> = (0..16).filter(|&i| m[i]).collect();
        // Cell centres are at 0.5, 1.5, 2.5, 3.5. The square holds x and y centres 1.5 and 2.5:
        // rows 1 and 2 (y = 2.5, 1.5) and columns 1 and 2.
        assert_eq!(on, vec![5, 6, 9, 10]);
    }

    #[test]
    fn a_diagonal_line_touches_its_cells_and_the_distance_grows_from_it() {
        let m = line_mask_all_touched(&[[0.5, 3.5], [3.5, 0.5]], 4, 4, &unit());
        assert!(m[0] && m[5] && m[10] && m[15]);
        let mut one = vec![false; 16];
        one[0] = true;
        let d = distance_to(&one, 4, 4, 2.0);
        assert_eq!((d[0], d[1], d[5], d[3]), (0.0, 2.0, 8.0f64.sqrt(), 6.0));
    }

    #[test]
    fn layer_volumes_sum_to_the_cut() {
        let lv = layer_volumes(&[2.5, 0.25]);
        assert_eq!((lv[0], lv[1], lv[2], lv[3]), (1.25, 1.0, 0.5, 0.0));
    }
}
