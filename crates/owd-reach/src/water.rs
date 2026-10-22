//! The water grid (spec 0013, ticket T2; ADR 0011): water made from the Sentinel-2 scenes themselves,
//! the seed of the bank lines.
//!
//! A raster in the scene's UTM zone, 10 m pixels, three states per pixel: water, land, or unknown.
//! Each pixel records where its state came from: the index of the scene that saw it clear, or the
//! JRC mask.
//!
//! - **Water index.** MNDWI = (green - SWIR) / (green + SWIR) (Xu 2006, Int. J. Remote Sensing
//!   27(14)), the 20 m SWIR sharpened to 10 m by the near-infrared; a clear pixel at or above the
//!   threshold is water. The threshold is the split of the corridor's index into its two classes
//!   (Otsu), within -0.3 to +0.2; where the corridor is not two classes, the default. The scene classification (SCL) marks
//!   no data, cloud shadow, cloud and cirrus as not clear (`sentinel2`).
//! - **Ranking.** Scenes are ranked by the share of the river corridor they see clear (the route
//!   buffered by the largest half-width), not by their overall cloud: a scene with 5 % cloud can have
//!   it on the river. Less overall cloud breaks a tie.
//! - **Composite.** The best scene classifies every corridor pixel it sees clear; the next fills the
//!   pixels still unknown; at most `max_scenes` scenes.
//! - **JRC fill.** A pixel no scene sees is classified by the JRC mask, when one is given, and says so.
//!
//! Provisional defaults until calibrated against a surveyed reach: MNDWI 0.0, up to 3 scenes.

use crate::sentinel2::Imagery;
use owd_format::design::crs::Utm;

type P2 = [f64; 2];

/// MNDWI at or above this is water where a corridor cannot be split (provisional). On the Sassandra
/// scene of spec 0013 the index has two modes, about -0.55 (land) and +0.35 (water).
pub const DEFAULT_MNDWI_THRESHOLD: f64 = 0.0;

/// The most scenes a part's grid is made from (provisional).
pub const DEFAULT_MAX_SCENES: usize = 3;

/// A pixel's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Land,
    Water,
    /// No scene saw it clear, and no JRC fill was given or the JRC window does not cover it.
    Unknown,
}

/// Where a pixel's state came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The scene at this index of [`WaterGrid::scenes`].
    Scene(u8),
    Jrc,
    None,
}

/// A scene used in a grid, with the share of the corridor it saw clear.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneUse {
    pub id: String,
    pub date: String,
    pub cloud_pct: f64,
    /// The share of the corridor's pixels this scene saw clear, 0 to 1.
    pub corridor_clear: f64,
    /// The share of the corridor's pixels this scene classified (the ones no earlier scene saw).
    pub corridor_used: f64,
    /// The MNDWI from which this scene's pixels were water.
    pub mndwi_threshold: f64,
}

/// The water grid of a window.
#[derive(Debug, Clone)]
pub struct WaterGrid {
    pub utm: Utm,
    /// Easting of the west edge and northing of the north edge.
    pub x0: f64,
    pub y1: f64,
    pub px: f64,
    pub cols: usize,
    pub rows: usize,
    pub state: Vec<State>,
    pub origin: Vec<Origin>,
    /// The scenes, best first, that classified at least one pixel.
    pub scenes: Vec<SceneUse>,
    pub threshold: f64,
    /// Whether each pixel lies in the corridor.
    pub corridor: Vec<bool>,
    /// The share of the corridor's pixels the JRC fill classified.
    pub corridor_jrc: f64,
    /// The share of the corridor's pixels left unknown.
    pub corridor_unknown: f64,
}

