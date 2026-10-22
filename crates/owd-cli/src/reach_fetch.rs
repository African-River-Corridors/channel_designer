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

/// Native HTTP. Every request goes through [`agent`], so requests to one host reuse its pooled
/// connections (#375).
pub struct HttpFetch;

/// The time limits of a native request (#372). A server that accepts the connection and never
/// answers fails the request, so a caller can move on (the Overpass search to its next mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// To resolve the host and open the connection.
    pub connect: std::time::Duration,
    /// From the request sent to the response head.
    pub response: std::time::Duration,
    /// For the whole body, from the response head on.
    pub body: std::time::Duration,
}

/// The limits of every native request. Provisional, and long enough for the largest honest answer:
/// the Overpass queries ask the server for at most 120 s (`[timeout:120]`), and a busy mirror
/// queues before it starts, so the response head may take up to 300 s. A body may take 30 min: a
/// whole JRC tile at 0.5 MB/s is under 15 min, and a Sentinel-2 window or an Overpass answer far
/// less.
pub const LIMITS: Limits = Limits {
    connect: std::time::Duration::from_secs(30),
    response: std::time::Duration::from_secs(300),
    body: std::time::Duration::from_secs(1800),
};

/// An agent with `limits`.
pub fn agent_with(limits: &Limits) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_resolve(Some(limits.connect))
        .timeout_connect(Some(limits.connect))
        .timeout_recv_response(Some(limits.response))
        .timeout_recv_body(Some(limits.body))
        .build()
        .into()
}

/// The one HTTP agent of the process (#375), with [`LIMITS`] (#372). `ureq::get` makes a new agent
/// for each call, so no connection was reused: each range read opened a new TCP and TLS
/// connection, and on a long reach the closed ones used up the local ports (`os error 49`).
pub fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| agent_with(&LIMITS))
}

/// [`Fetch`] through a given agent. [`HttpFetch`] is this over [`agent`].
pub struct AgentFetch<'a>(pub &'a ureq::Agent);

/// The host of a URL, for messages.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

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
    if let ureq::Error::Timeout(t) = e {
        return FetchError::Failed {
            url: url.to_string(),
            reason: format!("{} timed out ({t})", host_of(url)),
        };
    }
    FetchError::Failed {
        url: url.to_string(),
        reason: e.to_string(),
    }
}

fn read_failed(url: &str, e: impl std::fmt::Display) -> FetchError {
    let reason = e.to_string();
    let reason = if reason.contains("timeout") || reason.contains("timed out") {
        format!("{} timed out ({reason})", host_of(url))
    } else {
        reason
    };
    FetchError::Failed {
        url: url.to_string(),
        reason,
    }
}

/// The `Range` header value for `len` bytes from `start`.
fn range_header(start: u64, len: u64) -> String {
    format!("bytes={start}-{}", start + len.saturating_sub(1))
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        AgentFetch(agent()).get(url)
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, FetchError> {
        AgentFetch(agent()).get_range(url, start, len)
    }
}

impl Fetch for AgentFetch<'_> {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let mut resp = self
            .0
            .get(url)
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
        let mut resp = self
            .0
            .get(url)
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
    let resp = agent()
        .get(url)
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
mod agent_tests {
    //! Label: property (a local keep-alive server counts the connections a run of range reads makes).
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A server that answers every request on a connection, keeping it open (HTTP/1.1
    /// keep-alive), and counts the connections it accepts. Returns its URL and the count.
    fn keep_alive_server() -> (String, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/scene.tif", listener.local_addr().unwrap());
        let conns = Arc::new(AtomicUsize::new(0));
        let c = conns.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(s) = stream else { continue };
                c.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let mut w = s.try_clone().unwrap();
                    let mut r = BufReader::new(s);
                    loop {
                        // Read one request head; stop when the client closes.
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match r.read_line(&mut line) {
                                Ok(0) | Err(_) => return,
                                Ok(_) => {}
                            }
                            if line == "\r\n" {
                                break;
                            }
                        }
                        let body = b"0123";
                        let head = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        if w.write_all(head.as_bytes()).is_err() || w.write_all(body).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (url, conns)
    }

    #[test]
    fn many_range_reads_to_one_host_reuse_one_connection() {
        let (url, conns) = keep_alive_server();
        for i in 0..50 {
            assert_eq!(HttpFetch.get_range(&url, i * 4, 4).unwrap(), b"0123");
        }
        let n = conns.load(Ordering::SeqCst);
        assert!(n <= 2, "50 reads made {n} connections");
    }
}

#[cfg(test)]
mod timeout_tests {
    //! Label: property (a local server that accepts the connection and never answers, #372).
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::time::{Duration, Instant};

    /// A server that accepts every connection, reads the request and never answers.
    fn silent_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for mut s in listener.incoming().flatten() {
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                held.push(s);
            }
        });
        format!("http://{at}/api/interpreter")
    }

    /// A server that answers every request with `body`.
    fn answering_server(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut s in listener.incoming().flatten() {
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(body);
            }
        });
        format!("http://{at}/api/interpreter")
    }

    fn short() -> ureq::Agent {
        agent_with(&Limits {
            connect: Duration::from_secs(1),
            response: Duration::from_millis(500),
            body: Duration::from_millis(500),
        })
    }

    #[test]
    fn a_server_that_never_answers_fails_within_the_limit_naming_the_host() {
        let url = silent_server();
        let a = short();
        let t = Instant::now();
        let e = AgentFetch(&a).get(&url).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        let m = e.to_string();
        let host = url.split('/').nth(2).unwrap();
        assert!(m.contains(host) && m.contains("timed out"), "{m}");
        assert!(matches!(e, FetchError::Failed { .. }));
        let e = AgentFetch(&a).get_range(&url, 0, 10).unwrap_err();
        assert!(e.to_string().contains("timed out"), "{e}");
    }

    #[test]
    fn on_a_timeout_the_overpass_search_moves_to_the_next_server() {
        let (hung, good) = (silent_server(), answering_server(br#"{"elements":[]}"#));
        let a = short();
        let f = AgentFetch(&a);
        let ov = owd_reach::overpass::Overpass::with_endpoint(&f, &format!("{hung},{good}"));
        assert_eq!(ov.search_rivers("Nowhere").unwrap(), vec![]);
        let good_host = good.split('/').nth(2).unwrap();
        assert_eq!(
            ov.answered_by().as_deref().map(owd_reach::overpass::host),
            Some(good_host)
        );
        assert!(
            ov.notes().iter().any(|n| n.contains("timed out")),
            "{:?}",
            ov.notes()
        );
    }

    #[test]
    fn the_limits_leave_room_for_the_largest_honest_answer() {
        // The Overpass queries ask the server for at most 120 s (`[timeout:120]`).
        assert!(LIMITS.response >= Duration::from_secs(240));
        assert!(LIMITS.connect >= Duration::from_secs(10));
        assert!(LIMITS.body >= Duration::from_secs(900));
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
