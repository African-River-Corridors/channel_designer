//! Bank lines seeded from Sentinel-2 water (spec 0013, ticket T3; ADR 0011).
//!
//! The route is cut into parts (5 km by default, overlapping). For each part:
//!
//! 1. The dry-season scenes over the part (Earth Search), in the study's UTM zone and under the cloud
//!    limit, the least cloudy few first.
//! 2. Each one's scene classification alone, over the part's window, gives the share of the river
//!    corridor it sees clear; they are ranked by that.
//! 3. The best few are read whole and made into the part's water grid (`water::compose`), the JRC
//!    mask filling what no scene sees.
//!
//! The banks are then traced along the whole route at once on all the parts' grids (the first part
//! whose grid knows a point decides it; past every grid, the JRC mask), so there is no join between
//! parts to smooth. A bank edge the JRC mask gave is flagged `jrc-fallback`. The JRC banks are traced
//! too, as a cross-check: the share of stations where the two widths agree within half.

use crate::banks::{
    BankParams, JrcSource, MaskBanks, WaterSource, banks_from_source, route_in_utm, window_bbox,
};
use crate::edgefit::SceneQuery;
use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::geo::LonLat;
use crate::jrc::{Occurrence, read_window};
use crate::sentinel2::{Scene, parse_scenes, read_clear, read_imagery, search_url, window_grid};
use crate::water::{SceneUse, SceneWindow, State, WaterGrid, clear_share, compose, corridor};
use owd_format::design::crs::Utm;

type P2 = [f64; 2];

/// The settings of the Sentinel-2 seed. Every value here is a provisional default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeedParams {
    /// The length of a part of the route, in metres.
    pub part_m: f64,
    /// The overlap of two parts, in metres.
    pub overlap_m: f64,
    /// The half-width of the river corridor that ranks scenes, in metres.
    pub corridor_half_m: f64,
    /// The most scenes a part's grid is made from.
    pub max_scenes: usize,
    /// How many of the least cloudy scenes are ranked by their corridor.
    pub candidates: usize,
    /// MNDWI from which a clear pixel is water.
    pub mndwi_threshold: f64,
    /// Two widths agree when they differ by at most this share of the Sentinel-2 width.
    pub agreement_share: f64,
}

impl Default for SeedParams {
    fn default() -> Self {
        SeedParams {
            part_m: 5_000.0,
            overlap_m: 500.0,
            corridor_half_m: 500.0,
            max_scenes: crate::water::DEFAULT_MAX_SCENES,
            candidates: 6,
            mndwi_threshold: crate::water::DEFAULT_MNDWI_THRESHOLD,
            agreement_share: 0.5,
        }
    }
}

/// A part of the route and what its water grid was made from.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedPart {
    /// Chainage along the route, in metres.
    pub from_m: f64,
    pub to_m: f64,
    /// The scenes used, best first.
    pub scenes: Vec<SceneUse>,
    /// The shares of the part's corridor the JRC mask filled, and left unknown.
    pub corridor_jrc: f64,
    pub corridor_unknown: f64,
    /// Why the part has no grid, if it has none.
    pub note: Option<String>,
}

/// The banks and how they were made.
#[derive(Debug, Clone)]
pub struct S2Banks {
    pub banks: MaskBanks,
    pub parts: Vec<SeedPart>,
    pub seed: SeedParams,
    /// The share of stations where the JRC width agrees with the Sentinel-2 width, if the JRC mask
    /// was read and has water there.
    pub jrc_agreement: Option<f64>,
    /// Why there is no JRC cross-check or fallback, if there is none.
    pub jrc_note: Option<String>,
}

impl S2Banks {
    /// Every scene used, once, in the order first used.
    pub fn scenes(&self) -> Vec<&SceneUse> {
        let mut out: Vec<&SceneUse> = Vec::new();
        for s in self.parts.iter().flat_map(|p| &p.scenes) {
            if !out.iter().any(|o| o.id == s.id) {
                out.push(s);
            }
        }
        out
    }
}

/// The parts' grids, and the JRC mask past them, as one [`WaterSource`].
struct Grids<'a> {
    grids: &'a [WaterGrid],
    jrc: Option<&'a JrcSource<'a>>,
}

impl Grids<'_> {
    fn state(&self, p: P2) -> (Option<bool>, bool) {
        for g in self.grids {
            match g.at(p).0 {
                State::Water => return (Some(true), g.at(p).1 == crate::water::Origin::Jrc),
                State::Land => return (Some(false), g.at(p).1 == crate::water::Origin::Jrc),
                State::Unknown => {}
            }
        }
        match self.jrc {
            Some(j) => (Some(j.level(p) >= j.threshold()), true),
            None => (None, false),
        }
    }
}

