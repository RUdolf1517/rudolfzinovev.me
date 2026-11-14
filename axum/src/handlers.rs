//! axum router and handlers.
//!
//! Shape of the port: the routing table, the middleware duties (CSP, security
//! headers, ETag/304, gzip, Server-Timing, telemetry) and the session/CSRF
//! rules are the same as in the zero-crate build. What disappears is 616 lines
//! of hand-written HTTP/1.1: parsing, keep-alive, chunked bodies, the thread
//! pool, and the request/response plumbing. hyper owns all of that now.
//!
//! Everything here is deliberately explicit — no extractor magic beyond
//! `State`, `Path`, `Query` and `HeaderMap`, so the port stays readable
//! side-by-side with `src/app.rs`.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use crate::db::ArticleFilter;
use crate::models::{Article, BackupStats, Lang, Kind, Profile, SessionInfo, ViewRecord};
use crate::state::{
    AppState, CSRF_COOKIE, SESSION_COOKIE, lang_from_accept_language, split_lang, visitor_hash,
};
use crate::templates::{self, Chrome};
use crate::util::{is_bot, referrer_host, truncate_chars};
use crate::{crypto, i18n};

pub type Shared = Arc<AppState>;

/// One handler signature for everything. Explicit rather than extracted: the
/// same `Req` is passed to helper methods that mirror `src/app.rs`, which keeps
/// the diff between the two implementations small and reviewable.
pub struct Req {
    pub started: std::time::Instant,
    pub method: axum::http::Method,
    pub path: String,
    pub query: Option<String>,
    pub headers: HeaderMap,
    pub form: Vec<(String, String)>,
    pub ip: String,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        let raw = self.header("cookie")?;
        raw.split(';').find_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            (k == name).then(|| v.trim().to_string())
        })
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        crate::state::query_param(self.query.as_deref(), name)
    }

    pub fn form_value(&self, name: &str) -> String {
        self.form
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    pub fn user_agent(&self) -> &str {
        self.header("user-agent").unwrap_or("")
    }

    pub fn accepts_gzip(&self) -> bool {
        self.header("accept-encoding")
            .map(|v| v.to_ascii_lowercase().contains("gzip"))
            .unwrap_or(false)
    }

    pub fn host(&self) -> Option<String> {
        self.header("host")
            .map(|h| h.split(':').next().unwrap_or(h).to_string())
    }
}

// ---------------------------------------------------------------- responses

/// Small response builder: status, content type, body, cookies.
pub struct Out {
    pub status: StatusCode,
    pub content_type: String,
    pub body: Vec<u8>,
    pub cookies: Vec<String>,
    pub cache: Option<String>,
    pub extra: Vec<(String, String)>,
}

impl Out {
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Out {
        Out {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
            content_type: content_type.to_string(),
            body,
            cookies: Vec::new(),
            cache: None,
            extra: Vec::new(),
        }
    }

    pub fn html(status: u16, body: String) -> Out {
        Out::new(status, "text/html; charset=utf-8", body.into_bytes())
    }

    pub fn text(status: u16, body: String) -> Out {
        Out::new(status, "text/plain; charset=utf-8", body.into_bytes())
    }

    pub fn redirect(to: &str) -> Out {
        let mut out = Out::new(303, "text/plain; charset=utf-8", Vec::new());
        out.extra.push(("Location".into(), to.to_string()));
        out
    }

    pub fn empty(status: u16) -> Out {
        Out::new(status, "text/plain; charset=utf-8", Vec::new())
    }

    pub fn cookie(mut self, value: String) -> Out {
        self.cookies.push(value);
        self
    }

    pub fn cache(mut self, value: &str) -> Out {
        self.cache = Some(value.to_string());
        self
    }

    pub fn header(mut self, key: &str, value: &str) -> Out {
        self.extra.push((key.to_string(), value.to_string()));
        self
    }

    /// Same rule as the zero-crate build: text-ish types, and only when the
    /// compressed body would actually be smaller.
    fn is_compressible(&self) -> bool {
        self.content_type.starts_with("text/")
            || self.content_type.starts_with("application/json")
            || self.content_type.starts_with("application/xml")
            || self.content_type.starts_with("application/rss")
            || self.content_type.starts_with("image/svg")
    }
}

