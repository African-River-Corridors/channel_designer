//! River search with a list to choose from (spec 0012, T13): the rivers and canals whose name starts
//! with or matches the text, one hit per OSM object, each with its region and country. Two rivers
//! with one name stay two hits, so the user picks the right one.
//!
//! The geocoder is Photon (OSM data, built for search as you type). The Nominatim policy forbids
//! type-ahead on its public API, so it is not used for the list; when Photon fails, the user may ask
//! Nominatim once, by pressing Enter ([`RiverSearch::search_once`]). Only this module knows the
//! geocoders: a change of provider touches nothing else. The only network access is the `Fetch` it
//! is given.

use crate::error::ReachError;
use crate::fetch::Fetch;
use crate::overpass::urlencode;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// The public Photon server.
pub const PHOTON_ENDPOINT: &str = "https://photon.komoot.io/api/";
/// The public Nominatim server, asked once on Enter when Photon fails.
pub const NOMINATIM_ENDPOINT: &str = "https://nominatim.openstreetmap.org/search";
/// The fewest letters worth a request: fewer match too much.
pub const MIN_CHARS: usize = 3;
/// How long the app waits after the last key before it asks, in milliseconds.
pub const DEBOUNCE_MS: u32 = 350;
/// The most hits asked for.
pub const LIMIT: usize = 10;
/// Way hits of one river closer than this, in degrees (about 2 km), are one hit.
const JOIN_DEG: f64 = 0.02;

/// An OSM waterway: a relation or a way, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OsmRef {
    Relation(i64),
    Way(i64),
}

impl fmt::Display for OsmRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OsmRef::Relation(id) => write!(f, "relation/{id}"),
            OsmRef::Way(id) => write!(f, "way/{id}"),
        }
    }
}

impl FromStr for OsmRef {
    type Err = ReachError;

    /// `relation/3328713` or `way/42`, as OSM writes them; `r3328713` and `w42` also do.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || {
            ReachError::Input(format!(
                "`{s}` is not an OSM waterway (for example relation/3328713 or way/42)"
            ))
        };
        let t = s.trim().to_ascii_lowercase();
        let (kind, id) = match t.split_once('/') {
            Some(p) => p,
            None if t.len() > 1 => t.split_at(1),
            None => return Err(bad()),
        };
        let id: i64 = id.parse().map_err(|_| bad())?;
        if id <= 0 {
            return Err(bad());
        }
        match kind {
            "relation" | "r" => Ok(OsmRef::Relation(id)),
            "way" | "w" => Ok(OsmRef::Way(id)),
            _ => Err(bad()),
        }
    }
}

/// One river found: what the list shows, and the OSM object to route.
#[derive(Debug, Clone, PartialEq)]
pub struct RiverHit {
    pub name: String,
    /// The OSM waterway kind: `river` or `canal`.
    pub kind: String,
    pub region: Option<String>,
    pub country: Option<String>,
    /// ISO 3166-1 alpha-2, upper case.
    pub country_code: Option<String>,
    pub osm: OsmRef,
    /// Other ways of the same river the geocoder listed apart (a river with no relation).
    pub also: Vec<OsmRef>,
    /// `[west, south, east, north]`.
    pub bbox: [f64; 4],
}

impl RiverHit {
    /// "Ankobra River — Western Region, Ghana": the name, then what of the region and country is known.
    pub fn label(&self) -> String {
        let place: Vec<&str> = [self.region.as_deref(), self.country.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if place.is_empty() {
            self.name.clone()
        } else {
            format!("{} \u{2014} {}", self.name, place.join(", "))
        }
    }
}

/// River search over a `Fetch`, with every answer cached by request.
pub struct RiverSearch<'a> {
    fetch: &'a dyn Fetch,
    endpoint: String,
    cache: RefCell<HashMap<String, Vec<RiverHit>>>,
}

/// A country filter as Photon takes it: two letters, upper case.
fn country_code(cc: &str) -> Result<String, ReachError> {
    let c = cc.trim();
    if c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()) {
        Ok(c.to_ascii_uppercase())
    } else {
        Err(ReachError::Input(format!(
            "`{cc}` is not a two-letter country code (for example GH)"
        )))
    }
}

impl<'a> RiverSearch<'a> {
    pub fn new(fetch: &'a dyn Fetch) -> Self {
        Self::with_endpoint(fetch, PHOTON_ENDPOINT)
    }

