//! Request state and the pieces every handler needs: language from the path,
//! owner-session lookup, flash messages, and the profile cache.
//!
//! Deliberately mirrors the zero-crate `App` (same TTL, same limiters, same
//! cookie names) so the two implementations stay behaviourally comparable.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::db::Store;
use crate::models::{Lang, Profile, SessionInfo, detect_lang};
use crate::{crypto, i18n};

pub const SESSION_COOKIE: &str = "rz_session";
pub const CSRF_COOKIE: &str = "rz_csrf";
const PROFILE_TTL_SECS: i64 = 30;

/// Fixed-window rate limiter: `limit` events per `window_secs`, then `lock_secs`
/// of refusal. The same shape as the zero-crate version, kept allocation-light.
pub struct RateLimiter {
    state: Mutex<Vec<(String, u64, u64, u64)>>, // key, window_start, count, locked_until
    limit: usize,
    window_secs: u64,
    lock_secs: u64,
}

impl RateLimiter {
    pub fn new(limit: usize, window_secs: u64, lock_secs: u64) -> RateLimiter {
        RateLimiter {
            state: Mutex::new(Vec::new()),
            limit,
            window_secs,
            lock_secs,
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    pub fn is_locked(&self, key: &str) -> bool {
        let now = Self::now();
        let mut st = self.state.lock().unwrap();
        if let Some(entry) = st.iter_mut().find(|e| e.0 == key) {
            if entry.3 > now {
                return true;
            }
            if entry.3 != 0 && entry.3 <= now {
                entry.3 = 0;
                entry.2 = 0;
                entry.1 = now;
            }
        }
        false
    }

    /// Returns `false` when the caller should be refused.
    pub fn check(&self, key: &str) -> bool {
        let now = Self::now();
        let mut st = self.state.lock().unwrap();
        if st.len() > 4096 {
            st.retain(|e| e.3 > now || now - e.1 < self.window_secs * 4);
        }
        match st.iter_mut().find(|e| e.0 == key) {
            None => {
                st.push((key.to_string(), now, 1, 0));
                true
            }
            Some(entry) => {
                if entry.3 > now {
                    return false;
                }
                if now - entry.1 >= self.window_secs {
                    entry.1 = now;
                    entry.2 = 1;
                    return true;
                }
                entry.2 += 1;
                if entry.2 as usize > self.limit {
                    entry.3 = now + self.lock_secs;
                    false
                } else {
                    true
                }
            }
        }
    }

    pub fn penalize(&self, key: &str) {
        let now = Self::now();
        let mut st = self.state.lock().unwrap();
        if let Some(entry) = st.iter_mut().find(|e| e.0 == key) {
            entry.2 += 1;
            if entry.2 as usize > self.limit {
                entry.3 = now + self.lock_secs;
            }
        } else {
            st.push((key.to_string(), now, 1, 0));
        }
    }

    pub fn reset(&self, key: &str) {
        let mut st = self.state.lock().unwrap();
        st.retain(|e| e.0 != key);
    }
}

/// Everything a handler shares. Cheap to clone: it sits behind `Arc` in the
/// router state, so handlers receive `State<Arc<AppState>>`.
pub struct AppState {
    pub store: Store,
    pub owner: String,
    pub profile: Mutex<(Profile, i64)>,
    pub login_limiter: RateLimiter,
    pub write_limiter: RateLimiter,
    pub started: Instant,
    pub requests: AtomicU64,
    pub active: AtomicUsize,
}

impl AppState {
    pub fn boot(
        data_dir: &Path,
        owner: &str,
        password: Option<&str>,
        seed: bool,
    ) -> Result<(AppState, Option<String>), String> {
        let store = Store::open(&data_dir.join("site.sqlite"))?;
        let generated = store.bootstrap_owner(owner, password)?;
        if seed {
            store.seed_if_empty()?;
        }
        let profile = store.profile()?;
        Ok((
            AppState {
                store,
                owner: owner.to_string(),
                profile: Mutex::new((profile, i18n::now_unix())),
                login_limiter: RateLimiter::new(6, 600, 600),
                write_limiter: RateLimiter::new(240, 60, 30),
                started: Instant::now(),
                requests: AtomicU64::new(0),
                active: AtomicUsize::new(0),
            },
            generated,
        ))
    }

    pub fn profile_now(&self) -> Profile {
        let mut guard = self.profile.lock().unwrap();
        let fresh = i18n::now_unix() - guard.1 < PROFILE_TTL_SECS;
        if !fresh {
            if let Ok(p) = self.store.profile() {
                guard.0 = p;
            }
            guard.1 = i18n::now_unix();
        }
        guard.0.clone()
    }

    pub fn invalidate_profile(&self) {
        let mut guard = self.profile.lock().unwrap();
        guard.1 = 0;
    }

    pub fn session(&self, token: Option<&str>) -> Option<SessionInfo> {
        let token = token?;
        if token.len() != 64 {
            return None;
        }
        self.store.session_by_token(token).ok().flatten()
    }

    pub fn is_owner(&self, token: Option<&str>) -> bool {
        self.session(token).is_some()
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn note_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }
}

/// One guard per in-flight request; the counter feeds `/healthz` only.
pub struct ActiveGuard<'a>(&'a AtomicUsize);

impl<'a> ActiveGuard<'a> {
    pub fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        ActiveGuard(counter)
    }
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// `/en/notes/x` splits into `(En, "/notes/x")`; `/notes/x` into `(Ru, …)`.
/// Language is a URL prefix, never a cookie: URLs stay canonical and cacheable.
pub fn split_lang(path: &str) -> (Lang, String) {
    if path == "/en" {
        return (Lang::En, "/".to_string());
    }
    if let Some(rest) = path.strip_prefix("/en/") {
        return (Lang::En, format!("/{rest}"));
    }
    (Lang::Ru, path.to_string())
}

/// `Accept-Language` is consulted only where no prefix exists (`/login`).
pub fn lang_from_accept_language(header: Option<&str>) -> Lang {
    let decorated = format!("accept-language: {}", header.unwrap_or(""));
    detect_lang(&decorated, None)
}

/// Query-string value by name, already percent-decoded.
pub fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if crate::util::url_decode(k) == name {
            Some(crate::util::url_decode(v))
        } else {
            None
        }
    })
}

