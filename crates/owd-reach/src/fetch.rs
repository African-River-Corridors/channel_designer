//! The one interface to the network (ADR 0010): get a URL, get a byte range. A native client and
//! the browser's `fetch` implement it; [`ReplayFetch`] replays recorded responses so no test in
//! `test` touches the network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

/// Why a fetch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The host answered with an error status, or the request did not complete.
    Failed { url: String, reason: String },
    /// The host has no such file (HTTP 404, or 403 where a bucket hides what it does not hold). The
    /// Copernicus DEM, for one, has no tiles over much of the sea.
    NotFound(String),
    /// A replay has no recording for this request.
    NotRecorded(String),
    /// The user pressed Cancel: the work stopped before this request.
    Cancelled,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Failed { url, reason } => write!(f, "cannot fetch {url}: {reason}"),
            FetchError::NotFound(url) => write!(f, "the source has no file at {url}"),
            FetchError::NotRecorded(r) => write!(f, "no recorded response for {r}"),
            FetchError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Get the bytes of a URL, or of a range of it. Blocking: the web runs it in the session Worker.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError>;
    /// `len` bytes from byte `start`. A host may return fewer at the end of the file.
    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError>;
    /// What the work is doing now, for a progress bar and status line (#320). Work with counted
    /// steps (DEM tiles, SWORD reaches) says so; every request is reported by [`Reporting`].
    fn progress(&self, _step: &Step) {}
}

/// One report of progress: step `done` of `total` (`total` 0 when the count is not known), and
/// what is being done, in words.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub done: usize,
    pub total: usize,
    pub what: String,
}

impl Step {
    pub fn new(done: usize, total: usize, what: impl Into<String>) -> Step {
        Step {
            done,
            total,
            what: what.into(),
        }
    }

    /// "fetching DEM tile 5 of 18", or the words alone when there is no count.
    pub fn line(&self) -> String {
        if self.total > 0 {
            format!("{} {} of {}", self.what, self.done, self.total)
        } else {
            self.what.clone()
        }
    }
}

/// What a request to this URL is doing, in words, from its host.
pub fn describe(url: &str) -> &'static str {
    let host = url
        .split_once("://")
        .map_or(url, |(_, r)| r)
        .split('/')
        .next()
        .unwrap_or("");
    let has = |s: &str| host.contains(s) || url.contains(s);
    if has("overpass") {
        "searching OpenStreetMap waterways"
    } else if has("photon") || has("nominatim") {
        "searching river names"
    } else if has("earth-search") || has("stac") {
        "finding a dry-season Sentinel-2 scene"
    } else if has("sentinel-cogs") || has("sentinel-s2") {
        "reading Sentinel-2 imagery"
    } else if has("global-surface-water") || has("occurrence_") {
        "reading the JRC water mask"
    } else if has("zenodo") {
        "reading SWORD river reaches"
    } else if has("hydrocron") || has("podaac") {
        "reading SWOT water levels"
    } else if has("copernicus-dem") {
        "fetching the Copernicus DEM"
    } else {
        "reading open data"
    }
}

/// A `Fetch` that reports every request to `report`, and stops with [`FetchError::Cancelled`]
/// before the next request once `stop` says so. Requests are counted per kind of work.
pub struct Reporting<'a> {
    inner: &'a dyn Fetch,
    report: &'a dyn Fn(&Step),
    stop: &'a dyn Fn() -> bool,
    count: std::cell::Cell<usize>,
    kind: std::cell::Cell<&'static str>,
}

impl<'a> Reporting<'a> {
    pub fn new(
        inner: &'a dyn Fetch,
        report: &'a dyn Fn(&Step),
        stop: &'a dyn Fn() -> bool,
    ) -> Reporting<'a> {
        Reporting {
            inner,
            report,
            stop,
            count: std::cell::Cell::new(0),
            kind: std::cell::Cell::new(""),
        }
    }

    fn before(&self, url: &str) -> Result<(), FetchError> {
        if (self.stop)() {
            return Err(FetchError::Cancelled);
        }
        let kind = describe(url);
        if kind != self.kind.get() {
            self.kind.set(kind);
            self.count.set(0);
        }
        self.count.set(self.count.get() + 1);
        (self.report)(&Step::new(
            0,
            0,
            format!("{kind} (request {})", self.count.get()),
        ));
        Ok(())
    }
}

impl Fetch for Reporting<'_> {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        self.before(url)?;
        self.inner.get(url)
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        self.before(url)?;
        self.inner.get_range(url, start, len)
    }

    fn progress(&self, step: &Step) {
        (self.report)(step);
    }
}

/// Replays recorded responses by request. Remembers every request made, for tests.
#[derive(Default)]
pub struct ReplayFetch {
    full: HashMap<String, Vec<u8>>,
    ranges: HashMap<(String, u64, u64), Vec<u8>>,
    containing: Vec<(String, Vec<u8>)>,
    log: RefCell<Vec<String>>,
}

