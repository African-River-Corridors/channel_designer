//! Meander analysis: bends tighter than a radius threshold along a centreline. Mirrors
//! `alignment/meanders.py`.
//!
//! Resample, three-point circumradius and signed curvature, runs of one curvature sign at least
//! `min_points` long whose least radius is within the threshold and whose deflection is at least
//! `min_deflection_deg`, then one record per meander.

use super::limits::circumcircle;
use crate::geom::Pt;
use crate::numpy::{chainages, py_mod, sign, unwrap};

/// `resample_polyline` (the meanders copy, with its own messages).
pub fn resample_polyline(points: &[Pt], step: f64) -> Result<Vec<Pt>, String> {
    if points.len() < 2 {
        return Err("Centreline must contain at least two vertices.".into());
    }
    if step <= 0.0 {
        return Err("Sample step must be positive.".into());
    }
    let cum = chainages(points);
    let total = cum[cum.len() - 1];
    if total <= 0.0 {
        return Err("Centreline has zero length, cannot resample.".into());
    }
    Ok(super::centreline::resample_at(points, &cum, total, step))
}

/// Curvature, radius and signed curvature (positive for a left turn) at each point.
#[derive(Debug, Clone, PartialEq)]
pub struct Curvature {
    pub curvature: Vec<f64>,
    pub radius: Vec<f64>,
    pub signed: Vec<f64>,
}

/// `curvature_radius`.
pub fn curvature_radius(points: &[Pt]) -> Result<Curvature, String> {
    let n = points.len();
    if n < 3 {
        return Err("Need at least three points to compute curvature.".into());
    }
    let mut curv = vec![0.0; n];
    let mut rad = vec![f64::INFINITY; n];
    let mut signed = vec![0.0; n];
    for i in 1..n - 1 {
        let (p0, p1, p2) = (points[i - 1], points[i], points[i + 1]);
        let Some((_, r)) = circumcircle(p0, p1, p2) else {
            continue;
        };
        if r < 1e-9 {
            rad[i] = 0.0;
            continue;
        }
        curv[i] = 1.0 / r;
        rad[i] = r;
        let cross = (p1[0] - p0[0]) * (p2[1] - p1[1]) - (p1[1] - p0[1]) * (p2[0] - p1[0]);
        if cross > 1e-9 {
            signed[i] = curv[i];
        } else if cross < -1e-9 {
            signed[i] = -curv[i];
        }
    }
    for v in [&mut curv, &mut rad, &mut signed] {
        v[0] = v[1];
        v[n - 1] = v[n - 2];
    }
    Ok(Curvature {
        curvature: curv,
        radius: rad,
        signed,
    })
}

/// `compute_heading`: the unwrapped heading of each segment.
pub fn compute_heading(points: &[Pt]) -> Vec<f64> {
    let h: Vec<f64> = points
        .windows(2)
        .map(|w| libm::atan2(w[1][1] - w[0][1], w[1][0] - w[0][0]))
        .collect();
    unwrap(&h)
}

/// `detect_curvature_runs`: index ranges `[start, end)` of one curvature sign, at least
/// `min_points` long (|signed| < 1e-6 counts as no sign).
pub fn detect_curvature_runs(signed: &[f64], min_points: i64) -> Vec<(usize, usize)> {
    let signs: Vec<f64> = signed
        .iter()
        .map(|&k| if k.abs() < 1e-6 { 0.0 } else { sign(k) })
        .collect();
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    let mut current = 0.0;
    let long = |a: usize, b: usize| (b - a) as i64 >= min_points;
    for (i, &s) in signs.iter().enumerate() {
        if s == 0.0 {
            if let Some(a) = start
                && long(a, i)
            {
                runs.push((a, i));
            }
            start = None;
            current = 0.0;
            continue;
        }
        if current == 0.0 {
            start = Some(i);
            current = s;
            continue;
        }
        if s != current {
            if let Some(a) = start
                && long(a, i)
            {
                runs.push((a, i));
            }
            start = Some(i);
            current = s;
        }
    }
    if let Some(a) = start
        && long(a, signed.len())
    {
        runs.push((a, signed.len()));
    }
    runs
}

