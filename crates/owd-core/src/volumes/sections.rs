//! Cross-section earthworks along an alignment: the station walk around the cut models. Mirrors
//! `volumes/sections.py`.
//!
//! Walks the alignment every `station_m`, samples a transverse terrain profile out to
//! `+-maxw_m` at `dx_m` spacing, splits it wet/dry against the present river polygon, applies Cut
//! Model A or B ([`crate::cut_model`]), and integrates cut area x station spacing into dredge
//! (wet) and excavation (dry) volumes. The pool water level per chainage comes from the caller.
//!
//! Terrain comes in as a sampler `terrain(xs, ys) -> z` (NaN where unknown), so the same code
//! runs on an analytic surface, a grid, or a raster DEM.
//!
//! Quantities only: m2, m3, metres of face height.

use crate::cut_model::{
    CUT_MODEL, CUT_MODEL_B, DX_M, MAXW_M, STATION_M, WET_SLOPE_H_PER_V, cut_model_a_bench_gate,
    cut_model_a_section_face, normalise_model, section_area,
};
use crate::geom::{Polygon, Pt, linear};
use crate::py_sum;
use crate::pyrepr::repr_str;

/// A terrain sampler: heights for the points `(xs[i], ys[i])`, NaN where unknown.
pub type TerrainSampler<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// The pool water level: a number, or a function of chainage.
pub enum Level<'a> {
    Fixed(f64),
    AtChainage(&'a dyn Fn(f64) -> f64),
}

impl Level<'_> {
    fn at(&self, s: f64) -> f64 {
        match self {
            Level::Fixed(v) => *v,
            Level::AtChainage(f) => f(s),
        }
    }
}

/// The bottom half-width: one number, one per station, or a function of the station points.
pub enum HalfWidth<'a> {
    Fixed(f64),
    PerStation(Vec<f64>),
    FromStations(&'a dyn Fn(&[Pt]) -> Vec<f64>),
}

/// One transverse profile and its cut. Offsets are signed: negative left bank, positive right.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossSection {
    pub chainage_m: f64,
    pub x: f64,
    pub y: f64,
    pub heading_rad: f64,
    pub offsets: Vec<f64>,
    /// NaN where unknown.
    pub terrain: Vec<f64>,
    /// True inside the present river.
    pub wet: Vec<bool>,
    /// Bottom half-width.
    pub hb: f64,
    pub wl: f64,
    pub grade: f64,
    pub depth: f64,
    pub model: String,
    /// Design face elevation, NaN where no cut face.
    pub cutface: Vec<f64>,
    pub dredge_area_m2: f64,
    pub excav_area_m2: f64,
    pub max_face_h_m: f64,
    pub bench_left: bool,
    pub bench_right: bool,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionVolumes {
    pub stations: Vec<CrossSection>,
    pub station_m: f64,
}

/// Dredge and excavation volumes of one tag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TagVolumes {
    pub dredge_m3: f64,
    pub excav_m3: f64,
}

impl SectionVolumes {
    pub fn dredge_m3(&self) -> f64 {
        py_sum(self.stations.iter().map(|s| s.dredge_area_m2)) * self.station_m
    }

    pub fn excav_m3(&self) -> f64 {
        py_sum(self.stations.iter().map(|s| s.excav_area_m2)) * self.station_m
    }

    pub fn total_m3(&self) -> f64 {
        self.dredge_m3() + self.excav_m3()
    }

    pub fn max_face_h_m(&self) -> f64 {
        let mut it = self.stations.iter().map(|s| s.max_face_h_m);
        match it.next() {
            None => 0.0,
            Some(first) => it.fold(first, |m, x| if x > m { x } else { m }),
        }
    }

    /// Volumes per tag, in the order each tag first appears.
    pub fn by_tag(&self) -> Vec<(String, TagVolumes)> {
        let mut out: Vec<(String, TagVolumes)> = Vec::new();
        for s in &self.stations {
            let i = match out.iter().position(|(t, _)| *t == s.tag) {
                Some(i) => i,
                None => {
                    out.push((
                        s.tag.clone(),
                        TagVolumes {
                            dredge_m3: 0.0,
                            excav_m3: 0.0,
                        },
                    ));
                    out.len() - 1
                }
            };
            out[i].1.dredge_m3 += s.dredge_area_m2 * self.station_m;
            out[i].1.excav_m3 += s.excav_area_m2 * self.station_m;
        }
        out
    }
}

