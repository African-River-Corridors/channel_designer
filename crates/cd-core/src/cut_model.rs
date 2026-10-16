//! Cross-section cut models A and B: the above-waterline cut rule. Mirrors
//! `volumes/cut_model.py` of the frozen Python. Pure maths, no DEM, no I/O.
//!
//! Terrain heights are `f64`; a missing height (Python `None`) is NaN, and every rule treats NaN
//! as "no ground here", as the Python does with `z is None or not np.isfinite(z)`.

use crate::pyrepr::repr_str;
use crate::{py_max, py_min};

pub const STATION_M: f64 = 25.0;
pub const MAXW_M: f64 = 200.0;
pub const DX_M: f64 = 1.0;

pub const CUT_MODEL: &str = "cut_model_a";
pub const CUT_MODEL_B: &str = "cut_model_b";

pub const WET_SLOPE_H_PER_V: f64 = 3.0;
pub const PRISM_CLEAR_M: f64 = 5.0;
pub const PRISM_INTRUDE_M: f64 = 2.0;
pub const BERM_M: f64 = 2.5;
pub const ABOVE_SLOPE_H_PER_V: f64 = 1.5;
pub const DAYLIGHT_RUN_M: f64 = 15.0;

/// No bend widening beyond this radius (treated as straight).
pub const WIDEN_RMAX: f64 = 2000.0;

/// One value per bank. `plus` is the side with offset >= 0 (Python key `1`), `minus` the other
/// (key `-1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerSide<T> {
    pub plus: T,
    pub minus: T,
}

impl<T: Copy> PerSide<T> {
    /// `side` is `1` or `-1`.
    pub fn get(&self, side: i8) -> T {
        if side < 0 { self.minus } else { self.plus }
    }
}

/// Canonical model id. Accepts "a", "b" and the long ids (any case, outer spaces ignored).
/// Anything else comes back lower-cased and trimmed, so the caller can refuse it.
pub fn normalise_model(name: &str) -> String {
    let n = name.trim().to_lowercase();
    match n.as_str() {
        "cut_model_a" | "a" => CUT_MODEL.into(),
        "cut_model_b" | "b" => CUT_MODEL_B.into(),
        _ => n,
    }
}

/// Per-point bend radius (m) via the 3-point Menger circumradius, then a rolling mean of the
/// finite values within +-2 points. Straight gives infinity.
pub fn local_radii(xs: &[f64], ys: &[f64], stride: usize) -> Vec<f64> {
    let n = xs.len();
    let mut r = vec![f64::INFINITY; n];
    for i in 0..n {
        let a = i.saturating_sub(stride);
        let c = (n - 1).min(i + stride);
        if c < a + 2 {
            continue;
        }
        let (ax, ay) = (xs[a], ys[a]);
        let (bx, by) = (xs[i], ys[i]);
        let (cx, cy) = (xs[c], ys[c]);
        let area = ((bx - ax) * (cy - ay) - (cx - ax) * (by - ay)).abs() * 0.5;
        if area < 1e-6 {
            continue;
        }
        let ab = libm::hypot(bx - ax, by - ay);
        let bc = libm::hypot(cx - bx, cy - by);
        let ca = libm::hypot(ax - cx, ay - cy);
        r[i] = ab * bc * ca / (4.0 * area);
    }
    let mut out = r.clone();
    for (i, slot) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(2);
        let hi = n.min(i + 3);
        let mut sum = 0.0;
        let mut count = 0usize;
        for &v in &r[lo..hi] {
            if v.is_finite() {
                sum += v;
                count += 1;
            }
        }
        if count > 0 {
            *slot = sum / count as f64;
        }
    }
    out
}

/// Per-station channel bottom half-width including bend widening.
///
/// `base = width_mult * beam / 2`; `dF_C = c_c * LOA^2 / max(R, R_min)`, capped at `LOA`; half of
/// `dF_C` is added per side.
pub fn widened_hb(
    xy: &[[f64; 2]],
    loa: f64,
    beam: f64,
    radius_multiple: f64,
    width_mult: f64,
    c_c: f64,
    rmax: f64,
) -> Vec<f64> {
    let base = width_mult * beam / 2.0;
    let rmin = radius_multiple * loa;
    let xs: Vec<f64> = xy.iter().map(|p| p[0]).collect();
    let ys: Vec<f64> = xy.iter().map(|p| p[1]).collect();
    local_radii(&xs, &ys, 2)
        .into_iter()
        .map(|ri| {
            if ri.is_finite() && 0.0 < ri && ri < rmax {
                let dfc = py_min(c_c * loa * loa / py_max(ri, rmin), loa);
                base + 0.5 * dfc
            } else {
                base
            }
        })
        .collect()
}

