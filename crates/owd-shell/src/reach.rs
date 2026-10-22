//! "Start from a river" (spec 0012, T10): the steps that make a study's inputs from open data, for
//! the app. The work is `owd_reach`'s; this module only holds what a step needs between calls (the
//! planned route, the mask banks, the JRC tiles the user dropped) and runs one step at a time on a
//! copy of the session, so the shell can keep the page live while it fetches and drop the result on
//! Cancel. Network access is the [`Fetch`] the shell gives: native HTTP on the desktop, the
//! browser's on the web, recorded answers in tests.

use crate::state::AppState;
use owd_format::design::Session;
use owd_format::design::provenance::Mark;
use owd_format::geotiff::Dem;
use owd_reach::banks::{BankParams, MaskBanks, window_bbox};
use owd_reach::edgefit::{FitParams, SceneQuery, default_range, fit_to_scene, record_edge_fit};
use owd_reach::level::{Datum, LevelParams, SWOT_SCIENCE_START, level_from_sources, record_level};
use owd_reach::overpass::{DEFAULT_ENDPOINT, Overpass};
use owd_reach::reach::{Planned, plan_points, record_reach};
use owd_reach::river::{WholeRiver, record_river_reach, whole_river};
use owd_reach::search::{OsmRef, RiverHit, RiverSearch};
use owd_reach::seed::{SeedParams, banks_from_s2, record_s2_banks};
use owd_reach::{DroppedFiles, Fetch};
use serde_json::{Value, json};
use std::sync::Arc;

/// Where the sources are. The defaults are the public services; tests point them at recordings.
#[derive(Debug, Clone, PartialEq)]
pub struct ReachEndpoints {
    pub overpass: String,
    pub photon: String,
    pub nominatim: String,
    pub stac: String,
    pub sword_record: String,
    pub hydrocron: String,
}

impl Default for ReachEndpoints {
    fn default() -> Self {
        ReachEndpoints {
            overpass: DEFAULT_ENDPOINT.to_string(),
            photon: owd_reach::search::PHOTON_ENDPOINT.to_string(),
            nominatim: owd_reach::search::NOMINATIM_ENDPOINT.to_string(),
            stac: owd_reach::sentinel2::SEARCH_ENDPOINT.to_string(),
            sword_record: owd_reach::sword::RECORD_API.to_string(),
            hydrocron: owd_reach::level::HYDROCRON_ENDPOINT.to_string(),
        }
    }
}

impl ReachEndpoints {
    /// The defaults with the members a JSON object gives laid over them.
    pub fn from_json(v: &Value) -> ReachEndpoints {
        let d = ReachEndpoints::default();
        let s = |k: &str, def: String| v[k].as_str().map_or(def, str::to_string);
        ReachEndpoints {
            overpass: s("overpass", d.overpass),
            photon: s("photon", d.photon),
            nominatim: s("nominatim", d.nominatim),
            stac: s("stac", d.stac),
            sword_record: s("sword_record", d.sword_record),
            hydrocron: s("hydrocron", d.hydrocron),
        }
    }
}

/// What the steps keep between calls. Not saved: a reopened study plans its route again.
#[derive(Debug, Default, Clone)]
pub struct ReachMemo {
    plan: Option<Planned>,
    /// The whole river the user chose from the search, to cut reaches out of with no new request,
    /// and the search it came from (`osm-photon` or `osm-nominatim`).
    river: Option<(WholeRiver, Option<String>)>,
    banks: Option<MaskBanks>,
    /// JRC tiles the user gave, by file name.
    tiles: Vec<(String, Arc<Vec<u8>>)>,
}

/// One step after the route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReachStep {
    /// Banks, polygon and centreline from Sentinel-2 water, the JRC mask where no scene sees.
    Banks,
    /// Move the mask banks onto the water edge in Sentinel-2.
    EdgeFit,
    /// The default water level from SWORD and SWOT, with the DEM where they give none.
    Level { percentile: Option<f64> },
}

impl ReachStep {
    pub fn of(name: &str, percentile: Option<f64>) -> Result<ReachStep, String> {
        match name {
            "banks" => Ok(ReachStep::Banks),
            "edge_fit" => Ok(ReachStep::EdgeFit),
            "level" => Ok(ReachStep::Level { percentile }),
            o => Err(format!("`{o}` is not a step (banks, edge_fit or level)")),
        }
    }
}