/// Everything the old `finalize_response` did, expressed as an axum `Layer`
/// would be more idiomatic — but keeping it as one function means there is
/// exactly one place where headers, ETag and gzip are decided.
fn finish(req: &Req, mut out: Out, gzip: bool) -> Response {
    // 304 for a conditional GET: the ETag is derived from the uncompressed body,
    // so it is stable across gzip on/off and across machines.
    let etag = if out.status == StatusCode::OK && !out.body.is_empty() {
        Some(format!("\"{}\"", &crypto::sha256_hex(&out.body)[..20]))
    } else {
        None
    };
    if let (Some(etag), Some(inm)) = (etag.as_deref(), req.header("if-none-match")) {
        if inm.trim() == etag || inm.trim() == "*" {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            let h = resp.headers_mut();
            h.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, must-revalidate"));
            return resp;
        }
    }

    if gzip && req.accepts_gzip() && out.is_compressible() && out.body.len() > 700 {
        if let Some(compressed) = crypto::gzip_compress(&out.body) {
            if compressed.len() < out.body.len() {
                out.body = compressed;
                out.extra.push(("Content-Encoding".into(), "gzip".into()));
                out.extra.push(("Vary".into(), "Accept-Encoding".into()));
            }
        }
    }

    let mut resp = Response::builder()
        .status(out.status)
        .header(header::CONTENT_TYPE, out.content_type)
        .header(header::SERVER, "rudolfzinovev-axum")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("x-frame-options", "DENY")
        .header(header::REFERRER_POLICY, "strict-origin-when-cross-origin")
        .header(
            "permissions-policy",
            "geolocation=(), microphone=(), camera=(), interest-cohort=()",
        )
        .header(
            header::CONTENT_SECURITY_POLICY,
            format!(
                "default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{}'; \
                 img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; \
                 form-action 'self'; frame-ancestors 'none'",
                crate::util::script_csp_hash()
            ),
        )
        .header(
            header::CACHE_CONTROL,
            out.cache
                .clone()
                .unwrap_or_else(|| "no-cache, must-revalidate".to_string()),
        );
    if let Some(etag) = etag {
        resp = resp.header(header::ETAG, etag);
    }
    for (k, v) in &out.extra {
        resp = resp.header(k.as_str(), v.as_str());
    }
    for cookie in &out.cookies {
        resp = resp.header(header::SET_COOKIE, cookie.as_str());
    }
    let body = Body::from(out.body);
    resp.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ------------------------------------------------------------------ router

pub fn router(state: Shared, gzip: bool) -> Router {
    Router::new()
        // assets and machine endpoints
        .route("/assets/style.css", get(css))
        .route("/assets/app.js", get(js))
        .route("/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon))
        .route("/robots.txt", get(robots))
        .route("/sitemap.xml", get(sitemap))
        // public
        .route("/", get(home))
        .route("/en", get(home))
        .route("/notes", get(notes))
        .route("/en/notes", get(notes))
        .route("/projects", get(projects))
        .route("/en/projects", get(projects))
        .route("/tags", get(tags))
        .route("/en/tags", get(tags))
        .route("/tags/{slug}", get(notes_by_tag))
        .route("/en/tags/{slug}", get(notes_by_tag))
        .route("/notes/{slug}", get(note))
        .route("/en/notes/{slug}", get(note))
        .route("/projects/{slug}", get(project))
        .route("/en/projects/{slug}", get(project))
        .route("/about", get(about))
        .route("/en/about", get(about))
        .route("/feed.xml", get(feed))
        .route("/en/feed.xml", get(feed))
        .route("/projects/feed.xml", get(projects_feed))
        .route("/en/projects/feed.xml", get(projects_feed))
        // old addresses, kept alive as 303
        .route("/articles", get(old_articles))
        .route("/en/articles", get(old_articles))
        .route("/articles/{slug}", get(old_article))
        .route("/en/articles/{slug}", get(old_article))
        // auth
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
        // cabinet
        .route("/cabinet", get(cabinet_overview))
        .route("/cabinet/articles", get(cabinet_articles))
        .route(
            "/cabinet/articles/new",
            get(cabinet_new).post(cabinet_create),
        )
        .route(
            "/cabinet/articles/{id}",
            get(cabinet_edit).post(cabinet_update),
        )
        .route("/cabinet/articles/{id}/status", post(cabinet_status))
        .route("/cabinet/articles/{id}/delete", post(cabinet_delete))
        .route(
            "/cabinet/profile",
            get(cabinet_profile).post(cabinet_profile_save),
        )
        .route("/cabinet/telemetry", get(cabinet_telemetry))
        .route("/cabinet/security", get(cabinet_security))
        .route("/cabinet/security/password", post(cabinet_password))
        .route("/cabinet/security/revoke", post(cabinet_revoke))
        .route("/cabinet/security/revoke-all", post(cabinet_revoke_all))
        .route("/cabinet/backup", get(cabinet_backup))
        .route("/cabinet/backup.json", get(cabinet_backup_json))
        // anything else
        .fallback(not_found_route)
        .with_state(Handler {
            state,
            gzip,
        })
}

/// Router state: the app plus the two knobs that change per deployment.
#[derive(Clone)]
pub struct Handler {
    pub state: Shared,
    pub gzip: bool,
}

// ---------------------------------------------------- axum <-> Req adapters

impl Handler {
    fn req(&self, method: &axum::http::Method, path: String, query: Option<String>, headers: &HeaderMap, form_body: Option<String>, ip: &str) -> Req {
        Req {
            started: std::time::Instant::now(),
            method: method.clone(),
            path,
            query,
            headers: headers.clone(),
            form: form_body
                .map(|b| crate::util::parse_form(&b))
                .unwrap_or_default(),
            ip: ip.to_string(),
        }
    }

    fn chrome<'a>(&self, req: &'a Req, lang: Lang, profile: &'a Profile) -> Chrome<'a> {
        Chrome {
            lang,
            path: &req.path,
            profile,
            owner: self.state.is_owner(req.cookie(SESSION_COOKIE).as_deref()),
        }
    }

    /// Telemetry, Server-Timing and the final headers. Every handler returns
    /// through here, so there is one place to audit.
    fn done(&self, req: &Req, mut out: Out) -> Response {
        let render_us = req.started.elapsed().as_micros().max(40);
        if out.status == StatusCode::OK && out.content_type.starts_with("text/html") {
            // Render time travels in a header, not in the HTML: the body stays
            // byte-identical, so the ETag keeps revalidating.
            out.extra.push((
                "Server-Timing".into(),
                format!("render;dur={:.2};desc=\"html render\"", render_us as f64 / 1000.0),
            ));
        }
        if req.method == axum::http::Method::GET && should_measure(&req.path) {
            let (lang, _) = split_lang(&req.path);
            let day = i18n::format_date_utc(i18n::now_unix());
            let _ = self.state.store.record_view(&ViewRecord {
                ts: i18n::now_unix(),
                path: req.path.clone(),
                lang: lang.code().to_string(),
                status_code: out.status.as_u16() as i64,
                render_us: render_us as i64,
                bytes: out.body.len() as i64,
                referrer: req.header("referer").map(referrer_host).unwrap_or_default(),
                ua: truncate_chars(req.user_agent(), 200),
                is_bot: is_bot(req.user_agent()),
                visitor_hash: visitor_hash(&req.ip, req.user_agent(), &day),
            });
        }
        self.state.note_request();
        finish(req, out, self.gzip)
    }
}

fn should_measure(path: &str) -> bool {
    !path.starts_with("/assets/")
        && !path.starts_with("/cabinet")
        && path != "/login"
        && path != "/healthz"
        && !path.starts_with("/favicon")
}

// ------------------------------------------------------------ tiny helpers

fn lang_of(path: &str) -> (Lang, String) {
    split_lang(path)
}

fn flash(req: &Req, lang: Lang) -> Option<String> {
    let tr = crate::i18n::t(lang);
    let code = req.query_param("msg")?;
    let message = match code.as_str() {
        "saved" => tr.cabinet_saved.to_string(),
        "deleted" => match lang {
            Lang::Ru => "Запись удалена.".to_string(),
            Lang::En => "Entry deleted.".to_string(),
        },
        "published" => match lang {
            Lang::Ru => "Запись опубликована.".to_string(),
            Lang::En => "Entry published.".to_string(),
        },
        "draft" => match lang {
            Lang::Ru => "Запись возвращена в черновики.".to_string(),
            Lang::En => "Entry moved back to drafts.".to_string(),
        },
        "password" => tr.cabinet_password_changed.to_string(),
        "revoked" => match lang {
            Lang::Ru => "Сессия завершена.".to_string(),
            Lang::En => "Session revoked.".to_string(),
        },
        _ => return None,
    };
    Some(message)
}

fn session_cookie(req: &Req, token: &str) -> String {
    let secure = req
        .header("x-forwarded-proto")
        .map(|p| p.eq_ignore_ascii_case("https"))
        .unwrap_or(false);
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        crate::db::SESSION_TTL_SECS,
        if secure { "; Secure" } else { "" }
    )
}

fn csrf_ok(req: &Req) -> bool {
    crate::state::same_origin(
        req.header("origin"),
        req.header("referer"),
        req.host().as_deref(),
        req.header("sec-fetch-site"),
    )
}

// --------------------------------------------------------- static endpoints

async fn css(State(h): State<Handler>) -> Response {
    let req = h.req(&axum::http::Method::GET, "/assets/style.css".into(), None, &HeaderMap::new(), None, "");
    h.done(&req, Out::new(200, "text/css; charset=utf-8", templates::CSS.as_bytes().to_vec()).cache("public, max-age=31536000, immutable"))
}

async fn js(State(h): State<Handler>) -> Response {
    let req = h.req(&axum::http::Method::GET, "/assets/app.js".into(), None, &HeaderMap::new(), None, "");
    h.done(&req, Out::new(200, "text/javascript; charset=utf-8", templates::JS.as_bytes().to_vec()).cache("public, max-age=31536000, immutable"))
}

async fn favicon(State(h): State<Handler>) -> Response {
    let req = h.req(&axum::http::Method::GET, "/favicon.svg".into(), None, &HeaderMap::new(), None, "");
    h.done(&req, Out::new(200, "image/svg+xml", templates::FAVICON.as_bytes().to_vec()).cache("public, max-age=31536000, immutable"))
}

async fn robots(State(h): State<Handler>) -> Response {
    let req = h.req(&axum::http::Method::GET, "/robots.txt".into(), None, &HeaderMap::new(), None, "");
    let body = format!(
        "User-agent: *\nAllow: /\nDisallow: /cabinet\nDisallow: /login\nSitemap: {}/sitemap.xml\n",
        h.state.profile_now().site_url.trim_end_matches('/')
    );
    h.done(&req, Out::text(200, body).cache("public, max-age=3600"))
}

async fn sitemap(State(h): State<Handler>) -> Response {
    let req = h.req(&axum::http::Method::GET, "/sitemap.xml".into(), None, &HeaderMap::new(), None, "");
    let articles = h.state.store.list_articles(&ArticleFilter::default()).unwrap_or_default();
    let xml = templates::sitemap_xml(&h.state.profile_now(), &articles);
    h.done(&req, Out::new(200, "application/xml; charset=utf-8", xml.into_bytes()).cache("public, max-age=1800"))
}

// ------------------------------------------------------------ public pages

async fn home(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), uri.query().map(str::to_string), &headers, None, "");
    let (lang, _) = lang_of(&path);
    let profile = h.state.profile_now();
    let lang_code = lang.code();
    let notes = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang_code),
            kind: Some("note"),
            limit: Some(5),
            ..Default::default()
        })
        .unwrap_or_default();
    let projects = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang_code),
            kind: Some("project"),
            limit: Some(4),
            ..Default::default()
        })
        .unwrap_or_default();
    let notes_total = h.state.store.count_kind(lang_code, "note").unwrap_or(0);
    let projects_total = h.state.store.count_kind(lang_code, "project").unwrap_or(0);
    let chrome = h.chrome(&req, lang, &profile);
    let html = templates::home(&chrome, &notes, &projects, notes_total, projects_total);
    h.done(&req, Out::html(200, html))
}

