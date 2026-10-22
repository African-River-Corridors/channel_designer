//! The source registry (ADR 0010): one entry per open data source, with its URL pattern, licence,
//! the notice it requires, and whether a browser page can read it (CORS). A new source is an entry
//! and a reader; its licence is checked before it lands.
//!
//! Not listed on purpose: HydroSHEDS (fetched at run time, never bundled, and not in v1), and any
//! imagery with non-open terms. Banks taken from OSM would stay a separate layer under ODbL; v1 keeps
//! only the OSM way ids in the document.

/// One data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// A stable id, written into the document's provenance.
    pub id: &'static str,
    pub name: &'static str,
    /// The URL or URL template; `{...}` marks a part the reader fills in.
    pub url_pattern: &'static str,
    pub licence: &'static str,
    /// The notice shown on the map and carried in every export. `{year}` is the year of the data.
    pub notice: &'static str,
    /// True when a browser page can read it. A source it cannot read is offered as plain download
    /// links on the web and never proxied.
    pub browser_readable: bool,
}

impl Source {
    /// Why this entry is not allowed in the registry, if it is not.
    pub fn problem(&self) -> Option<String> {
        let blank = |s: &str| s.trim().is_empty();
        if blank(self.id) || blank(self.name) || blank(self.url_pattern) {
            return Some(format!(
                "source `{}` needs an id, a name and a URL",
                self.id
            ));
        }
        if blank(self.licence) {
            return Some(format!("source `{}` has no licence", self.id));
        }
        if blank(self.notice) {
            return Some(format!("source `{}` has no notice", self.id));
        }
        None
    }

    /// The notice with `{year}` filled in from a date (`YYYY...`). Without a year the placeholder is
    /// dropped with the word "in" never added: the notice reads without a year.
    pub fn notice_for(&self, date: &str) -> String {
        let year: String = date.chars().take(4).collect();
        if year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) {
            self.notice.replace("{year}", &year)
        } else {
            self.notice.replace(" {year}", "").replace("{year}", "")
        }
    }
}

const REGISTRY: &[Source] = &[
    Source {
        id: "osm-overpass",
        name: "OpenStreetMap waterways (Overpass)",
        url_pattern: "https://overpass-api.de/api/interpreter?data={query}",
        licence: "ODbL 1.0",
        notice: "\u{a9} OpenStreetMap contributors",
        browser_readable: true,
    },
    Source {
        id: "osm-photon",
        name: "OpenStreetMap river search (Photon)",
        url_pattern: "https://photon.komoot.io/api/?q={text}",
        licence: "ODbL 1.0",
        notice: "\u{a9} OpenStreetMap contributors",
        browser_readable: true,
    },
    Source {
        id: "jrc-gsw",
        name: "JRC Global Surface Water, occurrence",
        url_pattern: "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_{lon}_{lat}v1_4_2021.tif",
        licence: "Copernicus, free and open; cite Pekel et al. 2016",
        notice: "Source: EC JRC/Google. Pekel et al. 2016, Nature 540, 418-422",
        browser_readable: false,
    },
    Source {
        id: "sentinel2-l2a",
        name: "Sentinel-2 L2A (Earth Search STAC)",
        url_pattern: "https://earth-search.aws.element84.com/v1/search",
        licence: "Copernicus, free and open",
        notice: "Contains modified Copernicus Sentinel data {year}",
        browser_readable: true,
    },
    Source {
        id: "cop-dem-glo30",
        name: "Copernicus GLO-30 DEM",
        url_pattern: "https://copernicus-dem-30m.s3.eu-central-1.amazonaws.com/{tile}/{tile}.tif",
        licence: "Copernicus DEM licence, commercial use allowed",
        notice: owd_format::design::dem_source::COPERNICUS_NOTICE,
        browser_readable: false,
    },
    Source {
        id: "sword",
        name: "SWORD v17c river reaches (Zenodo shapefile release)",
        url_pattern: "https://zenodo.org/api/records/22259077",
        licence: "CC BY 4.0",
        notice: "SWORD v17c, Altenau et al. 2021, CC BY 4.0",
        browser_readable: true,
    },
    Source {
        id: "swot-hydrocron",
        name: "SWOT river levels (Hydrocron)",
        url_pattern: "https://soto.podaac.earthdatacloud.nasa.gov/hydrocron/v1/timeseries?feature={feature}&feature_id={id}",
        licence: "NASA open data",
        notice: "SWOT data: NASA and CNES, served by PO.DAAC Hydrocron",
        browser_readable: true,
    },
];

/// Every source, in a fixed order.
pub fn registry() -> &'static [Source] {
    REGISTRY
}

/// One source by id.
pub fn source(id: &str) -> Option<&'static Source> {
    REGISTRY.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn every_entry_has_a_licence_and_a_notice() {
        for s in registry() {
            assert_eq!(s.problem(), None, "{s:?}");
        }
    }

    #[test]
    fn ids_are_unique_and_found_by_lookup() {
        for (i, s) in registry().iter().enumerate() {
            assert_eq!(source(s.id), Some(s));
            assert!(registry()[i + 1..].iter().all(|o| o.id != s.id), "{}", s.id);
        }
        assert!(source("nope").is_none());
    }

    #[test]
    fn an_entry_without_a_licence_or_notice_is_refused() {
        let ok = *source("osm-overpass").unwrap();
        assert!(
            Source { licence: " ", ..ok }
                .problem()
                .unwrap()
                .contains("licence")
        );
        assert!(
            Source { notice: "", ..ok }
                .problem()
                .unwrap()
                .contains("notice")
        );
        assert!(Source { id: "", ..ok }.problem().is_some());
    }

    #[test]
    fn no_hydrosheds_and_the_cors_split_is_as_the_grill_says() {
        assert!(
            registry()
                .iter()
                .all(|s| !s.id.to_lowercase().contains("hydro") || s.id == "swot-hydrocron")
        );
        // Readable from a page: Overpass, Photon, Earth Search, Hydrocron, SWORD on Zenodo. Not: DEM, JRC.
        for (id, cors) in [
            ("osm-overpass", true),
            ("osm-photon", true),
            ("sentinel2-l2a", true),
            ("swot-hydrocron", true),
            ("sword", true),
            ("cop-dem-glo30", false),
            ("jrc-gsw", false),
        ] {
            assert_eq!(source(id).unwrap().browser_readable, cors, "{id}");
        }
    }

    #[test]
    fn the_year_fills_the_sentinel_notice() {
        let s = source("sentinel2-l2a").unwrap();
        assert_eq!(
            s.notice_for("2025-02-14"),
            "Contains modified Copernicus Sentinel data 2025"
        );
        assert_eq!(
            s.notice_for("unknown"),
            "Contains modified Copernicus Sentinel data"
        );
        assert_eq!(
            source("osm-overpass").unwrap().notice_for("2026-01-01"),
            "\u{a9} OpenStreetMap contributors"
        );
    }
}
