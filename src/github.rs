//! GitHub integration: reads a public repository through the REST API and turns
//! it into a factual timeline (releases, commits, README changes) that is kept
//! strictly apart from the owner's own notes.
//!
//! Networking is plain `TcpStream` + OpenSSL through libssl, because the project
//! links zero crates. Every call is optional: when the network, TLS or the API
//! says no, the project page falls back to the last cached snapshot and the
//! cabinet reports the reason instead of failing the request.

use std::ffi::{CString, c_int, c_long, c_void};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::i18n::now_unix;
use crate::json::{self, Json};

/// Overridable so a broken or blackholed endpoint can be exercised in tests
/// (`RZ_GITHUB_HOST=127.0.0.1:9`). Production always talks to the real host.
const API_HOST: &str = "api.github.com";

fn api_host() -> String {
    std::env::var("RZ_GITHUB_HOST").unwrap_or_else(|_| API_HOST.to_string())
}
const USER_AGENT: &str = "rudolfzinovev.me (+https://rudolfzinovev.me)";
const MAX_BODY: usize = 512 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const IO_TIMEOUT: Duration = Duration::from_secs(12);
/// The API allows 60 anonymous calls per hour per IP, so a refresh never
/// happens more often than this and the cabinet can force one by hand.
pub const REFRESH_INTERVAL_SECS: i64 = 30 * 60;
pub const MAX_EVENTS: usize = 40;

// --------------------------------------------------------------- libssl FFI

#[link(name = "ssl")]
unsafe extern "C" {
    fn TLS_client_method() -> *const c_void;
    fn SSL_CTX_new(method: *const c_void) -> *mut c_void;
    fn SSL_CTX_free(ctx: *mut c_void);
    fn SSL_CTX_set_default_verify_paths(ctx: *mut c_void) -> c_int;
    fn SSL_CTX_set_verify(ctx: *mut c_void, mode: c_int, callback: *const c_void);
    fn SSL_new(ctx: *mut c_void) -> *mut c_void;
    fn SSL_free(ssl: *mut c_void);
    fn SSL_ctrl(ssl: *mut c_void, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
    fn SSL_set_fd(ssl: *mut c_void, fd: c_int) -> c_int;
    fn SSL_connect(ssl: *mut c_void) -> c_int;
    fn SSL_write(ssl: *mut c_void, buf: *const c_void, num: c_int) -> c_int;
    fn SSL_read(ssl: *mut c_void, buf: *mut c_void, num: c_int) -> c_int;
    fn SSL_get_error(ssl: *const c_void, ret: c_int) -> c_int;
    fn SSL_get_verify_result(ssl: *const c_void) -> c_long;
    fn SSL_shutdown(ssl: *mut c_void) -> c_int;
    fn ERR_clear_error();
}

const SSL_VERIFY_PEER: c_int = 0x01;
/// `SSL_CTRL_SET_TLSEXT_HOSTNAME` (55); `TLSEXT_NAMETYPE_host_name` is 0.
const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
const SSL_ERROR_WANT_READ: c_int = 2;
const SSL_ERROR_WANT_WRITE: c_int = 3;
const SSL_ERROR_ZERO_RETURN: c_int = 6;
const X509_V_OK: c_long = 0;

/// One HTTPS exchange, in the only shape the GitHub API needs: GET a path,
/// read the body. Returns `(status, body)`.
fn https_get(host: &str, path: &str, accept: &str) -> Result<(u16, String), String> {
    // `host:port` is only used by the failure drill; the real API is always 443.
    let (dns_name, port) = match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => {
            (name.to_string(), port.parse::<u16>().unwrap_or(443))
        }
        _ => (host.to_string(), 443u16),
    };
    let addr = (dns_name.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("dns {dns_name}: {e}"))?
        .next()
        .ok_or_else(|| format!("dns {dns_name}: no address"))?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| format!("connect {dns_name}: {e}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
    let raw_fd = {
        use std::os::unix::io::AsRawFd;
        stream.as_raw_fd()
    };

    unsafe { ERR_clear_error() };
    let ctx = unsafe { SSL_CTX_new(TLS_client_method()) };
    if ctx.is_null() {
        return Err("SSL_CTX_new failed".to_string());
    }
    let _guard_ctx = CtxGuard(ctx);
    unsafe {
        SSL_CTX_set_default_verify_paths(ctx);
        SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, std::ptr::null());
    }
    let ssl = unsafe { SSL_new(ctx) };
    if ssl.is_null() {
        return Err("SSL_new failed".to_string());
    }
    let _guard_ssl = SslGuard(ssl);
    let host_c = CString::new(host).map_err(|e| e.to_string())?;
    unsafe {
        SSL_ctrl(
            ssl,
            SSL_CTRL_SET_TLSEXT_HOSTNAME,
            0,
            host_c.as_ptr() as *mut c_void,
        );
        if SSL_set_fd(ssl, raw_fd) != 1 {
            return Err("SSL_set_fd failed".to_string());
        }
        if SSL_connect(ssl) != 1 {
            return Err(format!("tls handshake failed ({})", ssl_error(ssl)));
        }
        if SSL_get_verify_result(ssl) != X509_V_OK {
            return Err("tls certificate verification failed".to_string());
        }
    }

    let sni_host = dns_name.clone();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {sni_host}\r\nUser-Agent: {USER_AGENT}\r\n\
         Accept: {accept}\r\nX-GitHub-Api-Version: 2022-11-28\r\nConnection: close\r\n\r\n"
    );
    ssl_write_all(ssl, request.as_bytes())?;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = unsafe { SSL_read(ssl, chunk.as_mut_ptr() as *mut c_void, chunk.len() as c_int) };
        if n > 0 {
            raw.extend_from_slice(&chunk[..n as usize]);
            if raw.len() > MAX_BODY {
                return Err("response too large".to_string());
            }
            continue;
        }
        let err = unsafe { SSL_get_error(ssl, n) };
        if err == SSL_ERROR_WANT_READ || err == SSL_ERROR_WANT_WRITE {
            continue;
        }
        if err == SSL_ERROR_ZERO_RETURN || n == 0 {
            break;
        }
        // A truncated body after valid headers is still worth parsing.
        if !raw.is_empty() {
            break;
        }
        return Err(format!("tls read failed ({err})"));
    }
    unsafe {
        SSL_shutdown(ssl);
    }

    parse_http_response(&raw)
}

