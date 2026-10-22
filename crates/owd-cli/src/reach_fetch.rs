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

/// Download a whole file to `dest` (#321), reporting megabytes as it goes and stopping between
/// chunks once `stop` says so. The file appears only when complete: it is written beside `dest`
/// and renamed.
pub fn download(
    url: &str,
    dest: &std::path::Path,
    what: &str,
    report: &dyn Fn(&owd_reach::Step),
    stop: &dyn Fn() -> bool,
) -> Result<u64, String> {
    use std::io::{Read, Write};
    let resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("cannot fetch {url}: {e}"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let mb = |b: u64| (b as f64 / 1_048_576.0).ceil() as usize;
    let part = dest.with_extension("part");
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut out = std::fs::File::create(&part)
        .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    let mut body = resp.into_body().into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    loop {
        if stop() {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err("cancelled".into());
        }
        let n = body
            .read(&mut buf)
            .map_err(|e| format!("cannot fetch {url}: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        done += n as u64;
        report(&owd_reach::Step::sized(
            mb(done),
            mb(total.max(done)),
            what,
            "MB",
        ));
    }
    drop(out);
    std::fs::rename(&part, dest).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    Ok(done)
}

/// A `Fetch` that serves a URL from a file of the same name in `dir` when there is one, and asks
/// `inner` otherwise: tiles downloaded into a study's folder are read from there (#321).
pub struct LocalFirst<'a> {
    pub dir: std::path::PathBuf,
    pub inner: &'a dyn Fetch,
}

impl LocalFirst<'_> {
    fn local(&self, url: &str) -> Option<std::path::PathBuf> {
        let name = url.split(['?', '#']).next()?.rsplit('/').next()?;
        let p = self.dir.join(name);
        (!name.is_empty() && p.is_file()).then_some(p)
    }
}

impl Fetch for LocalFirst<'_> {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        match self.local(url) {
            Some(p) => std::fs::read(&p).map_err(|e| read_failed(url, e)),
            None => self.inner.get(url),
        }
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        use std::io::{Read, Seek, SeekFrom};
        let Some(p) = self.local(url) else {
            return self.inner.get_range(url, start, len);
        };
        let mut f = std::fs::File::open(&p).map_err(|e| read_failed(url, e))?;
        f.seek(SeekFrom::Start(start))
            .map_err(|e| read_failed(url, e))?;
        let mut out = Vec::new();
        f.take(len)
            .read_to_end(&mut out)
            .map_err(|e| read_failed(url, e))?;
        Ok(out)
    }

    fn progress(&self, step: &owd_reach::Step) {
        self.inner.progress(step);
    }
}

#[cfg(test)]
mod local_tests {
    //! Label: property
    use super::*;

    #[test]
    fn a_file_in_the_folder_is_read_from_disk_and_any_other_url_from_the_network() {
        let dir = std::env::temp_dir().join(format!("owd-local-first-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("occurrence_10W_10Nv1_4_2021.tif"), b"0123456789").unwrap();
        let net = owd_reach::ReplayFetch::new()
            .with_response("https://x.example/other.tif", b"net".to_vec());
        let f = LocalFirst {
            dir: dir.clone(),
            inner: &net,
        };
        let url = "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_10W_10Nv1_4_2021.tif";
        assert_eq!(f.get(url).unwrap(), b"0123456789");
        assert_eq!(f.get_range(url, 3, 4).unwrap(), b"3456");
        assert_eq!(
            f.get_range(url, 8, 10).unwrap(),
            b"89",
            "fewer at the end of the file"
        );
        assert_eq!(f.get("https://x.example/other.tif").unwrap(), b"net");
        assert_eq!(
            net.requests().len(),
            1,
            "only the other file went to the network"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_reports_megabytes_and_a_stopped_one_leaves_no_file() {
        use std::io::{Read as _, Write as _};
        let body = vec![7u8; 3 * 1_048_576 + 5];
        let served = body.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/occurrence_x.tif", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    served.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&served);
            }
        });
        let dir = std::env::temp_dir().join(format!("owd-download-{}", std::process::id()));
        let dest = dir.join("occurrence_x.tif");
        let seen = std::cell::RefCell::new(Vec::new());
        let report = |s: &owd_reach::Step| seen.borrow_mut().push(s.line());
        let n = download(
            &url,
            &dest,
            "downloading JRC tile 1 of 1:",
            &report,
            &|| false,
        )
        .unwrap();
        assert_eq!(n, body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(
            seen.borrow().last().unwrap(),
            "downloading JRC tile 1 of 1: 4 of 4 MB"
        );
        let stopped = dir.join("stopped.tif");
        let e = download(&url, &stopped, "x", &report, &|| true).unwrap_err();
        assert_eq!(e, "cancelled");
        assert!(!stopped.exists() && !stopped.with_extension("part").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