impl WaterSource for Grids<'_> {
    fn level(&self, p: P2) -> f64 {
        if self.state(p).0 == Some(true) {
            100.0
        } else {
            0.0
        }
    }
    fn threshold(&self) -> f64 {
        50.0
    }
    fn pixel_m(&self) -> f64 {
        10.0
    }
    fn source(&self) -> &'static str {
        "sentinel2-l2a"
    }
    fn fallback(&self, p: P2) -> bool {
        self.state(p).1
    }
    fn no_water(&self) -> String {
        "the Sentinel-2 scenes show no water along this route, and neither does the JRC mask where \
         they cannot see. Check the route, widen the dates or the cloud limit, or seed from the JRC \
         mask (--seed jrc)."
            .into()
    }
}

/// The chainage of each route vertex.
fn chainage(xy: &[P2]) -> Vec<f64> {
    let mut s = vec![0.0];
    for w in xy.windows(2) {
        s.push(s[s.len() - 1] + ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt());
    }
    s
}

/// The route between two chainages.
fn part_of(xy: &[P2], s: &[f64], a: f64, b: f64) -> Vec<P2> {
    let at = |c: f64| owd_format::design::midline::point_at(xy, s, c);
    let mut out = vec![at(a)];
    out.extend(
        xy.iter()
            .zip(s)
            .filter(|(_, c)| **c > a && **c < b)
            .map(|(p, _)| *p),
    );
    out.push(at(b));
    out
}

/// The box of a line in UTM with a margin, and the same box in degrees.
fn boxes(pts: &[P2], utm: Utm, margin: f64) -> Result<([f64; 4], P2, P2), ReachError> {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let (min, max) = (
        [lo[0] - margin, lo[1] - margin],
        [hi[0] + margin, hi[1] + margin],
    );
    let c = utm
        .inverse_all(&[min, max, [min[0], max[1]], [max[0], min[1]]])
        .map_err(ReachError::Input)?;
    let bbox = [
        c.iter().map(|p| p[0]).fold(f64::MAX, f64::min),
        c.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
        c.iter().map(|p| p[0]).fold(f64::MIN, f64::max),
        c.iter().map(|p| p[1]).fold(f64::MIN, f64::max),
    ];
    Ok((bbox, min, max))
}

/// The scenes over a box worth ranking: dry season, in the study's zone, under the cloud limit,
/// covering the box where the item says where it is; the least cloudy `n`.
fn candidates(
    scenes: Vec<Scene>,
    q: &SceneQuery,
    utm: Utm,
    need: [f64; 4],
    n: usize,
) -> Vec<Scene> {
    let mut v: Vec<Scene> = scenes
        .into_iter()
        .filter(|s| {
            let month: Option<u32> = s.date.get(5..7).and_then(|m| m.parse().ok());
            month.is_some_and(|m| q.months.contains(&m))
                && s.cloud_pct <= q.max_cloud_pct
                && s.epsg.is_none_or(|e| e == utm.epsg())
                && s.bbox.is_none_or(|b| {
                    b[0] <= need[0] && b[1] <= need[1] && b[2] >= need[2] && b[3] >= need[3]
                })
        })
        .collect();
    v.sort_by(|a, b| {
        a.cloud_pct
            .total_cmp(&b.cloud_pct)
            .then_with(|| b.date.cmp(&a.date))
    });
    v.truncate(n);
    v
}

