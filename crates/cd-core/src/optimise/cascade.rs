//! Weir cascade search: where the weirs go, how many, and how high each pool sits. Mirrors
//! `optimise/cascade.py`.
//!
//! A cascade is a monotone staircase on one chainage axis, so this is a SHORTEST PATH ON A DAG,
//! solved exactly: no metaheuristic, no local optimum.
//!
//! ```text
//! state   (site i, pool level h)   a weir at site i holding the pool UPSTREAM of it at h
//! edge    (i, h) -> (j, h + l)     the next weir at site j, lifting by l
//! cost    reach_cost(i -> j at h) + structure_cost(l)
//!
//! source  a virtual weir at `start_m` with h = `base_level_m`
//! sink    any state whose final pool, running to `terminal_m`, is at least
//!         `terminal_min_level_m`
//! ```
//!
//! Every cost is a caller-supplied function returning a unitless WEIGHT. The crate prices
//! nothing. A typical reach cost is earthworks from a [`LevelTable`]
//! ([`earthworks_reach_cost`]), plus any terms the caller cares about, each weighted by the
//! caller.
//!
//! The pool levels the DP can reach are the base level plus every sum of allowed lifts up to
//! `level_cap_m`. [`cascade_levels`] lists them; build a [`LevelTable`] on exactly those so no
//! level is read by rounding to a neighbour.
//!
//! Guards (each one is a silent failure in a looser version of this search):
//! - a level outside the table is refused, never clamped to the table's edge;
//! - a terminal level above the level cap is refused, rather than reporting "no feasible
//!   cascade";
//! - candidate sites beyond the terminal are dropped, and the result says how many.

use crate::cut_model::{CUT_MODEL, DX_M, MAXW_M, STATION_M, WET_SLOPE_H_PER_V};
use crate::geom::{Polygon, Pt};
use crate::py_sum;
use crate::volumes::sections::{self, DEFAULT_CHUNK, HalfWidth, Level, TerrainSampler};
use ovcd_core::pyfmt::{general, round_to};
use std::collections::HashMap;

/// A refusal: a Python `ValueError`, with its text.
pub type Refusal = String;

/// `reach_cost(start_m, end_m, level_m) -> weight`.
pub type ReachCost<'a> = &'a dyn Fn(f64, f64, f64) -> Result<f64, Refusal>;
/// `structure_cost(lift_m) -> weight`.
pub type StructureCost<'a> = &'a dyn Fn(f64) -> f64;

const LEVEL_DECIMALS: usize = 3;

/// The cascade problem. Chainages in metres on the caller's axis, levels in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeConfig {
    /// Where a weir may stand.
    pub candidate_sites_m: Vec<f64>,
    /// Allowed lift per structure.
    pub lifts_m: Vec<f64>,
    /// Chainage the cascade must reach.
    pub terminal_m: f64,
    /// The top pool must be at least this.
    pub terminal_min_level_m: f64,
    /// Water level below the first weir.
    pub base_level_m: f64,
    /// Chainage of the virtual source.
    pub start_m: f64,
    /// Minimum distance between structures.
    pub min_spacing_m: f64,
    /// Highest pool; `None` means terminal min + max lift, and `new` fills it in.
    pub level_cap_m: Option<f64>,
    /// Ranked distinct endings to return.
    pub k_best: i64,
    pub site_ids: Option<Vec<String>>,
}

impl CascadeConfig {
    /// A config with the Python defaults for everything but the four required fields.
    pub fn new(
        candidate_sites_m: Vec<f64>,
        lifts_m: Vec<f64>,
        terminal_m: f64,
        terminal_min_level_m: f64,
    ) -> Self {
        Self {
            candidate_sites_m,
            lifts_m,
            terminal_m,
            terminal_min_level_m,
            base_level_m: 0.0,
            start_m: 0.0,
            min_spacing_m: 0.0,
            level_cap_m: None,
            k_best: 5,
            site_ids: None,
        }
    }

    /// The Python `__post_init__`: checks, and the default level cap. Call it once, before use.
    pub fn validated(mut self) -> Result<Self, Refusal> {
        let min_lift = self.lifts_m.iter().copied().fold(f64::INFINITY, f64::min);
        if self.lifts_m.is_empty() || min_lift <= 0.0 {
            return Err("lifts_m must be non-empty and every lift > 0".into());
        }
        if let Some(ids) = &self.site_ids
            && ids.len() != self.candidate_sites_m.len()
        {
            return Err("site_ids must match candidate_sites_m one to one".into());
        }
        let max_lift = self
            .lifts_m
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let cap = *self
            .level_cap_m
            .get_or_insert(self.terminal_min_level_m + max_lift);
        if self.terminal_min_level_m > cap + 1e-9 {
            return Err(format!(
                "terminal_min_level_m {} m is above level_cap_m {} m, so no cascade can reach \
                 it. Raise level_cap_m to at least {}.",
                general(self.terminal_min_level_m),
                general(cap),
                general(self.terminal_min_level_m + max_lift)
            ));
        }
        Ok(self)
    }