/// One step running on a copy of the session.
pub struct ReachJob {
    step: ReachStep,
    session: Session,
    memo: ReachMemo,
    dem: Option<Arc<Dem>>,
    /// The document when the step began: the result is kept only if it has not changed since.
    base: Value,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The DEM height at a longitude and latitude, from a DEM in WGS84 or in the study's UTM zone.
fn dem_sampler(
    dem: &Dem,
    utm: owd_format::design::crs::Utm,
) -> Option<Box<dyn Fn([f64; 2]) -> f64 + '_>> {
    match dem.epsg {
        Some(4326) => Some(Box::new(move |p: [f64; 2]| dem.sample_bilinear(p[0], p[1]))),
        Some(e) if e == utm.epsg() => Some(Box::new(move |p: [f64; 2]| {
            utm.forward(p[0], p[1])
                .map_or(f64::NAN, |q| dem.sample_bilinear(q[0], q[1]))
        })),
        _ => None,
    }
}

impl ReachJob {
    /// Run the step. `fetch` reaches the network; `date` is today (`YYYY-MM-DD`, from the shell).
    pub fn run(
        mut self,
        fetch: &dyn Fetch,
        at: &ReachEndpoints,
        date: &str,
    ) -> Result<ReachJob, String> {
        match self.step {
            ReachStep::Banks => {
                let plan = self.memo.plan.as_ref().ok_or("pick the reach first")?;
                let dropped = self
                    .memo
                    .tiles
                    .iter()
                    .fold(DroppedFiles::new(), |d, (n, b)| d.with_file(n, b.to_vec()));
                let jrc: &dyn Fetch = if self.memo.tiles.is_empty() {
                    fetch
                } else {
                    &dropped
                };
                // Spec 0013: the banks are traced on the water of dry-season Sentinel-2 scenes, the
                // JRC mask filling what no scene sees and checking the widths.
                let line = plan.route.line();
                let (from, to) = default_range(date).map_err(err)?;
                let q = SceneQuery {
                    endpoint: at.stac.clone(),
                    from,
                    to,
                    months: owd_reach::sentinel2::default_dry_months(line[line.len() / 2][1]),
                    max_cloud_pct: owd_reach::sentinel2::DEFAULT_MAX_CLOUD_PCT,
                };
                let b = banks_from_s2(
                    fetch,
                    Some(jrc),
                    &line,
                    &BankParams::default(),
                    &q,
                    &SeedParams::default(),
                )
                .map_err(err)?;
                record_s2_banks(&mut self.session, &b).map_err(err)?;
                self.memo.banks = Some(b.banks);
            }
            ReachStep::EdgeFit => {
                let banks = self.memo.banks.as_ref().ok_or("make the banks first")?;
                let (from, to) = default_range(date).map_err(err)?;
                let mid = banks.centreline[banks.centreline.len() / 2];
                let lat = banks.utm.inverse(mid[0], mid[1]).map_err(err)?[1];
                let q = SceneQuery {
                    endpoint: at.stac.clone(),
                    from,
                    to,
                    months: owd_reach::sentinel2::default_dry_months(lat),
                    max_cloud_pct: owd_reach::sentinel2::DEFAULT_MAX_CLOUD_PCT,
                };
                let f = fit_to_scene(fetch, banks, &FitParams::default(), &q).map_err(err)?;
                record_edge_fit(&mut self.session, &f).map_err(err)?;
            }
            ReachStep::Level { percentile } => {
                let (centre, utm) = self.session.water_reference().map_err(err)?;
                let mut params = LevelParams::default();
                if let Some(p) = percentile {
                    params.percentile = p;
                }
                let sampler = self.dem.as_deref().and_then(|d| dem_sampler(d, utm));
                let level = level_from_sources(
                    fetch,
                    &at.sword_record,
                    &at.hydrocron,
                    &centre,
                    utm,
                    (SWOT_SCIENCE_START, date),
                    sampler.as_deref(),
                    &params,
                    Datum::Egm2008,
                )
                .map_err(err)?;
                let mark = if params == LevelParams::default() {
                    Mark::Default
                } else {
                    Mark::User
                };
                let copernicus = !self.session.dem_source_tiles().is_empty();
                let notice =
                    copernicus.then_some(owd_format::design::dem_source::COPERNICUS_NOTICE);
                record_level(&mut self.session, &level, mark, notice, date).map_err(err)?;
            }
        }
        Ok(self)
    }
}

