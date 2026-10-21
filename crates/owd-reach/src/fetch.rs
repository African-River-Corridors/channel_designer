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
    /// A replay has no recording for this request.
    NotRecorded(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Failed { url, reason } => write!(f, "cannot fetch {url}: {reason}"),
            FetchError::NotRecorded(r) => write!(f, "no recorded response for {r}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Get the bytes of a URL, or of a range of it. Blocking: the web runs it in the session Worker.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError>;
    /// `len` bytes from byte `start`. A host may return fewer at the end of the file.
    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError>;
}

/// Replays recorded responses by request. Remembers every request made, for tests.
#[derive(Default)]
pub struct ReplayFetch {
    full: HashMap<String, Vec<u8>>,
    ranges: HashMap<(String, u64, u64), Vec<u8>>,
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
}
