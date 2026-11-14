//! Router and request orchestration: public pages, RSS/sitemap, auth, the owner
//! cabinet, telemetry capture and cache/security policy decisions.

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use crate::crypto;
use crate::db::{ArticleFilter, NewArticle, Store};
use crate::http::{self, RateLimiter, Request, Response};
use crate::i18n::{self, Lang, Theme, t};
use crate::models::{Article, BackupStats, Profile, ViewRecord, detect_lang, slug_of};
use crate::templates::{self, Chrome};

const SESSION_COOKIE: &str = "rz_session";
const CSRF_COOKIE: &str = "rz_csrf";
const THEME_COOKIE: &str = "rz_theme";
const PROFILE_TTL_SECS: i64 = 5;

pub struct App {
    pub store: Store,
    owner: String,
    generated_password: Option<String>,
    profile: Mutex<(Profile, i64)>,
    login_limiter: RateLimiter,
    write_limiter: RateLimiter,
}

impl App {
    pub fn bootstrap(data_dir: &Path, owner: &str, password: Option<&str>, seed: bool) -> Result<App, String> {
        let db_path = data_dir.join("site.sqlite");
        let store = Store::open(&db_path)?;
        let generated_password = store.bootstrap_owner(owner, password)?;
        if seed {
            store.seed_if_empty()?;
        }
        let profile = store.profile()?;
        Ok(App {
            store,
            owner: owner.to_string(),
            generated_password,
            profile: Mutex::new((profile, i18n::now_unix())),
            login_limiter: RateLimiter::new(6, 600, 600),
            write_limiter: RateLimiter::new(240, 60, 30),
        })
    }

    pub fn generated_password(&self) -> Option<&str> {
        self.generated_password.as_deref()
    }

    fn profile_now(&self) -> Profile {
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

    fn invalidate_profile(&self) {
        let mut guard = self.profile.lock().unwrap();
        guard.1 = 0;
    }

    // ------------------------------------------------------------- routing

    pub fn handle(&self, req: &Request) -> Response {
        let started = Instant::now();
        let mut resp = self.route(req);

        // Exact server render time travels in a header (visible in devtools and read
        // by app.js), so the HTML stays byte-stable and ETag revalidation works.
        if resp
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.starts_with("text/html"))
        {
            let us = started.elapsed().as_micros().max(40);
            resp.headers.push((
                "Server-Timing".into(),
                format!("render;dur={:.2};desc=\"html render\"", us as f64 / 1000.0),
            ));
        }

        // Public-page telemetry (never for the cabinet, assets or auth screens).
        if req.method == "GET" && should_measure(&req.path) {
            let lang = self.lang_for(req);
            let _ = self.store.record_view(&ViewRecord {
                ts: i18n::now_unix(),
                path: req.path.clone(),
                lang: lang.code().to_string(),
                status_code: resp.status as i64,
                render_us: started.elapsed().as_micros() as i64,
                bytes: resp.body.len() as i64,
                // Referrer is reduced to its host so nothing sensitive is stored.
                referrer: req
                    .header("referer")
                    .map(referrer_host)
                    .unwrap_or_default(),
                ua: req.user_agent().to_string(),
                is_bot: http::is_bot(req.user_agent()),
                visitor_hash: visitor_hash(req),
            });
        }
        resp
    }