impl WaterGrid {
    /// The pixel that holds a point, if inside the grid.
    pub fn index(&self, p: P2) -> Option<usize> {
        let c = ((p[0] - self.x0) / self.px).floor();
        let r = ((self.y1 - p[1]) / self.px).floor();
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows)
            .then(|| r as usize * self.cols + c as usize)
    }

    /// The state at a point (unknown outside the grid).
    pub fn at(&self, p: P2) -> (State, Origin) {
        self.index(p).map_or((State::Unknown, Origin::None), |i| {
            (self.state[i], self.origin[i])
        })
    }

    /// The centre of a pixel.
    pub fn centre(&self, i: usize) -> P2 {
        let (r, c) = (i / self.cols, i % self.cols);
        [
            self.x0 + (c as f64 + 0.5) * self.px,
            self.y1 - (r as f64 + 0.5) * self.px,
        ]
    }
}

/// MNDWI of every pixel of a window (`NaN` where the pixel is not clear).
///
/// The shortwave-infrared band is 20 m (repeated to 2 x 2 pixels by `sentinel2`). Used as it is, a
/// shore pixel mixes water and land in its 20 m cell and the edge falls a pixel or two inside the
/// water. So it is sharpened to 10 m by the near-infrared, which tells water from land the same way
/// and is 10 m: each pixel's SWIR is scaled by its NIR over the mean NIR of its 20 m cell.
pub fn mndwi(im: &Imagery) -> Vec<f32> {
    let (g, n, s) = (&im.bands[0], &im.bands[2], &im.bands[3]);
    let (cols, rows) = (im.cols, im.rows);
    // The 20 m cells start at even columns and rows: `sentinel2` reads whole 20 m cells.
    let cell_mean = |i: usize| -> f32 {
        let (r, c) = (i / cols, i % cols);
        let (r0, c0) = (r - r % 2, c - c % 2);
        let mut sum = 0.0;
        let mut k = 0.0;
        for rr in r0..(r0 + 2).min(rows) {
            for cc in c0..(c0 + 2).min(cols) {
                sum += n[rr * cols + cc];
                k += 1.0;
            }
        }
        sum / k
    };
    (0..g.len())
        .map(|i| {
            let m = cell_mean(i);
            let swir = if m > 1e-4 { s[i] * n[i] / m } else { s[i] };
            let d = g[i] + swir;
            if !im.valid[i] || d <= 0.0 {
                f32::NAN
            } else {
                (g[i] - swir) / d
            }
        })
        .collect()
}

/// The bounds of the split between water and land, and what a corridor must hold to be split.
pub const SPLIT_RANGE: (f64, f64) = (-0.3, 0.2);
const SPLIT_MIN_SHARE: f64 = 0.05;
const SPLIT_MIN_GAP: f64 = 0.3;