/// `np.arange(start, stop, step)`: `ceil((stop - start) / step)` values `start + i * delta`, with
/// `delta = (start + step) - start`.
pub(crate) fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil();
    let n = if n > 0.0 { n as usize } else { 0 };
    let delta = (start + step) - start;
    (0..n).map(|i| start + i as f64 * delta).collect()
}

/// `np.linspace(0, stop, num)`: `i * step`, the last value `stop`.
fn linspace0(stop: f64, num: usize) -> Vec<f64> {
    if num == 1 {
        return vec![0.0];
    }
    let div = (num - 1) as f64;
    let step = stop / div;
    let mut y: Vec<f64> = (0..num)
        .map(|i| {
            if step == 0.0 {
                (i as f64 / div) * stop
            } else {
                i as f64 * step
            }
        })
        .collect();
    y[num - 1] = stop;
    y
}

/// The stations of a line: chainages, plan points and headings (rad), one each per station.
pub type StationFrame = (Vec<f64>, Vec<Pt>, Vec<f64>);

/// `(chainages, xy, headings)` every `station_m` along `line`.
pub fn station_frame(line: &[Pt], station_m: f64) -> Result<StationFrame, String> {
    let length = linear::length(line);
    // `int(line.length // station_m) + 1`: floor division of floats.
    let n = (length / station_m).floor() as i64 + 1;
    let n = if n > 0 { n as usize } else { 0 };
    let dsl = if n == 0 {
        Vec::new()
    } else {
        linspace0(length, n)
    };
    let xy: Vec<Pt> = dsl
        .iter()
        .map(|&d| linear::interpolate(line, d).unwrap_or([f64::NAN, f64::NAN]))
        .collect();
    if xy.len() < 2 {
        return Err(
            "Shape of array too small to calculate a numerical gradient, at least \
                    (edge_order + 1) elements are required."
                .into(),
        );
    }
    // `np.gradient(xy, axis=0)`: forward and backward differences at the ends, central between.
    let m = xy.len();
    let grad = |c: usize| -> Vec<f64> {
        (0..m)
            .map(|i| {
                if i == 0 {
                    xy[1][c] - xy[0][c]
                } else if i == m - 1 {
                    xy[m - 1][c] - xy[m - 2][c]
                } else {
                    (xy[i + 1][c] - xy[i - 1][c]) / 2.0
                }
            })
            .collect()
    };
    let (gx, gy) = (grad(0), grad(1));
    let th: Vec<f64> = (0..m).map(|i| libm::atan2(gy[i], gx[i])).collect();
    Ok((dsl, xy, th))
}

/// Why `section_volumes` refused: a Python `ValueError`.
pub type Refusal = String;

