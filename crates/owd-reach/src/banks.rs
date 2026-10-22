//! The first bank lines for a route, from the JRC water mask (spec 0012, ticket T3).
//!
//! The route (an OSM line) says where the river is, roughly. At stations along it, rays cast to the
//! left and right across the water find where the occurrence falls under the threshold: that is the
//! bank. The midpoints of the two banks give a better centreline, and a second pass casts from it.
//! Output is in the study's UTM zone (ADR 0003): left and right bank, centreline, and the river
//! polygon (left bank, then right bank back). A station where the water is narrower than the minimum
//! trusted width, or not found, is flagged, and so is the stretch it belongs to.
//!
//! Defaults (provisional until calibrated against a surveyed reach): occurrence of 75 % or more is
//! water; under 3 pixels (90 m on JRC) is low-confidence width.

use crate::bankcheck::{CheckParams, check_and_repair, merge, rechain};
use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::geo::LonLat;
use crate::jrc::{Occurrence, read_window};
use crate::provenance::provenance_record;
use owd_format::design::crs::Utm;
use owd_format::design::provenance::{Mark, with_settings};
use owd_format::design::{Role, Session};
use serde_json::{Value, json};

/// The mask's nominal pixel size, for the minimum trusted width.
pub const JRC_PIXEL_M: f64 = 30.0;

/// The date range of the JRC occurrence layer, written as the input's date.
pub const JRC_DATE: &str = "1984-2021";

/// Settings of the bank generation. Every value here is a provisional default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BankParams {
    /// Occurrence, in percent, from which a pixel counts as water.
    pub threshold_pct: f64,
    /// A width under this many mask pixels is low-confidence.
    pub min_width_px: f64,
    /// The distance between stations along the route, in metres.
    pub station_step_m: f64,
    /// The farthest a bank may be from the route, in metres, each side.
    pub max_half_width_m: f64,
    /// A gap in the water shorter than this, in metres, does not end a ray.
    pub gap_tolerance_m: f64,
    /// How far from the route a station may look for water, in metres.
    pub seed_search_m: f64,
}

impl Default for BankParams {
    fn default() -> Self {
        BankParams {
            threshold_pct: 75.0,
            min_width_px: 3.0,
            station_step_m: 20.0,
            max_half_width_m: 1_500.0,
            gap_tolerance_m: 60.0,
            seed_search_m: 90.0,
        }
    }
}

impl BankParams {
    /// The narrowest width trusted, in metres.
    pub fn min_width_m(&self) -> f64 {
        self.min_width_px * JRC_PIXEL_M
    }
}

/// Why a stretch is flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchKind {
    /// Water was found, narrower than the minimum trusted width.
    Narrow,
    /// No water at the threshold near the route.
    NoWater,
    /// The imagery gave no clear edge (cloud, shadow, turbid water): the mask bank was kept.
    NoEdge,
    /// The imagery shows water (or land) running on past the largest shift: the bank moved by that
    /// shift, and the edge lies farther.
    AtShiftLimit,
    /// A bank check failed here (spec 0013): the bank was repaired, or (a narrow width alone) flagged.
    Repaired(crate::bankcheck::Checks),
    /// No Sentinel-2 scene saw the bank clear: the JRC mask gave it (spec 0013).
    JrcFallback,
}

impl StretchKind {
    /// The name the document writes.
    pub fn name(self) -> &'static str {
        match self {
            StretchKind::Narrow => "narrow",
            StretchKind::NoWater => "no-water",
            StretchKind::NoEdge => "no-edge",
            StretchKind::AtShiftLimit => "at-shift-limit",
            StretchKind::Repaired(_) => "repaired",
            StretchKind::JrcFallback => "jrc-fallback",
        }
    }
}

/// The stretches as the document writes them.
pub fn stretches_value(stretches: &[Stretch]) -> Vec<Value> {
    stretches
        .iter()
        .map(|s| {
            let mut v = json!({
                "from_m": s.from_m, "to_m": s.to_m, "kind": s.kind.name(), "min_width_m": s.min_width_m,
            });
            if let StretchKind::Repaired(c) = s.kind {
                v["checks"] = json!(c.names());
            }
            v
        })
        .collect()
}

