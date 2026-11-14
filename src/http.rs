//! Hand-written HTTP/1.1 server: keep-alive, thread pool, gzip, ETag/304,
//! URL/form decoding, cookies, CSP nonces, security headers and a login rate limiter.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::crypto;

pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 512 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------- request

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub remote_ip: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn user_agent(&self) -> &str {
        self.header("user-agent").unwrap_or("")
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        let raw = self.header("cookie")?;
        for part in raw.split(';') {
            let part = part.trim();
            if let Some((k, v)) = part.split_once('=')
                && k.trim() == name
            {
                return Some(url_decode(v.trim()));
            }
        }
        None
    }

    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn form(&self) -> HashMap<String, String> {
        let body = String::from_utf8_lossy(&self.body);
        let mut map = HashMap::new();
        for pair in body.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            map.insert(url_decode(k), url_decode(v));
        }
        map
    }

    pub fn accepts_gzip(&self) -> bool {
        self.header("accept-encoding")
            .map(|v| v.to_ascii_lowercase().contains("gzip"))
            .unwrap_or(false)
    }

}

// --------------------------------------------------------------- response

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        Response {
            status,
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body,
        }
    }

    pub fn html(status: u16, body: String) -> Response {
        let mut resp = Response::new(status, "text/html; charset=utf-8", body.into_bytes());
        resp.set_csp();
        resp
    }

    pub fn text(status: u16, body: String) -> Response {
        Response::new(status, "text/plain; charset=utf-8", body.into_bytes())
    }

    pub fn json(status: u16, body: String) -> Response {
        Response::new(status, "application/json; charset=utf-8", body.into_bytes())
    }

    pub fn redirect(location: &str) -> Response {
        let mut resp = Response::new(303, "text/plain; charset=utf-8", Vec::new());
        resp.headers.push(("Location".into(), location.into()));
        resp
    }

    pub fn empty(status: u16) -> Response {
        Response::new(status, "text/plain; charset=utf-8", Vec::new())
    }

    pub fn header(mut self, key: &str, value: &str) -> Response {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    /// Hash-pinned CSP: no per-response nonce, so the HTML stays byte-identical
    /// and `ETag`/`304` revalidation actually works.
    pub fn set_csp(&mut self) {
        let csp = format!(
            "default-src 'none'; \
             style-src 'unsafe-inline'; \
             script-src 'sha256-{}'; \
             img-src 'self' data:; \
             font-src 'self'; \
             connect-src 'self'; \
             base-uri 'none'; \
             form-action 'self'; \
             frame-ancestors 'none'",
            crate::templates::script_csp_hash()
        );
        self.headers.push(("Content-Security-Policy".into(), csp));
    }

    pub fn is_compressible(&self) -> bool {
        self.headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("content-type")
                && (v.starts_with("text/")
                    || v.starts_with("application/json")
                    || v.starts_with("application/xml")
                    || v.starts_with("image/svg"))
        })
    }
}

// ------------------------------------------------------------- utilities

pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn is_bot(user_agent: &str) -> bool {
    let ua = user_agent.to_ascii_lowercase();
    [
        "bot", "crawl", "spider", "curl/", "wget", "python-requests", "headless", "monitor",
        "uptime", "facebookexternalhit", "preview",
    ]
    .iter()
    .any(|needle| ua.contains(needle))
}

// ------------------------------------------------------------ rate limiter

/// Sliding-window limiter keyed by client address. Used for `/login` and `/cabinet` writes.
pub struct RateLimiter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
    limit: usize,
    window: Duration,
    lock_duration: Duration,
    locked: Mutex<HashMap<String, Instant>>,
}

