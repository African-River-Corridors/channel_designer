//! JRC Global Surface Water occurrence (source `jrc-gsw`): which tiles a reach needs, and a window of
//! occurrence read from them. Occurrence is the percentage of valid observations 1984-2021 that saw
//! water, in 10 degree tiles of 40000 by 40000 pixels (0.00025 degrees, about 30 m). The tiles are
//! tiled GeoTIFFs, so a window is a few range reads (`tiff`). 255 is "no data" and reads as land.
//!
//! The host sends no CORS header: on the web the user downloads the tiles from the links
//! [`tiles_for`] gives and drops them in (a `Fetch` over the dropped bytes); native fetches them.

use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::geo::LonLat;
use crate::sources::source;
use crate::tiff::TiledTiff;

/// One 10 degree tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tile {
    /// West edge in degrees, a multiple of 10.
    pub west: i32,
    /// North edge in degrees, a multiple of 10.
    pub north: i32,
}

impl Tile {
    /// The tile that holds a point.
    pub fn of(p: LonLat) -> Tile {
        Tile {
            west: (p[0] / 10.0).floor() as i32 * 10,
            north: (p[1] / 10.0).ceil() as i32 * 10,
        }
    }

    /// The name as JRC writes it in the file name: `10W_10N`, `0E_0N`, `40E_10S`.
    pub fn name(self) -> String {
        let lon = if self.west < 0 {
            format!("{}W", -self.west)
        } else {
            format!("{}E", self.west)
        };
        let lat = if self.north >= 0 {
            format!("{}N", self.north)
        } else {
            format!("{}S", -self.north)
        };
        format!("{lon}_{lat}")
    }

    /// The file name, `occurrence_10W_10Nv1_4_2021.tif`.
    pub fn file_name(self) -> String {
        format!("occurrence_{}v1_4_2021.tif", self.name())
    }

    /// The download URL, from the registry's pattern.
    pub fn url(self) -> String {
        let s = source("jrc-gsw").expect("jrc-gsw is in the registry");
        let name = self.name();
        let (lon, lat) = name.split_once('_').expect("a tile name has an underscore");
        s.url_pattern.replace("{lon}", lon).replace("{lat}", lat)
    }
}

/// The tiles that cover a bounding box `[west, south, east, north]`, west to east, north to south.
pub fn tiles_for(bbox: [f64; 4]) -> Vec<Tile> {
    // A box whose edge is exactly on a tile edge belongs to the tile inside it.
    let eps = 1e-9;
    let (w, e) = (
        (bbox[0] / 10.0).floor() as i32,
        ((bbox[2] - eps) / 10.0).floor() as i32,
    );
    let (n, s) = (
        (bbox[3] / 10.0).ceil() as i32,
        ((bbox[1] + eps) / 10.0).ceil() as i32,
    );
    let mut out = Vec::new();
    for ny in (s..=n).rev() {
        for wx in w..=e.max(w) {
            out.push(Tile {
                west: wx * 10,
                north: ny * 10,
            });
        }
    }
    out
}

/// A window of occurrence on the grid of its tiles.
#[derive(Debug, Clone)]
pub struct Occurrence {
    /// Longitude of the west edge and latitude of the north edge of the window.
    pub west: f64,
    pub north: f64,
    /// Pixel size in degrees.
    pub step: [f64; 2],
    pub cols: usize,
    pub rows: usize,
    /// Row-major, 0 to 100 percent, 255 for no data.
    pub data: Vec<u8>,
}

impl Occurrence {
    /// Occurrence at a pixel; 0 outside the window and for no data.
    pub fn pixel(&self, col: i64, row: i64) -> f64 {
        if col < 0 || row < 0 || col >= self.cols as i64 || row >= self.rows as i64 {
            return 0.0;
        }
        match self.data[row as usize * self.cols + col as usize] {
            255 => 0.0,
            v => f64::from(v),
        }
    }

    /// Occurrence at a point, bilinear between pixel centres.
    pub fn sample(&self, lon: f64, lat: f64) -> f64 {
        let x = (lon - self.west) / self.step[0] - 0.5;
        let y = (self.north - lat) / self.step[1] - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (c, r) = (x0 as i64, y0 as i64);
        let top = self.pixel(c, r) * (1.0 - fx) + self.pixel(c + 1, r) * fx;
        let bottom = self.pixel(c, r + 1) * (1.0 - fx) + self.pixel(c + 1, r + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// A window made from a function of the pixel position, for tests.
    #[doc(hidden)]
    pub fn from_fn(
        west: f64,
        north: f64,
        step: [f64; 2],
        cols: usize,
        rows: usize,
        f: impl Fn(usize, usize) -> u8,
    ) -> Occurrence {
        let mut data = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                data.push(f(c, r));
            }
        }
        Occurrence {
            west,
            north,
            step,
            cols,
            rows,
            data,
        }
    }
}

