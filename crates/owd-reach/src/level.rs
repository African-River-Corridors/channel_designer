//! The default low-water level along the river centreline (spec 0012, T6; ADR 0006).
//!
//! 1. The SWORD reaches along the river ([`crate::sword`]), each placed on the river centreline's
//!    chainage by projecting its line.
//! 2. Each reach's SWOT water surface elevations from Hydrocron (source `swot-hydrocron`), good and
//!    suspect quality only. The level of the reach is their 10th percentile (a provisional default),
//!    set at the middle of the reach.
//! 3. Where a reach has too few observations, is narrower than 100 m (provisional), or there is no
//!    reach: the DEM water surface sampled on the centreline, made to never rise downstream (the
//!    closest such profile in least squares: pool adjacent violators). Which way is downstream comes
//!    from SWORD's distance to the outlet, else from the DEM.
//! 4. Where a DEM level sits above the level just upstream of it, across a join with a SWOT level,
//!    it is lowered to that level, and the provenance records it (#280).
//!
//! Datum: SWOT's `wse` is the height above the EGM2008 geoid that comes with each observation
//! (`geoid_hght`, the geoid's height above the WGS84 ellipsoid). The Copernicus GLO-30 DEM is in
//! EGM2008 too, so for it there is nothing to convert. For a DEM in ellipsoidal heights each
//! observation becomes `wse + geoid_hght`. The conversion used is written into the provenance.

use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::geo::LonLat;
use crate::sword::Reach;
use owd_core::geom::linear;
use owd_format::design::Session;
use owd_format::design::crs::Utm;
use owd_format::design::provenance::{Mark, record, with_settings};
use serde_json::{Value, json};

/// The Hydrocron time-series endpoint.
pub const HYDROCRON_ENDPOINT: &str =
    "https://soto.podaac.earthdatacloud.nasa.gov/hydrocron/v1/timeseries";

/// The first day of SWOT's science orbit: earlier passes were the calibration phase.
pub const SWOT_SCIENCE_START: &str = "2023-07-21";

/// Settings of the default level. Every value marked so is a provisional default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelParams {
    /// The percentile of a reach's levels taken as low water (provisional: 10).
    pub percentile: f64,
    /// A reach narrower than this takes the DEM instead (provisional: 100 m).
    pub min_swot_width_m: f64,
    /// A reach needs at least this many good observations.
    pub min_observations: usize,
    /// The worst reach quality flag kept: 0 good, 1 suspect, 2 degraded, 3 bad.
    pub max_quality: u8,
    /// A SWORD reach belongs to the river where its line is within this distance of the centreline.
    pub match_m: f64,
    /// The spacing of the DEM samples on the centreline.
    pub dem_step_m: f64,
}

impl Default for LevelParams {
    fn default() -> Self {
        LevelParams {
            percentile: 10.0,
            min_swot_width_m: 100.0,
            min_observations: 3,
            max_quality: 1,
            match_m: 300.0,
            dem_step_m: 100.0,
        }
    }
}

/// The vertical datum of the document's DEM, which the level is given in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Datum {
    /// Heights above the EGM2008 geoid, as the Copernicus GLO-30 DEM and SWOT's `wse` are.
    Egm2008,
    /// Heights above the WGS84 ellipsoid.
    Ellipsoid,
}

impl Datum {
    pub fn name(self) -> &'static str {
        match self {
            Datum::Egm2008 => "EGM2008",
            Datum::Ellipsoid => "WGS84 ellipsoid",
        }
    }

    /// A SWOT observation in this datum.
    pub fn from_swot(self, o: &Observation) -> f64 {
        match self {
            Datum::Egm2008 => o.wse_m,
            Datum::Ellipsoid => o.wse_m + o.geoid_m,
        }
    }

    /// How SWOT's levels were brought into this datum, for the provenance.
    pub fn conversion(self) -> &'static str {
        match self {
            Datum::Egm2008 => {
                "SWOT wse is the height above the EGM2008 geoid, the DEM's datum: no conversion"
            }
            Datum::Ellipsoid => {
                "SWOT wse is the height above the EGM2008 geoid; the DEM is in ellipsoidal heights, so each observation is wse + geoid_hght (the geoid's height above the WGS84 ellipsoid, given with it)"
            }
        }
    }
}