/// A flagged stretch, by chainage along the generated centreline.
#[derive(Debug, Clone, PartialEq)]
pub struct Stretch {
    pub from_m: f64,
    pub to_m: f64,
    pub kind: StretchKind,
    /// The narrowest width in the stretch, in metres (0 for no water).
    pub min_width_m: f64,
}

/// What the mask gives.
#[derive(Debug, Clone)]
pub struct MaskBanks {
    pub utm: Utm,
    pub params: BankParams,
    /// The mid-line of the banks (spec 0013), at stations `station_step_m` apart.
    pub centreline: Vec<[f64; 2]>,
    pub left: Vec<[f64; 2]>,
    pub right: Vec<[f64; 2]>,
    /// Width at each station of the centreline, in metres.
    pub widths: Vec<f64>,
    /// Closed ring: left bank, then right bank back.
    pub polygon: Vec<[f64; 2]>,
    pub stretches: Vec<Stretch>,
    /// The mask's pixel size in metres (30 for JRC, 10 for Sentinel-2): the minimum trusted width is
    /// `params.min_width_px` of them.
    pub pixel_m: f64,
    /// The registry id of the seed: `jrc-gsw` or `sentinel2-l2a`.
    pub source: &'static str,
}

impl MaskBanks {
    /// The narrowest width trusted, in metres.
    pub fn min_width_m(&self) -> f64 {
        self.params.min_width_px * self.pixel_m
    }
}

/// What bank lines are traced on: a water mask, read at points in the study's UTM zone.
pub trait WaterSource {
    /// How much a point is water, compared with [`WaterSource::threshold`].
    fn level(&self, p: [f64; 2]) -> f64;
    /// From this level a point is water.
    fn threshold(&self) -> f64;
    /// The mask's pixel size in metres.
    fn pixel_m(&self) -> f64;
    /// The registry id of the mask.
    fn source(&self) -> &'static str;
    /// Whether the state at a point came from the JRC fallback, not from the mask itself.
    fn fallback(&self, _p: [f64; 2]) -> bool {
        false
    }
    /// What to say when the mask has no water along a route.
    fn no_water(&self) -> String;
}

/// The JRC occurrence as a [`WaterSource`]: percent of observations that saw water.
pub(crate) struct JrcSource<'a> {
    pub(crate) occ: &'a Occurrence,
    pub(crate) grid: &'a GeoGrid,
    pub(crate) threshold_pct: f64,
}

impl WaterSource for JrcSource<'_> {
    fn level(&self, p: [f64; 2]) -> f64 {
        let ll = self.grid.lonlat(p);
        self.occ.sample(ll[0], ll[1])
    }
    fn threshold(&self) -> f64 {
        self.threshold_pct
    }
    fn pixel_m(&self) -> f64 {
        JRC_PIXEL_M
    }
    fn source(&self) -> &'static str {
        "jrc-gsw"
    }
    fn no_water(&self) -> String {
        format!(
            "the JRC water mask has no water at {}% or more along this route. The river may be too \
             narrow for a 30 m mask, or too rarely seen. Lower the threshold (a provisional default), \
             or load bank lines of your own.",
            self.threshold_pct
        )
    }
}

/// UTM to longitude and latitude, by bilinear interpolation of a coarse grid (one projection set-up,
/// not one per sample). The grid is 250 m; the error of the interpolation is far below a millimetre.
pub(crate) struct GeoGrid {
    x0: f64,
    y0: f64,
    step: f64,
    nx: usize,
    ny: usize,
    pts: Vec<LonLat>,
}