/// One river search hit as the app lists it.
pub fn hit_value(h: &RiverHit) -> Value {
    json!({
        "osm_id": h.osm.to_string(), "label": h.label(), "name": h.name, "kind": h.kind,
        "region": h.region, "country": h.country, "country_code": h.country_code,
        "also": h.also.iter().map(ToString::to_string).collect::<Vec<_>>(), "bbox": h.bbox,
    })
}

/// The rivers and canals for a text (spec 0012, T13), as a JSON list, best first. Under three
/// letters it asks nothing. `once` asks Nominatim, one request, for when Photon fails; otherwise
/// Photon, as the user types. Network work only: it changes no state.
pub fn search_rivers(
    fetch: &dyn Fetch,
    at: &ReachEndpoints,
    text: &str,
    country: Option<&str>,
    once: bool,
) -> Result<Value, String> {
    let country = country.filter(|c| !c.trim().is_empty());
    let s = RiverSearch::with_endpoint(fetch, &at.photon);
    let hits = if once {
        s.search_once(&at.nominatim, text, country)
    } else {
        s.search(text, country)
    }
    .map_err(|e| match e {
        owd_reach::ReachError::Fetch(_) | owd_reach::ReachError::Parse(_) if !once => {
            "The river search (Photon) is not answering, or is busy. Press Enter to search \
             OpenStreetMap's Nominatim once."
                .to_string()
        }
        e => err(e),
    })?;
    Ok(Value::Array(hits.iter().map(hit_value).collect()))
}

