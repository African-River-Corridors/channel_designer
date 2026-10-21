//! The native [`owd_reach::Fetch`]: HTTPS through `ureq`, as `owd fetch-dem` does. The web gives
//! `owd-reach` the browser's `fetch` instead.

use owd_reach::{Fetch, FetchError};

/// Refuse a response larger than this (a Sentinel-2 or JRC window is far smaller).
const MAX_BYTES: u64 = 400_000_000;

/// Native HTTP.
pub struct HttpFetch;

fn failed(url: &str, e: impl std::fmt::Display) -> FetchError {
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
        let mut resp = ureq::get(url).call().map_err(|e| failed(url, e))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| failed(url, e))
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        let mut resp = ureq::get(url)
            .header("Range", range_header(start, len))
            .call()
            .map_err(|e| failed(url, e))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| failed(url, e))
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