/// One SWOT observation of a reach.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// `YYYY-MM-DDThh:mm:ssZ`.
    pub time: String,
    /// Water surface elevation above the EGM2008 geoid, metres.
    pub wse_m: f64,
    /// The geoid's height above the WGS84 ellipsoid there, metres.
    pub geoid_m: f64,
    /// Reach quality: 0 good, 1 suspect, 2 degraded, 3 bad.
    pub quality: u8,
}

/// The Hydrocron request for a reach's time series between two dates (`YYYY-MM-DD`).
pub fn hydrocron_url(endpoint: &str, reach_id: &str, from: &str, to: &str) -> String {
    format!(
        "{endpoint}?feature=Reach&feature_id={reach_id}&start_time={from}T00:00:00Z&end_time={to}T23:59:59Z&output=csv&fields=reach_id,time_str,wse,wse_u,reach_q,width,geoid_hght"
    )
}

/// The observations in a Hydrocron answer (CSV inside JSON). Passes with no data and fill values are
/// left out; a reach Hydrocron does not know gives none.
pub fn parse_hydrocron(body: &[u8]) -> Result<Vec<Observation>, ReachError> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| ReachError::Parse(format!("the Hydrocron answer is not JSON: {e}")))?;
    if v["error"].as_str().is_some_and(|e| e.contains("not found")) {
        return Ok(Vec::new());
    }
    let csv = v["results"]["csv"].as_str().ok_or_else(|| {
        ReachError::Parse(format!(
            "the Hydrocron answer has no CSV results: {}",
            v["error"].as_str().unwrap_or("")
        ))
    })?;
    let mut lines = csv.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    let col = |k: &str| {
        head.iter()
            .position(|h| *h == k)
            .ok_or_else(|| ReachError::Parse(format!("the Hydrocron CSV has no `{k}` column")))
    };
    let (t, w, g, q) = (
        col("time_str")?,
        col("wse")?,
        col("geoid_hght")?,
        col("reach_q")?,
    );
    let fill = |x: f64| !x.is_finite() || x < -1e10;
    Ok(lines
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let time = f.get(t)?.to_string();
            let wse: f64 = f.get(w)?.parse().ok()?;
            let geoid: f64 = f.get(g)?.parse().ok()?;
            let quality: u8 = f.get(q)?.parse().ok()?;
            (time != "no_data" && !fill(wse) && !fill(geoid)).then_some(Observation {
                time,
                wse_m: wse,
                geoid_m: geoid,
                quality,
            })
        })
        .collect())
}

/// The `p`th percentile (0 to 100) of `values`, by linear interpolation between the closest ranks
/// (Hyndman and Fan's type 7, numpy's default): rank `p/100 * (n - 1)` from 0.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() || !(0.0..=100.0).contains(&p) {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let r = p / 100.0 * (v.len() - 1) as f64;
    let (i, f) = (r.floor() as usize, r - r.floor());
    Some(if i + 1 < v.len() {
        v[i] + f * (v[i + 1] - v[i])
    } else {
        v[i]
    })
}

/// The closest profile to `levels` (in downstream order) that never rises downstream, in least
/// squares: pool adjacent violators.
pub fn never_rising(levels: &[f64]) -> Vec<f64> {
    // Blocks of (mean, count); a block higher than the one before it merges into it.
    let mut blocks: Vec<(f64, usize)> = Vec::new();
    for &v in levels {
        blocks.push((v, 1));
        while blocks.len() > 1 && blocks[blocks.len() - 1].0 > blocks[blocks.len() - 2].0 {
            let (b, nb) = blocks.pop().expect("two blocks");
            let (a, na) = blocks.pop().expect("two blocks");
            let n = na + nb;
            blocks.push(((a * na as f64 + b * nb as f64) / n as f64, n));
        }
    }
    blocks
        .into_iter()
        .flat_map(|(m, n)| std::iter::repeat_n(m, n))
        .collect()
}

/// Where a stretch of the profile came from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A SWORD reach's SWOT observations.
    Swot {
        reach_id: String,
        observations: usize,
        first: String,
        last: String,
    },
    /// The DEM water surface.
    Dem,
}

