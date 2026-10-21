//! A synthetic meandering river for examples and tests: generated in code, no data files. Mirrors
//! `synthetic.py`.
//!
//! `synthetic_river()` gives a sine-generated meandering centreline in a local metric frame, its
//! two bank lines, the wetted polygon, and an analytic terrain `terrain(x, y) -> z` with a
//! floodplain that rises gently upstream, a parabolic channel bed inside the banks and an optional
//! hill beside the river. The numbers are illustrative, not from any real river.

use crate::geom::offset::{self, Cap};
use crate::geom::{GeomError, Geometry, Polygon, Pt, linear};

pub mod fixtures;
use core::f64::consts::PI;
pub use fixtures::{
    WaterFixture, fixed_tail, narrowing_reach, open_basin_90, open_basin_max_radius, straight_canal,
};

/// The parameters of `synthetic_river`, with the Python defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct RiverParams {
    pub length_m: f64,
    pub wavelength_m: f64,
    pub amplitude_m: f64,
    pub width_m: f64,
    pub bed_depth_m: f64,
    pub floodplain_above_wl_m: f64,
    pub bed_slope: f64,
    pub water_level_at_mouth_m: f64,
    pub hill_height_m: f64,
    pub hill_at: Pt,
    pub hill_radius_m: f64,
}

impl Default for RiverParams {
    fn default() -> Self {
        RiverParams {
            length_m: 6000.0,
            wavelength_m: 1500.0,
            amplitude_m: 260.0,
            width_m: 40.0,
            bed_depth_m: 2.0,
            floodplain_above_wl_m: 3.0,
            bed_slope: 2.0e-4,
            water_level_at_mouth_m: 0.0,
            hill_height_m: 15.0,
            hill_at: [1875.0, 150.0],
            hill_radius_m: 150.0,
        }
    }
}

/// The synthetic river (Python `SyntheticRiver`).
#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticRiver {
    pub centreline: Vec<Pt>,
    pub left_bank: Geometry,
    pub right_bank: Geometry,
    pub polygon: Polygon,
    pub params: RiverParams,
}

/// numpy `np.arange(0.0, stop, step)`: `ceil(stop / step)` values `i * step`.
fn arange(stop: f64, step: f64) -> Vec<f64> {
    let n = (stop / step).ceil();
    let n = if n > 0.0 { n as usize } else { 0 };
    (0..n).map(|i| i as f64 * step).collect()
}

/// `sine_generated_centreline`: y = A·sin(2πx/λ) from x = 0 to `length_m`, every `step_m`.
pub fn sine_generated_centreline(
    length_m: f64,
    wavelength_m: f64,
    amplitude_m: f64,
    step_m: f64,
) -> Vec<Pt> {
    arange(length_m + step_m, step_m)
        .into_iter()
        .map(|x| [x, amplitude_m * libm::sin(2.0 * PI * x / wavelength_m)])
        .collect()
}

/// `synthetic_river(**params)`.
pub fn synthetic_river(params: RiverParams) -> Result<SyntheticRiver, GeomError> {
    let line = sine_generated_centreline(
        params.length_m,
        params.wavelength_m,
        params.amplitude_m,
        10.0,
    );
    let half = params.width_m / 2.0;
    // shapely 2's defaults: quad_segs=16, round joins.
    let left_bank = offset::offset_curve(&line, half, 16)?;
    let right_bank = offset::offset_curve(&line, -half, 16)?;
    let polygon = match offset::buffer_line(&line, half, 16, Cap::Flat)? {
        Geometry::Polygon(p) => p,
        other => {
            return Err(GeomError::Unsupported(format!(
                "river polygon came out as {other:?}"
            )));
        }
    };
    Ok(SyntheticRiver {
        centreline: line,
        left_bank,
        right_bank,
        polygon,
        params,
    })
}

impl SyntheticRiver {
    /// Natural low water level at chainage `s` along the centreline (m).
    pub fn water_level(&self, s: f64) -> f64 {
        let x = linear::interpolate(&self.centreline, s).map_or(f64::NAN, |p| p[0]);
        self.params.water_level_at_mouth_m + self.params.bed_slope * x
    }

    /// The terrain height at one point.
    pub fn terrain(&self, x: f64, y: f64) -> f64 {
        let p = &self.params;
        let half = p.width_m / 2.0;
        let d = linear::point_line_distance([x, y], &self.centreline);
        let wl = p.water_level_at_mouth_m + p.bed_slope * x;
        let flood = wl + p.floodplain_above_wl_m;
        let inside = d < half;
        let r = d / half;
        let bed = wl - p.bed_depth_m * (1.0 - r * r);
        let z = if inside {
            bed
        } else {
            flood + 0.002 * (d - half).max(0.0)
        };
        let (dx, dy) = (x - p.hill_at[0], y - p.hill_at[1]);
        let r2 = dx * dx + dy * dy;
        let hill = p.hill_height_m * libm::exp(-r2 / (2.0 * (p.hill_radius_m * p.hill_radius_m)));
        z + hill * if inside { 0.0 } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn arange_matches_numpy_for_the_default_river() {
        let x = arange(6010.0, 10.0);
        assert_eq!(x.len(), 601);
        assert_eq!((x[0], x[600]), (0.0, 6000.0));
    }

    #[test]
    fn the_default_river_has_banks_either_side() {
        let r = synthetic_river(RiverParams::default()).unwrap();
        assert_eq!(r.centreline.len(), 601);
        assert!(matches!(r.left_bank, Geometry::LineString(_)));
        assert!(r.terrain(1000.0, 1000.0) > r.terrain(0.0, 0.0));
    }
}
