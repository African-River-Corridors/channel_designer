//! The native [`owd_reach::Fetch`]: HTTPS through `ureq`, as `owd fetch-dem` does. The web gives
//! `owd-reach` the browser's `fetch` instead.

use owd_reach::{Fetch, FetchError};

/// Refuse a response larger than this (a Sentinel-2 or JRC window is far smaller).
const MAX_BYTES: u64 = 400_000_000;

/// Overpass and other public hosts ask clients to say who they are.
const USER_AGENT: &str = concat!(
    "open-waterway-designer/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/African-River-Corridors/open-waterway-designer)"
);

/// Native HTTP.
pub struct HttpFetch;

/// Progress on stderr (#320): every counted step, and the first request of each kind of work and
/// every 25th after it, so a long run says what it is doing without a flood of lines.
pub fn stderr_reporter() -> impl Fn(&owd_reach::Step) {
    let last = std::cell::RefCell::new(String::new());
    let count = std::cell::Cell::new(0usize);
    move |s: &owd_reach::Step| {
        let kind = s.what.split(" (request").next().unwrap_or("").to_string();
        let new_kind = *last.borrow() != kind;
        if new_kind {
            *last.borrow_mut() = kind;
            count.set(0);
        }
        count.set(count.get() + 1);
        if s.total > 0 || new_kind || count.get().is_multiple_of(25) {
            eprintln!("  {}", s.line());
        }
    }
}

fn failed(url: &str, e: ureq::Error) -> FetchError {
    if matches!(e, ureq::Error::StatusCode(404 | 403)) {
        return FetchError::NotFound(url.to_string());
    }
    FetchError::Failed {
        url: url.to_string(),
        reason: e.to_string(),
    }
}

fn read_failed(url: &str, e: impl std::fmt::Display) -> FetchError {
    FetchError::Failed {
        url: url.to_string(),
        reason: e.to_string(),
    }
}

/// The `Range` header value for `len` bytes from `start`.
fn range_header(start: u64, len: u64) -> String {
    format!("bytes={start}-{}", start + len.saturating_sub(1))
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let mut resp = ureq::get(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "*/*")
            .call()
            .map_err(|e| failed(url, e))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| read_failed(url, e))
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        let mut resp = ureq::get(url)
            .header("User-Agent", USER_AGENT)
            .header("Range", range_header(start, len))
            .call()
            .map_err(|e| failed(url, e))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| read_failed(url, e))
    }
}

#[cfg(test)]
mod tests {
    //! Label: sourced - RFC 9110 section 14.1.2: byte ranges are inclusive at both ends.
    use super::range_header;

    #[test]
    fn a_range_is_inclusive_of_its_last_byte() {
        assert_eq!(range_header(0, 1), "bytes=0-0");
        assert_eq!(range_header(100, 50), "bytes=100-149");
    }
}