/// `find_meander_runs`: the runs whose least radius is at or below `threshold`.
pub fn find_meander_runs(
    radius: &[f64],
    signed: &[f64],
    threshold: f64,
    min_points: i64,
) -> Vec<(usize, usize)> {
    detect_curvature_runs(signed, min_points)
        .into_iter()
        .filter(|&(a, b)| {
            let r = &radius[a..b];
            r.iter().any(|x| x.is_finite()) && nanmin(r) <= threshold
        })
        .collect()
}

/// `np.nanmin`: the least value, NaNs ignored.
fn nanmin(v: &[f64]) -> f64 {
    v.iter()
        .copied()
        .filter(|x| !x.is_nan())
        .fold(f64::NAN, |m, x| if m.is_nan() || x < m { x } else { m })
}

/// `np.nanargmin`: the index of the first least value, NaNs ignored.
fn nanargmin(v: &[f64]) -> usize {
    let mut best: Option<usize> = None;
    for (i, &x) in v.iter().enumerate() {
        if !x.is_nan() && best.is_none_or(|b| x < v[b]) {
            best = Some(i);
        }
    }
    best.unwrap_or(0)
}

/// `tangent_angle`: the mean of the headings either side of point `index`.
pub fn tangent_angle(heading: &[f64], index: i64) -> f64 {
    if heading.is_empty() {
        return 0.0;
    }
    if index <= 0 {
        return heading[0];
    }
    if index as usize >= heading.len() {
        return heading[heading.len() - 1];
    }
    let i = index as usize;
    0.5 * (heading[i - 1] + heading[i])
}

/// One meander record (`build_meander_records`).
#[derive(Debug, Clone, PartialEq)]
pub struct MeanderRecord {
    pub apex_index: usize,
    pub apex_distance_m: f64,
    pub apex_location: Pt,
    pub radius: f64,
    pub deflection_deg: f64,
    pub tangent_angle_deg: f64,
    pub left: bool,
    pub circle_center: Pt,
    pub id_number: usize,
}

/// Python `math.degrees`.
fn degrees(r: f64) -> f64 {
    r * (180.0 / core::f64::consts::PI)
}

/// `find_meanders`: meanders tighter than `max_radius` (m), sorted by apex chainage.
pub fn find_meanders(
    centreline: &[Pt],
    max_radius: f64,
    sample_step: f64,
    min_points: i64,
    min_deflection_deg: f64,
) -> Result<Vec<MeanderRecord>, String> {
    let pts = resample_polyline(centreline, sample_step)?;
    let Curvature { radius, signed, .. } = curvature_radius(&pts)?;
    let heading = compute_heading(&pts);
    let runs = find_meander_runs(&radius, &signed, max_radius, min_points);
    let mut found = Vec::new();
    // `summarise_meanders`
    for (a, b) in runs {
        let local = &radius[a..b];
        if !local.iter().any(|x| x.is_finite()) {
            continue;
        }
        let (s, e) = (a, b - 1);
        let hs = s.saturating_sub(1);
        let he = e.min(heading.len() - 1);
        let turn = degrees(heading[he] - heading[hs]).abs();
        if turn < min_deflection_deg {
            continue;
        }
        let apex = a + nanargmin(local);
        if apex == 0 || apex >= pts.len() - 1 {
            continue;
        }
        let Some((centre, r)) = circumcircle(pts[apex - 1], pts[apex], pts[apex + 1]) else {
            continue;
        };
        found.push((apex, r, centre, sign(signed[apex]), turn));
    }
    let ch = chainages(&pts);
    let mut records: Vec<MeanderRecord> = found
        .into_iter()
        .filter(|&(_, r, ..)| r.is_finite() && r <= max_radius)
        .map(|(apex, r, centre, orient, turn)| MeanderRecord {
            apex_index: apex,
            apex_distance_m: ch[apex],
            apex_location: pts[apex],
            radius: r,
            deflection_deg: turn,
            tangent_angle_deg: py_mod(degrees(tangent_angle(&heading, apex as i64)) + 360.0, 360.0),
            left: orient >= 0.0,
            circle_center: centre,
            id_number: 0,
        })
        .collect();
    records.sort_by(|x, y| x.apex_distance_m.total_cmp(&y.apex_distance_m));
    for (i, r) in records.iter_mut().enumerate() {
        r.id_number = i + 1;
    }
    Ok(records)
}