impl RateLimiter {
    pub fn new(limit: usize, window_secs: u64, lock_secs: u64) -> RateLimiter {
        RateLimiter {
            hits: Mutex::new(HashMap::new()),
            limit,
            window: Duration::from_secs(window_secs),
            lock_duration: Duration::from_secs(lock_secs),
            locked: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_locked(&self, key: &str) -> bool {
        let mut locked = self.locked.lock().unwrap();
        match locked.get(key) {
            Some(until) if *until > Instant::now() => true,
            Some(_) => {
                locked.remove(key);
                false
            }
            None => false,
        }
    }

    /// Returns true when the request is allowed.
    pub fn check(&self, key: &str) -> bool {
        if self.is_locked(key) {
            return false;
        }
        let mut hits = self.hits.lock().unwrap();
        let now = Instant::now();
        let entry = hits.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < self.window);
        if entry.len() >= self.limit {
            return false;
        }
        entry.push(now);
        true
    }

    /// Called after a failed authentication attempt: locks the key after too many misses.
    pub fn penalize(&self, key: &str) {
        let mut hits = self.hits.lock().unwrap();
        let entry = hits.entry(key.to_string()).or_default();
        if entry.len() >= self.limit {
            self.locked
                .lock()
                .unwrap()
                .insert(key.to_string(), Instant::now() + self.lock_duration);
            entry.clear();
        }
    }

    pub fn reset(&self, key: &str) {
        self.hits.lock().unwrap().remove(key);
    }
}

// --------------------------------------------------------------- the server

type Handler = dyn Fn(&Request) -> Response + Send + Sync + 'static;

pub struct ServerConfig {
    pub workers: usize,
    pub gzip_enabled: bool,
}

pub struct Server {
    listener: TcpListener,
    handler: Arc<Handler>,
    config: ServerConfig,
    served: Arc<AtomicU64>,
}

impl Server {
    pub fn bind(addr: &str, handler: Arc<Handler>, config: ServerConfig) -> Result<Server, String> {
        let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
        Ok(Server {
            listener,
            handler,
            config,
            served: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn local_addr(&self) -> String {
        self.listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "?".into())
    }

    pub fn run(self) -> Result<(), String> {
        let workers = self.config.workers.max(1);
        let mut senders = Vec::with_capacity(workers);
        for _ in 0..workers {
            let (tx, rx) = mpsc::channel::<TcpStream>();
            senders.push(tx);
            let handler = Arc::clone(&self.handler);
            let gzip = self.config.gzip_enabled;
            let served = Arc::clone(&self.served);
            std::thread::spawn(move || {
                for stream in rx {
                    let _ = handle_connection(stream, &*handler, gzip);
                    served.fetch_add(1, Ordering::Relaxed);
                }
            });
        }

        let mut next = 0usize;
        for incoming in self.listener.incoming() {
            match incoming {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    if senders[next].send(stream).is_err() {
                        break;
                    }
                    next = (next + 1) % senders.len();
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        Ok(())
    }
}

fn handle_connection(mut stream: TcpStream, handler: &Handler, gzip_enabled: bool) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IDLE_TIMEOUT));
    let remote_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".into());
    let mut carry: Vec<u8> = Vec::with_capacity(2048);

    loop {
        let Some((req, used)) = read_request(&mut stream, &mut carry, &remote_ip) else {
            return Ok(());
        };
        let kept_alive = !req
            .header("connection")
            .map(|c| c.eq_ignore_ascii_case("close"))
            .unwrap_or(false);

        let mut resp = handler(&req);
        carry.drain(..used.min(carry.len()));

        finalize_response(&mut resp, &req, gzip_enabled);
        if !kept_alive {
            resp.headers.push(("Connection".into(), "close".into()));
        } else {
            resp.headers.push(("Connection".into(), "keep-alive".into()));
        }
        resp.headers
            .push(("Keep-Alive".into(), format!("timeout={}", IDLE_TIMEOUT.as_secs())));

        write_response(&mut stream, &req.method, resp)?;
        if !kept_alive {
            return Ok(());
        }
    }
}

fn read_request(stream: &mut TcpStream, carry: &mut Vec<u8>, remote_ip: &str) -> Option<(Request, usize)> {
    let mut buf = [0u8; 8192];
    // Read until we have the full header block.
    let header_end = loop {
        if let Some(pos) = find_double_crlf(carry) {
            break pos;
        }
        if carry.len() > MAX_HEADER_BYTES {
            return None;
        }
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        carry.extend_from_slice(&buf[..n]);
    };

    let head = String::from_utf8_lossy(&carry[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_ascii_uppercase();
    let target = parts.next()?.to_string();
    let _version = parts.next().unwrap_or("HTTP/1.1");

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let content_length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return None;
    }

    let body_start = header_end + 4;
    while carry.len() < body_start + content_length {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&buf[..n]);
    }
    let body = carry
        .get(body_start..body_start + content_length)
        .map(|s| s.to_vec())
        .unwrap_or_default();
    let used = body_start + content_length;

    let (path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let _ = &raw_query;
    let query = raw_query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (url_decode(k), url_decode(v))
        })
        .collect();