/// A stretch of the river centreline and the source of its level.
#[derive(Debug, Clone, PartialEq)]
pub struct Stretch {
    pub from_m: f64,
    pub to_m: f64,
    pub source: Source,
    /// Why the DEM was used here, if it was.
    pub why: Option<String>,
}

/// A level lowered so the whole profile never rises downstream (#280).
#[derive(Debug, Clone, PartialEq)]
pub struct Lowered {
    /// Where on the river centreline.
    pub chainage_m: f64,
    /// The level its source gave.
    pub was_m: f64,
    /// The level it was lowered to: the level just upstream of it.
    pub now_m: f64,
}

/// The default level.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelProfile {
    /// `(river chainage, level)`, chainage not decreasing.
    pub points: Vec<[f64; 2]>,
    pub stretches: Vec<Stretch>,
    /// True when the river flows toward increasing chainage.
    pub downstream_is_increasing: bool,
    /// How the flow direction was found.
    pub direction_from: &'static str,
    pub params: LevelParams,
    pub datum: Datum,
    /// The dates of the first and last SWOT observation used (`YYYY-MM-DD`), if any.
    pub swot_dates: Option<(String, String)>,
    /// The levels lowered so the whole profile never rises downstream, across the joins of SWOT and
    /// DEM stretches.
    pub lowered: Vec<Lowered>,
}

/// A SWORD reach placed on the centreline.
struct Placed<'a> {
    reach: &'a Reach,
    from_m: f64,
    to_m: f64,
}

/// The reaches along the centreline, by chainage, with where they start and end on it.
fn place<'a>(
    reaches: &'a [Reach],
    centre: &[[f64; 2]],
    utm: Utm,
    match_m: f64,
) -> Result<Vec<Placed<'a>>, ReachError> {
    let mut out = Vec::new();
    for r in reaches {
        let xy = utm.forward_all(&r.line).map_err(ReachError::Input)?;
        let on: Vec<f64> = xy
            .iter()
            .filter_map(|p| {
                let s = linear::project(centre, *p);
                let q = linear::interpolate(centre, s)?;
                let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                (d <= match_m).then_some(s)
            })
            .collect();
        // Most of the reach must follow the river, not cross it.
        if on.len() * 2 < xy.len() {
            continue;
        }
        let (a, b) = on
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), s| (a.min(*s), b.max(*s)));
        if b - a > 1.0 {
            out.push(Placed {
                reach: r,
                from_m: a,
                to_m: b,
            });
        }
    }
    out.sort_by(|a, b| a.from_m.total_cmp(&b.from_m));
    Ok(out)
}

/// Lower each DEM level that sits above the level just upstream of it, so the profile never rises
/// downstream across the joins of SWOT and DEM stretches as well as within them (#280). A DEM
/// stretch is made never rising on its own, so this only bites where it meets the level upstream of
/// it. SWOT levels are kept as observed. `points` are by chainage; `dem` holds the chainages of the
/// DEM levels.
fn clamp_dem_joins(
    points: &mut [(f64, f64)],
    dem: &[f64],
    downstream_is_increasing: bool,
) -> Vec<Lowered> {
    let mut order: Vec<usize> = (0..points.len()).collect();
    if !downstream_is_increasing {
        order.reverse();
    }
    let mut lowered = Vec::new();
    let mut upstream = f64::INFINITY;
    for i in order {
        let (s, v) = points[i];
        let is_dem = dem.contains(&s);
        if is_dem && v > upstream {
            points[i].1 = upstream;
            lowered.push(Lowered {
                chainage_m: s,
                was_m: v,
                now_m: upstream,
            });
        }
        upstream = points[i].1;
    }
    lowered.sort_by(|a, b| a.chainage_m.total_cmp(&b.chainage_m));
    lowered
}

/// What the level needs from outside: the observations of a reach, and the DEM water surface.
pub struct LevelInputs<'a> {
    /// Hydrocron, by reach id: the observations between the dates asked for.
    pub observations: &'a dyn Fn(&str) -> Result<Vec<Observation>, ReachError>,
    /// The DEM height at a point (longitude, latitude), in the document's vertical datum; NaN where
    /// there is none. `None` when the document has no DEM.
    pub dem: Option<&'a dyn Fn(LonLat) -> f64>,
}