    fn cap(&self) -> f64 {
        self.level_cap_m.unwrap_or(f64::NAN)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CascadeWeir {
    pub site_id: String,
    pub chainage_m: f64,
    /// Water level of the pool UPSTREAM of this weir.
    pub pool_level_m: f64,
    pub lift_m: f64,
    pub structure_cost: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CascadeReach {
    pub start_m: f64,
    pub end_m: f64,
    pub level_m: f64,
    pub cost: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cascade {
    pub total_cost: f64,
    pub weirs: Vec<CascadeWeir>,
    pub reaches: Vec<CascadeReach>,
}

/// One weir for `place_weirs_explicit`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplicitWeir {
    pub chainage_m: f64,
    pub pool_mwl_m: f64,
    pub structure_height_m: Option<f64>,
}

impl Cascade {
    pub fn reach_cost(&self) -> f64 {
        py_sum(self.reaches.iter().map(|r| r.cost))
    }

    pub fn structure_cost(&self) -> f64 {
        py_sum(self.weirs.iter().map(|w| w.structure_cost))
    }

    /// `(start_m, end_m, level_m)`, downstream to upstream, base reach included.
    pub fn pools(&self) -> Vec<(f64, f64, f64)> {
        self.reaches
            .iter()
            .map(|r| (r.start_m, r.end_m, r.level_m))
            .collect()
    }

    pub fn as_explicit_weirs(&self, structure_height_m: Option<f64>) -> Vec<ExplicitWeir> {
        self.weirs
            .iter()
            .map(|w| ExplicitWeir {
                chainage_m: w.chainage_m,
                pool_mwl_m: w.pool_level_m,
                structure_height_m,
            })
            .collect()
    }
}

/// Ranked cascades plus what the search did.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeSearch {
    pub cascades: Vec<Cascade>,
    pub levels_m: Vec<f64>,
    pub states_explored: usize,
    pub feasible_endings: usize,
    pub sites_dropped_beyond_terminal: usize,
}

impl CascadeSearch {
    pub fn best(&self) -> Option<&Cascade> {
        self.cascades.first()
    }
}

fn cmp(a: f64, b: f64) -> core::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(core::cmp::Ordering::Equal)
}

/// A float as a map key: equal floats give equal keys (`-0.0` and `0.0` too).
fn key(x: f64) -> u64 {
    (x + 0.0).to_bits()
}

/// Every pool level the DP can reach: base + any sum of lifts, up to the cap.
pub fn cascade_levels(cfg: &CascadeConfig) -> Vec<f64> {
    let cap = cfg.cap();
    let base = round_to(cfg.base_level_m, LEVEL_DECIMALS) + 0.0;
    let mut reach: Vec<f64> = vec![base];
    let mut frontier: Vec<f64> = vec![base];
    while !frontier.is_empty() {
        let mut next: Vec<f64> = Vec::new();
        for &r in &frontier {
            for &l in &cfg.lifts_m {
                let v = round_to(r + l, LEVEL_DECIMALS) + 0.0;
                if v <= cap + 1e-9 && !reach.contains(&v) && !next.contains(&v) {
                    next.push(v);
                }
            }
        }
        reach.extend(next.iter().copied());
        frontier = next;
    }
    reach.sort_by(|a, b| cmp(*a, *b));
    reach
}

/// The candidate sites inside `(start, terminal]`, sorted, and how many were dropped.
fn sites(cfg: &CascadeConfig) -> (Vec<(f64, String)>, usize) {
    let ids: Vec<String> = match &cfg.site_ids {
        Some(ids) => ids.clone(),
        None => (0..cfg.candidate_sites_m.len())
            .map(|i| format!("S{}", i + 1))
            .collect(),
    };
    let mut rows: Vec<(f64, String)> = cfg.candidate_sites_m.iter().copied().zip(ids).collect();
    // Python sorts the (site, id) tuples: by site, then by id.
    rows.sort_by(|a, b| cmp(a.0, b.0).then_with(|| a.1.cmp(&b.1)));
    let total = rows.len();
    let kept: Vec<(f64, String)> = rows
        .into_iter()
        .filter(|(c, _)| cfg.start_m < *c && *c <= cfg.terminal_m)
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}

/// Exact least-cost cascade(s). Returns up to `k_best` distinct endings, cheapest first.
///
/// "Distinct endings" means distinct (last weir, top pool level) states, each with its own
/// cheapest history, not the k cheapest cascades overall.
pub fn search_cascade(
    cfg: &CascadeConfig,
    reach_cost: ReachCost,
    structure_cost: StructureCost,
) -> Result<CascadeSearch, Refusal> {
    let (kept, dropped) = sites(cfg);
    let mut s: Vec<(f64, String)> = vec![(cfg.start_m, "SOURCE".to_string())];
    s.extend(kept);
    let lvls = cascade_levels(cfg);
    let li: HashMap<u64, usize> = lvls.iter().enumerate().map(|(i, &v)| (key(v), i)).collect();
    let base = round_to(cfg.base_level_m, LEVEL_DECIMALS);
    let n_l = lvls.len();
    let inf = f64::INFINITY;
    let mut dp = vec![vec![inf; n_l]; s.len()];
    let mut par = vec![vec![(usize::MAX, usize::MAX, usize::MAX); n_l]; s.len()];
    dp[0][li[&key(base)]] = 0.0;
    let lifts = &cfg.lifts_m;
    let s_cost: Vec<f64> = lifts.iter().map(|&l| structure_cost(l)).collect();
    for j in 1..s.len() {
        for i in 0..j {
            if s[j].0 - s[i].0 < cfg.min_spacing_m {
                continue;
            }
            for (hi, &h) in lvls.iter().enumerate() {
                if dp[i][hi] == inf {
                    continue;
                }
                let b = dp[i][hi] + reach_cost(s[i].0, s[j].0, h)?;
                for (lix, &l) in lifts.iter().enumerate() {
                    let h2 = round_to(h + l, LEVEL_DECIMALS);
                    let Some(&k) = li.get(&key(h2)) else {
                        continue;
                    };
                    let c = b + s_cost[lix];
                    if c < dp[j][k] {
                        dp[j][k] = c;
                        par[j][k] = (i, hi, lix);
                    }
                }
            }
        }
    }
    let mut finals: Vec<(f64, usize, usize)> = Vec::new();
    for j in 1..s.len() {
        for (hi, &h) in lvls.iter().enumerate() {
            if dp[j][hi] == inf || h < cfg.terminal_min_level_m - 1e-9 {
                continue;
            }
            finals.push((dp[j][hi] + reach_cost(s[j].0, cfg.terminal_m, h)?, j, hi));
        }
    }
    finals.sort_by(|a, b| cmp(a.0, b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    // Python's `finals[:k_best]`: a negative count drops from the end.
    let take = if cfg.k_best >= 0 {
        std::cmp::min(cfg.k_best as usize, finals.len())
    } else {
        finals
            .len()
            .saturating_sub(cfg.k_best.unsigned_abs() as usize)
    };
    let mut out = Vec::new();
    for &(tot, j, hi) in &finals[..take] {
        let mut chain: Vec<(String, f64, f64, usize)> = Vec::new();
        let (mut cj, mut ch) = (j, hi);
        while cj > 0 {
            let (pi, ph, pl) = par[cj][ch];
            chain.push((s[cj].1.clone(), s[cj].0, lvls[ch], pl));
            cj = pi;
            ch = ph;
        }
        chain.reverse();
        out.push(assemble(
            cfg,
            &chain,
            &cfg.lifts_m,
            reach_cost,
            &s_cost,
            Some(tot),
        )?);
    }
    let explored = dp.iter().flatten().filter(|v| v.is_finite()).count();
    Ok(CascadeSearch {
        cascades: out,
        levels_m: lvls,
        states_explored: explored,
        feasible_endings: finals.len(),
        sites_dropped_beyond_terminal: dropped,
    })
}

/// `chain` is `(site id, chainage, pool level, lift index)`; `lifts` and `s_cost` are indexed by
/// lift index.
fn assemble(
    cfg: &CascadeConfig,
    chain: &[(String, f64, f64, usize)],
    lifts: &[f64],
    reach_cost: ReachCost,
    s_cost: &[f64],
    total: Option<f64>,
) -> Result<Cascade, Refusal> {
    let weirs: Vec<CascadeWeir> = chain
        .iter()
        .map(|(id, km, lvl, li)| CascadeWeir {
            site_id: id.clone(),
            chainage_m: *km,
            pool_level_m: *lvl,
            lift_m: lifts[*li],
            structure_cost: s_cost[*li],
        })
        .collect();
    let mut bounds: Vec<(f64, f64)> = vec![(cfg.start_m, cfg.base_level_m)];
    bounds.extend(weirs.iter().map(|w| (w.chainage_m, w.pool_level_m)));
    let mut ends: Vec<f64> = weirs.iter().map(|w| w.chainage_m).collect();
    ends.push(cfg.terminal_m);
    let mut reaches = Vec::new();
    for (&(a, lvl), &b) in bounds.iter().zip(&ends) {
        reaches.push(CascadeReach {
            start_m: a,
            end_m: b,
            level_m: lvl,
            cost: reach_cost(a, b, lvl)?,
        });
    }
    let mut c = Cascade {
        total_cost: 0.0,
        weirs,
        reaches,
    };
    c.total_cost = match total {
        Some(t) => t,
        None => c.reach_cost() + c.structure_cost(),
    };
    Ok(c)
}

/// Cost of a FIXED cascade, `weirs = [(chainage_m, lift_m), ...]`, under the same objective. The
/// only fair way to compare two cascades is one objective, both scored.
pub fn score_cascade(
    cfg: &CascadeConfig,
    weirs: &[(f64, f64)],
    reach_cost: ReachCost,
    structure_cost: StructureCost,
) -> Result<Cascade, Refusal> {
    let mut spec: Vec<(f64, f64)> = weirs.to_vec();
    spec.sort_by(|a, b| cmp(a.0, b.0).then(cmp(a.1, b.1)));
    let mut lvl = round_to(cfg.base_level_m, LEVEL_DECIMALS);
    let mut chain = Vec::new();
    let mut lifts = Vec::new();
    for (n, &(c, l)) in spec.iter().enumerate() {
        lvl = round_to(lvl + l, LEVEL_DECIMALS);
        chain.push((format!("W{}", n + 1), c, lvl, n));
        lifts.push(l);
    }
    let s_cost: Vec<f64> = lifts.iter().map(|&l| structure_cost(l)).collect();
    assemble(cfg, &chain, &lifts, reach_cost, &s_cost, None)
}

// =====================================================================================
// Earthworks by level: the usual reach cost
// =====================================================================================

/// Cut area per station at each candidate pool level.
///
/// `dredge_m2` / `excav_m2` are (stations, levels). `chainage_m` is each station's chainage on
/// the cascade's axis (pass your own axis to [`build_level_table`] when the cascade is set out on
/// a different line from the one the volumes are measured on).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelTable {
    pub chainage_m: Vec<f64>,
    pub levels_m: Vec<f64>,
    pub dredge_m2: Vec<Vec<f64>>,
    pub excav_m2: Vec<Vec<f64>>,
    pub station_m: f64,
    pub level_tol_m: f64,
}

impl LevelTable {
    pub fn new(
        chainage_m: Vec<f64>,
        levels_m: Vec<f64>,
        dredge_m2: Vec<Vec<f64>>,
        excav_m2: Vec<Vec<f64>>,
        station_m: f64,
    ) -> Self {
        Self {
            chainage_m,
            levels_m,
            dredge_m2,
            excav_m2,
            station_m,
            level_tol_m: 1e-6,
        }
    }

    pub fn level_index(&self, level_m: f64) -> Result<usize, Refusal> {
        let mut best = 0;
        let mut best_d = f64::INFINITY;
        for (i, &l) in self.levels_m.iter().enumerate() {
            let d = (l - level_m).abs();
            if d < best_d {
                best = i;
                best_d = d;
            }
        }
        if (self.levels_m[best] - level_m).abs() > self.level_tol_m {
            let lo = self.levels_m.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = self
                .levels_m
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            return Err(format!(
                "pool level {} m is not in the table (levels {}-{} m). Build the table on \
                 cascade_levels(cfg); a level is never read from its neighbour.",
                general(level_m),
                general(lo),
                general(hi)
            ));
        }
        Ok(best)
    }

    /// Cumulative dredge and excavation volumes (m3), stations sorted by chainage, with a zero
    /// row first: `(chainages, cum_dredge, cum_excav)`.
    fn cumulative(&self) -> (Vec<f64>, Vec<Vec<f64>>, Vec<Vec<f64>>) {
        let mut order: Vec<usize> = (0..self.chainage_m.len()).collect();
        order.sort_by(|&a, &b| cmp(self.chainage_m[a], self.chainage_m[b]));
        let ch: Vec<f64> = order.iter().map(|&i| self.chainage_m[i]).collect();
        let n_l = self.levels_m.len();
        let cum = |m: &Vec<Vec<f64>>| -> Vec<Vec<f64>> {
            let mut rows = vec![vec![0.0; n_l]];
            let mut run = vec![0.0; n_l];
            for &i in &order {
                for (k, r) in run.iter_mut().enumerate() {
                    *r += m[i][k];
                }
                rows.push(run.clone());
            }
            rows.into_iter()
                .map(|r| r.into_iter().map(|v| v * self.station_m).collect())
                .collect()
        };
        (ch, cum(&self.dredge_m2), cum(&self.excav_m2))
    }

    /// `(dredge m3, excavation m3)` over stations with `start_m <= chainage < end_m`.
    pub fn reach_volumes(
        &self,
        start_m: f64,
        end_m: f64,
        level_m: f64,
    ) -> Result<(f64, f64), Refusal> {
        let (ch, cd, ce) = self.cumulative();
        let j = self.level_index(level_m)?;
        // np.searchsorted(ch, x): the first index with ch[i] >= x.
        let i0 = ch.partition_point(|&c| c < start_m);
        let i1 = ch.partition_point(|&c| c < end_m);
        Ok((cd[i1][j] - cd[i0][j], ce[i1][j] - ce[i0][j]))
    }
}

/// Section parameters that `build_level_table` hands to `section_volumes` unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionOptions {
    pub wet_slope: f64,
    pub dx_m: f64,
    pub daylight_run_m: Option<f64>,
    pub chunk: usize,
}

impl Default for SectionOptions {
    fn default() -> Self {
        Self {
            wet_slope: WET_SLOPE_H_PER_V,
            dx_m: DX_M,
            daylight_run_m: None,
            chunk: DEFAULT_CHUNK,
        }
    }
}

/// Run `section_volumes` once per level, with the pool water level = that level and grade =
/// level - depth, and keep the per-station areas. `chainage_m` (one per station) re-labels the
/// stations onto the cascade's axis.
#[allow(clippy::too_many_arguments)]
pub fn build_level_table(
    line: &[Pt],
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    levels_m: &[f64],
    depth_m: f64,
    hb: &HalfWidth,
    model: Option<&str>,
    station_m: Option<f64>,
    maxw_m: Option<f64>,
    chainage_m: Option<&[f64]>,
    options: SectionOptions,
) -> Result<LevelTable, Refusal> {
    let mut levels: Vec<f64> = levels_m.to_vec();
    levels.sort_by(|a, b| cmp(*a, *b));
    let station = station_m.unwrap_or(STATION_M);
    let mut dredge: Vec<Vec<f64>> = Vec::new();
    let mut excav: Vec<Vec<f64>> = Vec::new();
    let mut ch: Vec<f64> = Vec::new();
    for (k, &lvl) in levels.iter().enumerate() {
        let v = sections::section_volumes(
            line,
            terrain,
            river_polygon,
            &Level::Fixed(lvl),
            depth_m,
            hb,
            model.unwrap_or(CUT_MODEL),
            options.wet_slope,
            station,
            maxw_m.unwrap_or(MAXW_M),
            options.dx_m,
            options.daylight_run_m,
            None,
            options.chunk,
        )?;
        if k == 0 {
            let n = v.stations.len();
            dredge = vec![vec![0.0; levels.len()]; n];
            excav = vec![vec![0.0; levels.len()]; n];
            ch = v.stations.iter().map(|s| s.chainage_m).collect();
        }
        for (i, st) in v.stations.iter().enumerate() {
            dredge[i][k] = st.dredge_area_m2;
            excav[i][k] = st.excav_area_m2;
        }
    }
    if let Some(c) = chainage_m {
        if c.len() != dredge.len() {
            return Err(format!(
                "chainage_m needs one value per station ({})",
                dredge.len()
            ));
        }
        ch = c.to_vec();
    }
    Ok(LevelTable::new(ch, levels, dredge, excav, station))
}

/// `reach_cost(start_m, end_m, level_m) = wet_weight x dredge + dry_weight x excavation`.
///
/// Weights, not prices: 1.0 / 1.0 minimises volume.
pub fn earthworks_reach_cost(
    table: &LevelTable,
    wet_weight: f64,
    dry_weight: f64,
) -> impl Fn(f64, f64, f64) -> Result<f64, Refusal> + '_ {
    move |a, b, level| {
        let (d, e) = table.reach_volumes(a, b, level)?;
        Ok(wet_weight * d + dry_weight * e)
    }
}