/// Read a whole river from OSM (spec 0012, T14): `osm` and `also` as a search hit gives them
/// (`relation/N`, `way/N`). Network work only.
pub fn read_river(
    fetch: &dyn Fetch,
    at: &ReachEndpoints,
    osm: &str,
    also: &[String],
) -> Result<WholeRiver, String> {
    let osm: OsmRef = osm.parse().map_err(err)?;
    let also = also
        .iter()
        .map(|a| a.parse::<OsmRef>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let ov = Overpass::with_endpoint(fetch, &at.overpass);
    whole_river(&ov, osm, &also).map_err(err)
}

/// Plan a reach over Overpass: the route from a start, through via points, to an end. A river
/// name, if given, is searched too. Network work only: it changes no state.
pub fn plan_reach(
    fetch: &dyn Fetch,
    at: &ReachEndpoints,
    river: Option<&str>,
    start: [f64; 2],
    end: [f64; 2],
    via: &[[f64; 2]],
) -> Result<Planned, String> {
    let river = river.map(str::trim).filter(|r| !r.is_empty());
    let ov = Overpass::with_endpoint(fetch, &at.overpass);
    plan_points(&ov, river, start, end, via).map_err(err)
}

impl AppState {
    /// Start a new study from a reach: plan the route over Overpass from a start, an end and via
    /// points ([`plan_reach`]), and record it in a new, unsaved design ([`AppState::start_reach`]).
    #[allow(clippy::too_many_arguments)]
    pub fn reach_route(
        &mut self,
        fetch: &dyn Fetch,
        at: &ReachEndpoints,
        river: Option<&str>,
        start: [f64; 2],
        end: [f64; 2],
        via: &[[f64; 2]],
        date: &str,
    ) -> Result<(), String> {
        self.refuse_while_running()?;
        let plan = plan_reach(fetch, at, river, start, end, via)?;
        self.start_reach(plan, date)
    }

    /// Start a new study from a whole river the user chose from the search: Start at the mouth, End
    /// at the upstream end. The river is kept, so [`AppState::reach_cut`] needs no new request.
    /// `search` is the source the user found it with (`osm-photon` or `osm-nominatim`).
    pub fn start_river(
        &mut self,
        river: WholeRiver,
        search: Option<&str>,
        date: &str,
    ) -> Result<(), String> {
        let plan = river.planned();
        self.start_reach_with(plan, Some((river, search.map(str::to_string))), date)
    }

    /// Cut a reach out of the river chosen from the search: the route along its own ways from
    /// `start`, through `via`, to `end`. A new, unsaved study, as for [`AppState::reach_route`].
    pub fn reach_cut(
        &mut self,
        start: [f64; 2],
        end: [f64; 2],
        via: &[[f64; 2]],
        date: &str,
    ) -> Result<(), String> {
        self.refuse_while_running()?;
        let river = self
            .reach_memo()
            .river
            .clone()
            .ok_or("choose a river from the search first")?;
        let plan = river.0.cut(start, end, via).map_err(err)?;
        self.start_reach_with(plan, Some(river), date)
    }

    /// True while the study holds nothing but its reach: re-cutting it then loses no work, so the
    /// app need not ask before it.
    pub fn reach_route_only(&self) -> bool {
        self.reach_memo().plan.is_some()
            && self.reach_memo().banks.is_none()
            && self.session().document()["provenance"]
                .as_object()
                .is_none_or(|p| p.keys().all(|k| k == "route"))
    }

    /// Record a planned reach in a new, unsaved design named after the river. The JRC tiles given
    /// so far are kept.
    pub fn start_reach(&mut self, plan: Planned, date: &str) -> Result<(), String> {
        self.start_reach_with(plan, None, date)
    }

    fn start_reach_with(
        &mut self,
        plan: Planned,
        river: Option<(WholeRiver, Option<String>)>,
        date: &str,
    ) -> Result<(), String> {
        self.refuse_while_running()?;
        let title = plan
            .spec
            .river
            .clone()
            .unwrap_or_else(|| "Reach study".to_string());
        let tiles = std::mem::take(&mut self.reach_memo_mut().tiles);
        let mut next = AppState::new_design(&title, date);
        let recorded = match &river {
            Some((r, search)) => {
                record_river_reach(next.session_mut(), &plan, r.osm, search.as_deref(), date)
            }
            None => record_reach(next.session_mut(), &plan, date),
        };
        if let Err(e) = recorded {
            self.reach_memo_mut().tiles = tiles;
            return Err(err(e));
        }
        *next.reach_memo_mut() = ReachMemo {
            plan: Some(plan),
            river,
            banks: None,
            tiles,
        };
        next.set_dirty();
        *self = next;
        Ok(())
    }

    /// Give a JRC occurrence tile the user downloaded (the web has no other way to read it).
    pub fn reach_add_tile(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        if !name.starts_with("occurrence_") || !name.ends_with(".tif") {
            return Err(format!(
                "`{name}` is not a JRC occurrence tile (occurrence_<lon>_<lat>v1_4_2021.tif)"
            ));
        }
        let memo = self.reach_memo_mut();
        memo.tiles.retain(|(n, _)| n != name);
        memo.tiles.push((name.to_string(), Arc::new(bytes)));
        Ok(())
    }

    /// Get a step ready to run on a copy of the session.
    pub fn begin_reach(&self, step: ReachStep) -> Result<ReachJob, String> {
        self.refuse_while_running()?;
        Ok(ReachJob {
            step,
            session: self.session().clone(),
            memo: self.reach_memo().clone(),
            dem: self.usable_dem_for_reach(),
            base: self.session().document().clone(),
        })
    }

    /// Keep a step's result, unless the design changed while it ran.
    pub fn keep_reach(&mut self, job: ReachJob) -> Result<(), String> {
        if self.session().document() != &job.base {
            return Err("the design changed while the step ran: run it again".into());
        }
        self.replace_session(job.session);
        *self.reach_memo_mut() = job.memo;
        self.set_dirty();
        Ok(())
    }

    /// Give the depth of a river with no survey (a bed model): `points` are `[chainage, depth]`.
    pub fn set_bed(
        &mut self,
        points: &[[f64; 2]],
        season: Option<&str>,
        date: &str,
    ) -> Result<(), String> {
        self.refuse_while_running()?;
        let mut s = self.session().clone();
        s.set_bed_model(points, season.filter(|t| !t.trim().is_empty()), date)
            .map_err(err)?;
        self.replace_session(s);
        self.set_dirty();
        Ok(())
    }

    /// Answer "what fits" over every vessel class: plan only, or full (with depth and the verdict)
    /// when the design has a bed model, a water level and its DEM.
    pub fn run_fits(&mut self) -> Result<(), String> {
        self.refuse_while_running()?;
        let mut s = self.session().clone();
        let dem = self.usable_dem_for_reach();
        let epsg = s.document()["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32);
        let sampler = match dem.as_deref() {
            Some(d) => Some(owd_format::design::terrain::dem_sampler(d, epsg).map_err(err)?),
            None => None,
        };
        s.run_fits_with(
            None,
            sampler
                .as_ref()
                .map(|f| f as &(dyn Fn(&[f64], &[f64]) -> Vec<f64> + Sync)),
        )
        .map_err(err)?;
        self.replace_session(s);
        self.set_dirty();
        Ok(())
    }

    /// What the view shows of the reach besides the document: the planned route as a line, the
    /// JRC tiles it needs (with the ones given) and the DEM tiles, as download links.
    pub fn reach_view(&self) -> Value {
        let memo = self.reach_memo();
        let Some(plan) = &memo.plan else {
            return json!({"route": null, "jrc_tiles": [], "dem_tiles": [], "has_mask_banks": false, "fits_stretches": self.fits_stretches()});
        };
        let line = plan.route.line();
        let given: Vec<&str> = memo.tiles.iter().map(|(n, _)| n.as_str()).collect();
        let jrc: Vec<Value> = window_bbox(&line, &BankParams::default())
            .map(owd_reach::jrc::tiles_for)
            .unwrap_or_default()
            .into_iter()
            .map(|t| json!({"name": t.file_name(), "url": t.url(), "given": given.contains(&t.file_name().as_str())}))
            .collect();
        let dem: Vec<Value> = owd_reach::geo::bbox(&line)
            .map(|b| owd_reach::dem::download_list(b, 1000.0))
            .unwrap_or_default()
            .into_iter()
            .map(|(name, url)| json!({"name": name, "url": url}))
            .collect();
        json!({
            "fits_stretches": self.fits_stretches(),
            "route": line.iter().map(|p| json!([p[0], p[1]])).collect::<Vec<_>>(),
            "length_m": plan.route.length_m,
            "as_drawn": plan.route.as_drawn_count(),
            "jrc_tiles": jrc,
            "dem_tiles": dem,
            "has_mask_banks": memo.banks.is_some(),
            "start": plan.spec.start,
            "end": plan.spec.end,
            "river": memo.river.as_ref().map(|(r, _)| json!({
                "osm_id": r.osm.to_string(),
                "name": r.name,
                "length_m": r.route.length_m,
                "line": r.route.line().iter().map(|p| json!([p[0], p[1]])).collect::<Vec<_>>(),
            })),
        })
    }

    /// The constraint stretches of the "what fits" answer, for the map: by ruleset and vessel
    /// class, each with its kind and its two ends in longitude and latitude.
    fn fits_stretches(&self) -> Value {
        use owd_format::design::crs::Crs;
        let doc = self.session().document();
        let utm = doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        let (Some(utm), Some(rulesets)) = (utm, doc["fits"]["results"]["rule_sets"].as_array())
        else {
            return json!({});
        };
        let ll = |p: &Value| -> Value {
            match (p[0].as_f64(), p[1].as_f64()) {
                (Some(x), Some(y)) => utm.inverse(x, y).map_or(Value::Null, |q| json!(q)),
                _ => Value::Null,
            }
        };
        let mut out = serde_json::Map::new();
        for rs in rulesets {
            let mut classes = serde_json::Map::new();
            for c in rs["classes"].as_array().into_iter().flatten() {
                let list: Vec<Value> = c["stretches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|st| json!({"kind": st["kind"], "from": ll(&st["start"]), "to": ll(&st["end"])}))
                    .collect();
                classes.insert(
                    c["vessel_standard_id"].as_str().unwrap_or("").to_string(),
                    json!(list),
                );
            }
            out.insert(
                rs["rule_set"].as_str().unwrap_or("").to_string(),
                Value::Object(classes),
            );
        }
        Value::Object(out)
    }
}