/// The default level along `centre` (the river centreline in `utm`), from the SWORD `reaches` near it.
pub fn default_level(
    centre: &[[f64; 2]],
    utm: Utm,
    reaches: &[Reach],
    inputs: &LevelInputs,
    params: &LevelParams,
    datum: Datum,
) -> Result<LevelProfile, ReachError> {
    let len = linear::length(centre);
    if centre.len() < 2 || len <= 0.0 {
        return Err(ReachError::Input(
            "the river centreline is too short".into(),
        ));
    }
    let placed = place(reaches, centre, utm, params.match_m)?;

    // Which way the river flows: SWORD's distance to the outlet falls downstream.
    let mid = |p: &Placed| (p.from_m + p.to_m) / 2.0;
    let known: Vec<(f64, f64)> = placed
        .iter()
        .filter(|p| p.reach.dist_out_m.is_finite())
        .map(|p| (mid(p), p.reach.dist_out_m))
        .collect();
    let dem_at = |s: f64| -> f64 {
        let Some(dem) = inputs.dem else {
            return f64::NAN;
        };
        let Some(p) = linear::interpolate(centre, s) else {
            return f64::NAN;
        };
        utm.inverse(p[0], p[1]).map_or(f64::NAN, dem)
    };
    let (downstream_is_increasing, direction_from) = if known.len() >= 2 {
        let (first, last) = (known[0], known[known.len() - 1]);
        (last.1 < first.1, "SWORD distance to the outlet")
    } else {
        let ends = |a: f64, b: f64| {
            let v: Vec<f64> = (0..=10)
                .map(|k| dem_at(a + (b - a) * f64::from(k) / 10.0))
                .filter(|v| v.is_finite())
                .collect();
            (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
        };
        match (ends(0.0, len * 0.1), ends(len * 0.9, len)) {
            (Some(a), Some(b)) => (b < a, "DEM water surface"),
            _ => (true, "assumed: the route's start is upstream"),
        }
    };

    // SWOT levels, one per reach, at its middle.
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut stretches: Vec<Stretch> = Vec::new();
    let mut dates: Vec<String> = Vec::new();
    let mut covered: Vec<(f64, f64)> = Vec::new();
    let mut fallback_why: Vec<(f64, f64, String)> = Vec::new();
    let mut dem_at_points: Vec<f64> = Vec::new();
    for p in &placed {
        let obs: Vec<Observation> = (inputs.observations)(&p.reach.id)?
            .into_iter()
            .filter(|o| o.quality <= params.max_quality)
            .collect();
        let why = if p.reach.width_m < params.min_swot_width_m {
            Some(format!(
                "SWORD reach {} is {:.0} m wide, under {} m",
                p.reach.id, p.reach.width_m, params.min_swot_width_m
            ))
        } else if obs.len() < params.min_observations {
            Some(format!(
                "SWORD reach {} has {} good SWOT observation(s), under {}",
                p.reach.id,
                obs.len(),
                params.min_observations
            ))
        } else {
            None
        };
        if let Some(w) = why {
            fallback_why.push((p.from_m, p.to_m, w));
            continue;
        }
        let vals: Vec<f64> = obs.iter().map(|o| datum.from_swot(o)).collect();
        let level =
            percentile(&vals, params.percentile).expect("observations and a valid percentile");
        let mut t: Vec<String> = obs
            .iter()
            .map(|o| o.time.chars().take(10).collect())
            .collect();
        t.sort();
        dates.extend([t[0].clone(), t[t.len() - 1].clone()]);
        points.push((mid(p), level));
        covered.push((p.from_m, p.to_m));
        stretches.push(Stretch {
            from_m: p.from_m,
            to_m: p.to_m,
            source: Source::Swot {
                reach_id: p.reach.id.clone(),
                observations: obs.len(),
                first: t[0].clone(),
                last: t[t.len() - 1].clone(),
            },
            why: None,
        });
    }

    // The DEM on every stretch no SWOT level covers, never rising downstream within each stretch.
    let n = (len / params.dem_step_m).ceil().max(1.0) as usize;
    let stations: Vec<f64> = (0..=n).map(|k| len * k as f64 / n as f64).collect();
    let free: Vec<bool> = stations
        .iter()
        .map(|s| !covered.iter().any(|(a, b)| s >= a && s <= b))
        .collect();
    let mut k = 0;
    while k < stations.len() {
        if !free[k] {
            k += 1;
            continue;
        }
        let start = k;
        while k < stations.len() && free[k] {
            k += 1;
        }
        let run: Vec<(f64, f64)> = stations[start..k]
            .iter()
            .map(|s| (*s, dem_at(*s)))
            .filter(|(_, v)| v.is_finite())
            .collect();
        if run.is_empty() {
            continue;
        }
        let mut ordered = run.clone();
        if !downstream_is_increasing {
            ordered.reverse();
        }
        let levels: Vec<f64> = ordered.iter().map(|(_, v)| *v).collect();
        for ((s, _), v) in ordered.iter().zip(never_rising(&levels)) {
            points.push((*s, v));
            dem_at_points.push(*s);
        }
        let (a, b) = (stations[start], stations[k - 1]);
        let why = fallback_why
            .iter()
            .filter(|(fa, fb, _)| *fa <= b && *fb >= a)
            .map(|(_, _, w)| w.clone())
            .collect::<Vec<_>>();
        stretches.push(Stretch {
            from_m: a,
            to_m: b,
            source: Source::Dem,
            why: Some(if why.is_empty() {
                "no SWORD reach along this stretch".to_string()
            } else {
                why.join("; ")
            }),
        });
    }
    if points.is_empty() {
        return Err(ReachError::NoWater(
            "no SWOT level and no DEM along this river: give a level of your own (owd import FILE --water-level METRES)".into(),
        ));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let lowered = clamp_dem_joins(&mut points, &dem_at_points, downstream_is_increasing);
    stretches.sort_by(|a, b| a.from_m.total_cmp(&b.from_m));
    dates.sort();
    Ok(LevelProfile {
        points: points.into_iter().map(|(s, v)| [s, v]).collect(),
        stretches,
        downstream_is_increasing,
        direction_from,
        params: *params,
        datum,
        swot_dates: (!dates.is_empty()).then(|| (dates[0].clone(), dates[dates.len() - 1].clone())),
        lowered,
    })
}

/// Fetch the SWORD reaches near the centreline and their SWOT levels, then make the default level.
#[allow(clippy::too_many_arguments)]
pub fn level_from_sources(
    fetch: &dyn Fetch,
    sword_record: &str,
    hydrocron: &str,
    centre: &[[f64; 2]],
    utm: Utm,
    dates: (&str, &str),
    dem: Option<&dyn Fn(LonLat) -> f64>,
    params: &LevelParams,
    datum: Datum,
) -> Result<LevelProfile, ReachError> {
    let ll = utm.inverse_all(centre).map_err(ReachError::Input)?;
    let b = crate::geo::bbox(&ll).ok_or_else(|| ReachError::Input("an empty centreline".into()))?;
    // A margin of the match distance, in degrees at the poorer scale.
    let m = params.match_m / (111_320.0 * b[1].abs().min(80.0).to_radians().cos().max(0.2));
    let reaches = crate::sword::reaches_in(
        fetch,
        sword_record,
        [b[0] - m, b[1] - m, b[2] + m, b[3] + m],
    )?;
    let observations = |id: &str| -> Result<Vec<Observation>, ReachError> {
        parse_hydrocron(&fetch.get(&hydrocron_url(hydrocron, id, dates.0, dates.1))?)
    };
    default_level(
        centre,
        utm,
        &reaches,
        &LevelInputs {
            observations: &observations,
            dem,
        },
        params,
        datum,
    )
}

/// Record the level in a document as its water level profile (the one every stage reads), with its
/// provenance: `default` for the shipped settings, `user` when the user chose the percentile.
/// `dem_notice` is the licence notice of the document's DEM (Copernicus GLO-30), carried when the DEM
/// gave any of it; `None` for a DEM of the user's own. `date` (`YYYY-MM-DD`, from the caller) dates
/// a level the DEM alone gave.
pub fn record_level(
    session: &mut Session,
    level: &LevelProfile,
    mark: Mark,
    dem_notice: Option<&str>,
    date: &str,
) -> Result<(), ReachError> {
    let err = |e: owd_format::design::DesignError| ReachError::Input(e.to_string());
    session.set_water_profile(&level.points).map_err(err)?;
    let swot = crate::sources::source("swot-hydrocron").expect("swot-hydrocron is in the registry");
    let sword = crate::sources::source("sword").expect("sword is in the registry");
    let used_dem = level.stretches.iter().any(|s| s.source == Source::Dem);
    let used_swot = level.swot_dates.is_some();
    let mut notices: Vec<String> = Vec::new();
    if used_swot {
        notices.push(swot.notice.to_string());
        notices.push(sword.notice.to_string());
    }
    if used_dem {
        notices.push(
            dem_notice
                .unwrap_or("Water surface from the study's own DEM")
                .to_string(),
        );
    }
    let (source, date) = match (&level.swot_dates, dem_notice) {
        (Some((a, b)), _) => ("swot-hydrocron", format!("{a}/{b}")),
        (None, Some(_)) => ("cop-dem-glo30", date.to_string()),
        (None, None) => ("dem", date.to_string()),
    };
    let p = &level.params;
    let settings = json!({
        "percentile": p.percentile,
        "min_swot_width_m": p.min_swot_width_m,
        "min_observations": p.min_observations,
        "max_reach_quality": p.max_quality,
        "datum": level.datum.name(),
        "datum_conversion": level.datum.conversion(),
        "flow": if level.downstream_is_increasing { "toward increasing chainage" } else { "toward decreasing chainage" },
        "flow_from": level.direction_from,
        "stretches": level.stretches.iter().map(|s| {
            let mut v = json!({"from_m": s.from_m, "to_m": s.to_m});
            match &s.source {
                Source::Swot { reach_id, observations, first, last } => {
                    v["source"] = json!("swot");
                    v["sword_reach"] = json!(reach_id);
                    v["observations"] = json!(observations);
                    v["dates"] = json!(format!("{first}/{last}"));
                }
                Source::Dem => {
                    v["source"] = json!("dem");
                    v["why"] = json!(s.why);
                }
            }
            v
        }).collect::<Vec<_>>(),
        "lowered": level.lowered.iter().map(|w| json!({
            "chainage_m": w.chainage_m, "was_m": w.was_m, "now_m": w.now_m, "source": "dem",
            "why": "above the level just upstream of it: the level never rises downstream",
        })).collect::<Vec<_>>(),
        "provisional": ["percentile", "min_swot_width_m"],
    });
    let rec = record(
        source,
        &date,
        &notices.join("; "),
        mark,
        mark == Mark::Default,
    );
    session
        .set_provenance("water_level", with_settings(rec, settings))
        .map_err(err)
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn the_percentile_reads_between_ranks_and_refuses_nothing_to_read() {
        assert_eq!(percentile(&[3.0, 1.0, 2.0, 4.0, 5.0], 50.0), Some(3.0));
        // Rank 0.1 * 4 = 0.4: 1 + 0.4 * (2 - 1).
        assert!((percentile(&[5.0, 4.0, 3.0, 2.0, 1.0], 10.0).unwrap() - 1.4).abs() < 1e-12);
        assert_eq!(percentile(&[7.0], 10.0), Some(7.0));
        assert_eq!(percentile(&[], 10.0), None);
        assert_eq!(percentile(&[1.0], 101.0), None);
    }

    #[test]
    fn never_rising_pools_a_rise_into_its_mean_and_keeps_a_fall() {
        assert_eq!(never_rising(&[5.0, 4.0, 3.0]), vec![5.0, 4.0, 3.0]);
        assert_eq!(
            never_rising(&[5.0, 3.0, 4.0, 2.0]),
            vec![5.0, 3.5, 3.5, 2.0]
        );
        assert_eq!(never_rising(&[1.0, 2.0, 3.0]), vec![2.0, 2.0, 2.0]);
    }
}