fn ssl_error(ssl: *mut c_void) -> c_int {
    let rc = unsafe { SSL_get_error(ssl as *const c_void, -1) };
    rc
}

fn ssl_write_all(ssl: *mut c_void, mut data: &[u8]) -> Result<(), String> {
    while !data.is_empty() {
        let n = unsafe {
            SSL_write(ssl, data.as_ptr() as *const c_void, data.len().min(16384) as c_int)
        };
        if n <= 0 {
            return Err(format!("tls write failed ({})", unsafe {
                SSL_get_error(ssl as *const c_void, n)
            }));
        }
        data = &data[n as usize..];
    }
    Ok(())
}

struct CtxGuard(*mut c_void);
impl Drop for CtxGuard {
    fn drop(&mut self) {
        unsafe { SSL_CTX_free(self.0) };
    }
}

struct SslGuard(*mut c_void);
impl Drop for SslGuard {
    fn drop(&mut self) {
        unsafe { SSL_free(self.0) };
    }
}

fn parse_http_response(raw: &[u8]) -> Result<(u16, String), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "malformed response: no header end".to_string())?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut body = raw[split + 4..].to_vec();
    let mut lines = head.lines();
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("malformed status line: {status_line:?}"))?;

    let mut chunked = false;
    let mut length: Option<usize> = None;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim();
        if key == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if key == "content-length" {
            length = value.parse::<usize>().ok();
        }
    }
    if chunked {
        body = dechunk(&body);
    } else if let Some(len) = length {
        body.truncate(len);
    }
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let Some(end) = body[pos..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size_text = String::from_utf8_lossy(&body[pos..pos + end]).into_owned();
        let size_text = size_text.split(';').next().unwrap_or("").trim().to_string();
        let Ok(size) = usize::from_str_radix(&size_text, 16) else {
            break;
        };
        pos += end + 2;
        if size == 0 {
            break;
        }
        let take = size.min(body.len().saturating_sub(pos));
        out.extend_from_slice(&body[pos..pos + take]);
        pos += take + 2;
    }
    out
}

// ------------------------------------------------------------- normalisation