impl GeoGrid {
    pub(crate) fn new(utm: Utm, min: [f64; 2], max: [f64; 2]) -> Result<GeoGrid, ReachError> {
        let step = 250.0;
        let nx = ((max[0] - min[0]) / step).ceil() as usize + 2;
        let ny = ((max[1] - min[1]) / step).ceil() as usize + 2;
        let mut xy = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                xy.push([min[0] + i as f64 * step, min[1] + j as f64 * step]);
            }
        }
        let pts = utm.inverse_all(&xy).map_err(ReachError::Input)?;
        Ok(GeoGrid {
            x0: min[0],
            y0: min[1],
            step,
            nx,
            ny,
            pts,
        })
    }

    pub(crate) fn lonlat(&self, p: [f64; 2]) -> LonLat {
        let fx = ((p[0] - self.x0) / self.step).clamp(0.0, (self.nx - 1) as f64 - 1e-9);
        let fy = ((p[1] - self.y0) / self.step).clamp(0.0, (self.ny - 1) as f64 - 1e-9);
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let at = |a: usize, b: usize| self.pts[b * self.nx + a];
        let mix = |k: usize| {
            let (a, b, c, d) = (
                at(i, j)[k],
                at(i + 1, j)[k],
                at(i, j + 1)[k],
                at(i + 1, j + 1)[k],
            );
            (a * (1.0 - tx) + b * tx) * (1.0 - ty) + (c * (1.0 - tx) + d * tx) * ty
        };
        [mix(0), mix(1)]
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Points every `step` metres along a polyline.
fn resample_xy(line: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let mut out = vec![line[0]];
    let mut carry = 0.0;
    for w in line.windows(2) {
        let seg = dist(w[0], w[1]);
        if seg == 0.0 {
            continue;
        }
        let mut at = step - carry;
        while at < seg {
            let f = at / seg;
            out.push([
                w[0][0] + f * (w[1][0] - w[0][0]),
                w[0][1] + f * (w[1][1] - w[0][1]),
            ]);
            at += step;
        }
        carry = seg - (at - step);
    }
    let last = *line.last().expect("a line has a last point");
    if dist(*out.last().expect("out has a point"), last) > step * 0.25 || out.len() == 1 {
        out.push(last);
    }
    out
}

/// Unit left normals of a polyline, from the direction over a few stations each side.
fn normals(line: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = line.len();
    (0..n)
        .map(|i| {
            let (a, b) = (line[i.saturating_sub(3)], line[(i + 3).min(n - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-9);
            [-dy / l, dx / l]
        })
        .collect()
}

struct Cast {
    /// The centre of the water across the station.
    centre: [f64; 2],
    left: [f64; 2],
    right: [f64; 2],
    width: f64,
    /// An edge of this cast came from the JRC fallback.
    fallback: bool,
}

struct Mask<'a> {
    src: &'a dyn WaterSource,
    params: &'a BankParams,
}

impl Mask<'_> {
    fn level(&self, p: [f64; 2]) -> f64 {
        self.src.level(p)
    }

    /// Distance along `dir` from `from` to where the water ends, by 5 m steps and a linear crossing.
    fn reach(&self, from: [f64; 2], dir: [f64; 2]) -> f64 {
        let thr = self.src.threshold();
        let step = 5.0;
        let mut last_water = 0.0;
        let mut last_level = self.level(from);
        let mut d = step;
        while d <= self.params.max_half_width_m {
            let l = self.level([from[0] + dir[0] * d, from[1] + dir[1] * d]);
            if l >= thr {
                last_water = d;
                last_level = l;
            } else if d - last_water > self.params.gap_tolerance_m {
                break;
            }
            d += step;
        }
        // The first sample past the last water is below the threshold: place the edge on the line
        // between them.
        let next = last_water + step;
        let ln = self.level([from[0] + dir[0] * next, from[1] + dir[1] * next]);
        if ln < thr && last_level > ln {
            last_water + step * ((last_level - thr) / (last_level - ln)).clamp(0.0, 1.0)
        } else {
            last_water
        }
    }

    /// Cast across the water at a station, or `None` where there is none within reach.
    fn cast(&self, p: [f64; 2], n: [f64; 2]) -> Option<Cast> {
        let thr = self.src.threshold();
        let seed = if self.level(p) >= thr {
            Some(p)
        } else {
            // Nearest water within reach, looking along and across the route (a mask artefact can
            // cut the river across, so water may be along it, not across it).
            let t = [n[1], -n[0]];
            let r = (self.params.seed_search_m / 5.0).floor() as i32;
            let mut offsets: Vec<(i32, i32)> = (-r..=r)
                .flat_map(|a| (-r..=r).map(move |b| (a, b)))
                .filter(|(a, b)| a * a + b * b <= r * r)
                .collect();
            offsets.sort_by_key(|(a, b)| a * a + b * b);
            offsets.into_iter().find_map(|(a, b)| {
                let q = [
                    p[0] + 5.0 * (f64::from(a) * t[0] + f64::from(b) * n[0]),
                    p[1] + 5.0 * (f64::from(a) * t[1] + f64::from(b) * n[1]),
                ];
                (self.level(q) >= thr).then_some(q)
            })
        }?;
        let dl = self.reach(seed, n);
        let dr = self.reach(seed, [-n[0], -n[1]]);
        // A bank whose edge the mask took from the JRC fallback (a pixel inside it, a pixel out).
        let at = |d: f64| [seed[0] + n[0] * d, seed[1] + n[1] * d];
        let fallback = [dl - 5.0, dl + 5.0, -dr + 5.0, -dr - 5.0]
            .iter()
            .any(|d| self.src.fallback(at(*d)));
        Some(Cast {
            fallback,
            centre: [
                seed[0] + n[0] * (dl - dr) / 2.0,
                seed[1] + n[1] * (dl - dr) / 2.0,
            ],
            left: [seed[0] + n[0] * dl, seed[1] + n[1] * dl],
            right: [seed[0] - n[0] * dr, seed[1] - n[1] * dr],
            width: dl + dr,
        })
    }
}

/// A moving average over `2 * k + 1` stations (ends use what is there).
fn smooth(pts: &[[f64; 2]], k: usize) -> Vec<[f64; 2]> {
    (0..pts.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(k), (i + k).min(pts.len() - 1));
            let n = (b - a + 1) as f64;
            let s = pts[a..=b]
                .iter()
                .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
            [s[0] / n, s[1] / n]
        })
        .collect()
}

