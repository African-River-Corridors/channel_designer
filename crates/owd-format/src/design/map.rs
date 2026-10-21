//! The plan map's geometry (spec 0003): the inputs and the design line options in WGS84, with each
//! option's tight bends and chainage ticks. The app draws what this returns and computes nothing.

use super::crs::Crs;
use super::geojson::all_positions;
use super::session::{DesignError, Session};
use owd_core::alignment::limits::compute_chainages;
use owd_core::alignment::radius_check::{MAX_STRIDE_M, RADIUS_TOL, default_check_chord};
use owd_core::geom::Pt;
use serde_json::{Value, json};

type Convert<'a> = &'a dyn Fn(&[Pt]) -> Result<Vec<Pt>, String>;

fn line(c: Convert, pts: &[Pt]) -> Result<Value, String> {
    Ok(json!({"type": "LineString", "coordinates": c(pts)?}))
}

/// The point `s` metres along `pts` (`chain` holds the chainage of each vertex).
fn point_at(pts: &[Pt], chain: &[f64], s: f64) -> Pt {
    let i = chain.partition_point(|&c| c <= s).clamp(1, pts.len() - 1);
    let (c0, c1) = (chain[i - 1], chain[i]);
    let t = if c1 > c0 { (s - c0) / (c1 - c0) } else { 0.0 };
    [
        pts[i - 1][0] + t * (pts[i][0] - pts[i - 1][0]),
        pts[i - 1][1] + t * (pts[i][1] - pts[i - 1][1]),
    ]
}

fn resample(pts: &[Pt], chain: &[f64], h: f64) -> Vec<(f64, Pt)> {
    let total = *chain.last().unwrap_or(&0.0);
    let n = (total / h).floor() as usize;
    (0..=n)
        .map(|i| (i as f64 * h, point_at(pts, chain, i as f64 * h)))
        .collect()
}

/// A tick spacing of 1, 2 or 5 times a power of ten that gives roughly ten ticks.
fn tick_spacing(total: f64) -> f64 {
    let raw = (total / 10.0).max(1.0);
    let mag = libm::pow(10.0, libm::floor(libm::log10(raw)));
    [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag)
}

fn label(s: f64) -> String {
    format!(
        "{}+{:03}",
        (s / 1000.0).floor() as i64,
        (s % 1000.0).round() as i64
    )
}