/// `RUdolf1517/tzim` → `RUdolf1517/tzim`; anything that is not exactly two
/// `owner/name` segments of safe characters is rejected.
pub fn normalise_repo(input: &str) -> Option<String> {
    let input = input.trim();
    let input = input
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/")
        .trim_start_matches("github.com/")
        .trim_end_matches(".git")
        .trim_matches('/');
    let mut parts = input.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 39
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };
    if !ok(owner) || !ok(name) {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// A single factual line in a repository's automatic history.
#[derive(Debug, Clone, PartialEq)]
pub struct GhEvent {
    /// `release` | `commit` | `readme`
    pub kind: String,
    pub title: String,
    /// Short secondary line: version, author, short sha.
    pub detail: String,
    pub url: String,
    pub at: i64,
}

impl GhEvent {
    pub fn from_row(row: &[crate::sqlite::SqlVal]) -> Option<GhEvent> {
        if row.len() < 6 {
            return None;
        }
        Some(GhEvent {
            kind: row[0].as_str().to_string(),
            title: row[1].as_str().to_string(),
            detail: row[2].as_str().to_string(),
            url: row[3].as_str().to_string(),
            at: row[4].as_i64(),
        })
    }
}

/// Facts about a repository that the page shows above the timeline.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // full_name is printed by the `github` diagnostic
pub struct GhRepo {
    pub full_name: String,
    pub description: String,
    pub stars: i64,
    pub forks: i64,
    pub language: String,
    pub spdx: String,
    pub homepage: String,
    pub default_branch: String,
    pub open_issues: i64,
    pub pushed_at: i64,
    pub created_at: i64,
    pub archived: bool,
    pub release_tag: String,
    pub release_name: String,
    pub release_url: String,
    pub release_at: i64,
}

/// Fields the API returns that we surface; everything else is ignored.
fn repo_from_json(v: &Json) -> GhRepo {
    GhRepo {
        full_name: v.str_at("full_name"),
        description: v.str_at("description"),
        stars: v.int_at("stargazers_count"),
        forks: v.int_at("forks_count"),
        language: v.str_at("language"),
        spdx: v
            .get("license")
            .map(|l| l.str_at("spdx_id"))
            .filter(|s| !s.is_empty() && s != "NOASSERTION")
            .unwrap_or_default(),
        homepage: v.str_at("homepage"),
        default_branch: v.str_at("default_branch"),
        open_issues: v.int_at("open_issues_count"),
        pushed_at: parse_iso8601(&v.str_at("pushed_at")).unwrap_or(0),
        created_at: parse_iso8601(&v.str_at("created_at")).unwrap_or(0),
        archived: v.get("archived").and_then(Json::as_bool).unwrap_or(false),
        ..GhRepo::default()
    }
}

/// `2026-11-14T09:31:07Z` → unix seconds. The API always speaks UTC.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> {
        std::str::from_utf8(bytes.get(a..b)?).ok()?.parse::<i64>().ok()
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    Some(days * 86_400 + h * 3_600 + mi * 60 + sec)
}

/// Howard Hinnant's civil-from-days, inverted: days since the unix epoch.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ------------------------------------------------------------------- client

/// Everything fetched in one refresh, already normalised.
pub struct Fetched {
    pub repo: GhRepo,
    pub events: Vec<GhEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    Network,
    NotFound,
    RateLimited,
    Api(u16),
}

impl FetchError {
    pub fn code(self) -> &'static str {
        match self {
            FetchError::Network => "network",
            FetchError::NotFound => "not-found",
            FetchError::RateLimited => "rate-limited",
            FetchError::Api(_) => "api-error",
        }
    }
}

fn api(path: &str) -> Result<Json, FetchError> {
    let host = api_host();
    let (status, body) = https_get(&host, path, "application/vnd.github+json")
        .map_err(|_| FetchError::Network)?;
    match status {
        200 => json::parse(&body).map_err(|_| FetchError::Network),
        404 => Err(FetchError::NotFound),
        403 | 429 => Err(FetchError::RateLimited),
        other => Err(FetchError::Api(other)),
    }
}