async fn section(h: &Handler, req: &Req, kind: Kind) -> Response {
    let (lang, _) = lang_of(&req.path);
    let profile = h.state.profile_now();
    let query = req
        .query_param("q")
        .map(|q| truncate_chars(q.trim(), 80))
        .filter(|q| !q.is_empty());
    let entries = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang.code()),
            kind: Some(kind.code()),
            query: query.as_deref(),
            ..Default::default()
        })
        .unwrap_or_default();
    let tags = h.state.store.tag_cloud(lang.code(), kind.code()).unwrap_or_default();
    let chrome = h.chrome(req, lang, &profile);
    let html = templates::section_index(&chrome, kind, &entries, &tags, None, query.as_deref());
    h.done(req, Out::html(200, html))
}

async fn notes(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    section(&h, &req, Kind::Note).await
}

async fn projects(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    section(&h, &req, Kind::Project).await
}

async fn notes_by_tag(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(slug): Path<String>,
) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), uri.query().map(str::to_string), &headers, None, "");
    let (lang, _) = lang_of(&path);
    let profile = h.state.profile_now();
    let entries = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang.code()),
            kind: Some("note"),
            tag: Some(slug.as_str()),
            ..Default::default()
        })
        .unwrap_or_default();
    let tags = h.state.store.tag_cloud(lang.code(), "note").unwrap_or_default();
    let chrome = h.chrome(&req, lang, &profile);
    let html = templates::section_index(&chrome, Kind::Note, &entries, &tags, Some(&slug), None);
    h.done(&req, Out::html(200, html))
}