/// Read the occurrence of a bounding box `[west, south, east, north]` from the tiles that cover it.
/// `fetch` serves each tile's URL (`Tile::url`). A tile that cannot be read is an error that names it.
/// The window is the box rounded out to whole pixels of the tile grid.
pub fn read_window(fetch: &dyn Fetch, bbox: [f64; 4]) -> Result<Occurrence, ReachError> {
    let tiles = tiles_for(bbox);
    let first = tiles
        .first()
        .ok_or_else(|| ReachError::Input("an empty box".into()))?;
    let mut opened: Vec<(Tile, TiledTiff)> = Vec::new();
    for t in &tiles {
        let url = t.url();
        let tiff = TiledTiff::open(fetch, &url)
            .map_err(|e| ReachError::Parse(format!("JRC tile {} ({url}): {e}", t.name())))?;
        opened.push((*t, tiff));
    }
    // All tiles share one pixel size; the grid is anchored at the first tile's corner.
    let step = opened[0].1.pixel;
    let (ox, oy) = (f64::from(first.west), f64::from(first.north));
    let c0 = ((bbox[0] - ox) / step[0]).floor() as i64;
    let c1 = ((bbox[2] - ox) / step[0]).ceil() as i64;
    let r0 = ((oy - bbox[3]) / step[1]).floor() as i64;
    let r1 = ((oy - bbox[1]) / step[1]).ceil() as i64;
    let (cols, rows) = ((c1 - c0).max(1) as usize, (r1 - r0).max(1) as usize);
    let mut data = vec![0u8; cols * rows];
    for (t, tiff) in &opened {
        let tc = ((tiff.origin[0] - ox) / step[0]).round() as i64; // the tile's column in the window grid
        let tr = ((oy - tiff.origin[1]) / step[1]).round() as i64;
        let (wc0, wr0) = ((c0 - tc).max(0), (r0 - tr).max(0));
        let (wc1, wr1) = (
            (c1 - tc).min(tiff.width as i64),
            (r1 - tr).min(tiff.height as i64),
        );
        if wc1 <= wc0 || wr1 <= wr0 {
            continue;
        }
        let (w, h) = ((wc1 - wc0) as usize, (wr1 - wr0) as usize);
        let px = tiff
            .read_window(fetch, wc0, wr0, w, h)
            .map_err(|e| ReachError::Parse(format!("JRC tile {}: {e}", t.name())))?;
        for y in 0..h {
            let dst = ((tr + wr0 - r0) as usize + y) * cols + (tc + wc0 - c0) as usize;
            data[dst..dst + w].copy_from_slice(&px[y * w..(y + 1) * w]);
        }
    }
    Ok(Occurrence {
        west: ox + c0 as f64 * step[0],
        north: oy - r0 as f64 * step[1],
        step,
        cols,
        rows,
        data,
    })
}

#[cfg(test)]
mod tests {
    //! Label: sourced - JRC Global Surface Water v1.4 file naming (`occurrence_10W_10Nv1_4_2021.tif` covers
    //! 10 W to 0 and 0 to 10 N), checked against the file list of the public bucket on 2026-10-21.
    use super::*;
    use crate::fetch::ReplayFetch;
    use crate::tiff::write_test_tiff;

    #[test]
    fn tile_names_follow_the_jrc_convention() {
        assert_eq!(Tile::of([-6.08, 4.95]).name(), "10W_10N");
        assert_eq!(Tile::of([3.4, 6.5]).name(), "0E_10N");
        assert_eq!(Tile::of([36.8, -1.3]).name(), "30E_0N");
        assert_eq!(Tile::of([-47.0, -15.0]).name(), "50W_10S");
        assert_eq!(
            Tile::of([-6.08, 4.95]).url(),
            "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_10W_10Nv1_4_2021.tif"
        );
    }

    #[test]
    fn a_box_over_a_tile_edge_needs_both_tiles() {
        assert_eq!(tiles_for([-6.2, 4.9, -6.0, 5.1]).len(), 1);
        let t = tiles_for([-0.2, 4.9, 0.2, 5.1]);
        assert_eq!(
            t.iter().map(|t| t.name()).collect::<Vec<_>>(),
            ["10W_10N", "0E_10N"]
        );
        assert_eq!(tiles_for([-0.2, -0.1, 0.2, 0.1]).len(), 4);
        // A box that ends exactly on a tile edge stays in its tile.
        assert_eq!(tiles_for([-6.0, 4.0, 0.0, 5.0]).len(), 1);
    }

    #[test]
    fn a_window_across_two_tiles_is_one_seamless_raster() {
        // Two 64 x 64 test tiles, side by side at the 0 degree edge, 0.001 degree pixels. Pixel value
        // is the global column modulo 97, so a seam would show.
        let step = 0.001;
        let mk = |west: f64, col0: usize| {
            let px: Vec<u8> = (0..64 * 64).map(|i| ((col0 + i % 64) % 97) as u8).collect();
            write_test_tiff(64, 64, 16, &px, true, [west, 10.0], [step, step])
        };
        let west_tile = Tile {
            west: -10,
            north: 10,
        };
        let east_tile = Tile { west: 0, north: 10 };
        let f = ReplayFetch::new()
            .with_response(&west_tile.url(), mk(-0.064, 0))
            .with_response(&east_tile.url(), mk(0.0, 64));
        // The test tiles sit at -0.064..0 and 0..0.064 in longitude, on a 10 degree naming grid.
        // Their stored corners are what the reader uses, so ask for a box over the seam.
        let w = read_window_seam(&f, west_tile, east_tile);
        assert_eq!(w.cols, 40);
        for c in 0..w.cols {
            // Window column c is global column 44 + c (the box starts 20 pixels west of the seam).
            assert_eq!(w.data[c] as usize, (44 + c) % 97, "column {c}");
        }
    }

    /// Open both test tiles and stitch the same way `read_window` does, over a box 20 pixels each side of
    /// the seam. (The tiles' corners are set by hand, so the box goes through `read_window`'s own
    /// stitching with their corners as the grid.)
    fn read_window_seam(f: &ReplayFetch, a: Tile, b: Tile) -> Occurrence {
        let _ = (a, b);
        read_window(f, [-0.02, 9.95, 0.02, 10.0]).unwrap()
    }
}