/// Design cut-face elevation at offset `o` (>= 0, from the centreline). Infinity means no cut.
pub fn cut_model_a_face(
    o: f64,
    side_bench: bool,
    hb: f64,
    grade: f64,
    wl: f64,
    depth: f64,
    wet_slope: f64,
) -> f64 {
    if o <= hb {
        return grade;
    }
    let r = o - hb;
    let r_wl = wet_slope * depth;
    if r <= r_wl {
        return grade + r / wet_slope;
    }
    if !side_bench {
        return f64::INFINITY;
    }
    let r2 = r - r_wl;
    if r2 <= BERM_M {
        return wl;
    }
    wl + (r2 - BERM_M) / ABOVE_SLOPE_H_PER_V
}

/// The side of each point: -1 for a negative offset, else 1.
fn default_sides(offsets: &[f64]) -> Vec<i8> {
    offsets
        .iter()
        .map(|&o| if o < 0.0 { -1 } else { 1 })
        .collect()
}

/// Per-side clearance-prism gate for one transverse profile: does natural ground intrude the
/// channel-top prism by more than `PRISM_INTRUDE_M`?
pub fn cut_model_a_bench_gate(
    offsets: &[f64],
    terrain: &[f64],
    hb: f64,
    wl: f64,
    depth: f64,
    sidewhich: Option<&[i8]>,
    wet_slope: f64,
) -> PerSide<bool> {
    let o_wl = hb + wet_slope * depth;
    let default;
    let sidewhich = match sidewhich {
        Some(s) => s,
        None => {
            default = default_sides(offsets);
            &default
        }
    };
    let mut bench = [false, false];
    for (slot, s) in [(0usize, 1i8), (1, -1)] {
        let mut min_o: Option<f64> = None;
        for k in 0..offsets.len() {
            if sidewhich[k] != s {
                continue;
            }
            let z = terrain[k];
            if !z.is_finite() {
                continue;
            }
            let o = offsets[k].abs();
            if o <= o_wl && z > wl && min_o.is_none_or(|m| o < m) {
                min_o = Some(o);
            }
        }
        let intrusion = min_o.map_or(0.0, |m| o_wl - m);
        bench[slot] = intrusion > PRISM_INTRUDE_M;
    }
    PerSide {
        plus: bench[0],
        minus: bench[1],
    }
}