async fn tags(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    // Tag cloud for notes plus the entries that carry them.
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), uri.query().map(str::to_string), &headers, None, "");
    let (lang, _) = lang_of(&path);
    let profile = h.state.profile_now();
    let entries = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang.code()),
            kind: Some("note"),
            ..Default::default()
        })
        .unwrap_or_default();
    let cloud = h.state.store.tag_cloud(lang.code(), "note").unwrap_or_default();
    let chrome = h.chrome(&req, lang, &profile);
    let html = templates::section_index(&chrome, Kind::Note, &entries, &cloud, None, None);
    h.done(&req, Out::html(200, html))
}

async fn entry(h: &Handler, req: &Req, kind: Kind, slug: &str) -> Response {
    let (lang, _) = lang_of(&req.path);
    let article = match h.state.store.article_by_slug(lang.code(), slug) {
        // A note must not be reachable under /projects and vice versa.
        Ok(Some(a)) if a.is_published() && a.kind == kind.code() => a,
        _ => return not_found(h, req).await,
    };
    let other = h
        .state
        .store
        .translation_of(&article.translation_key, lang.other().code())
        .ok()
        .flatten()
        .filter(|a| a.is_published());
    let same_lang = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang.code()),
            kind: Some(kind.code()),
            limit: Some(24),
            ..Default::default()
        })
        .unwrap_or_default();
    let mine: Vec<String> = article.tag_slugs();
    let mut related: Vec<Article> = same_lang
        .iter()
        .filter(|a| a.id != article.id)
        .filter(|a| a.tag_slugs().iter().any(|t| mine.contains(t)))
        .take(3)
        .cloned()
        .collect();
    if related.is_empty() {
        related = same_lang
            .iter()
            .filter(|a| a.id != article.id)
            .take(3)
            .cloned()
            .collect();
    }
    let _ = h.state.store.increment_views(article.id);
    let profile = h.state.profile_now();
    let chrome = h.chrome(req, lang, &profile);
    let html = if article.is_project() {
        // Same data as the zero-crate server: facts, timeline, journal — kept apart.
        let link = h.state.store.repo_link(article.id).ok().flatten();
        let events = if link.as_ref().map(|l| l.is_linked()).unwrap_or(false) {
            h.state.store.repo_events(article.id, 24).unwrap_or_default()
        } else {
            Vec::new()
        };
        let notes = h
            .state
            .store
            .project_notes(article.id, lang.code(), req.cookie(SESSION_COOKIE).is_some())
            .unwrap_or_default();
        let view = templates::ProjectView {
            link: link.as_ref(),
            events: &events,
            notes: &notes,
        };
        templates::entry_page_with(&chrome, &article, other.as_ref(), &related, Some(&view))
    } else {
        templates::entry_page_with(&chrome, &article, other.as_ref(), &related, None)
    };
    h.done(req, Out::html(200, html))
}

async fn note(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(slug): Path<String>,
) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    entry(&h, &req, Kind::Note, &slug).await
}

async fn project(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(slug): Path<String>,
) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    entry(&h, &req, Kind::Project, &slug).await
}