    fn route(&self, req: &Request) -> Response {
        let path = req.path.as_str();
        let method = req.method.as_str();

        // ---------------------------------------------------------- assets
        if method == "GET" || method == "HEAD" {
            match path {
                "/assets/style.css" => {
                    return Response::new(200, "text/css; charset=utf-8", templates::CSS.as_bytes().to_vec())
                        .header("Cache-Control", "public, max-age=31536000, immutable");
                }
                "/assets/app.js" => {
                    return Response::new(
                        200,
                        "text/javascript; charset=utf-8",
                        templates::JS.as_bytes().to_vec(),
                    )
                    .header("Cache-Control", "public, max-age=31536000, immutable");
                }
                "/favicon.svg" | "/favicon.ico" => {
                    return Response::new(200, "image/svg+xml", templates::FAVICON.as_bytes().to_vec())
                        .header("Cache-Control", "public, max-age=31536000, immutable");
                }
                "/robots.txt" => {
                    let body = format!(
                        "User-agent: *\nAllow: /\nDisallow: /cabinet\nDisallow: /login\nSitemap: {}/sitemap.xml\n",
                        self.profile_now().site_url.trim_end_matches('/')
                    );
                    return Response::text(200, body)
                        .header("Cache-Control", "public, max-age=3600");
                }
                "/sitemap.xml" => {
                    let articles = self
                        .store
                        .list_articles(&ArticleFilter::default())
                        .unwrap_or_default();
                    return Response::new(
                        200,
                        "application/xml; charset=utf-8",
                        templates::sitemap_xml(&self.profile_now(), &articles).into_bytes(),
                    )
                    .header("Cache-Control", "public, max-age=1800");
                }
                "/healthz" => {
                    return Response::text(200, format!("ok {}\n", http::truncate_chars(crate::sqlite::Connection::sqlite_version().as_str(), 32)));
                }
                _ => {}
            }
        }

        // ------------------------------------------------------------ auth
        if path == "/login" {
            return match method {
                "GET" => self.login_page(req, None, false),
                "POST" => self.login_submit(req),
                _ => Response::empty(405),
            };
        }
        if path == "/logout" {
            if method != "POST" {
                return Response::empty(405);
            }
            if !self.csrf_ok(req) {
                return Response::text(403, "csrf rejected\n".into());
            }
            let resp = Response::redirect("/");
            return self.clear_session(req, resp);
        }

        // --------------------------------------------------------- cabinet
        if path == "/cabinet" || path.starts_with("/cabinet/") {
            return self.cabinet_route(req);
        }

        if method != "GET" && method != "HEAD" {
            return Response::empty(405);
        }

        // ---------------------------------------------------------- feeds
        let (lang, rest) = split_lang(path);
        match rest.as_str() {
            "/feed.xml" => {
                let articles = self
                    .store
                    .list_articles(&ArticleFilter {
                        lang: Some(lang.code()),
                        limit: Some(30),
                        ..Default::default()
                    })
                    .unwrap_or_default();
                let xml = templates::feed_xml(lang, &self.profile_now(), &articles);
                Response::new(200, "application/rss+xml; charset=utf-8", xml.into_bytes())
                    .header("Cache-Control", "public, max-age=600")
            }
            "/" => self.home_page(req, lang),
            "/articles" => self.articles_page(req, lang, None),
            "/about" => self.about_page(req, lang),
            "/tags" => self.tags_page(req, lang),
            _ => {
                if let Some(slug) = rest.strip_prefix("/tags/") {
                    return self.articles_page(req, lang, Some(slug));
                }
                if let Some(slug) = rest.strip_prefix("/articles/") {
                    return self.article_detail(req, lang, slug);
                }
                self.not_found(req, lang)
            }
        }
    }

    // ------------------------------------------------------------ helpers

    fn lang_for(&self, req: &Request) -> Lang {
        let (lang, _) = split_lang(&req.path);
        let _ = lang;
        // Path already encodes the language; fall back to Accept-Language only at "/".
        detect_lang(&format!("accept-language: {}", req.header("accept-language").unwrap_or("")), None)
    }

    fn theme(&self, req: &Request) -> Theme {
        if let Some(t) = req.cookie(THEME_COOKIE).and_then(|v| Theme::from_code(&v)) {
            return t;
        }
        match req.header("sec-ch-prefers-color-scheme") {
            Some("light") => Theme::Paper,
            _ => Theme::Obsidian,
        }
    }