/// Mutating requests must come from the same origin. Checked on the server side
/// of every POST; a cookie alone is never enough.
pub fn same_origin(origin: Option<&str>, referer: Option<&str>, host: Option<&str>, fetch_site: Option<&str>) -> bool {
    if let Some(site) = fetch_site {
        if site.eq_ignore_ascii_case("cross-site") || site.eq_ignore_ascii_case("same-site") {
            return false;
        }
    }
    let Some(host) = host else { return false };
    let host = host.to_ascii_lowercase();
    let strip = |v: &str| {
        v.to_ascii_lowercase()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string()
    };
    if let Some(origin) = origin {
        return strip(origin) == host;
    }
    if let Some(referer) = referer {
        let r = strip(referer);
        return r == host || r.starts_with(&format!("{host}/"));
    }
    false
}

/// Stable, non-reversible visitor id: the counter can tell two readers apart
/// without ever storing an address.
pub fn visitor_hash(ip: &str, ua: &str, day: &str) -> String {
    let material = format!("{day}|{ip}|{ua}");
    crypto::sha256_hex(material.as_bytes())[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_prefix_wins_and_en_is_stripped() {
        assert_eq!(split_lang("/en/notes/x"), (Lang::En, "/notes/x".to_string()));
        assert_eq!(split_lang("/en"), (Lang::En, "/".to_string()));
        assert_eq!(split_lang("/notes/x"), (Lang::Ru, "/notes/x".to_string()));
    }

    #[test]
    fn rate_limiter_locks_after_limit() {
        let rl = RateLimiter::new(3, 60, 60);
        assert!(rl.check("ip"));
        assert!(rl.check("ip"));
        assert!(rl.check("ip"));
        assert!(!rl.check("ip"));
        assert!(rl.is_locked("ip"));
        assert!(rl.check("other"));
    }

    #[test]
    fn origin_must_match_host() {
        assert!(same_origin(Some("https://rz.me"), None, Some("rz.me"), None));
        assert!(!same_origin(Some("https://evil.tld"), None, Some("rz.me"), None));
        assert!(!same_origin(None, None, Some("rz.me"), Some("cross-site")));
        assert!(same_origin(None, Some("https://rz.me/cabinet"), Some("rz.me"), None));
        assert!(!same_origin(None, None, Some("rz.me"), None));
    }

    #[test]
    fn query_params_are_decoded() {
        let q = Some("q=%D1%80%D1%83%D1%81%D1%82&msg=saved");
        assert_eq!(query_param(q, "q").as_deref(), Some("руст"));
        assert_eq!(query_param(q, "msg").as_deref(), Some("saved"));
        assert_eq!(query_param(q, "nope"), None);
    }
}