    Some((
        Request {
            method,
            path: normalize_path(&path),
            query,
            headers,
            body,
            remote_ip: remote_ip.to_string(),
        },
        used,
    ))
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".into();
    }
    let mut decoded = url_decode(path);
    if !decoded.starts_with('/') {
        decoded.insert(0, '/');
    }
    // Collapse duplicate slashes and strip a possible trailing slash.
    while decoded.contains("//") {
        decoded = decoded.replace("//", "/");
    }
    if decoded.len() > 1 && decoded.ends_with('/') {
        decoded.pop();
    }
    decoded
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn finalize_response(resp: &mut Response, req: &Request, gzip_enabled: bool) {
    resp.headers.push((
        "Server".into(),
        "rudolfzinovev-rust".into(),
    ));
    resp.headers
        .push(("X-Content-Type-Options".into(), "nosniff".into()));
    resp.headers
        .push(("X-Frame-Options".into(), "DENY".into()));
    resp.headers
        .push(("Referrer-Policy".into(), "strict-origin-when-cross-origin".into()));
    resp.headers.push((
        "Permissions-Policy".into(),
        "geolocation=(), microphone=(), camera=(), interest-cohort=()".into(),
    ));
    if resp
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("cache-control"))
    {
        resp.headers.push((
            "Cache-Control".into(),
            "no-cache, must-revalidate".into(),
        ));
    }

    // Weak-ish strong ETag over the uncompressed body.
    if resp.status == 200 && !resp.body.is_empty() && resp.body.len() <= 4 * 1024 * 1024 {
        let etag = format!("\"{}\"", &crypto::sha256_hex(&resp.body)[..20]);
        let inm = req.header("if-none-match").map(|v| v.trim().to_string());
        resp.headers.push(("ETag".into(), etag.clone()));
        if let Some(inm) = inm
            && (inm == etag || inm == "*")
        {
            resp.status = 304;
            resp.body.clear();
        }
    }

    if resp.status == 304 {
        resp.headers.retain(|(k, _)| {
            !k.eq_ignore_ascii_case("content-type") && !k.eq_ignore_ascii_case("content-length")
        });
        return;
    }

    let should_gzip = gzip_enabled
        && req.accepts_gzip()
        && resp.is_compressible()
        && resp.body.len() > 700;
    if should_gzip && let Some(compressed) = crypto::gzip_compress(&resp.body) {
        if compressed.len() < resp.body.len() {
            resp.body = compressed;
            resp.headers.push(("Content-Encoding".into(), "gzip".into()));
            resp.headers.push(("Vary".into(), "Accept-Encoding".into()));
        }
    }
}

fn write_response(stream: &mut TcpStream, method: &str, resp: Response) -> std::io::Result<()> {
    let reason = match resp.status {
        200 => "OK",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason);
    let has_len = resp
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
    for (k, v) in &resp.headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    if !has_len && !(resp.status == 304) {
        head.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    }
    head.push_str("\r\n");

    let mut out = Vec::with_capacity(head.len() + resp.body.len());
    out.extend_from_slice(head.as_bytes());
    if method != "HEAD" && resp.status != 304 {
        out.extend_from_slice(&resp.body);
    }
    stream.write_all(&out)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_forms() {
        assert_eq!(url_decode("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(url_decode("100%"), "100%");
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("/articles//x/"), "/articles/x");
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("/%D1%81%D1%82"), "/ст");
    }

    #[test]
    fn rate_limiter_locks_after_failures() {
        let rl = RateLimiter::new(2, 60, 60);
        assert!(rl.check("ip"));
        assert!(rl.check("ip"));
        rl.penalize("ip");
        assert!(!rl.check("ip"));
    }

    #[test]
    fn bot_detection() {
        assert!(is_bot("Mozilla/5.0 (compatible; Googlebot/2.1)"));
        assert!(!is_bot("Mozilla/5.0 (X11; Linux x86_64) Gecko/20100101 Firefox/128.0"));
    }
}