    fn chrome<'a>(&'a self, req: &'a Request, lang: Lang, profile: &'a Profile, owner: bool) -> Chrome<'a> {
        Chrome {
            lang,
            theme: self.theme(req),
            path: &req.path,
            profile,
            owner,
        }
    }

    fn is_owner(&self, req: &Request) -> bool {
        self.session(req).is_some()
    }

    fn session(&self, req: &Request) -> Option<crate::models::SessionInfo> {
        let token = req.cookie(SESSION_COOKIE)?;
        if token.len() != 64 {
            return None;
        }
        self.store.session_by_token(&token).ok().flatten()
    }

    fn session_cookie(&self, req: &Request, token: &str) -> String {
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

    fn clear_session(&self, req: &Request, mut resp: Response) -> Response {
        if let Some(token) = req.cookie(SESSION_COOKIE) {
            let hash = crypto::sha256_hex(token.as_bytes());
            let _ = self.store.revoke_session(&hash);
        }
        let secure = req
            .header("x-forwarded-proto")
            .map(|p| p.eq_ignore_ascii_case("https"))
            .unwrap_or(false);
        resp.headers.push((
            "Set-Cookie".into(),
            format!(
                "{SESSION_COOKIE}=deleted; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
                if secure { "; Secure" } else { "" }
            ),
        ));
        resp
    }

    /// CSRF for state-changing cabinet requests: Origin/Referer must match Host.
    fn csrf_ok(&self, req: &Request) -> bool {
        if let Some(fetch_site) = req.header("sec-fetch-site")
            && (fetch_site.eq_ignore_ascii_case("cross-site") || fetch_site.eq_ignore_ascii_case("same-site"))
        {
            return false;
        }
        let Some(host) = req.header("host") else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        if let Some(origin) = req.header("origin") {
            let origin = origin.to_ascii_lowercase();
            let stripped = origin
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            return stripped == host;
        }
        if let Some(referer) = req.header("referer") {
            let referer = referer.to_ascii_lowercase();
            let stripped = referer
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            return stripped == host || stripped.starts_with(&format!("{host}/"));
        }
        false
    }

    fn flash(&self, req: &Request) -> Option<String> {
        let code = req.query_param("msg")?;
        let lang = self.lang_for(req);
        let tr = t(lang);
        let message = match code {
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

    // --------------------------------------------------------- public pages

    fn home_page(&self, req: &Request, lang: Lang) -> Response {
        let profile = self.profile_now();
        let lang_code = lang.code();
        let latests = self
            .store
            .list_articles(&ArticleFilter {
                lang: Some(lang_code),
                limit: Some(5),
                ..Default::default()
            })
            .unwrap_or_default();
        let all = self
            .store
            .list_articles(&ArticleFilter {
                lang: Some(lang_code),
                ..Default::default()
            })
            .unwrap_or_default();
        let tags = self.store.tag_cloud(lang_code).unwrap_or_default();
        let chrome = self.chrome(req, lang, &profile, self.is_owner(req));
        let html = templates::home(&chrome, &latests, &tags, all.len() as i64);
        Response::html(200, html)
    }

    fn articles_page(&self, req: &Request, lang: Lang, tag: Option<&str>) -> Response {
        let profile = self.profile_now();
        let query = req
            .query_param("q")
            .map(|q| http::truncate_chars(q.trim(), 80))
            .filter(|q| !q.is_empty());
        let articles = self
            .store
            .list_articles(&ArticleFilter {
                lang: Some(lang.code()),
                tag,
                query: query.as_deref(),
                ..Default::default()
            })
            .unwrap_or_default();
        let tags = self.store.tag_cloud(lang.code()).unwrap_or_default();
        let chrome = self.chrome(req, lang, &profile, self.is_owner(req));
        let html = templates::articles_index(&chrome, &articles, &tags, tag, query.as_deref());
        Response::html(200, html)
    }

    fn tags_page(&self, req: &Request, lang: Lang) -> Response {
        let profile = self.profile_now();
        let tags = self.store.tag_cloud(lang.code()).unwrap_or_default();
        let all = self
            .store
            .list_articles(&ArticleFilter {
                lang: Some(lang.code()),
                ..Default::default()
            })
            .unwrap_or_default();
        let chrome = self.chrome(req, lang, &profile, self.is_owner(req));
        let html = templates::articles_index(&chrome, &all, &tags, None, None);
        Response::html(200, html)
    }

    fn about_page(&self, req: &Request, lang: Lang) -> Response {
        let profile = self.profile_now();
        let tags = self.store.tag_cloud(lang.code()).unwrap_or_default();
        let count = self
            .store
            .list_articles(&ArticleFilter {
                lang: Some(lang.code()),
                ..Default::default()
            })
            .map(|a| a.len() as i64)
            .unwrap_or(0);
        let chrome = self.chrome(req, lang, &profile, self.is_owner(req));
        let html = templates::about(&chrome, count, &tags);
        Response::html(200, html)
    }

    fn article_detail(&self, req: &Request, lang: Lang, slug: &str) -> Response {
        let slug = slug.trim();
        let article = match self.store.article_by_slug(lang.code(), slug) {
            Ok(Some(a)) if a.is_published() => a,
            _ => return self.not_found(req, lang),
        };
        let other = self
            .store
            .translation_of(&article.translation_key, lang.other().code())
            .ok()
            .flatten()
            .filter(|a| a.is_published());
        let same_lang = self
            .store
            .list_articles(&ArticleFilter {
                lang: Some(lang.code()),
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
        // The counter feeds the cabinet; it is deliberately not rendered on the
        // public page so the HTML stays byte-stable for ETag revalidation.
        let _ = self.store.increment_views(article.id);
        let profile = self.profile_now();
        let chrome = self.chrome(req, lang, &profile, self.is_owner(req));
        let html = templates::article_page(&chrome, &article, other.as_ref(), &related);
        Response::html(200, html)
    }

    fn not_found(&self, req: &Request, lang: Lang) -> Response {
        let profile = self.profile_now();
        let chrome = self.chrome(req, lang, &profile, self.is_owner(req));
        Response::html(404, templates::not_found(&chrome))
    }

    // ---------------------------------------------------------------- auth

    fn login_page(&self, req: &Request, error: Option<&str>, limited: bool) -> Response {
        let profile = self.profile_now();
        let lang = detect_lang(
            &format!("accept-language: {}", req.header("accept-language").unwrap_or("")),
            None,
        );
        let existing = req.cookie(CSRF_COOKIE).filter(|v| v.len() == 48);
        let csrf = existing.unwrap_or_else(|| crypto::random_token_hex(24));
        let mut chrome = self.chrome(req, lang, &profile, false);
        chrome.path = "/login";
        let html = templates::login(&chrome, error, &csrf, limited);
        let mut resp = Response::html(if error.is_some() { 401 } else { 200 }, html);
        resp.headers.push((
            "Set-Cookie".into(),
            format!("{CSRF_COOKIE}={csrf}; Path=/; HttpOnly; SameSite=Strict; Max-Age=1800"),
        ));
        resp
    }

    fn login_submit(&self, req: &Request) -> Response {
        let limited = !self.login_limiter.check(&req.remote_ip);
        if limited {
            return self.login_page(req, Some(t(Lang::Ru).login_rate_limited), true);
        }
        let form = req.form();
        let supplied_csrf = form.get("csrf").cloned().unwrap_or_default();
        let cookie_csrf = req.cookie(CSRF_COOKIE).unwrap_or_default();
        if supplied_csrf.is_empty() || supplied_csrf != cookie_csrf {
            return self.login_page(req, Some("CSRF token mismatch."), false);
        }
        let username = form.get("username").cloned().unwrap_or_default();
        let password = form.get("password").cloned().unwrap_or_default();
        match self.store.authenticate(&username, &password) {
            Ok(Some(user_id)) => {
                self.login_limiter.reset(&req.remote_ip);
                let ip_hash = crypto::sha256_hex(req.remote_ip.as_bytes());
                match self.store.create_session(user_id, &ip_hash, req.user_agent()) {
                    Ok((token, _csrf)) => {
                        let mut resp = Response::redirect("/cabinet");
                        resp.headers
                            .push(("Set-Cookie".into(), self.session_cookie(req, &token)));
                        resp
                    }
                    Err(err) => self.error_page(req, &err),
                }
            }
            Ok(None) => {
                self.login_limiter.penalize(&req.remote_ip);
                self.login_page(req, Some(t(Lang::Ru).login_error), false)
            }
            Err(err) => self.error_page(req, &err),
        }
    }

    fn error_page(&self, req: &Request, err: &str) -> Response {
        eprintln!("[error] {} {}: {}", req.method, req.path, err);
        let profile = self.profile_now();
        let lang = self.lang_for(req);
        let chrome = self.chrome(req, lang, &profile, false);
        Response::html(500, templates::server_error(&chrome))
    }

    // ------------------------------------------------------------- cabinet

    fn cabinet_route(&self, req: &Request) -> Response {
        let Some(session) = self.session(req) else {
            if req.method == "GET" && req.path == "/cabinet" {
                return Response::redirect("/login");
            }
            return Response::text(401, "authentication required\n".into());
        };
        let method = req.method.as_str();
        let mutating = method == "POST";
        if mutating && !self.csrf_ok(req) {
            return Response::text(403, "csrf rejected: send Origin matching Host\n".into());
        }
        if mutating && !self.write_limiter.check(&req.remote_ip) {
            return Response::text(429, "slow down\n".into());
        }

        let path = req.path.as_str();
        match (method, path) {
            ("GET", "/cabinet") => return self.cabinet_overview(req),
            ("GET", "/cabinet/articles") => return self.cabinet_articles_page(req),
            ("GET", "/cabinet/articles/new") => return self.cabinet_editor_page(req, None, &[]),
            ("POST", "/cabinet/articles/new") => return self.cabinet_editor_save(req, None),
            ("GET", "/cabinet/profile") => return self.cabinet_profile_page(req, None),
            ("POST", "/cabinet/profile") => return self.cabinet_profile_save(req),
            ("GET", "/cabinet/telemetry") => return self.cabinet_telemetry_page(req),
            ("GET", "/cabinet/security") => return self.cabinet_security_page(req, None, None),
            ("GET", "/cabinet/backup") => return self.cabinet_backup_page(req),
            ("GET", "/cabinet/backup.json") => return self.cabinet_backup_json(req),
            ("POST", "/cabinet/security/password") => return self.cabinet_password_change(req, &session),
            ("POST", "/cabinet/security/revoke") => {
                let token_hash = req.form().get("token").cloned().unwrap_or_default();
                if token_hash != session.token_hash && token_hash.len() == 64 {
                    self.store.revoke_session(&token_hash).ok();
                }
                Response::redirect("/cabinet/security?msg=revoked")
            }
            ("POST", "/cabinet/security/revoke-all") => {
                self.store.revoke_other_sessions(&session.token_hash).ok();
                Response::redirect("/cabinet/security?msg=revoked")
            }
            _ => {
                if let Some(rest) = path.strip_prefix("/cabinet/articles/") {
                    match (method, rest) {
                        ("GET", id) if !id.contains('/') => {
                            let Ok(id) = id.parse::<i64>() else {
                                return Response::text(400, "bad id\n".into());
                            };
                            let article = self.store.article_by_id(id).ok().flatten();
                            return self.cabinet_editor_page(req, article.as_ref(), &[]);
                        }
                        ("POST", id) if !id.contains('/') => {
                            let Ok(id) = id.parse::<i64>() else {
                                return Response::text(400, "bad id\n".into());
                            };
                            return self.cabinet_editor_save(req, Some(id));
                        }
                        ("POST", rest) if rest.ends_with("/delete") => {
                            let id = rest.trim_end_matches("/delete");
                            if let Ok(id) = id.parse::<i64>() {
                                self.store.delete_article(id).ok();
                            }
                            Response::redirect("/cabinet/articles?msg=deleted")
                        }
                        ("POST", rest) if rest.ends_with("/status") => {
                            let id = rest.trim_end_matches("/status");
                            let status = req.form().get("status").cloned().unwrap_or_default();
                            if let Ok(id) = id.parse::<i64>()
                                && (status == "draft" || status == "published")
                            {
                                self.store.set_article_status(id, &status).ok();
                            }
                            let msg = if status == "published" { "published" } else { "draft" };
                            Response::redirect(&format!("/cabinet/articles?msg={msg}"))
                        }
                        _ => Response::empty(405),
                    }
                } else {
                    Response::empty(404)
                }
            }
        }
    }

    fn cabinet_overview(&self, req: &Request) -> Response {
        let profile = self.profile_now();
        let stats = self.store.backup_stats().unwrap_or(BackupStats {
            articles: 0,
            published: 0,
            drafts: 0,
            views: 0,
        });
        let tele = self.store.telemetry().unwrap_or_default();
        let recent = self
            .store
            .list_articles(&ArticleFilter {
                include_drafts: true,
                limit: Some(8),
                ..Default::default()
            })
            .unwrap_or_default();
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let flash = self.flash(req);
        Response::html(
            200,
            templates::cabinet_overview(&chrome, &stats, &tele, &recent, flash.as_deref()),
        )
    }

    fn cabinet_articles_page(&self, req: &Request) -> Response {
        let profile = self.profile_now();
        let articles = self
            .store
            .list_articles(&ArticleFilter {
                include_drafts: true,
                ..Default::default()
            })
            .unwrap_or_default();
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let flash = self.flash(req);
        Response::html(
            200,
            templates::cabinet_articles(&chrome, &articles, flash.as_deref()),
        )
    }

    fn cabinet_editor_page(&self, req: &Request, article: Option<&Article>, errors: &[String]) -> Response {
        let profile = self.profile_now();
        let tags = self.store.tag_cloud("ru").unwrap_or_default();
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let flash = self.flash(req);
        Response::html(
            200,
            templates::cabinet_editor(&chrome, article, &tags, flash.as_deref(), errors),
        )
    }

    fn cabinet_editor_save(&self, req: &Request, id: Option<i64>) -> Response {
        let form = req.form();
        let get = |k: &str| form.get(k).cloned().unwrap_or_default();
        let title = http::truncate_chars(get("title").trim(), 180);
        if title.is_empty() {
            return self.cabinet_editor_page(
                req,
                id.and_then(|i| self.store.article_by_id(i).ok().flatten()).as_ref(),
                &["Title is required.".to_string()],
            );
        }
        let lang = if get("lang") == "en" { "en" } else { "ru" };
        let mut slug = get("slug").trim().to_lowercase();
        if slug.is_empty() {
            slug = slug_of(&title);
        }
        let status = if get("status") == "published" { "published" } else { "draft" };
        let featured = get("featured") == "1";
        let summary = http::truncate_chars(get("summary").trim(), 320);
        let body = get("body_md");
        let tags = get("tags")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let published_at = get("published_at").parse::<i64>().ok().filter(|v| *v > 0);

        // Slug collisions inside the same language get a suffix instead of a constraint error.
        let mut candidate = slug.clone();
        let mut n = 2;
        loop {
            let clash = self
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

        let input = NewArticle {
            translation_key: None,
            lang,
            slug: &candidate,
            title: &title,
            summary: &summary,
            body_md: &body,
            tags: &tags,
            status,
            featured,
            published_at,
        };

        let outcome = match id {
            Some(id) => self.store.update_article(id, &input).map(|_| id),
            None => {
                // Pair the new entry with an existing translation when one matches by tags+title.
                self.store.create_article(&input)
            }
        };
        match outcome {
            Ok(saved) => Response::redirect(&format!("/cabinet/articles/{saved}?msg=saved")),
            Err(err) => {
                let article = id.and_then(|i| self.store.article_by_id(i).ok().flatten());
                self.cabinet_editor_page(req, article.as_ref(), &[err])
            }
        }
    }

    fn cabinet_profile_page(&self, req: &Request, flash: Option<&str>) -> Response {
        let profile = self.profile_now();
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let owned = flash.map(|f| f.to_string()).or_else(|| self.flash(req));
        Response::html(
            200,
            templates::cabinet_profile(&chrome, &profile, owned.as_deref()),
        )
    }

    fn cabinet_profile_save(&self, req: &Request) -> Response {
        let form = req.form();
        let get = |k: &str| form.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
        let mut p = self.store.profile().unwrap_or_default();
        let assign = |slot: &mut String, value: String, max: usize| {
            *slot = http::truncate_chars(value.trim(), max);
        };
        assign(&mut p.owner_name, get("owner_name"), 80);
        assign(&mut p.handle, get("handle"), 40);
        assign(&mut p.role_ru, get("role_ru"), 120);
        assign(&mut p.role_en, get("role_en"), 120);
        assign(&mut p.manifesto_ru, get("manifesto_ru"), 900);
        assign(&mut p.manifesto_en, get("manifesto_en"), 900);
        assign(&mut p.now_ru, get("now_ru"), 900);
        assign(&mut p.now_en, get("now_en"), 900);
        assign(&mut p.location, get("location"), 80);
        assign(&mut p.email, get("email"), 120);
        assign(&mut p.github, get("github"), 200);
        assign(&mut p.telegram, get("telegram"), 200);
        assign(&mut p.matrix, get("matrix"), 120);
        assign(&mut p.site_url, get("site_url"), 200);
        if p.site_url.is_empty() {
            p.site_url = "https://rudolfzinovev.me".into();
        }
        match self.store.save_profile(&p) {
            Ok(()) => {
                self.invalidate_profile();
                Response::redirect("/cabinet/profile?msg=saved")
            }
            Err(err) => self.cabinet_profile_page(req, Some(&err)),
        }
    }

    fn cabinet_telemetry_page(&self, req: &Request) -> Response {
        let profile = self.profile_now();
        let tele = self.store.telemetry().unwrap_or_default();
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let flash = self.flash(req);
        Response::html(
            200,
            templates::cabinet_telemetry(&chrome, &tele, flash.as_deref()),
        )
    }

    fn cabinet_security_page(&self, req: &Request, flash: Option<&str>, error: Option<&str>) -> Response {
        let profile = self.profile_now();
        let session = self.session(req);
        let current = session.as_ref().map(|s| s.token_hash.clone()).unwrap_or_default();
        let sessions = self.store.list_sessions(&current).unwrap_or_default();
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let owned = flash.map(|f| f.to_string()).or_else(|| self.flash(req));
        Response::html(
            200,
            templates::cabinet_security(&chrome, &sessions, owned.as_deref(), error),
        )
    }

    fn cabinet_password_change(&self, req: &Request, session: &crate::models::SessionInfo) -> Response {
        let form = req.form();
        let current = form.get("current").cloned().unwrap_or_default();
        let new = form.get("new").cloned().unwrap_or_default();
        let repeat = form.get("repeat").cloned().unwrap_or_default();
        let tr = t(Lang::Ru);
        if self.store.authenticate(&self.owner, &current).ok().flatten().is_none() {
            return self.cabinet_security_page(req, None, Some(tr.cabinet_wrong_password));
        }
        if new.len() < 12 || new != repeat {
            return self.cabinet_security_page(req, None, Some(tr.cabinet_password_mismatch));
        }
        if let Err(err) = self.store.set_owner_password(&self.owner, &new) {
            return self.cabinet_security_page(req, None, Some(&err));
        }
        self.store.revoke_other_sessions(&session.token_hash).ok();
        Response::redirect("/cabinet/security?msg=password")
    }

    fn cabinet_backup_page(&self, req: &Request) -> Response {
        let profile = self.profile_now();
        let stats = self.store.backup_stats().unwrap_or(BackupStats {
            articles: 0,
            published: 0,
            drafts: 0,
            views: 0,
        });
        let chrome = self.chrome(req, Lang::Ru, &profile, true);
        let flash = self.flash(req);
        Response::html(
            200,
            templates::cabinet_backup(&chrome, &stats, flash.as_deref()),
        )
    }

    fn cabinet_backup_json(&self, req: &Request) -> Response {
        match self.store.export_json() {
            Ok(json) => Response::json(200, json)
                .header(
                    "Content-Disposition",
                    "attachment; filename=\"rudolfzinovev-backup.json\"",
                )
                .header("Cache-Control", "no-store"),
            Err(err) => self.error_page(req, &err),
        }
    }
}

fn split_lang(path: &str) -> (Lang, String) {
    if path == "/en" {
        return (Lang::En, "/".to_string());
    }
    if let Some(rest) = path.strip_prefix("/en/") {
        return (Lang::En, format!("/{rest}"));
    }
    (Lang::Ru, path.to_string())
}

fn should_measure(path: &str) -> bool {
    !(path.starts_with("/assets")
        || path.starts_with("/cabinet")
        || path == "/login"
        || path == "/healthz"
        || path.starts_with("/favicon")
        || path == "/robots.txt")
}

/// Keep only the host of a referrer; never store paths or query strings.
fn referrer_host(referrer: &str) -> String {
    let stripped = referrer
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    http::truncate_chars(stripped.split('/').next().unwrap_or(""), 80)
}

fn visitor_hash(req: &Request) -> String {
    let mut buf = String::with_capacity(96);
    buf.push_str(&req.remote_ip);
    buf.push('|');
    buf.push_str(req.user_agent());
    buf.push_str("|rz-visitor|");
    // Day-scoped so the identifier cannot be used for long-term tracking.
    buf.push_str(&i18n::format_date_utc(i18n::now_unix()));
    crypto::sha256_hex(buf.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_app(name: &str) -> App {
        let dir = std::env::temp_dir().join(format!("rz-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::bootstrap(&dir, "owner", Some("test-password-123"), true).expect("app");
        // Deterministic owner password for the auth test.
        app.store.set_owner_password("owner", "test-password-123").unwrap();
        app.generated_password = None;
        app
    }

    fn request(method: &str, path: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: vec![
                ("host".into(), "localhost:8080".into()),
                ("user-agent".into(), "test-agent".into()),
                ("accept-encoding".into(), "gzip".into()),
            ],
            body: Vec::new(),
            remote_ip: "127.0.0.1".into(),
        }
    }

    #[test]
    fn public_pages_and_language_prefixes() {
        let app = temp_app("public");
        let home = app.handle(&request("GET", "/"));
        assert_eq!(home.status, 200);
        let html = String::from_utf8(home.body).unwrap();
        assert!(html.contains("lang=\"ru\""));
        assert!(!html.contains("@@RENDER_US@@"));

        let en = app.handle(&request("GET", "/en"));
        assert_eq!(en.status, 200);
        assert!(String::from_utf8(en.body).unwrap().contains("lang=\"en\""));

        let missing = app.handle(&request("GET", "/nope"));
        assert_eq!(missing.status, 404);

        let feed = app.handle(&request("GET", "/en/feed.xml"));
        assert_eq!(feed.status, 200);
        assert!(String::from_utf8(feed.body).unwrap().contains("<rss"));
    }

    #[test]
    fn article_detail_and_escalation_guard() {
        let app = temp_app("detail");
        let resp = app.handle(&request("GET", "/articles/http-server-bez-krejtov"));
        assert_eq!(resp.status, 200);
        let cabinet = app.handle(&request("GET", "/cabinet"));
        assert_eq!(cabinet.status, 303);
        assert_eq!(cabinet.headers.iter().find(|(k, _)| k == "Location").unwrap().1, "/login");
    }

    #[test]
    fn login_flow_sets_session_and_csrf() {
        let app = temp_app("login");
        let page = app.handle(&request("GET", "/login"));
        assert_eq!(page.status, 200);
        let cookie = page
            .headers
            .iter()
            .filter(|(k, _)| k == "Set-Cookie")
            .map(|(_, v)| v.clone())
            .find(|v| v.starts_with("rz_csrf="))
            .expect("csrf cookie");
        let csrf = cookie
            .trim_start_matches("rz_csrf=")
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let mut req = request("POST", "/login");
        req.headers.push(("cookie".into(), format!("rz_csrf={csrf}")));
        req.headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        req.body = format!("csrf={csrf}&username=owner&password=test-password-123").into_bytes();
        let resp = app.handle(&req);
        assert_eq!(resp.status, 303);
        let session_cookie = resp
            .headers
            .iter()
            .find(|(k, v)| k == "Set-Cookie" && v.starts_with("rz_session="))
            .map(|(_, v)| v.clone())
            .expect("session cookie");
        let token = session_cookie
            .trim_start_matches("rz_session=")
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let mut cabinet_req = request("GET", "/cabinet");
        cabinet_req
            .headers
            .push(("cookie".into(), format!("rz_session={token}")));
        let cabinet = app.handle(&cabinet_req);
        assert_eq!(cabinet.status, 200);
        assert!(String::from_utf8(cabinet.body).unwrap().contains("Overview"));

        // Mutating request without Origin/Referer must be rejected.
        let mut post = request("POST", "/cabinet/profile");
        post.headers.push(("cookie".into(), format!("rz_session={token}")));
        post.headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        post.body = b"owner_name=Hacked".to_vec();
        assert_eq!(app.handle(&post).status, 403);

        // With a matching Origin it goes through.
        post.headers.push(("origin".into(), "http://localhost:8080".into()));
        assert_eq!(app.handle(&post).status, 303);
        assert_eq!(app.store.profile().unwrap().owner_name, "Hacked");
    }

    fn login_cookie(app: &App) -> String {
        let page = app.handle(&request("GET", "/login"));
        let csrf = page
            .headers
            .iter()
            .find(|(k, v)| k == "Set-Cookie" && v.starts_with("rz_csrf="))
            .map(|(_, v)| v.trim_start_matches("rz_csrf=").split(';').next().unwrap().to_string())
            .expect("csrf cookie");
        let mut req = request("POST", "/login");
        req.headers.push(("cookie".into(), format!("rz_csrf={csrf}")));
        req.headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        req.body = format!("csrf={csrf}&username=owner&password=test-password-123").into_bytes();
        let resp = app.handle(&req);
        assert_eq!(resp.status, 303);
        resp.headers
            .iter()
            .find(|(k, v)| k == "Set-Cookie" && v.starts_with("rz_session="))
            .map(|(_, v)| v.trim_start_matches("rz_session=").split(';').next().unwrap().to_string())
            .expect("session cookie")
    }

    #[test]
    fn cabinet_write_paths_return_redirects() {
        let app = temp_app("writes");
        let token = login_cookie(&app);
        let cookie = format!("rz_session={token}");

        // create a draft
        let mut create = request("POST", "/cabinet/articles/new");
        create.headers.push(("cookie".into(), cookie.clone()));
        create.headers.push(("origin".into(), "http://localhost:8080".into()));
        create.headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        create.body = b"title=Draft+entry&slug=draft-entry&lang=ru&summary=s&body_md=%23+h&tags=t&status=draft".to_vec();
        let resp = app.handle(&create);
        assert_eq!(resp.status, 303, "create must redirect, not fall through");
        let location = resp.headers.iter().find(|(k, _)| k == "Location").unwrap().1.clone();
        assert!(location.starts_with("/cabinet/articles/"));
        let id: i64 = location
            .trim_start_matches("/cabinet/articles/")
            .split('?')
            .next()
            .unwrap()
            .parse()
            .unwrap();

        // draft is not public
        assert_eq!(app.handle(&request("GET", "/articles/draft-entry")).status, 404);

        // publish it
        let mut publish = request("POST", &format!("/cabinet/articles/{id}/status"));
        publish.headers.push(("cookie".into(), cookie.clone()));
        publish.headers.push(("origin".into(), "http://localhost:8080".into()));
        publish.headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        publish.body = b"status=published".to_vec();
        let resp = app.handle(&publish);
        assert_eq!(resp.status, 303);
        assert_eq!(app.handle(&request("GET", "/articles/draft-entry")).status, 200);

        // delete it
        let mut delete = request("POST", &format!("/cabinet/articles/{id}/delete"));
        delete.headers.push(("cookie".into(), cookie.clone()));
        delete.headers.push(("origin".into(), "http://localhost:8080".into()));
        delete.body.clear();
        let resp = app.handle(&delete);
        assert_eq!(resp.status, 303);
        assert_eq!(app.handle(&request("GET", "/articles/draft-entry")).status, 404);
    }

    #[test]
    fn homepage_reports_render_time_and_is_compressible() {
        let app = temp_app("cache");
        let resp = app.handle(&request("GET", "/"));
        assert!(resp.is_compressible());
        let html = String::from_utf8(resp.body).unwrap();
        assert!(html.contains("data-render-us"));
        assert!(!html.contains("@@RENDER_US@@"));
    }
}