/// Make the banks of a route on Sentinel-2 water. `jrc` reaches the JRC tiles (for the fallback and
/// the cross-check); when they cannot be read the seed goes on without them and says so.
pub fn banks_from_s2(
    fetch: &dyn Fetch,
    jrc: Option<&dyn Fetch>,
    route: &[LonLat],
    params: &BankParams,
    q: &SceneQuery,
    sp: &SeedParams,
) -> Result<S2Banks, ReachError> {
    let (utm, xy, geo) = route_in_utm(route, params)?;
    // The JRC window, for the fallback and the cross-check.
    let (occ, jrc_note): (Option<Occurrence>, Option<String>) = match jrc {
        None => (None, Some("no JRC tiles were given".into())),
        Some(f) => {
            let bbox = window_bbox(route, params)
                .ok_or_else(|| ReachError::Input("an empty route".into()))?;
            match read_window(f, bbox) {
                Ok(o) => (Some(o), None),
                Err(e) => (None, Some(format!("the JRC tiles could not be read: {e}"))),
            }
        }
    };
    let jrc_src = occ.as_ref().map(|o| JrcSource {
        occ: o,
        grid: &geo,
        threshold_pct: params.threshold_pct,
    });
    let fill = |p: P2| -> Option<bool> { jrc_src.as_ref().map(|j| j.level(p) >= j.threshold()) };
    let fill_ref: Option<&dyn Fn(P2) -> Option<bool>> =
        if jrc_src.is_some() { Some(&fill) } else { None };

    let s = chainage(&xy);
    let length = s[s.len() - 1];
    let count = ((length / sp.part_m).ceil() as usize).max(1);
    let mut grids: Vec<WaterGrid> = Vec::new();
    let mut parts: Vec<SeedPart> = Vec::new();
    for k in 0..count {
        let a = (k as f64 * sp.part_m - sp.overlap_m / 2.0).max(0.0);
        let b = ((k + 1) as f64 * sp.part_m + sp.overlap_m / 2.0).min(length);
        let line = part_of(&xy, &s, a, b);
        let mut part = SeedPart {
            from_m: a,
            to_m: b,
            scenes: Vec::new(),
            corridor_jrc: 0.0,
            corridor_unknown: 1.0,
            note: None,
        };
        let (bbox, min, max) = boxes(&line, utm, params.max_half_width_m.min(1_000.0))?;
        let body = fetch.get(&search_url(
            &q.endpoint,
            bbox,
            &q.from,
            &q.to,
            q.max_cloud_pct,
            100,
        ))?;
        let cands = candidates(parse_scenes(&body)?, q, utm, bbox, sp.candidates);
        if cands.is_empty() {
            part.note = Some(format!(
                "no Sentinel-2 scene between {} and {} over this part in the dry-season months {:?} with {}% cloud or less",
                q.from, q.to, q.months, q.max_cloud_pct
            ));
            parts.push(part);
            continue;
        }
        // Rank by the corridor each sees clear, from the scene classification alone.
        let (x0, y1, cols, rows) = window_grid(min, max);
        let corr = corridor(&line, sp.corridor_half_m, x0, y1, 10.0, cols, rows);
        // A scene whose classification cannot be fetched is left out of the ranking: one missing file
        // must not stop the reach.
        let mut ranked: Vec<(f64, &Scene)> = Vec::new();
        for c in &cands {
            match read_clear(fetch, c, min, max) {
                Ok(clear) => ranked.push((clear_share(&clear, &corr), c)),
                Err(ReachError::Fetch(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        if ranked.is_empty() {
            part.note = Some("no scene's classification could be read over this part".into());
            parts.push(part);
            continue;
        }
        ranked.sort_by(|x, y| {
            y.0.total_cmp(&x.0)
                .then(x.1.cloud_pct.total_cmp(&y.1.cloud_pct))
        });
        let mut ims = Vec::new();
        for (_, sc) in ranked.iter().take(sp.max_scenes) {
            ims.push((read_imagery(fetch, sc, min, max)?, *sc));
        }
        let wins: Vec<SceneWindow> = ims
            .iter()
            .map(|(im, sc)| SceneWindow {
                id: &sc.id,
                date: &sc.date,
                cloud_pct: sc.cloud_pct,
                imagery: im,
            })
            .collect();
        let g = compose(
            &wins,
            &line,
            sp.corridor_half_m,
            sp.mndwi_threshold,
            sp.max_scenes,
            fill_ref,
        )
        .map_err(ReachError::Input)?;
        part.scenes = g.scenes.clone();
        part.corridor_jrc = g.corridor_jrc;
        part.corridor_unknown = g.corridor_unknown;
        parts.push(part);
        grids.push(g);
    }
    if grids.is_empty() && jrc_src.is_none() {
        return Err(ReachError::NoWater(format!(
            "no Sentinel-2 scene covers any part of the route ({}), and no JRC tile was read. \
             Raise the cloud limit, widen the dates, or give the JRC tiles",
            parts
                .first()
                .and_then(|p| p.note.clone())
                .unwrap_or_default()
        )));
    }
    let src = Grids {
        grids: &grids,
        jrc: jrc_src.as_ref(),
    };
    let banks = banks_from_source(utm, &xy, &src, params)?;
    // The cross-check: the JRC banks at the same threshold, widths compared station by station.
    let jrc_agreement = jrc_src
        .as_ref()
        .map(|j| match banks_from_source(utm, &xy, j, params) {
            Ok(jb) => agreement(&banks, &jb, sp.agreement_share),
            Err(_) => 0.0,
        });
    Ok(S2Banks {
        banks,
        parts,
        seed: *sp,
        jrc_agreement,
        jrc_note,
    })
}

/// The share of the stations of `a` whose width the nearest station of `b` matches within `share`.
fn agreement(a: &MaskBanks, b: &MaskBanks, share: f64) -> f64 {
    if a.centreline.is_empty() {
        return 0.0;
    }
    let mut ok = 0usize;
    for (p, w) in a.centreline.iter().zip(&a.widths) {
        let (mut best, mut wb) = (f64::MAX, 0.0);
        for (q, v) in b.centreline.iter().zip(&b.widths) {
            let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
            if d < best {
                best = d;
                wb = *v;
            }
        }
        if best.sqrt() <= w.max(50.0) && (wb - w).abs() <= share * w {
            ok += 1;
        }
    }
    ok as f64 / a.centreline.len() as f64
}

/// Record seeded banks in a document: the four inputs, each marked `default`, from `sentinel2-l2a`
/// with the dates of the scenes used, the Sentinel notice of each year and the JRC notice (the
/// fallback and the cross-check), and the settings: the scenes of each part with the share of the
/// corridor they saw, the thresholds, the JRC agreement, and which values are provisional.
pub fn record_s2_banks(
    session: &mut owd_format::design::Session,
    b: &S2Banks,
) -> Result<(), ReachError> {
    use owd_format::design::provenance::{Mark, record, with_settings};
    use serde_json::json;
    let s2 = crate::sources::source("sentinel2-l2a").expect("sentinel2-l2a is in the registry");
    let jrc = crate::sources::source("jrc-gsw").expect("jrc-gsw is in the registry");
    let scenes = b.scenes();
    let mut years: Vec<String> = scenes
        .iter()
        .map(|s| s.date.chars().take(4).collect())
        .collect();
    years.sort();
    years.dedup();
    let mut notices: Vec<String> = years.iter().map(|y| s2.notice_for(y)).collect();
    if b.jrc_note.is_none() {
        notices.push(jrc.notice.to_string());
    }
    let mut dates: Vec<&str> = scenes.iter().map(|s| s.date.as_str()).collect();
    dates.sort();
    let date = match (dates.first(), dates.last()) {
        (Some(a), Some(z)) if a != z => format!("{a}/{z}"),
        (Some(a), _) => (*a).to_string(),
        _ => crate::banks::JRC_DATE.to_string(),
    };
    let scene = |s: &SceneUse| {
        json!({
            "id": s.id, "date": s.date, "cloud_pct": s.cloud_pct,
            "corridor_clear": s.corridor_clear, "corridor_used": s.corridor_used,
            "mndwi_threshold": s.mndwi_threshold,
        })
    };
    let p = &b.banks.params;
    let settings = json!({
        "seed": "sentinel2-l2a",
        "water_index": "MNDWI",
        "mndwi_threshold_default": b.seed.mndwi_threshold,
        "mndwi_threshold_range": [crate::water::SPLIT_RANGE.0, crate::water::SPLIT_RANGE.1],
        "max_scenes": b.seed.max_scenes,
        "corridor_half_m": b.seed.corridor_half_m,
        "scenes": scenes.iter().map(|s| scene(s)).collect::<Vec<_>>(),
        "parts": b.parts.iter().map(|q| json!({
            "from_m": q.from_m, "to_m": q.to_m,
            "scenes": q.scenes.iter().map(scene).collect::<Vec<_>>(),
            "corridor_jrc": q.corridor_jrc, "corridor_unknown": q.corridor_unknown,
            "note": q.note,
        })).collect::<Vec<_>>(),
        "jrc_threshold_pct": p.threshold_pct,
        "jrc_agreement": b.jrc_agreement,
        "jrc_agreement_share": b.seed.agreement_share,
        "jrc_note": b.jrc_note,
        "min_trusted_width_m": b.banks.min_width_m(),
        "station_step_m": p.station_step_m,
        "max_half_width_m": p.max_half_width_m,
        "gap_tolerance_m": p.gap_tolerance_m,
        "provisional": ["mndwi_threshold_default", "mndwi_threshold_range", "max_scenes", "corridor_half_m", "jrc_threshold_pct", "min_trusted_width_m"],
    });
    let rec = record(
        "sentinel2-l2a",
        &date,
        &notices.join("; "),
        Mark::Default,
        true,
    );
    crate::banks::record_banks_as(session, &b.banks, &with_settings(rec, settings))
}