async fn about(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), uri.query().map(str::to_string), &headers, None, "");
    let (lang, _) = lang_of(&path);
    let profile = h.state.profile_now();
    let notes = h.state.store.count_kind(lang.code(), "note").unwrap_or(0);
    let projects = h.state.store.count_kind(lang.code(), "project").unwrap_or(0);
    let chrome = h.chrome(&req, lang, &profile);
    let html = templates::about(&chrome, notes, projects);
    h.done(&req, Out::html(200, html))
}

async fn feed(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), None, &headers, None, "");
    let (lang, _) = lang_of(&path);
    let articles = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang.code()),
            kind: Some("note"),
            limit: Some(30),
            ..Default::default()
        })
        .unwrap_or_default();
    let xml = templates::feed_xml(lang, &h.state.profile_now(), &articles);
    h.done(&req, Out::new(200, "application/rss+xml; charset=utf-8", xml.into_bytes()).cache("public, max-age=600"))
}

async fn projects_feed(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), None, &headers, None, "");
    let (lang, _) = lang_of(&path);
    let articles = h
        .state
        .store
        .list_articles(&ArticleFilter {
            lang: Some(lang.code()),
            kind: Some("project"),
            limit: Some(30),
            ..Default::default()
        })
        .unwrap_or_default();
    let xml = templates::feed_xml(lang, &h.state.profile_now(), &articles);
    h.done(&req, Out::new(200, "application/rss+xml; charset=utf-8", xml.into_bytes()).cache("public, max-age=600"))
}

async fn old_articles(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), None, &headers, None, "");
    let (lang, _) = lang_of(&path);
    h.done(&req, Out::redirect(&format!("{}notes", lang.prefix())))
}

async fn old_article(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(slug): Path<String>,
) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path.clone(), None, &headers, None, "");
    let (lang, _) = lang_of(&path);
    h.done(&req, Out::redirect(&templates::entry_path(lang.prefix(), "notes", &slug)))
}

async fn not_found(h: &Handler, req: &Req) -> Response {
    let (lang, _) = lang_of(&req.path);
    let profile = h.state.profile_now();
    let chrome = h.chrome(req, lang, &profile);
    h.done(req, Out::html(404, templates::not_found(&chrome)))
}

async fn not_found_route(State(h): State<Handler>) -> Response {
    let req = Req {
        started: std::time::Instant::now(),
        method: axum::http::Method::GET,
        path: "/".into(),
        query: None,
        headers: HeaderMap::new(),
        form: Vec::new(),
        ip: String::new(),
    };
    h.done(&req, Out::html(404, templates::not_found(&h.chrome(&req, Lang::Ru, &h.state.profile_now()))))
}

// ------------------------------------------------------------------- auth

async fn login_page(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    render_login(&h, &req, None, false)
}

fn render_login(h: &Handler, req: &Req, error: Option<&str>, limited: bool) -> Response {
    if error.is_none() && h.state.is_owner(req.cookie(SESSION_COOKIE).as_deref()) {
        return h.done(req, Out::redirect("/cabinet"));
    }
    let profile = h.state.profile_now();
    let lang = lang_from_accept_language(req.header("accept-language"));
    let csrf = req
        .cookie(CSRF_COOKIE)
        .filter(|v| v.len() == 48)
        .unwrap_or_else(|| crypto::random_token_hex(24));
    let mut chrome = h.chrome(req, lang, &profile);
    chrome.path = "/login";
    let html = templates::login(&chrome, error, &csrf, limited);
    let out = Out::html(if error.is_some() { 401 } else { 200 }, html)
        .cookie(format!("{CSRF_COOKIE}={csrf}; Path=/; HttpOnly; SameSite=Strict; Max-Age=1800"));
    h.done(req, out)
}

async fn login_submit(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: String,
) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::POST, path, uri.query().map(str::to_string), &headers, Some(body), "");
    let limited = !h.state.login_limiter.check(&req.ip);
    if limited {
        return render_login(&h, &req, Some(crate::i18n::t(Lang::Ru).login_rate_limited), true);
    }
    let supplied = req.form_value("csrf");
    let cookie_csrf = req.cookie(CSRF_COOKIE).unwrap_or_default();
    if supplied.is_empty() || supplied != cookie_csrf {
        return render_login(&h, &req, Some("CSRF token mismatch."), false);
    }
    let username = req.form_value("username");
    let password = req.form_value("password");
    match h.state.store.authenticate(&username, &password) {
        Ok(Some(user_id)) => {
            h.state.login_limiter.reset(&req.ip);
            let ip_hash = crypto::sha256_hex(req.ip.as_bytes());
            match h.state.store.create_session(user_id, &ip_hash, req.user_agent()) {
                Ok((token, _csrf)) => {
                    let out = Out::redirect("/cabinet").cookie(session_cookie(&req, &token));
                    h.done(&req, out)
                }
                Err(_) => h.done(&req, Out::html(500, templates::server_error(&h.chrome(&req, Lang::Ru, &h.state.profile_now())))),
            }
        }
        Ok(None) => {
            h.state.login_limiter.penalize(&req.ip);
            render_login(&h, &req, Some(crate::i18n::t(Lang::Ru).login_error), false)
        }
        Err(_) => h.done(&req, Out::html(500, templates::server_error(&h.chrome(&req, Lang::Ru, &h.state.profile_now())))),
    }
}