impl ReplayFetch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the response to `get(url)`. A recorded whole file also answers any range of it.
    pub fn with_response(mut self, url: &str, body: impl Into<Vec<u8>>) -> Self {
        self.full.insert(url.to_string(), body.into());
        self
    }

    /// Record a response for any `get(url)` whose URL contains `part` (for a query whose exact URL
    /// depends on a computed bounding box). An exact recording wins.
    pub fn with_response_containing(mut self, part: &str, body: impl Into<Vec<u8>>) -> Self {
        self.containing.push((part.to_string(), body.into()));
        self
    }

    /// Record the response to one `get_range(url, start, len)`.
    pub fn with_range(mut self, url: &str, start: u64, len: u64, body: impl Into<Vec<u8>>) -> Self {
        self.ranges
            .insert((url.to_string(), start, len), body.into());
        self
    }

    /// The requests made so far, in order, as `GET url` or `GET url bytes=start+len`.
    pub fn requests(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

impl Fetch for ReplayFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        self.log.borrow_mut().push(format!("GET {url}"));
        self.full
            .get(url)
            .cloned()
            .or_else(|| {
                self.containing
                    .iter()
                    .find(|(part, _)| url.contains(part.as_str()))
                    .map(|(_, b)| b.clone())
            })
            .ok_or_else(|| FetchError::NotRecorded(format!("GET {url}")))
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        let what = format!("GET {url} bytes={start}+{len}");
        self.log.borrow_mut().push(what.clone());
        if let Some(b) = self.ranges.get(&(url.to_string(), start, len)) {
            return Ok(b.clone());
        }
        match self.full.get(url) {
            Some(b) => {
                let from = (start as usize).min(b.len());
                let to = from.saturating_add(len as usize).min(b.len());
                Ok(b[from..to].to_vec())
            }
            None => Err(FetchError::NotRecorded(what)),
        }
    }
}

/// Files the user downloaded and dropped in, served by the file name at the end of the URL. This is
/// how a source with no CORS header (JRC, the Copernicus DEM) is read on the web, and how the CLI reads
/// a tile it was given. A URL with no file says which one to fetch.
#[derive(Default)]
pub struct DroppedFiles {
    files: Vec<(String, Vec<u8>)>,
}

impl DroppedFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file by its name (for example `occurrence_10W_10Nv1_4_2021.tif`).
    pub fn with_file(mut self, name: &str, bytes: Vec<u8>) -> Self {
        self.files.push((name.to_string(), bytes));
        self
    }

    fn find(&self, url: &str) -> Result<&[u8], FetchError> {
        let name = url.rsplit('/').next().unwrap_or(url);
        self.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_slice())
            .ok_or_else(|| FetchError::Failed {
                url: url.to_string(),
                reason: format!(
                    "the file `{name}` was not given: download it from that address and add it"
                ),
            })
    }
}

impl Fetch for DroppedFiles {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        self.find(url).map(<[u8]>::to_vec)
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        let b = self.find(url)?;
        let from = (start as usize).min(b.len());
        let to = from.saturating_add(len as usize).min(b.len());
        Ok(b[from..to].to_vec())
    }
}

#[cfg(test)]
mod reporting_tests {
    //! Label: property
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn every_request_is_reported_by_its_source_and_cancel_stops_the_next() {
        let f = ReplayFetch::new()
            .with_response_containing("overpass", b"{}".to_vec())
            .with_response(
                "https://sentinel-cogs.s3.us-west-2.amazonaws.com/a/B04.tif",
                b"x".to_vec(),
            )
            .with_response(
                "https://sentinel-cogs.s3.us-west-2.amazonaws.com/a/B08.tif",
                b"x".to_vec(),
            );
        let seen = RefCell::new(Vec::new());
        let stop = std::cell::Cell::new(false);
        let report = |s: &Step| seen.borrow_mut().push(s.line());
        let halt = || stop.get();
        let r = Reporting::new(&f, &report, &halt);
        r.get("https://overpass-api.de/api/interpreter?data=x")
            .unwrap();
        r.get_range(
            "https://sentinel-cogs.s3.us-west-2.amazonaws.com/a/B04.tif",
            0,
            1,
        )
        .unwrap();
        r.get_range(
            "https://sentinel-cogs.s3.us-west-2.amazonaws.com/a/B08.tif",
            0,
            1,
        )
        .unwrap();
        r.progress(&Step::new(3, 12, "SWOT level for SWORD reach"));
        assert_eq!(
            *seen.borrow(),
            vec![
                "searching OpenStreetMap waterways (request 1)",
                "reading Sentinel-2 imagery (request 1)",
                "reading Sentinel-2 imagery (request 2)",
                "SWOT level for SWORD reach 3 of 12",
            ]
        );
        stop.set(true);
        assert_eq!(
            r.get("https://overpass-api.de/x").unwrap_err(),
            FetchError::Cancelled
        );
        assert_eq!(f.requests().len(), 3, "nothing is asked after Cancel");
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn a_replay_answers_recorded_requests_and_refuses_the_rest() {
        let f = ReplayFetch::new()
            .with_response("https://a/x", b"0123456789".to_vec())
            .with_range("https://b/y", 4, 2, b"zz".to_vec());
        assert_eq!(f.get("https://a/x").unwrap(), b"0123456789");
        assert_eq!(f.get_range("https://a/x", 3, 4).unwrap(), b"3456");
        assert_eq!(f.get_range("https://a/x", 8, 10).unwrap(), b"89");
        assert_eq!(f.get_range("https://b/y", 4, 2).unwrap(), b"zz");
        assert!(matches!(
            f.get("https://c/z"),
            Err(FetchError::NotRecorded(_))
        ));
        assert!(f.get_range("https://b/y", 0, 2).is_err());
        assert_eq!(f.requests().len(), 6);
        assert_eq!(f.requests()[0], "GET https://a/x");
    }

    #[test]
    fn dropped_files_answer_by_name_and_a_missing_one_says_what_to_download() {
        let d = DroppedFiles::new().with_file("a.tif", b"0123456789".to_vec());
        assert_eq!(d.get("https://host/dir/a.tif").unwrap(), b"0123456789");
        assert_eq!(d.get_range("https://other/a.tif", 2, 3).unwrap(), b"234");
        let e = d.get("https://host/dir/b.tif").unwrap_err().to_string();
        assert!(
            e.contains("b.tif") && e.contains("https://host/dir/b.tif"),
            "{e}"
        );
    }
}