/// Make bank lines, a centreline and a river polygon from a window of occurrence along a route.
pub fn banks_from_mask(
    route: &[LonLat],
    occ: &Occurrence,
    params: &BankParams,
) -> Result<MaskBanks, ReachError> {
    let (utm, xy, grid) = route_in_utm(route, params)?;
    let src = JrcSource {
        occ,
        grid: &grid,
        threshold_pct: params.threshold_pct,
    };
    banks_from_source(utm, &xy, &src, params)
}

/// The route in the UTM zone of its middle, and the grid that takes points back to longitude and
/// latitude over its window.
pub(crate) fn route_in_utm(
    route: &[LonLat],
    params: &BankParams,
) -> Result<(Utm, Vec<[f64; 2]>, GeoGrid), ReachError> {
    if route.len() < 2 {
        return Err(ReachError::Input(
            "a route needs at least two points".into(),
        ));
    }
    let mid = route[route.len() / 2];
    let utm = Utm::for_point(mid[0], mid[1]);
    let xy = utm.forward_all(route).map_err(ReachError::Input)?;
    let pad = params.max_half_width_m + 500.0;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &xy {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let grid = GeoGrid::new(utm, [lo[0] - pad, lo[1] - pad], [hi[0] + pad, hi[1] + pad])?;
    Ok((utm, xy, grid))
}

/// Make bank lines, a centreline and a river polygon along a route (`xy`, in `utm`) on any water
/// mask.
pub fn banks_from_source(
    utm: Utm,
    xy: &[[f64; 2]],
    src: &dyn WaterSource,
    params: &BankParams,
) -> Result<MaskBanks, ReachError> {
    let mask = Mask { src, params };

    let mut line = resample_xy(xy, params.station_step_m);
    let mut casts: Vec<Option<Cast>> = Vec::new();
    let mut reference: Vec<[f64; 2]> = line.clone();
    for pass in 0..2 {
        let n = normals(&line);
        casts = line
            .iter()
            .zip(&n)
            .map(|(p, n)| mask.cast(*p, *n))
            .collect();
        if pass == 0 {
            // A better centreline for the second cast: the middle of the water, smoothed, with the
            // route where there is none.
            let centres: Vec<[f64; 2]> = line
                .iter()
                .zip(&casts)
                .map(|(p, c)| c.as_ref().map_or(*p, |c| c.centre))
                .collect();
            // And the line the checks read the banks along (spec 0013): the route moved to the
            // middle of the water by the running median of the casts' offsets from it. A median, so
            // that a cast that found other water cannot throw it off.
            let off: Vec<Option<f64>> = line
                .iter()
                .zip(&n)
                .zip(&casts)
                .map(|((p, n), c)| {
                    c.as_ref()
                        .map(|c| (c.centre[0] - p[0]) * n[0] + (c.centre[1] - p[1]) * n[1])
                })
                .collect();
            let k = (250.0 / params.station_step_m).round() as usize;
            let moved: Vec<[f64; 2]> = line
                .iter()
                .zip(&n)
                .enumerate()
                .map(|(i, (p, nn))| {
                    let (a, b) = (i.saturating_sub(k), (i + k).min(off.len() - 1));
                    let mut v: Vec<f64> = off[a..=b].iter().flatten().copied().collect();
                    v.sort_by(f64::total_cmp);
                    let m = if v.is_empty() { 0.0 } else { v[v.len() / 2] };
                    [p[0] + nn[0] * m, p[1] + nn[1] * m]
                })
                .collect();
            reference = smooth(&moved, 2);
            line = smooth(&centres, 2);
        }
    }

    // Chainage along the final stations.
    let mut chain = vec![0.0];
    for w in line.windows(2) {
        chain.push(chain.last().unwrap() + dist(w[0], w[1]));
    }
    let half = params.station_step_m / 2.0;
    let min_w = params.min_width_px * src.pixel_m();
    // A stretch takes the most serious kind in it.
    let rank = |k: StretchKind| match k {
        StretchKind::NoWater => 3,
        StretchKind::Narrow => 2,
        _ => 1,
    };
    let mut stretches: Vec<Stretch> = Vec::new();
    for (i, c) in casts.iter().enumerate() {
        let (kind, w) = match c {
            None => (Some(StretchKind::NoWater), 0.0),
            Some(c) if c.width < min_w => (Some(StretchKind::Narrow), c.width),
            Some(c) if c.fallback => (Some(StretchKind::JrcFallback), c.width),
            Some(c) => (None, c.width),
        };
        let Some(kind) = kind else { continue };
        let (from, to) = ((chain[i] - half).max(0.0), chain[i] + half);
        match stretches.last_mut() {
            Some(s)
                if from <= s.to_m + half
                    && (s.kind == kind || rank(s.kind) > 1 && rank(kind) > 1) =>
            {
                s.to_m = to;
                s.min_width_m = s.min_width_m.min(w);
                if rank(kind) > rank(s.kind) {
                    s.kind = kind;
                }
            }
            _ => stretches.push(Stretch {
                from_m: from,
                to_m: to,
                kind,
                min_width_m: w,
            }),
        }
    }

    let found: Vec<(&Cast, f64)> = casts
        .iter()
        .zip(&chain)
        .filter_map(|(c, d)| c.as_ref().map(|c| (c, *d)))
        .collect();
    if found.len() < 2 {
        return Err(ReachError::NoWater(src.no_water()));
    }
    let left: Vec<[f64; 2]> = found.iter().map(|(c, _)| c.left).collect();
    let right: Vec<[f64; 2]> = found.iter().map(|(c, _)| c.right).collect();
    let centre: Vec<[f64; 2]> = found.iter().map(|(c, _)| c.centre).collect();
    // Spec 0013: check and repair the banks, and make the centreline their mid-line. The checks read
    // the banks along the route moved onto the water by a running median, not along the casts' own
    // centres: a cast that found other water throws its centre far off.
    let _ = centre;
    let checked = check_and_repair(
        &left,
        &right,
        &reference,
        &CheckParams::at_step(params.station_step_m),
    )?;
    let stretches = merge(
        rechain(&stretches, &line, &checked.centreline),
        checked.stretches,
    );
    let (left, right, centreline, widths) = (
        checked.left,
        checked.right,
        checked.centreline,
        checked.widths,
    );
    let mut polygon = left.clone();
    polygon.extend(right.iter().rev());
    polygon.push(left[0]);
    Ok(MaskBanks {
        utm,
        params: *params,
        centreline,
        left,
        right,
        widths,
        polygon,
        stretches,
        pixel_m: src.pixel_m(),
        source: src.source(),
    })
}

/// The bounding box `[west, south, east, north]` of the window to read for a route.
pub fn window_bbox(route: &[LonLat], params: &BankParams) -> Option<[f64; 4]> {
    let b = crate::geo::bbox(route)?;
    // Degrees of margin for the farthest bank, at the poorer (east-west) scale.
    let m = (params.max_half_width_m + 500.0)
        / (111_320.0 * b[1].abs().min(80.0).to_radians().cos().max(0.2));
    Some([b[0] - m, b[1] - m, b[2] + m, b[3] + m])
}

/// Read the occurrence window along a route from the JRC tiles and make the banks.
pub fn banks_from_jrc(
    fetch: &dyn Fetch,
    route: &[LonLat],
    params: &BankParams,
) -> Result<MaskBanks, ReachError> {
    let bbox =
        window_bbox(route, params).ok_or_else(|| ReachError::Input("an empty route".into()))?;
    let occ = read_window(fetch, bbox)?;
    banks_from_mask(route, &occ, params)
}

fn utm_geojson(geometry: Value, utm: Utm) -> String {
    let mut g = geometry;
    g["crs"] = json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", utm.epsg())}});
    g.to_string()
}

fn coords(pts: &[[f64; 2]]) -> Value {
    json!(pts.iter().map(|p| json!([p[0], p[1]])).collect::<Vec<_>>())
}

impl MaskBanks {
    /// The `low_confidence_width` value of the document.
    pub fn low_confidence_value(&self) -> Value {
        json!({
            "min_width_m": self.min_width_m(),
            "provisional": true,
            "source": self.source,
            "stretches": stretches_value(&self.stretches),
        })
    }
}

/// Record the banks in a document: the four inputs, each marked `default` with the JRC notice and the
/// settings it was made with (named provisional), and the low-confidence stretches.
pub fn record_banks(session: &mut Session, banks: &MaskBanks) -> Result<(), ReachError> {
    let settings = json!({
        "mask_threshold_pct": banks.params.threshold_pct,
        "min_trusted_width_m": banks.min_width_m(),
        "station_step_m": banks.params.station_step_m,
        "max_half_width_m": banks.params.max_half_width_m,
        "gap_tolerance_m": banks.params.gap_tolerance_m,
        "provisional": ["mask_threshold_pct", "min_trusted_width_m"],
    });
    let rec =
        provenance_record("jrc-gsw", JRC_DATE, Mark::Default, true).map_err(ReachError::Input)?;
    record_banks_as(session, banks, &with_settings(rec, settings))
}

/// Record banks in a document with the provenance record given (source, date, notice, settings):
/// the four inputs, each with the record, and the low-confidence stretches.
pub fn record_banks_as(
    session: &mut Session,
    banks: &MaskBanks,
    record: &Value,
) -> Result<(), ReachError> {
    let u = banks.utm;
    let line = |pts: &[[f64; 2]]| {
        utm_geojson(json!({"type": "LineString", "coordinates": coords(pts)}), u)
    };
    let poly = utm_geojson(
        json!({"type": "Polygon", "coordinates": [coords(&banks.polygon)]}),
        u,
    );
    let (l, r, c) = (
        line(&banks.left),
        line(&banks.right),
        line(&banks.centreline),
    );
    session
        .import(&[
            (Role::LeftBank, l.as_str()),
            (Role::RightBank, r.as_str()),
            (Role::Centreline, c.as_str()),
            (Role::RiverPolygon, poly.as_str()),
        ])
        .map_err(|e| ReachError::Input(e.to_string()))?;
    for input in ["left_bank", "right_bank", "centreline", "river_polygon"] {
        session
            .set_provenance(input, record.clone())
            .map_err(|e| ReachError::Input(e.to_string()))?;
    }
    session
        .set_low_confidence_width(banks.low_confidence_value())
        .map_err(|e| ReachError::Input(e.to_string()))
}