async fn logout(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri, body: String) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::POST, path, uri.query().map(str::to_string), &headers, Some(body), "");
    if !csrf_ok(&req) {
        return h.done(&req, Out::text(403, "csrf rejected\n".into()));
    }
    if let Some(token) = req.cookie(SESSION_COOKIE) {
        let hash = crypto::sha256_hex(token.as_bytes());
        h.state.store.revoke_session(&hash).ok();
    }
    let secure = req
        .header("x-forwarded-proto")
        .map(|p| p.eq_ignore_ascii_case("https"))
        .unwrap_or(false);
    let out = Out::redirect("/").cookie(format!(
        "{SESSION_COOKIE}=deleted; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    ));
    h.done(&req, out)
}

// ---------------------------------------------------------------- cabinet

/// Gate every cabinet route: session first, then CSRF on writes, then the
/// write limiter. Returns the session for handlers that need it.
fn cabinet_gate(h: &Handler, req: &Req) -> Result<SessionInfo, Out> {
    let Some(session) = h.state.session(req.cookie(SESSION_COOKIE).as_deref()) else {
        if req.method == axum::http::Method::GET && req.path == "/cabinet" {
            return Err(Out::redirect("/login"));
        }
        return Err(Out::text(401, "authentication required\n".into()));
    };
    if req.method == axum::http::Method::POST {
        if !csrf_ok(req) {
            return Err(Out::text(403, "csrf rejected: send Origin matching Host\n".into()));
        }
        if !h.state.write_limiter.check(&req.ip) {
            return Err(Out::text(429, "slow down\n".into()));
        }
    }
    Ok(session)
}

async fn cabinet_overview(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let profile = h.state.profile_now();
    let stats = h.state.store.backup_stats().unwrap_or(BackupStats {
        articles: 0,
        published: 0,
        drafts: 0,
        views: 0,
    });
    let chrome = h.chrome(&req, Lang::Ru, &profile);
    let flash = flash(&req, Lang::Ru);
    let html = templates::cabinet_overview(&chrome, &stats, flash.as_deref());
    h.done(&req, Out::html(200, html))
}

async fn cabinet_articles(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    let req = h.req(&axum::http::Method::GET, path, uri.query().map(str::to_string), &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let profile = h.state.profile_now();
    let articles = h
        .state
        .store
        .list_articles(&ArticleFilter {
            include_drafts: true,
            ..Default::default()
        })
        .unwrap_or_default();
    let chrome = h.chrome(&req, Lang::Ru, &profile);
    let flash = flash(&req, Lang::Ru);
    let html = templates::cabinet_articles(&chrome, &articles, flash.as_deref());
    h.done(&req, Out::html(200, html))
}

fn editor_page(h: &Handler, req: &Req, article: Option<&Article>, errors: &[String]) -> Response {
    let profile = h.state.profile_now();
    let tags = h.state.store.tag_cloud("ru", "note").unwrap_or_default();
    let project_tags = h.state.store.tag_cloud("ru", "project").unwrap_or_default();
    let chrome = h.chrome(req, Lang::Ru, &profile);
    let flash = flash(req, Lang::Ru);
    let html = templates::cabinet_editor(&chrome, article, &tags, &project_tags, flash.as_deref(), errors);
    h.done(req, Out::html(200, html))
}

async fn cabinet_new(State(h): State<Handler>, headers: HeaderMap) -> Response {
    let req = h.req(&axum::http::Method::GET, "/cabinet/articles/new".into(), None, &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    editor_page(&h, &req, None, &[])
}

async fn cabinet_edit(
    State(h): State<Handler>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let req = h.req(&axum::http::Method::GET, format!("/cabinet/articles/{id}"), None, &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let article = h.state.store.article_by_id(id).ok().flatten();
    editor_page(&h, &req, article.as_ref(), &[])
}

/// Shared by create and update: slug dedupe, translation key, kind, status.
fn save_entry(h: &Handler, req: &Req, id: Option<i64>) -> Response {
    let title = truncate_chars(req.form_value("title").trim(), 180);
    if title.is_empty() {
        let article = id.and_then(|i| h.state.store.article_by_id(i).ok().flatten());
        return editor_page(h, req, article.as_ref(), &["Title is required.".to_string()]);
    }
    let lang = if req.form_value("lang") == "en" { "en" } else { "ru" };
    let mut slug = req.form_value("slug").trim().to_lowercase();
    if slug.is_empty() {
        slug = crate::models::slug_of(&title);
    }
    let status = if req.form_value("status") == "published" { "published" } else { "draft" };
    let kind = if req.form_value("kind") == "project" { "project" } else { "note" };
    let featured = req.form_value("featured") == "1";
    let summary = truncate_chars(req.form_value("summary").trim(), 320);
    let body = req.form_value("body_md");
    let tags = req
        .form_value("tags")
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let published_at = req.form_value("published_at").parse::<i64>().ok().filter(|v| *v > 0);
    let requested_key = truncate_chars(req.form_value("translation_key").trim(), 64);

    // Slug collisions inside one language get a suffix, never a constraint error.
    let mut candidate = slug.clone();
    let mut n = 2;
    loop {
        let clash = h
            .state
            .store
            .article_by_slug(lang, &candidate)
            .ok()
            .flatten()
            .map(|a| Some(a.id) != id)
            .unwrap_or(false);
        if !clash {
            break;
        }
        candidate = format!("{slug}-{n}");
        n += 1;
    }

    let auto_key = if requested_key.is_empty() {
        h.state.store.translation_key_for_slug(&candidate).ok().flatten()
    } else {
        Some(requested_key.clone())
    };
    let input = crate::db::NewArticle {
        translation_key: auto_key.as_deref(),
        lang,
        slug: &candidate,
        title: &title,
        summary: &summary,
        body_md: &body,
        tags: &tags,
        status,
        kind,
        featured,
        published_at,
    };
    let outcome = match id {
        Some(id) => h.state.store.update_article(id, &input).map(|_| id),
        None => h.state.store.create_article(&input),
    };
    match outcome {
        Ok(saved) => h.done(req, Out::redirect(&format!("/cabinet/articles/{saved}?msg=saved"))),
        Err(err) => {
            let article = id.and_then(|i| h.state.store.article_by_id(i).ok().flatten());
            editor_page(h, req, article.as_ref(), &[err])
        }
    }
}

async fn cabinet_create(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: String,
) -> Response {
    let req = h.req(&axum::http::Method::POST, "/cabinet/articles/new".into(), uri.query().map(str::to_string), &headers, Some(body), "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    save_entry(&h, &req, None)
}

async fn cabinet_update(
    State(h): State<Handler>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    body: String,
) -> Response {
    let req = h.req(&axum::http::Method::POST, format!("/cabinet/articles/{id}"), uri.query().map(str::to_string), &headers, Some(body), "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    save_entry(&h, &req, Some(id))
}

async fn cabinet_status(
    State(h): State<Handler>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: String,
) -> Response {
    let req = h.req(&axum::http::Method::POST, format!("/cabinet/articles/{id}/status"), None, &headers, Some(body), "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let status = req.form_value("status");
    if status == "draft" || status == "published" {
        h.state.store.set_article_status(id, &status).ok();
    }
    let msg = if status == "published" { "published" } else { "draft" };
    h.done(&req, Out::redirect(&format!("/cabinet/articles?msg={msg}")))
}

async fn cabinet_delete(
    State(h): State<Handler>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: String,
) -> Response {
    let req = h.req(&axum::http::Method::POST, format!("/cabinet/articles/{id}/delete"), None, &headers, Some(body), "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    h.state.store.delete_article(id).ok();
    h.done(&req, Out::redirect("/cabinet/articles?msg=deleted"))
}

async fn cabinet_profile(State(h): State<Handler>, headers: HeaderMap) -> Response {
    let req = h.req(&axum::http::Method::GET, "/cabinet/profile".into(), None, &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let profile = h.state.profile_now();
    let chrome = h.chrome(&req, Lang::Ru, &profile);
    let flash = flash(&req, Lang::Ru);
    let html = templates::cabinet_profile(&chrome, &profile, flash.as_deref());
    h.done(&req, Out::html(200, html))
}

async fn cabinet_profile_save(
    State(h): State<Handler>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let req = h.req(&axum::http::Method::POST, "/cabinet/profile".into(), None, &headers, Some(body), "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let mut p = h.state.profile_now();
    // Only known fields are writable; anything else in the body is ignored.
    for field in [
        "owner_name", "handle", "location", "email", "github", "telegram", "matrix", "site_url",
        "role_ru", "role_en", "manifesto_ru", "manifesto_en", "now_ru", "now_en",
    ] {
        let value = req.form_value(field);
        match field {
            "owner_name" => p.owner_name = truncate_chars(&value, 120),
            "handle" => p.handle = truncate_chars(&value, 60),
            "location" => p.location = truncate_chars(&value, 80),
            "email" => p.email = truncate_chars(&value, 120),
            "github" => p.github = truncate_chars(&value, 200),
            "telegram" => p.telegram = truncate_chars(&value, 200),
            "matrix" => p.matrix = truncate_chars(&value, 120),
            "site_url" => p.site_url = truncate_chars(&value, 200),
            "role_ru" => p.role_ru = truncate_chars(&value, 160),
            "role_en" => p.role_en = truncate_chars(&value, 160),
            "manifesto_ru" => p.manifesto_ru = truncate_chars(&value, 1200),
            "manifesto_en" => p.manifesto_en = truncate_chars(&value, 1200),
            "now_ru" => p.now_ru = truncate_chars(&value, 800),
            "now_en" => p.now_en = truncate_chars(&value, 800),
            _ => {}
        }
    }
    match h.state.store.save_profile(&p) {
        Ok(()) => {
            h.state.invalidate_profile();
            h.done(&req, Out::redirect("/cabinet/profile?msg=saved"))
        }
        Err(err) => h.done(
            &req,
            Out::html(500, templates::server_error(&h.chrome(&req, Lang::Ru, &h.state.profile_now()))),
        ),
    }
}

async fn cabinet_telemetry(State(h): State<Handler>, headers: HeaderMap) -> Response {
    let req = h.req(&axum::http::Method::GET, "/cabinet/telemetry".into(), None, &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let profile = h.state.profile_now();
    let tele = h.state.store.telemetry().unwrap_or_default();
    let chrome = h.chrome(&req, Lang::Ru, &profile);
    let flash = flash(&req, Lang::Ru);
    let html = templates::cabinet_telemetry(&chrome, &tele, flash.as_deref());
    h.done(&req, Out::html(200, html))
}

async fn cabinet_security(State(h): State<Handler>, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let req = h.req(&axum::http::Method::GET, "/cabinet/security".into(), uri.query().map(str::to_string), &headers, None, "");
    let session = match cabinet_gate(&h, &req) {
        Ok(s) => s,
        Err(out) => return h.done(&req, out),
    };
    let profile = h.state.profile_now();
    let sessions = h.state.store.list_sessions(&session.token_hash).unwrap_or_default();
    let chrome = h.chrome(&req, Lang::Ru, &profile);
    let flash = flash(&req, Lang::Ru);
    let html = templates::cabinet_security(&chrome, &sessions, flash.as_deref(), None);
    h.done(&req, Out::html(200, html))
}

async fn cabinet_password(
    State(h): State<Handler>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let req = h.req(&axum::http::Method::POST, "/cabinet/security/password".into(), None, &headers, Some(body), "");
    let session = match cabinet_gate(&h, &req) {
        Ok(s) => s,
        Err(out) => return h.done(&req, out),
    };
    let current = req.form_value("current");
    let new = req.form_value("new");
    let repeat = req.form_value("repeat");
    let tr = crate::i18n::t(Lang::Ru);
    let profile = h.state.profile_now();
    let sessions = h.state.store.list_sessions(&session.token_hash).unwrap_or_default();
    if h.state.store.authenticate(&h.state.owner, &current).ok().flatten().is_none() {
        let chrome = h.chrome(&req, Lang::Ru, &profile);
        let html = templates::cabinet_security(&chrome, &sessions, None, Some(tr.cabinet_wrong_password));
        return h.done(&req, Out::html(200, html));
    }
    if new.len() < 12 || new != repeat {
        let chrome = h.chrome(&req, Lang::Ru, &profile);
        let html = templates::cabinet_security(&chrome, &sessions, None, Some(tr.cabinet_password_mismatch));
        return h.done(&req, Out::html(200, html));
    }
    if let Err(err) = h.state.store.set_owner_password(&h.state.owner, &new) {
        let chrome = h.chrome(&req, Lang::Ru, &profile);
        let html = templates::cabinet_security(&chrome, &sessions, None, Some(&err));
        return h.done(&req, Out::html(200, html));
    }
    h.state.store.revoke_other_sessions(&session.token_hash).ok();
    h.done(&req, Out::redirect("/cabinet/security?msg=password"))
}

async fn cabinet_revoke(State(h): State<Handler>, headers: HeaderMap, body: String) -> Response {
    let req = h.req(&axum::http::Method::POST, "/cabinet/security/revoke".into(), None, &headers, Some(body), "");
    let session = match cabinet_gate(&h, &req) {
        Ok(s) => s,
        Err(out) => return h.done(&req, out),
    };
    let token_hash = req.form_value("token");
    if token_hash != session.token_hash && token_hash.len() == 64 {
        h.state.store.revoke_session(&token_hash).ok();
    }
    h.done(&req, Out::redirect("/cabinet/security?msg=revoked"))
}

async fn cabinet_revoke_all(State(h): State<Handler>, headers: HeaderMap, body: String) -> Response {
    let req = h.req(&axum::http::Method::POST, "/cabinet/security/revoke-all".into(), None, &headers, Some(body), "");
    let session = match cabinet_gate(&h, &req) {
        Ok(s) => s,
        Err(out) => return h.done(&req, out),
    };
    h.state.store.revoke_other_sessions(&session.token_hash).ok();
    h.done(&req, Out::redirect("/cabinet/security?msg=revoked"))
}

async fn cabinet_backup(State(h): State<Handler>, headers: HeaderMap) -> Response {
    let req = h.req(&axum::http::Method::GET, "/cabinet/backup".into(), None, &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    let profile = h.state.profile_now();
    let stats = h.state.store.backup_stats().unwrap_or(BackupStats {
        articles: 0,
        published: 0,
        drafts: 0,
        views: 0,
    });
    let chrome = h.chrome(&req, Lang::Ru, &profile);
    let flash = flash(&req, Lang::Ru);
    let html = templates::cabinet_backup(&chrome, &stats, flash.as_deref());
    h.done(&req, Out::html(200, html))
}

async fn cabinet_backup_json(State(h): State<Handler>, headers: HeaderMap) -> Response {
    let req = h.req(&axum::http::Method::GET, "/cabinet/backup.json".into(), None, &headers, None, "");
    if let Err(out) = cabinet_gate(&h, &req) {
        return h.done(&req, out);
    }
    match h.state.store.export_json() {
        Ok(json) => h.done(
            &req,
            Out::new(200, "application/json; charset=utf-8", json.into_bytes())
                .header("Content-Disposition", "attachment; filename=\"rudolfzinovev-backup.json\"")
                .cache("no-store"),
        ),
        Err(_) => h.done(
            &req,
            Out::html(500, templates::server_error(&h.chrome(&req, Lang::Ru, &h.state.profile_now()))),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_skips_assets_and_private_pages() {
        assert!(should_measure("/"));
        assert!(should_measure("/notes/x"));
        assert!(!should_measure("/assets/style.css"));
        assert!(!should_measure("/cabinet"));
        assert!(!should_measure("/login"));
    }

    #[test]
    fn responses_are_compressible_by_type() {
        assert!(Out::html(200, "x".into()).is_compressible());
        assert!(Out::new(200, "application/rss+xml; charset=utf-8", Vec::new()).is_compressible());
        assert!(!Out::new(200, "image/png", Vec::new()).is_compressible());
    }
}