/// Dredge and excavation volumes along `line` under Cut Model A or B.
///
/// `hb` is the bottom half-width; `wl` the pool water level (grade = wl - depth). `tag(s)`
/// optionally labels each station (for `by_tag`). `chunk` only sets how many stations are sampled
/// per terrain call; it never changes a result.
#[allow(clippy::too_many_arguments)]
pub fn section_volumes(
    line: &[Pt],
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    wl: &Level,
    depth: f64,
    hb: &HalfWidth,
    model: &str,
    wet_slope: f64,
    station_m: f64,
    maxw_m: f64,
    dx_m: f64,
    daylight_run_m: Option<f64>,
    tag: Option<&dyn Fn(f64) -> String>,
    chunk: usize,
) -> Result<SectionVolumes, Refusal> {
    let mdl = normalise_model(model);
    if mdl != CUT_MODEL && mdl != CUT_MODEL_B {
        return Err(format!(
            "unknown cut model {}; use 'a' or 'b'",
            repr_str(model)
        ));
    }
    let (dsl, xy, th) = station_frame(line, station_m)?;
    let hb_st: Vec<f64> = match hb {
        HalfWidth::Fixed(v) => vec![*v; xy.len()],
        HalfWidth::PerStation(v) => v.clone(),
        HalfWidth::FromStations(f) => f(&xy),
    };
    if hb_st.len() != xy.len() {
        let shape = if hb_st.len() == 1 {
            "(1,)".to_string()
        } else {
            format!("({},)", hb_st.len())
        };
        return Err(format!(
            "hb must give one half-width per station ({}); got {shape}",
            xy.len()
        ));
    }

    let offs = arange(-maxw_m, maxw_m + 0.1 * dx_m, dx_m);
    // Every sample's wet test reads the one river: index it once (the same answers as `contains_xy`).
    let inside = crate::geom::overlay::PolygonIndex::new(river_polygon);
    let sidewhich: Vec<i8> = offs.iter().map(|&o| if o < 0.0 { -1 } else { 1 }).collect();
    let m = offs.len();
    let mut out = SectionVolumes {
        stations: Vec::new(),
        station_m,
    };
    let chunk = chunk.max(1);
    let mut a = 0;
    while a < xy.len() {
        let b = std::cmp::min(a + chunk, xy.len());
        let mut ax = Vec::with_capacity((b - a) * m);
        let mut ay = Vec::with_capacity((b - a) * m);
        for i in a..b {
            let nx = -libm::sin(th[i]);
            let ny = libm::cos(th[i]);
            for &o in &offs {
                ax.push(xy[i][0] + o * nx);
                ay.push(xy[i][1] + o * ny);
            }
        }
        let z_all = terrain(&ax, &ay);
        if z_all.len() != (b - a) * m {
            return Err(format!(
                "cannot reshape array of size {} into shape ({},{})",
                z_all.len(),
                b - a,
                m
            ));
        }
        for (j, i) in (a..b).enumerate() {
            let s = dsl[i];
            let wl_i = wl.at(s);
            let grade = wl_i - depth;
            let z = &z_all[j * m..(j + 1) * m];
            let w: Vec<bool> = (0..m)
                .map(|k| inside.contains_xy([ax[j * m + k], ay[j * m + k]]))
                .collect();
            let zl: Vec<f64> = z
                .iter()
                .map(|&v| if v.is_finite() { v } else { f64::NAN })
                .collect();
            let wet: Vec<bool> = (0..m).map(|k| zl[k].is_finite() && w[k]).collect();
            let (dr, ex, fh) = section_area(
                &zl,
                &offs,
                &sidewhich,
                hb_st[i],
                grade,
                wl_i,
                depth,
                &mdl,
                &wet,
                wet_slope,
                dx_m,
                daylight_run_m,
            )?;
            let bench = cut_model_a_bench_gate(
                &offs,
                &zl,
                hb_st[i],
                wl_i,
                depth,
                Some(&sidewhich),
                wet_slope,
            );
            let face = cut_model_a_section_face(
                &offs,
                &zl,
                hb_st[i],
                grade,
                wl_i,
                depth,
                wet_slope,
                &mdl,
                daylight_run_m,
            );
            out.stations.push(CrossSection {
                chainage_m: s,
                x: xy[i][0],
                y: xy[i][1],
                heading_rad: th[i],
                offsets: offs.clone(),
                terrain: z.to_vec(),
                wet: w,
                hb: hb_st[i],
                wl: wl_i,
                grade,
                depth,
                model: mdl.clone(),
                cutface: face.iter().map(|f| f.unwrap_or(f64::NAN)).collect(),
                dredge_area_m2: dr,
                excav_area_m2: ex,
                max_face_h_m: fh,
                bench_left: bench.minus,
                bench_right: bench.plus,
                tag: tag.map_or(String::new(), |t| t(s)),
            });
        }
        a += chunk;
    }
    Ok(out)
}

/// Defaults of the Python keyword arguments, for callers that take them all.
pub const DEFAULT_CHUNK: usize = 40;
pub const DEFAULT_STATION_M: f64 = STATION_M;
pub const DEFAULT_MAXW_M: f64 = MAXW_M;
pub const DEFAULT_DX_M: f64 = DX_M;
pub const DEFAULT_WET_SLOPE: f64 = WET_SLOPE_H_PER_V;

/// Wet cross-section area of a trapezoidal channel (m2): (bottom + top) / 2 x depth.
pub fn trapezoid_area(hb: f64, depth: f64, side_slope: f64) -> f64 {
    let top = 2.0 * hb + 2.0 * side_slope * depth;
    0.5 * (2.0 * hb + top) * depth
}