/// Cut model B only: per side, the offset at which the works stop (infinity if they never do).
#[allow(clippy::too_many_arguments)]
pub fn cut_model_b_daylight_limit(
    offsets: &[f64],
    terrain: &[f64],
    hb: f64,
    grade: f64,
    wl: f64,
    depth: f64,
    bench: PerSide<bool>,
    sidewhich: Option<&[i8]>,
    wet_slope: f64,
    run_m: Option<f64>,
) -> PerSide<f64> {
    let default;
    let sidewhich = match sidewhich {
        Some(s) => s,
        None => {
            default = default_sides(offsets);
            &default
        }
    };
    let run_m = run_m.unwrap_or(DAYLIGHT_RUN_M);
    let o_wl = hb + wet_slope * depth;
    let mut limits = [f64::INFINITY, f64::INFINITY];
    for (slot, s) in [(0usize, 1i8), (1, -1)] {
        let mut idx: Vec<usize> = (0..offsets.len()).filter(|&k| sidewhich[k] == s).collect();
        idx.sort_by(|&a, &b| {
            offsets[a]
                .abs()
                .partial_cmp(&offsets[b].abs())
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let mut run_start: Option<f64> = None;
        for k in idx {
            let o = offsets[k].abs();
            if o <= o_wl {
                continue;
            }
            let z = terrain[k];
            let f = cut_model_a_face(o, bench.get(s), hb, grade, wl, depth, wet_slope);
            let daylit = !z.is_finite() || !f.is_finite() || z <= f;
            if !daylit {
                run_start = None;
                continue;
            }
            let start = *run_start.get_or_insert(o);
            if o - start >= run_m {
                limits[slot] = start;
                break;
            }
        }
    }
    PerSide {
        plus: limits[0],
        minus: limits[1],
    }
}

/// (dredge area, excavation area, tallest face above the waterline) for one transverse profile.
///
/// `wet[k]` is true inside the present river (dredge), else dry (excavation).
#[allow(clippy::too_many_arguments)]
pub fn section_area(
    zs: &[f64],
    xs: &[f64],
    sidewhich: &[i8],
    hb: f64,
    grade: f64,
    wl: f64,
    depth: f64,
    model: &str,
    wet: &[bool],
    wet_slope: f64,
    dx: f64,
    run_m: Option<f64>,
) -> Result<(f64, f64, f64), String> {
    let mdl = normalise_model(model);
    if mdl != CUT_MODEL && mdl != CUT_MODEL_B {
        return Err(format!(
            "unknown cut model {}. Known: {} (A) and {} (B).",
            repr_str(model),
            repr_str(CUT_MODEL),
            repr_str(CUT_MODEL_B)
        ));
    }
    let bench = cut_model_a_bench_gate(xs, zs, hb, wl, depth, Some(sidewhich), wet_slope);
    let limits = (mdl == CUT_MODEL_B).then(|| {
        cut_model_b_daylight_limit(
            xs,
            zs,
            hb,
            grade,
            wl,
            depth,
            bench,
            Some(sidewhich),
            wet_slope,
            run_m,
        )
    });
    let (mut dredge, mut excav, mut max_face_h) = (0.0, 0.0, 0.0);
    for k in 0..xs.len() {
        let z = zs[k];
        if !z.is_finite() {
            continue;
        }
        let o = xs[k].abs();
        let s = sidewhich[k];
        if let Some(l) = &limits
            && o > l.get(s)
        {
            continue;
        }
        let f = cut_model_a_face(o, bench.get(s), hb, grade, wl, depth, wet_slope);
        if !f.is_finite() {
            continue;
        }
        let cut = z - f;
        if cut <= 0.0 {
            continue;
        }
        let a = cut * dx;
        if wet[k] {
            dredge += a;
        } else {
            excav += a;
        }
        if z > wl && (z - wl) > max_face_h {
            max_face_h = z - wl;
        }
    }
    Ok((dredge, excav, max_face_h))
}

/// Per-station bottom half-width from a [`ChannelShape`](crate::rules::rulesets::ChannelShape).
pub fn widened_hb_from_shape(
    xy: &[[f64; 2]],
    shape: &crate::rules::rulesets::ChannelShape,
    rmax: f64,
) -> Vec<f64> {
    let hb = shape.bottom_width_straight_m / 2.0;
    let xs: Vec<f64> = xy.iter().map(|p| p[0]).collect();
    let ys: Vec<f64> = xy.iter().map(|p| p[1]).collect();
    let rmin = shape.min_bend_radius_m;
    local_radii(&xs, &ys, 2)
        .into_iter()
        .map(|ri| {
            if ri.is_finite() && 0.0 < ri && ri < rmax {
                hb + 0.5 * shape.bend_widening_m(py_max(ri, rmin))
            } else {
                hb
            }
        })
        .collect()
}

/// Per-offset cut-face elevation for ONE transverse profile (for drawings). `None` where there is
/// no cut face: no cut above the water level, or beyond the Model B limit. The model matters: a
/// drawing that hatches a wedge the volume model no longer takes is the drift this guards.
#[allow(clippy::too_many_arguments)]
pub fn cut_model_a_section_face(
    offsets: &[f64],
    terrain: &[f64],
    hb: f64,
    grade: f64,
    wl: f64,
    depth: f64,
    wet_slope: f64,
    model: &str,
    run_m: Option<f64>,
) -> Vec<Option<f64>> {
    let bench = cut_model_a_bench_gate(offsets, terrain, hb, wl, depth, None, wet_slope);
    let limits = (normalise_model(model) == CUT_MODEL_B).then(|| {
        cut_model_b_daylight_limit(
            offsets, terrain, hb, grade, wl, depth, bench, None, wet_slope, run_m,
        )
    });
    offsets
        .iter()
        .map(|&off| {
            let s: i8 = if off < 0.0 { -1 } else { 1 };
            if let Some(l) = &limits
                && off.abs() > l.get(s)
            {
                return None;
            }
            let f = cut_model_a_face(off.abs(), bench.get(s), hb, grade, wl, depth, wet_slope);
            if f.is_finite() {
                Some(ovcd_core::pyfmt::round_to(f, 2))
            } else {
                None
            }
        })
        .collect()
}