/// One refresh: repository facts, the latest release, recent commits and the
/// date of the last README change. Commits are read from `main`-like default
/// branch reported by the API.
pub fn fetch(full_name: &str) -> Result<Fetched, FetchError> {
    let repo_json = api(&format!("/repos/{full_name}"))?;
    let mut repo = repo_from_json(&repo_json);
    let branch = if repo.default_branch.is_empty() {
        "main".to_string()
    } else {
        repo.default_branch.clone()
    };

    let mut events = Vec::new();

    // Latest release (a repository without releases is normal, not an error).
    if let Ok(rel) = api(&format!("/repos/{full_name}/releases/latest")) {
        let tag = rel.str_at("tag_name");
        if !tag.is_empty() {
            let at = parse_iso8601(&rel.str_at("published_at"))
                .or_else(|| parse_iso8601(&rel.str_at("created_at")))
                .unwrap_or(0);
            let name = {
                let n = rel.str_at("name");
                if n.is_empty() { tag.clone() } else { n }
            };
            repo.release_tag = tag.clone();
            repo.release_name = name.clone();
            repo.release_url = rel.str_at("html_url");
            repo.release_at = at;
            events.push(GhEvent {
                kind: "release".into(),
                title: name,
                detail: tag,
                url: repo.release_url.clone(),
                at,
            });
        }
    }

    // Recent commits on the default branch.
    if let Ok(commits) = api(&format!(
        "/repos/{full_name}/commits?sha={branch}&per_page=20"
    )) {
        for c in commits.as_arr().unwrap_or(&[]) {
            let sha = c.str_at("sha");
            let commit = match c.get("commit") {
                Some(v) => v,
                None => continue,
            };
            let message = commit.str_at("message");
            let title = message.lines().next().unwrap_or("").trim().to_string();
            if title.is_empty() {
                continue;
            }
            let author = commit
                .get("author")
                .map(|a| a.str_at("name"))
                .unwrap_or_default();
            let at = parse_iso8601(&commit.str_at("author").to_string())
                .or_else(|| {
                    commit
                        .get("author")
                        .and_then(|a| a.get("date"))
                        .and_then(Json::as_str)
                        .and_then(parse_iso8601)
                })
                .unwrap_or(0);
            events.push(GhEvent {
                kind: "commit".into(),
                title: truncate(&title, 140),
                detail: format!("{} · {}", short_sha(&sha), author),
                url: c.str_at("html_url"),
                at,
            });
        }
    }

    // Last README change: one commit that touched the README.
    for candidate in ["README.md", "readme.md", "README"] {
        let path = format!("/repos/{full_name}/commits?path={candidate}&per_page=1");
        if let Ok(list) = api(&path)
            && let Some(first) = list.as_arr().and_then(|a| a.first())
        {
            let commit = first.get("commit");
            let at = commit
                .and_then(|c| c.get("author"))
                .and_then(|a| a.get("date"))
                .and_then(Json::as_str)
                .and_then(parse_iso8601)
                .unwrap_or(0);
            if at > 0 {
                events.push(GhEvent {
                    kind: "readme".into(),
                    title: candidate.to_string(),
                    detail: commit
                        .map(|c| c.str_at("message").lines().next().unwrap_or("").to_string())
                        .unwrap_or_default(),
                    url: first.str_at("html_url"),
                    at,
                });
                break;
            }
        }
    }

    events.sort_by(|a, b| b.at.cmp(&a.at));
    events.truncate(MAX_EVENTS);
    Ok(Fetched { repo, events })
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `true` when a refresh is due. `etag`/`last` are stored per project.
pub fn refresh_due(last_fetch: i64, force: bool) -> bool {
    force || now_unix() - last_fetch >= REFRESH_INTERVAL_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_response_with_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        let (status, body) = parse_http_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "hello");
    }

    #[test]
    fn parses_chunked_bodies() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let (status, body) = parse_http_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "hello world");
    }

    #[test]
    fn parses_error_statuses() {
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(parse_http_response(raw).unwrap().0, 404);
    }

    #[test]
    fn normalises_repository_references() {
        assert_eq!(normalise_repo("RUdolf1517/tzim").as_deref(), Some("RUdolf1517/tzim"));
        assert_eq!(
            normalise_repo("https://github.com/RUdolf1517/tzim.git").as_deref(),
            Some("RUdolf1517/tzim")
        );
        assert_eq!(normalise_repo(" github.com/RUdolf1517/tzim/ ").as_deref(), Some("RUdolf1517/tzim"));
        assert_eq!(normalise_repo("RUdolf1517"), None);
        assert_eq!(normalise_repo("a/b/c"), None);
        assert_eq!(normalise_repo("RUdolf1517/tzi m"), None);
        assert_eq!(normalise_repo(""), None);
    }

    #[test]
    fn parses_iso_dates_to_unix() {
        // 2026-11-14T09:31:07Z
        assert_eq!(parse_iso8601("2026-11-14T09:31:07Z"), Some(1_794_648_667));
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2024-02-29T12:00:00Z").is_some(), true);
        assert_eq!(parse_iso8601("nonsense"), None);
        assert_eq!(parse_iso8601("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn refresh_interval_is_respected() {
        let now = now_unix();
        assert!(!refresh_due(now, false));
        assert!(refresh_due(now, true));
        assert!(refresh_due(now - REFRESH_INTERVAL_SECS - 1, false));
    }

    #[test]
    fn maps_repository_json() {
        let doc = json::parse(
            r#"{"full_name":"a/b","description":"d","stargazers_count":3,"forks_count":1,
                "language":"Rust","license":{"spdx_id":"MIT"},"homepage":"https://x",
                "default_branch":"main","open_issues_count":2,
                "pushed_at":"2026-11-14T09:31:07Z","created_at":"2020-01-01T00:00:00Z",
                "archived":false}"#,
        )
        .unwrap();
        let r = repo_from_json(&doc);
        assert_eq!(r.full_name, "a/b");
        assert_eq!(r.stars, 3);
        assert_eq!(r.spdx, "MIT");
        assert_eq!(r.pushed_at, 1_794_648_667);
        assert!(!r.archived);
    }
}