    /// Use another Photon server.
    pub fn with_endpoint(fetch: &'a dyn Fetch, endpoint: &str) -> Self {
        RiverSearch {
            fetch,
            endpoint: endpoint.to_string(),
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// The request URL for a text and an optional country (`GH`).
    pub fn url(&self, text: &str, country: Option<&str>) -> Result<String, ReachError> {
        let mut u = format!(
            "{}?q={}&osm_tag=waterway:river&osm_tag=waterway:canal&limit={LIMIT}&lang=en",
            self.endpoint,
            urlencode(text.trim())
        );
        if let Some(c) = country.map(str::trim).filter(|c| !c.is_empty()) {
            u.push_str(&format!("&countrycode={}", country_code(c)?));
        }
        Ok(u)
    }

    /// The Nominatim request URL for a text and an optional country.
    pub fn nominatim_url(
        &self,
        endpoint: &str,
        text: &str,
        country: Option<&str>,
    ) -> Result<String, ReachError> {
        let mut u = format!(
            "{endpoint}?q={}&format=jsonv2&addressdetails=1&limit=20&accept-language=en",
            urlencode(text.trim())
        );
        if let Some(c) = country.map(str::trim).filter(|c| !c.is_empty()) {
            u.push_str(&format!(
                "&countrycodes={}",
                country_code(c)?.to_ascii_lowercase()
            ));
        }
        Ok(u)
    }

    /// One Nominatim search, for when Photon fails: the user asks for it (Enter), never as they
    /// type. Same hits and cache as [`RiverSearch::search`].
    pub fn search_once(
        &self,
        endpoint: &str,
        text: &str,
        country: Option<&str>,
    ) -> Result<Vec<RiverHit>, ReachError> {
        if text.trim().chars().count() < MIN_CHARS {
            return Ok(Vec::new());
        }
        let url = self.nominatim_url(endpoint, text, country)?;
        if let Some(hits) = self.cache.borrow().get(&url) {
            return Ok(hits.clone());
        }
        let hits = parse_nominatim(&self.fetch.get(&url)?)?;
        self.cache.borrow_mut().insert(url, hits.clone());
        Ok(hits)
    }

    /// The rivers and canals for a text, best first. Under [`MIN_CHARS`] letters it asks nothing
    /// and finds nothing. A text asked before is answered from the cache.
    pub fn search(&self, text: &str, country: Option<&str>) -> Result<Vec<RiverHit>, ReachError> {
        if text.trim().chars().count() < MIN_CHARS {
            return Ok(Vec::new());
        }
        let url = self.url(text, country)?;
        if let Some(hits) = self.cache.borrow().get(&url) {
            return Ok(hits.clone());
        }
        let hits = parse_photon(&self.fetch.get(&url)?)?;
        self.cache.borrow_mut().insert(url, hits.clone());
        Ok(hits)
    }
}

fn near(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] + JOIN_DEG
        && b[0] <= a[2] + JOIN_DEG
        && a[1] <= b[3] + JOIN_DEG
        && b[1] <= a[3] + JOIN_DEG
}

/// Hits from a Photon answer, in its order. Ways of one river (same name, kind, region and
/// country, touching) are one hit. A feature that is not a waterway relation or way is skipped.
pub fn parse_photon(body: &[u8]) -> Result<Vec<RiverHit>, ReachError> {
    let v: Value = serde_json::from_slice(body).map_err(|e| {
        ReachError::Parse(format!(
            "the river search answer is not JSON ({e}); the server may be busy"
        ))
    })?;
    let features = v["features"]
        .as_array()
        .ok_or_else(|| ReachError::Parse("the river search answer has no `features`".into()))?;
    let mut out: Vec<RiverHit> = Vec::new();
    for f in features {
        let p = &f["properties"];
        let s = |k: &str| p[k].as_str().map(str::to_string);
        let (Some(name), Some(id)) = (s("name"), p["osm_id"].as_i64()) else {
            continue;
        };
        if p["osm_key"] != "waterway" {
            continue;
        }
        let osm = match p["osm_type"].as_str() {
            Some("R") => OsmRef::Relation(id),
            Some("W") => OsmRef::Way(id),
            _ => continue,
        };
        // Photon's extent is [west, north, east, south]; with none, the point.
        let num = |x: &Value| x.as_f64();
        let ext: Option<Vec<f64>> = p["extent"]
            .as_array()
            .and_then(|a| a.iter().map(num).collect());
        let pt: Option<Vec<f64>> = f["geometry"]["coordinates"]
            .as_array()
            .and_then(|a| a.iter().map(num).collect());
        let bbox = match (ext.as_deref(), pt.as_deref()) {
            (Some([w, n, e, s]), _) => [w.min(*e), s.min(*n), w.max(*e), s.max(*n)],
            (_, Some([x, y])) => [*x, *y, *x, *y],
            _ => continue,
        };
        let hit = RiverHit {
            name,
            kind: s("osm_value").unwrap_or_default(),
            region: s("state"),
            country: s("country"),
            country_code: s("countrycode").map(|c| c.to_ascii_uppercase()),
            osm,
            also: Vec::new(),
            bbox,
        };
        let same = out.iter_mut().find(|h| {
            matches!((h.osm, hit.osm), (OsmRef::Way(_), OsmRef::Way(_)))
                && h.name == hit.name
                && h.kind == hit.kind
                && h.region == hit.region
                && h.country_code == hit.country_code
                && near(&h.bbox, &hit.bbox)
        });
        match same {
            Some(h) => {
                h.also.push(hit.osm);
                h.bbox = [
                    h.bbox[0].min(hit.bbox[0]),
                    h.bbox[1].min(hit.bbox[1]),
                    h.bbox[2].max(hit.bbox[2]),
                    h.bbox[3].max(hit.bbox[3]),
                ];
            }
            None => out.push(hit),
        }
    }
    Ok(out)
}

/// Hits from a Nominatim `jsonv2` answer: only waterway rivers and canals, in its order.
pub fn parse_nominatim(body: &[u8]) -> Result<Vec<RiverHit>, ReachError> {
    let v: Value = serde_json::from_slice(body).map_err(|e| {
        ReachError::Parse(format!(
            "the river search answer is not JSON ({e}); the server may be busy"
        ))
    })?;
    let rows = v
        .as_array()
        .ok_or_else(|| ReachError::Parse("the river search answer is not a list".into()))?;
    let mut out = Vec::new();
    for r in rows {
        let kind = r["type"].as_str().unwrap_or_default();
        if r["category"] != "waterway" || !matches!(kind, "river" | "canal") {
            continue;
        }
        let Some(id) = r["osm_id"].as_i64() else {
            continue;
        };
        let osm = match r["osm_type"].as_str() {
            Some("relation") => OsmRef::Relation(id),
            Some("way") => OsmRef::Way(id),
            _ => continue,
        };
        // Nominatim's box is [south, north, west, east], as strings.
        let b: Option<Vec<f64>> = r["boundingbox"]
            .as_array()
            .and_then(|a| a.iter().map(|x| x.as_str()?.parse().ok()).collect());
        let Some([s, n, w, e]) = b.as_deref().map(|b| [b[0], b[1], b[2], b[3]]) else {
            continue;
        };
        let a = &r["address"];
        let s_ = |k: &str| a[k].as_str().map(str::to_string);
        out.push(RiverHit {
            name: r["name"].as_str().unwrap_or_default().to_string(),
            kind: kind.to_string(),
            region: s_("state"),
            country: s_("country"),
            country_code: s_("country_code").map(|c| c.to_ascii_uppercase()),
            osm,
            also: Vec::new(),
            bbox: [w, s, e, n],
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! Label: sourced - answers recorded from photon.komoot.io on 2026-10-22
    //! (`examples/reach/photon_*.json`, OSM data, © OpenStreetMap contributors).
    use super::*;
    use crate::fetch::ReplayFetch;

    const ANK: &str = include_str!("../../../examples/reach/photon_ank.json");
    const BLACK: &str = include_str!("../../../examples/reach/photon_black_river.json");

    #[test]
    fn three_letters_find_the_ankobra_with_its_region_and_country() {
        let f = ReplayFetch::new();
        let url = RiverSearch::new(&f).url("Ank", None).unwrap();
        let f = ReplayFetch::new().with_response(&url, ANK);
        let hits = RiverSearch::new(&f).search("Ank", None).unwrap();
        let ank = hits
            .iter()
            .find(|h| h.osm == OsmRef::Relation(3328713))
            .expect("the Ankobra is listed");
        assert_eq!(ank.label(), "Ankobra River \u{2014} Western Region, Ghana");
        assert_eq!(ank.country_code.as_deref(), Some("GH"));
        assert_eq!(ank.kind, "river");
        assert_eq!(ank.bbox, [-2.2745332, 4.8983929, -2.1050123, 5.7297061]);
    }

    #[test]
    fn rivers_with_one_name_stay_apart_and_ways_of_one_river_join() {
        let hits = parse_photon(BLACK.as_bytes()).unwrap();
        let black: Vec<&RiverHit> = hits.iter().filter(|h| h.name == "Black River").collect();
        let countries: std::collections::BTreeSet<_> = black
            .iter()
            .filter_map(|h| h.country_code.as_deref())
            .collect();
        assert!(countries.len() >= 4, "{countries:?}");
        assert!(black.len() >= 8, "{}", black.len());
        // The Ankona is two ways side by side in the recorded answer: one hit.
        let ank = parse_photon(ANK.as_bytes()).unwrap();
        let ankona: Vec<&RiverHit> = ank.iter().filter(|h| h.name == "Ankona").collect();
        assert_eq!(ankona.len(), 1);
        assert_eq!(ankona[0].osm, OsmRef::Way(1320715130));
        assert_eq!(ankona[0].also, vec![OsmRef::Way(1319230641)]);
    }

    #[test]
    fn the_country_filter_goes_to_the_server_and_a_bad_one_is_refused() {
        let f = ReplayFetch::new();
        let s = RiverSearch::new(&f);
        let u = s.url("Black River", Some("za")).unwrap();
        assert!(u.ends_with("&countrycode=ZA"), "{u}");
        assert!(u.contains("q=Black%20River"), "{u}");
        assert!(s.url("Black River", Some("ZAF")).is_err());
    }

    #[test]
    fn short_text_asks_nothing_and_a_repeat_is_cached() {
        let f = ReplayFetch::new();
        let url = RiverSearch::new(&f).url("Ank", None).unwrap();
        let f = ReplayFetch::new().with_response(&url, ANK);
        let s = RiverSearch::new(&f);
        assert!(s.search("An", None).unwrap().is_empty());
        assert!(f.requests().is_empty());
        let a = s.search("Ank", None).unwrap();
        let b = s.search(" Ank ", None).unwrap();
        assert_eq!(a, b);
        assert_eq!(f.requests().len(), 1);
    }

    #[test]
    fn an_error_page_is_a_clear_error() {
        let e = parse_photon(b"<html>502 Bad Gateway").unwrap_err();
        assert!(e.to_string().contains("not JSON"), "{e}");
        let e = parse_photon(b"{}").unwrap_err();
        assert!(e.to_string().contains("no `features`"), "{e}");
    }

    #[test]
    fn nominatim_once_finds_the_ankobra_and_keeps_only_waterways() {
        //! Label: sourced - `examples/reach/nominatim_ankobra.json`, recorded from
        //! nominatim.openstreetmap.org on 2026-10-22.
        const NOM: &str = include_str!("../../../examples/reach/nominatim_ankobra.json");
        let f = ReplayFetch::new();
        let url = RiverSearch::new(&f)
            .nominatim_url(NOMINATIM_ENDPOINT, "Ankobra", None)
            .unwrap();
        let f = ReplayFetch::new().with_response(&url, NOM);
        let s = RiverSearch::new(&f);
        let hits = s.search_once(NOMINATIM_ENDPOINT, "Ankobra", None).unwrap();
        // The answer also holds a village and a residential area: only the river is a hit.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].osm, OsmRef::Relation(3328713));
        assert_eq!(
            hits[0].label(),
            "Ankobra River \u{2014} Western Region, Ghana"
        );
        assert_eq!(hits[0].bbox, [-2.2745332, 4.8983929, -2.1050123, 5.7297061]);
        let again = s.search_once(NOMINATIM_ENDPOINT, "Ankobra", None).unwrap();
        assert_eq!(again, hits);
        assert_eq!(f.requests().len(), 1, "one request");
        let u = s
            .nominatim_url(NOMINATIM_ENDPOINT, "Ankobra", Some("GH"))
            .unwrap();
        assert!(u.ends_with("&countrycodes=gh"), "{u}");
    }

    #[test]
    fn osm_refs_read_and_print_as_osm_writes_them() {
        assert_eq!(
            "relation/3328713".parse::<OsmRef>().unwrap(),
            OsmRef::Relation(3328713)
        );
        assert_eq!("W42".parse::<OsmRef>().unwrap(), OsmRef::Way(42));
        assert_eq!(OsmRef::Way(42).to_string(), "way/42");
        for bad in ["node/1", "relation/x", "r", "relation/-3", ""] {
            assert!(bad.parse::<OsmRef>().is_err(), "{bad}");
        }
    }
}