/// Where the line's radius, read the way the radius check reads it (a chord of the option's own
/// length, three points), is below the allowed radius: `(chainage, radius, position)`.
fn tight_bends(pts: &[Pt], chain: &[f64], r_min: f64, chord: f64) -> Vec<(f64, f64, Pt)> {
    let half = chord / 2.0;
    let k = ((half / MAX_STRIDE_M).ceil() as usize).max(1);
    let h = half / k as f64;
    let s = resample(pts, chain, h);
    let limit = (1.0 - RADIUS_TOL) * r_min;
    let mut out: Vec<(f64, f64, Pt)> = Vec::new();
    let mut run: Option<(f64, f64, Pt)> = None;
    let flush = |run: &mut Option<(f64, f64, Pt)>, out: &mut Vec<(f64, f64, Pt)>| {
        if let Some(r) = run.take() {
            out.push(r);
        }
    };
    for i in k..s.len().saturating_sub(k) {
        let (a, b, c) = (s[i - k].1, s[i].1, s[i + k].1);
        let (ab, bc, ca) = (
            libm::hypot(b[0] - a[0], b[1] - a[1]),
            libm::hypot(c[0] - b[0], c[1] - b[1]),
            libm::hypot(a[0] - c[0], a[1] - c[1]),
        );
        let area = 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
        let r = if area > 1e-9 {
            ab * bc * ca / (4.0 * area)
        } else {
            f64::INFINITY
        };
        if r < limit {
            // Keep the tightest point of each run of tight points.
            match run {
                Some((_, rr, _)) if rr <= r => {}
                _ => run = Some((s[i].0, r, b)),
            }
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out
}

impl Session {
    /// The plan map's geometry in WGS84: the banks, the river centreline and polygon where there
    /// are any, and each design line option with its tight bends and chainage ticks. `bbox` is
    /// `[west, south, east, north]`.
    pub fn map_geometry(&self) -> Result<Value, DesignError> {
        let epsg = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .ok_or_else(|| DesignError::Input("the design has no project CRS yet".into()))?;
        let crs = Crs::from_epsg(epsg as u32).map_err(DesignError::Input)?;
        let convert = move |pts: &[Pt]| -> Result<Vec<Pt>, String> {
            match crs {
                Crs::Wgs84 => Ok(pts.to_vec()),
                Crs::Utm(u) => u.inverse_all(pts),
            }
        };
        let c: Convert = &convert;
        let err = DesignError::Input;
        let inp = &self.doc["inputs"];
        let mut bbox: Option<[f64; 4]> = None;
        let mut grow = |pts: &[Pt]| {
            for p in pts {
                let b = bbox.get_or_insert([p[0], p[1], p[0], p[1]]);
                *b = [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ];
            }
        };
        let mut out = json!({"crs": "EPSG:4326", "banks": [], "centreline": null,
            "river_polygon": null, "options": []});
        for (key, name) in [("left_bank", "left"), ("right_bank", "right")] {
            if inp[key]["type"] == "LineString" {
                let l = line(c, &all_positions(&inp[key])).map_err(err)?;
                grow(&all_positions(&l));
                out["banks"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"side": name, "line": l}));
            }
        }
        if let Ok(centre) = self.base_line() {
            let l = line(c, &centre).map_err(DesignError::Input)?;
            grow(&all_positions(&l));
            out["centreline"] = l;
        }
        if inp["river_polygon"]["type"] == "Polygon" {
            let rings: Vec<Value> = inp["river_polygon"]["coordinates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|ring| {
                    let pts = all_positions(ring);
                    c(&pts).map(|p| json!(p))
                })
                .collect::<Result<_, _>>()
                .map_err(DesignError::Input)?;
            out["river_polygon"] = json!({"type": "Polygon", "coordinates": rings});
        }
        for o in self.doc["options"].as_array().into_iter().flatten() {
            let pts = all_positions(&o["line"]);
            if pts.len() < 2 {
                continue;
            }
            let chain = compute_chainages(&pts);
            let total = *chain.last().unwrap();
            let r_min = o["checks"]["r_min_m"].as_f64().unwrap_or(f64::NAN);
            let chord = o["checks"]["radius_check_chord_m"]
                .as_f64()
                .filter(|x| x.is_finite() && *x > 0.0)
                .unwrap_or_else(|| default_check_chord(r_min));
            let bends: Vec<(f64, f64, Pt)> = if r_min.is_finite() && chord.is_finite() {
                tight_bends(&pts, &chain, r_min, chord)
            } else {
                vec![]
            };
            let step = tick_spacing(total);
            let tick_s: Vec<f64> = (0..=((total / step).floor() as usize))
                .map(|i| i as f64 * step)
                .collect();
            let pos = |ps: Vec<Pt>| c(&ps).map_err(DesignError::Input);
            let tick_pos = pos(tick_s.iter().map(|&s| point_at(&pts, &chain, s)).collect())?;
            let bend_pos = pos(bends.iter().map(|b| b.2).collect())?;
            let l = line(c, &pts).map_err(DesignError::Input)?;
            grow(&all_positions(&l));
            out["options"].as_array_mut().unwrap().push(json!({
                "id": o["id"], "name": o["name"], "method": o["method"],
                "line": l,
                "length_m": total,
                "radius_ok": o["checks"]["radius_ok"],
                "r_min_m": o["checks"]["r_min_m"],
                "measured_min_radius_m": o["checks"]["measured_min_radius_m"],
                "tight_bends": bends.iter().zip(&bend_pos).map(|(b, p)|
                    json!({"chainage_m": b.0, "radius_m": b.1, "position": p})).collect::<Vec<_>>(),
                "ticks": tick_s.iter().zip(&tick_pos).map(|(s, p)|
                    json!({"chainage_m": s, "label": label(*s), "position": p})).collect::<Vec<_>>(),
            }));
        }
        out["bbox"] = json!(bbox);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn ticks_are_one_two_or_five_times_a_power_of_ten() {
        assert_eq!(tick_spacing(1000.0), 100.0);
        assert_eq!(tick_spacing(2300.0), 500.0);
        assert_eq!(tick_spacing(9000.0), 1000.0);
        assert_eq!(tick_spacing(5.0), 1.0);
    }

    #[test]
    fn chainage_labels_read_km_plus_metres() {
        assert_eq!(label(0.0), "0+000");
        assert_eq!(label(1250.0), "1+250");
    }

    #[test]
    fn a_hairpin_is_tight_and_a_straight_is_not() {
        let straight: Vec<Pt> = (0..=100).map(|i| [i as f64 * 10.0, 0.0]).collect();
        let chain = compute_chainages(&straight);
        assert!(tight_bends(&straight, &chain, 255.0, 85.0).is_empty());
        // A circle of radius 50 m is tighter than 255 m everywhere.
        let arc: Vec<Pt> = (0..=60)
            .map(|i| {
                let a = i as f64 * 0.05;
                [50.0 * libm::sin(a), 50.0 * (1.0 - libm::cos(a))]
            })
            .collect();
        let chain = compute_chainages(&arc);
        let b = tight_bends(&arc, &chain, 255.0, 85.0);
        assert_eq!(b.len(), 1);
        assert!((b[0].1 - 50.0).abs() < 1.0, "{b:?}");
    }
}