/// The split of a set of index values into two classes that differ most (Otsu 1979, IEEE Trans.
/// Syst. Man Cybern. 9(1)), on bins of 0.01 from -1 to 1, clamped to [`SPLIT_RANGE`]. `None` when the
/// values are not two classes: either holds under 5 % of them, or their means are under 0.3 apart
/// (all water, or all land).
pub fn otsu_split(values: &[f32]) -> Option<f64> {
    const BINS: usize = 200;
    let mut hist = [0usize; BINS];
    let mut n = 0usize;
    for v in values.iter().filter(|v| v.is_finite()) {
        let b = (((f64::from(*v) + 1.0) / 2.0 * BINS as f64).floor() as isize)
            .clamp(0, BINS as isize - 1);
        hist[b as usize] += 1;
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let centre = |b: usize| -1.0 + (b as f64 + 0.5) * 2.0 / BINS as f64;
    let total: f64 = (0..BINS).map(|b| hist[b] as f64 * centre(b)).sum();
    let (mut w0, mut s0) = (0.0, 0.0);
    let mut best: Option<(f64, usize, f64, f64, f64)> = None;
    for b in 0..BINS - 1 {
        w0 += hist[b] as f64;
        s0 += hist[b] as f64 * centre(b);
        let w1 = n as f64 - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let (m0, m1) = (s0 / w0, (total - s0) / w1);
        let between = w0 * w1 * (m0 - m1).powi(2);
        if best.is_none_or(|x| between > x.0) {
            best = Some((between, b, w0 / n as f64, m0, m1));
        }
    }
    let (_, b, share, m0, m1) = best?;
    if share < SPLIT_MIN_SHARE || 1.0 - share < SPLIT_MIN_SHARE || m1 - m0 < SPLIT_MIN_GAP {
        return None;
    }
    Some((-1.0 + (b + 1) as f64 * 2.0 / BINS as f64).clamp(SPLIT_RANGE.0, SPLIT_RANGE.1))
}

/// The pixels of a window within `half_width` metres of a line (UTM, the window's zone).
pub fn corridor(
    line: &[P2],
    half_width: f64,
    x0: f64,
    y1: f64,
    px: f64,
    cols: usize,
    rows: usize,
) -> Vec<bool> {
    let mut out = vec![false; cols * rows];
    let h2 = half_width * half_width;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (lo, hi) = (
            [a[0].min(b[0]) - half_width, a[1].min(b[1]) - half_width],
            [a[0].max(b[0]) + half_width, a[1].max(b[1]) + half_width],
        );
        let c0 = ((lo[0] - x0) / px).floor().max(0.0) as usize;
        let c1 = (((hi[0] - x0) / px).ceil().max(0.0) as usize).min(cols);
        let r0 = ((y1 - hi[1]) / px).floor().max(0.0) as usize;
        let r1 = (((y1 - lo[1]) / px).ceil().max(0.0) as usize).min(rows);
        let d = [b[0] - a[0], b[1] - a[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        for r in r0..r1 {
            for c in c0..c1 {
                let i = r * cols + c;
                if out[i] {
                    continue;
                }
                let p = [x0 + (c as f64 + 0.5) * px, y1 - (r as f64 + 0.5) * px];
                let t = if l2 == 0.0 {
                    0.0
                } else {
                    (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
                };
                let q = [a[0] + t * d[0] - p[0], a[1] + t * d[1] - p[1]];
                out[i] = q[0] * q[0] + q[1] * q[1] <= h2;
            }
        }
    }
    out
}

/// The share of the corridor's pixels that a validity mask sees clear (0 for an empty corridor).
pub fn clear_share(valid: &[bool], corridor: &[bool]) -> f64 {
    let (mut n, mut k) = (0usize, 0usize);
    for (v, c) in valid.iter().zip(corridor) {
        if *c {
            n += 1;
            k += usize::from(*v);
        }
    }
    if n == 0 { 0.0 } else { k as f64 / n as f64 }
}

/// The order to use scenes in: the most corridor seen clear first, then the least overall cloud,
/// then the earlier in the list. `clear[k]` and `cloud_pct[k]` belong to scene `k`.
pub fn rank(clear: &[f64], cloud_pct: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..clear.len()).collect();
    idx.sort_by(|a, b| {
        clear[*b]
            .total_cmp(&clear[*a])
            .then(cloud_pct[*a].total_cmp(&cloud_pct[*b]))
            .then(a.cmp(b))
    });
    idx
}

/// A scene's window and what it is, for the composite.
pub struct SceneWindow<'a> {
    pub id: &'a str,
    pub date: &'a str,
    pub cloud_pct: f64,
    pub imagery: &'a Imagery,
}

/// The water grid of a window from scenes on the same 10 m grid (one UTM zone, the same window),
/// along a route line `line` with the corridor `half_width` metres either side. The scenes are ranked
/// here; at most `max_scenes` are used. Each scene's pixels are water from the [`otsu_split`] of its
/// clear corridor pixels, or from `threshold` where the corridor is not two classes. `jrc`, when given, says whether a point is water by the JRC
/// mask (`None` outside its window), for the pixels no scene sees.
pub fn compose(
    scenes: &[SceneWindow],
    line: &[P2],
    half_width: f64,
    threshold: f64,
    max_scenes: usize,
    jrc: Option<&dyn Fn(P2) -> Option<bool>>,
) -> Result<WaterGrid, String> {
    let first = scenes
        .first()
        .ok_or_else(|| "no scene to make the water grid from".to_string())?
        .imagery;
    for s in scenes {
        let im = s.imagery;
        if im.utm != first.utm
            || im.cols != first.cols
            || im.rows != first.rows
            || (im.x0 - first.x0).abs() > 1e-6
            || (im.y1 - first.y1).abs() > 1e-6
        {
            return Err(format!(
                "{}: not on the same window as {} (the scenes of one grid must share a grid)",
                s.id, scenes[0].id
            ));
        }
    }
    let (cols, rows, px) = (first.cols, first.rows, first.px);
    let corr = corridor(line, half_width, first.x0, first.y1, px, cols, rows);
    let in_corr = corr.iter().filter(|c| **c).count().max(1) as f64;
    let clear: Vec<f64> = scenes
        .iter()
        .map(|s| clear_share(&s.imagery.valid, &corr))
        .collect();
    let cloud: Vec<f64> = scenes.iter().map(|s| s.cloud_pct).collect();
    let order = rank(&clear, &cloud);
    let mut state = vec![State::Unknown; cols * rows];
    let mut origin = vec![Origin::None; cols * rows];
    let mut used: Vec<SceneUse> = Vec::new();
    for &k in order.iter().take(max_scenes) {
        let idx = mndwi(scenes[k].imagery);
        let in_corridor: Vec<f32> = idx
            .iter()
            .zip(&corr)
            .filter(|(_, c)| **c)
            .map(|(v, _)| *v)
            .collect();
        let threshold = otsu_split(&in_corridor).unwrap_or(threshold);
        let mut n_used = 0usize;
        let slot = used.len() as u8;
        for i in 0..cols * rows {
            if state[i] != State::Unknown || idx[i].is_nan() {
                continue;
            }
            state[i] = if f64::from(idx[i]) >= threshold {
                State::Water
            } else {
                State::Land
            };
            origin[i] = Origin::Scene(slot);
            n_used += usize::from(corr[i]);
        }
        if n_used == 0 && !used.is_empty() {
            // Nothing left in the corridor for this scene: it fills only pixels off the river.
            for o in origin.iter_mut().filter(|o| **o == Origin::Scene(slot)) {
                *o = Origin::None;
            }
            for (st, o) in state.iter_mut().zip(&origin) {
                if *o == Origin::None {
                    *st = State::Unknown;
                }
            }
            continue;
        }
        used.push(SceneUse {
            id: scenes[k].id.to_string(),
            date: scenes[k].date.to_string(),
            cloud_pct: scenes[k].cloud_pct,
            corridor_clear: clear[k],
            mndwi_threshold: threshold,
            corridor_used: n_used as f64 / in_corr,
        });
        if state
            .iter()
            .zip(&corr)
            .all(|(s, c)| !*c || *s != State::Unknown)
        {
            break;
        }
    }
    let mut grid = WaterGrid {
        utm: first.utm,
        x0: first.x0,
        y1: first.y1,
        px,
        cols,
        rows,
        state,
        origin,
        scenes: used,
        threshold,
        corridor: corr,
        corridor_jrc: 0.0,
        corridor_unknown: 0.0,
    };
    let mut n_jrc = 0usize;
    if let Some(f) = jrc {
        for i in 0..cols * rows {
            if grid.state[i] != State::Unknown {
                continue;
            }
            if let Some(w) = f(grid.centre(i)) {
                grid.state[i] = if w { State::Water } else { State::Land };
                grid.origin[i] = Origin::Jrc;
                n_jrc += usize::from(grid.corridor[i]);
            }
        }
    }
    let unknown = (0..cols * rows)
        .filter(|i| grid.corridor[*i] && grid.state[*i] == State::Unknown)
        .count();
    grid.corridor_jrc = n_jrc as f64 / in_corr;
    grid.corridor_unknown = unknown as f64 / in_corr;
    Ok(grid)
}
