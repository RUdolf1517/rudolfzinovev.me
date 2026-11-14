//! Persistence layer: schema, migrations, owner bootstrap, bilingual seed content,
//! article CRUD, site profile, sessions, telemetry and JSON backup export.

use std::path::Path;

use crate::crypto;
use crate::i18n::{self, now_unix};
use crate::markdown;
use crate::models::{Article, BackupStats, Profile, SessionInfo, Telemetry, ViewRecord, slug_of};
use crate::sqlite::{Connection, SqlParam};

pub const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;

const ARTICLE_COLS: &str = "id, translation_key, lang, slug, title, summary, body_md, tags, \
     status, featured, reading_minutes, published_at, updated_at, created_at, views";

pub struct Store {
    db: Connection,
}

#[derive(Debug)]
pub struct NewArticle<'a> {
    pub translation_key: Option<&'a str>,
    pub lang: &'a str,
    pub slug: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub body_md: &'a str,
    pub tags: &'a str,
    pub status: &'a str,
    pub featured: bool,
    pub published_at: Option<i64>,
}

#[derive(Debug, Default, Clone)]
pub struct ArticleFilter<'a> {
    pub lang: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub query: Option<&'a str>,
    pub include_drafts: bool,
    pub limit: Option<i64>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        let db = Connection::open(path)?;
        let store = Store { db };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), String> {
        self.db.exec_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS users (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 username TEXT NOT NULL UNIQUE,
                 password_hash TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS profile (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 owner_name TEXT NOT NULL,
                 handle TEXT NOT NULL,
                 role_ru TEXT NOT NULL,
                 role_en TEXT NOT NULL,
                 manifesto_ru TEXT NOT NULL,
                 manifesto_en TEXT NOT NULL,
                 now_ru TEXT NOT NULL,
                 now_en TEXT NOT NULL,
                 location TEXT NOT NULL,
                 email TEXT NOT NULL,
                 github TEXT NOT NULL,
                 telegram TEXT NOT NULL,
                 matrix TEXT NOT NULL,
                 site_url TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS articles (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 translation_key TEXT NOT NULL,
                 lang TEXT NOT NULL,
                 slug TEXT NOT NULL,
                 title TEXT NOT NULL,
                 summary TEXT NOT NULL DEFAULT '',
                 body_md TEXT NOT NULL DEFAULT '',
                 tags TEXT NOT NULL DEFAULT '',
                 status TEXT NOT NULL DEFAULT 'draft',
                 featured INTEGER NOT NULL DEFAULT 0,
                 reading_minutes INTEGER NOT NULL DEFAULT 1,
                 published_at INTEGER NOT NULL DEFAULT 0,
                 updated_at INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL DEFAULT 0,
                 views INTEGER NOT NULL DEFAULT 0,
                 UNIQUE (lang, slug)
             );
             CREATE INDEX IF NOT EXISTS idx_articles_lang_status
                 ON articles (lang, status, published_at DESC);
             CREATE TABLE IF NOT EXISTS sessions (
                 token_hash TEXT PRIMARY KEY,
                 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 csrf TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL,
                 last_seen INTEGER NOT NULL,
                 ip_hash TEXT NOT NULL DEFAULT '',
                 ua TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS views (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts INTEGER NOT NULL,
                 path TEXT NOT NULL,
                 lang TEXT NOT NULL,
                 status_code INTEGER NOT NULL,
                 render_us INTEGER NOT NULL,
                 bytes INTEGER NOT NULL,
                 referrer TEXT NOT NULL DEFAULT '',
                 ua TEXT NOT NULL DEFAULT '',
                 is_bot INTEGER NOT NULL DEFAULT 0,
                 visitor_hash TEXT NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_views_ts ON views (ts DESC);
             INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');",
        )
    }

    // ---------------------------------------------------------------- owner

    pub fn owner_exists(&self) -> Result<bool, String> {
        let row = self
            .db
            .query_one("SELECT COUNT(*) FROM users", &[])?;
        Ok(row.map(|r| r[0].as_i64() > 0).unwrap_or(false))
    }

    /// Creates the single owner account. Returns the generated password when none was supplied.
    pub fn bootstrap_owner(&self, username: &str, password: Option<&str>) -> Result<Option<String>, String> {
        if self.owner_exists()? {
            return Ok(None);
        }
        let (pw, generated) = match password {
            Some(p) if p.len() >= 8 => (p.to_string(), None),
            _ => {
                let generated = crypto::random_token_hex(9);
                (generated.clone(), Some(generated))
            }
        };
        let hash = crypto::hash_password(&pw);
        let ts = now_unix();
        self.db.insert(
            "INSERT INTO users (username, password_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            &[SqlParam::Text(username), SqlParam::Text(&hash), SqlParam::Int(ts)],
        )?;
        Ok(generated)
    }

    pub fn set_owner_password(&self, username: &str, new_password: &str) -> Result<(), String> {
        let hash = crypto::hash_password(new_password);
        self.db.execute(
            "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE username = ?3",
            &[
                SqlParam::Text(&hash),
                SqlParam::Int(now_unix()),
                SqlParam::Text(username),
            ],
        )?;
        Ok(())
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<Option<i64>, String> {
        let row = self.db.query_one(
            "SELECT id, password_hash FROM users WHERE username = ?1",
            &[SqlParam::Text(username)],
        )?;
        let Some(row) = row else {
            // Constant-ish work even for unknown users to keep timings comparable.
            let _ = crypto::verify_password(
                password,
                "pbkdf2_sha256$210000$00000000000000000000000000000000$0000000000000000000000000000000000000000000000000000000000000000",
            );
            return Ok(None);
        };
        let id = row[0].as_i64();
        let hash = row[1].as_str();
        if crypto::verify_password(password, hash) {
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    // -------------------------------------------------------------- sessions

    pub fn create_session(&self, user_id: i64, ip_hash: &str, ua: &str) -> Result<(String, String), String> {
        let token = crypto::random_token_hex(32);
        let token_hash = crypto::sha256_hex(token.as_bytes());
        let csrf = crypto::random_token_hex(24);
        let ts = now_unix();
        self.db.execute(
            "INSERT INTO sessions (token_hash, user_id, csrf, created_at, expires_at, last_seen, ip_hash, ua)
             VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?6, ?7)",
            &[
                SqlParam::Text(&token_hash),
                SqlParam::Int(user_id),
                SqlParam::Text(&csrf),
                SqlParam::Int(ts),
                SqlParam::Int(ts + SESSION_TTL_SECS),
                SqlParam::Text(ip_hash),
                SqlParam::Text(&crate::http::truncate_chars(ua, 120)),
            ],
        )?;
        Ok((token, csrf))
    }

    /// Resolve a session token, refreshing `last_seen` at most once a minute.
    pub fn session_by_token(&self, token: &str) -> Result<Option<SessionInfo>, String> {
        let token_hash = crypto::sha256_hex(token.as_bytes());
        let now = now_unix();
        let row = self.db.query_one(
            "SELECT token_hash, csrf, created_at, expires_at, last_seen, ip_hash, ua
             FROM sessions WHERE token_hash = ?1 AND expires_at > ?2",
            &[SqlParam::Text(&token_hash), SqlParam::Int(now)],
        )?;
        let Some(row) = row else {
            return Ok(None);
        };
        let last_seen = row[4].as_i64();
        if now - last_seen > 60 {
            self.db.execute(
                "UPDATE sessions SET last_seen = ?1 WHERE token_hash = ?2",
                &[SqlParam::Int(now), SqlParam::Text(&token_hash)],
            )?;
        }
        Ok(Some(SessionInfo {
            token_hash,
            csrf: row[1].as_str().to_string(),
            created_at: row[2].as_i64(),
            expires_at: row[3].as_i64(),
            last_seen: now.max(last_seen),
            ip_hash: row[5].as_str().to_string(),
            ua: row[6].as_str().to_string(),
            current: true,
        }))
    }

    pub fn list_sessions(&self, current_hash: &str) -> Result<Vec<SessionInfo>, String> {
        let rows = self.db.query_rows(
            "SELECT token_hash, csrf, created_at, expires_at, last_seen, ip_hash, ua
             FROM sessions WHERE expires_at > ?1 ORDER BY last_seen DESC",
            &[SqlParam::Int(now_unix())],
        )?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(SessionInfo {
                    token_hash: r[0].as_str().to_string(),
                    csrf: r[1].as_str().to_string(),
                    created_at: r[2].as_i64(),
                    expires_at: r[3].as_i64(),
                    last_seen: r[4].as_i64(),
                    ip_hash: r[5].as_str().to_string(),
                    ua: r[6].as_str().to_string(),
                    current: r[0].as_str() == current_hash,
                })
            })
            .collect())
    }

    pub fn revoke_session(&self, token_hash: &str) -> Result<(), String> {
        self.db.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            &[SqlParam::Text(token_hash)],
        )?;
        Ok(())
    }

    pub fn revoke_other_sessions(&self, keep_hash: &str) -> Result<usize, String> {
        self.db.execute(
            "DELETE FROM sessions WHERE token_hash <> ?1",
            &[SqlParam::Text(keep_hash)],
        )
    }

    // -------------------------------------------------------------- articles

    pub fn article_by_id(&self, id: i64) -> Result<Option<Article>, String> {
        let sql = format!("SELECT {ARTICLE_COLS} FROM articles WHERE id = ?1");
        let row = self.db.query_one(&sql, &[SqlParam::Int(id)])?;
        Ok(row.as_deref().and_then(Article::from_row))
    }

    pub fn article_by_slug(&self, lang: &str, slug: &str) -> Result<Option<Article>, String> {
        let sql = format!("SELECT {ARTICLE_COLS} FROM articles WHERE lang = ?1 AND slug = ?2");
        let row = self.db.query_one(&sql, &[SqlParam::Text(lang), SqlParam::Text(slug)])?;
        Ok(row.as_deref().and_then(Article::from_row))
    }

    pub fn translation_of(&self, key: &str, lang: &str) -> Result<Option<Article>, String> {
        let sql = format!(
            "SELECT {ARTICLE_COLS} FROM articles WHERE translation_key = ?1 AND lang = ?2 LIMIT 1"
        );
        let row = self.db.query_one(&sql, &[SqlParam::Text(key), SqlParam::Text(lang)])?;
        Ok(row.as_deref().and_then(Article::from_row))
    }

    /// Translation key already used by any language for this slug, if any.
    /// Lets the editor link a fresh RU entry to an existing EN entry that
    /// shares the slug, without asking the owner for internal identifiers.
    pub fn translation_key_for_slug(&self, slug: &str) -> Result<Option<String>, String> {
        let row = self.db.query_one(
            "SELECT translation_key FROM articles WHERE slug = ?1 LIMIT 1",
            &[SqlParam::Text(slug)],
        )?;
        Ok(row.map(|r| r[0].as_str().to_string()).filter(|k| !k.is_empty()))
    }

    pub fn list_articles(&self, filter: &ArticleFilter<'_>) -> Result<Vec<Article>, String> {
        let mut sql = format!("SELECT {ARTICLE_COLS} FROM articles WHERE 1 = 1");
        let mut params: Vec<SqlParam<'_>> = Vec::new();
        let mut idx = 1;

        if !filter.include_drafts {
            sql.push_str(" AND status = 'published' AND published_at <= strftime('%s','now')");
        }
        if let Some(lang) = filter.lang {
            sql.push_str(&format!(" AND lang = ?{idx}"));
            params.push(SqlParam::Text(lang));
            idx += 1;
        }
        if let Some(q) = filter.query {
            let needle = format!("%{}%", q.trim().to_lowercase());
            sql.push_str(&format!(
                " AND (lower(title) LIKE ?{idx} OR lower(summary) LIKE ?{idx} OR lower(body_md) LIKE ?{idx} OR lower(tags) LIKE ?{idx})"
            ));
            params.push(SqlParam::Text(Box::leak(needle.into_boxed_str())));
        }
        sql.push_str(" ORDER BY featured DESC, published_at DESC, id DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let rows = self.db.query_rows(&sql, &params)?;
        let mut articles: Vec<Article> = rows.iter().filter_map(|r| Article::from_row(r)).collect();
        if let Some(tag) = filter.tag {
            articles.retain(|a| a.has_tag_slug(tag));
        }
        Ok(articles)
    }

    /// Tags for one language, ordered by usage count.
    pub fn tag_cloud(&self, lang: &str) -> Result<Vec<(String, String, i64)>, String> {
        let rows = self.db.query_rows(
            "SELECT tags FROM articles WHERE lang = ?1 AND status = 'published'",
            &[SqlParam::Text(lang)],
        )?;
        let mut counts: Vec<(String, i64)> = Vec::new();
        for row in rows {
            for tag in row[0].as_str().split(',') {
                let tag = tag.trim();
                if tag.is_empty() {
                    continue;
                }
                if let Some(slot) = counts.iter_mut().find(|(t, _)| t == tag) {
                    slot.1 += 1;
                } else {
                    counts.push((tag.to_string(), 1));
                }
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(counts
            .into_iter()
            .map(|(tag, n)| {
                let slug = slug_of(&tag);
                (tag, slug, n)
            })
            .collect())
    }

    pub fn create_article(&self, input: &NewArticle<'_>) -> Result<i64, String> {
        let ts = now_unix();
        let key = match input.translation_key {
            Some(k) if !k.is_empty() => k.to_string(),
            _ => crypto::random_token_hex(6),
        };
        let published_at = match input.published_at {
            Some(p) => p,
            None if input.status == "published" => ts,
            None => 0,
        };
        let minutes = markdown::estimate_reading_minutes(input.body_md);
        self.db.insert(
            "INSERT INTO articles
               (translation_key, lang, slug, title, summary, body_md, tags, status, featured,
                reading_minutes, published_at, updated_at, created_at, views)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, 0)",
            &[
                SqlParam::Text(&key),
                SqlParam::Text(input.lang),
                SqlParam::Text(input.slug),
                SqlParam::Text(input.title),
                SqlParam::Text(input.summary),
                SqlParam::Text(input.body_md),
                SqlParam::Text(input.tags),
                SqlParam::Text(input.status),
                SqlParam::Int(input.featured as i64),
                SqlParam::Int(minutes),
                SqlParam::Int(published_at),
                SqlParam::Int(ts),
            ],
        )
    }

    pub fn update_article(&self, id: i64, input: &NewArticle<'_>) -> Result<(), String> {
        let existing = self
            .article_by_id(id)?
            .ok_or_else(|| "article not found".to_string())?;
        let published_at = match input.published_at {
            Some(p) if p > 0 => p,
            _ if input.status == "published" && existing.published_at == 0 => now_unix(),
            _ => existing.published_at,
        };
        let minutes = markdown::estimate_reading_minutes(input.body_md);
        self.db.execute(
            "UPDATE articles SET lang = ?1, slug = ?2, title = ?3, summary = ?4, body_md = ?5,
                 tags = ?6, status = ?7, featured = ?8, reading_minutes = ?9,
                 published_at = ?10, updated_at = ?11
             WHERE id = ?12",
            &[
                SqlParam::Text(input.lang),
                SqlParam::Text(input.slug),
                SqlParam::Text(input.title),
                SqlParam::Text(input.summary),
                SqlParam::Text(input.body_md),
                SqlParam::Text(input.tags),
                SqlParam::Text(input.status),
                SqlParam::Int(input.featured as i64),
                SqlParam::Int(minutes),
                SqlParam::Int(published_at),
                SqlParam::Int(now_unix()),
                SqlParam::Int(id),
            ],
        )?;
        Ok(())
    }

    pub fn set_article_status(&self, id: i64, status: &str) -> Result<(), String> {
        self.db.execute(
            "UPDATE articles SET status = ?1, updated_at = ?2,
                 published_at = CASE WHEN ?1 = 'published' AND published_at = 0 THEN ?2 ELSE published_at END
             WHERE id = ?3",
            &[SqlParam::Text(status), SqlParam::Int(now_unix()), SqlParam::Int(id)],
        )?;
        Ok(())
    }

    pub fn delete_article(&self, id: i64) -> Result<(), String> {
        self.db.execute("DELETE FROM articles WHERE id = ?1", &[SqlParam::Int(id)])?;
        Ok(())
    }

    pub fn increment_views(&self, id: i64) -> Result<(), String> {
        self.db.execute(
            "UPDATE articles SET views = views + 1 WHERE id = ?1",
            &[SqlParam::Int(id)],
        )?;
        Ok(())
    }

    // --------------------------------------------------------------- profile

    pub fn profile(&self) -> Result<Profile, String> {
        let row = self.db.query_one(
            "SELECT owner_name, handle, role_ru, role_en, manifesto_ru, manifesto_en,
                    now_ru, now_en, location, email, github, telegram, matrix, site_url
             FROM profile WHERE id = 1",
            &[],
        )?;
        match row {
            Some(r) => Ok(Profile {
                owner_name: r[0].as_str().to_string(),
                handle: r[1].as_str().to_string(),
                role_ru: r[2].as_str().to_string(),
                role_en: r[3].as_str().to_string(),
                manifesto_ru: r[4].as_str().to_string(),
                manifesto_en: r[5].as_str().to_string(),
                now_ru: r[6].as_str().to_string(),
                now_en: r[7].as_str().to_string(),
                location: r[8].as_str().to_string(),
                email: r[9].as_str().to_string(),
                github: r[10].as_str().to_string(),
                telegram: r[11].as_str().to_string(),
                matrix: r[12].as_str().to_string(),
                site_url: r[13].as_str().to_string(),
            }),
            None => {
                let p = Profile::default();
                self.save_profile(&p)?;
                Ok(p)
            }
        }
    }

    pub fn save_profile(&self, p: &Profile) -> Result<(), String> {
        self.db.execute(
            "INSERT INTO profile
               (id, owner_name, handle, role_ru, role_en, manifesto_ru, manifesto_en, now_ru, now_en,
                location, email, github, telegram, matrix, site_url, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
                owner_name = excluded.owner_name,
                handle = excluded.handle,
                role_ru = excluded.role_ru,
                role_en = excluded.role_en,
                manifesto_ru = excluded.manifesto_ru,
                manifesto_en = excluded.manifesto_en,
                now_ru = excluded.now_ru,
                now_en = excluded.now_en,
                location = excluded.location,
                email = excluded.email,
                github = excluded.github,
                telegram = excluded.telegram,
                matrix = excluded.matrix,
                site_url = excluded.site_url,
                updated_at = excluded.updated_at",
            &[
                SqlParam::Text(&p.owner_name),
                SqlParam::Text(&p.handle),
                SqlParam::Text(&p.role_ru),
                SqlParam::Text(&p.role_en),
                SqlParam::Text(&p.manifesto_ru),
                SqlParam::Text(&p.manifesto_en),
                SqlParam::Text(&p.now_ru),
                SqlParam::Text(&p.now_en),
                SqlParam::Text(&p.location),
                SqlParam::Text(&p.email),
                SqlParam::Text(&p.github),
                SqlParam::Text(&p.telegram),
                SqlParam::Text(&p.matrix),
                SqlParam::Text(&p.site_url),
                SqlParam::Int(now_unix()),
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ telemetry

    pub fn record_view(&self, v: &ViewRecord) -> Result<(), String> {
        self.db.execute(
            "INSERT INTO views (ts, path, lang, status_code, render_us, bytes, referrer, ua, is_bot, visitor_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            &[
                SqlParam::Int(v.ts),
                SqlParam::Text(&v.path),
                SqlParam::Text(&v.lang),
                SqlParam::Int(v.status_code),
                SqlParam::Int(v.render_us),
                SqlParam::Int(v.bytes),
                SqlParam::Text(&crate::http::truncate_chars(&v.referrer, 160)),
                SqlParam::Text(&crate::http::truncate_chars(&v.ua, 140)),
                SqlParam::Int(v.is_bot as i64),
                SqlParam::Text(&v.visitor_hash),
            ],
        )?;
        Ok(())
    }

    pub fn telemetry(&self) -> Result<Telemetry, String> {
        let today_start = now_unix() - 86_400;
        let scalar = |sql: &str, params: &[SqlParam<'_>]| -> i64 {
            self.db
                .query_one(sql, params)
                .ok()
                .flatten()
                .map(|r| r[0].as_i64())
                .unwrap_or(0)
        };

        let total_views = scalar("SELECT COUNT(*) FROM views", &[]);
        let views_today = scalar("SELECT COUNT(*) FROM views WHERE ts >= ?1", &[SqlParam::Int(today_start)]);
        let uniques_today = scalar(
            "SELECT COUNT(DISTINCT visitor_hash) FROM views WHERE ts >= ?1 AND visitor_hash <> ''",
            &[SqlParam::Int(today_start)],
        );
        let bots = scalar("SELECT COUNT(*) FROM views WHERE is_bot = 1", &[]);
        let bot_share = if total_views > 0 { bots * 100 / total_views } else { 0 };
        let avg_render_us = scalar("SELECT CAST(AVG(render_us) AS INTEGER) FROM views", &[]);
        let avg_bytes = scalar("SELECT CAST(AVG(bytes) AS INTEGER) FROM views", &[]);

        let mut render_times: Vec<i64> = self
            .db
            .query_rows("SELECT render_us FROM views ORDER BY render_us ASC", &[])?
            .iter()
            .map(|r| r[0].as_i64())
            .collect();
        let p95_render_us = if render_times.is_empty() {
            0
        } else {
            render_times.sort_unstable();
            let idx = ((render_times.len() as f64) * 0.95).floor() as usize;
            render_times[idx.min(render_times.len() - 1)]
        };

        let top_paths = self
            .db
            .query_rows(
                "SELECT path, COUNT(*) AS n FROM views GROUP BY path ORDER BY n DESC LIMIT 8",
                &[],
            )?
            .iter()
            .map(|r| (r[0].as_str().to_string(), r[1].as_i64()))
            .collect();

        let mut daily = Vec::new();
        for back in (0..14).rev() {
            let start = now_unix() - back * 86_400;
            let day_key = i18n::format_date_utc(start);
            daily.push((day_key, 0i64));
        }
        let rows = self.db.query_rows(
            "SELECT ts FROM views WHERE ts >= ?1",
            &[SqlParam::Int(now_unix() - 14 * 86_400)],
        )?;
        for row in rows {
            let key = i18n::format_date_utc(row[0].as_i64());
            if let Some(slot) = daily.iter_mut().find(|(d, _)| *d == key) {
                slot.1 += 1;
            }
        }

        Ok(Telemetry {
            total_views,
            views_today,
            uniques_today,
            bot_share,
            avg_render_us,
            p95_render_us,
            avg_bytes,
            top_paths,
            daily,
        })
    }

    pub fn backup_stats(&self) -> Result<BackupStats, String> {
        let scalar = |sql: &str| -> i64 {
            self.db
                .query_one(sql, &[])
                .ok()
                .flatten()
                .map(|r| r[0].as_i64())
                .unwrap_or(0)
        };
        Ok(BackupStats {
            articles: scalar("SELECT COUNT(*) FROM articles"),
            published: scalar("SELECT COUNT(*) FROM articles WHERE status = 'published'"),
            drafts: scalar("SELECT COUNT(*) FROM articles WHERE status = 'draft'"),
            views: scalar("SELECT COUNT(*) FROM views"),
        })
    }

    pub fn export_json(&self) -> Result<String, String> {
        let profile = self.profile()?;
        let articles = self.list_articles(&ArticleFilter {
            include_drafts: true,
            ..Default::default()
        })?;
        let stats = self.backup_stats()?;
        let telemetry = self.telemetry()?;

        let mut out = String::with_capacity(16 * 1024);
        out.push_str("{\n  \"generated_at\": ");
        out.push_str(&json_num(now_unix()));
        out.push_str(",\n  \"schema_version\": 1,\n  \"profile\": {\n");
        let fields = [
            ("owner_name", profile.owner_name.as_str()),
            ("handle", profile.handle.as_str()),
            ("role_ru", profile.role_ru.as_str()),
            ("role_en", profile.role_en.as_str()),
            ("manifesto_ru", profile.manifesto_ru.as_str()),
            ("manifesto_en", profile.manifesto_en.as_str()),
            ("now_ru", profile.now_ru.as_str()),
            ("now_en", profile.now_en.as_str()),
            ("location", profile.location.as_str()),
            ("email", profile.email.as_str()),
            ("github", profile.github.as_str()),
            ("telegram", profile.telegram.as_str()),
            ("matrix", profile.matrix.as_str()),
            ("site_url", profile.site_url.as_str()),
        ];
        for (i, (k, v)) in fields.iter().enumerate() {
            out.push_str(&format!("    \"{k}\": {}", json_str(v)));
            if i + 1 < fields.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  },\n  \"articles\": [\n");
        for (i, a) in articles.iter().enumerate() {
            out.push_str("    {\n");
            let pairs = [
                ("id", a.id.to_string()),
                ("translation_key", a.translation_key.clone()),
                ("lang", a.lang.clone()),
                ("slug", a.slug.clone()),
                ("title", a.title.clone()),
                ("summary", a.summary.clone()),
                ("body_md", a.body_md.clone()),
                ("tags", a.tags.clone()),
                ("status", a.status.clone()),
                ("featured", a.featured.to_string()),
                ("reading_minutes", a.reading_minutes.to_string()),
                ("published_at", a.published_at.to_string()),
                ("updated_at", a.updated_at.to_string()),
                ("created_at", a.created_at.to_string()),
                ("views", a.views.to_string()),
            ];
            for (j, (k, v)) in pairs.iter().enumerate() {
                let numeric = matches!(
                    *k,
                    "id" | "featured" | "reading_minutes" | "published_at" | "updated_at"
                        | "created_at" | "views"
                );
                out.push_str(&format!("      \"{k}\": "));
                out.push_str(&if numeric { v.clone() } else { json_str(v) });
                if j + 1 < pairs.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    }");
            if i + 1 < articles.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ],\n  \"stats\": {\n");
        out.push_str(&format!("    \"articles\": {},\n", stats.articles));
        out.push_str(&format!("    \"published\": {},\n", stats.published));
        out.push_str(&format!("    \"drafts\": {},\n", stats.drafts));
        out.push_str(&format!("    \"views\": {}\n", stats.views));
        out.push_str("  },\n  \"telemetry\": {\n");
        out.push_str(&format!("    \"total_views\": {},\n", telemetry.total_views));
        out.push_str(&format!("    \"views_today\": {},\n", telemetry.views_today));
        out.push_str(&format!("    \"uniques_today\": {},\n", telemetry.uniques_today));
        out.push_str(&format!("    \"avg_render_us\": {},\n", telemetry.avg_render_us));
        out.push_str(&format!("    \"p95_render_us\": {},\n", telemetry.p95_render_us));
        out.push_str(&format!("    \"avg_bytes\": {}\n", telemetry.avg_bytes));
        out.push_str("  }\n}\n");
        Ok(out)
    }

    // ---------------------------------------------------------------- seeds

    pub fn seed_if_empty(&self) -> Result<bool, String> {
        if self.article_count()? > 0 {
            return Ok(false);
        }
        let profile = Profile {
            owner_name: "Rudolf Zinovev".into(),
            handle: "rudolf".into(),
            role_ru: "инженер-программист · низкоуровневые системы".into(),
            role_en: "software engineer · low-level systems".into(),
            manifesto_ru: "Я строю программы, которые можно прочитать целиком за один вечер и собрать на любом VPS. Меньше зависимостей, больше ясности: сервер должен помещаться в голове, а страница — открываться раньше, чем палец отпустит клавишу.".into(),
            manifesto_en: "I build programs you can read in one evening and compile on any VPS. Fewer dependencies, more clarity: the server should fit in your head, and a page should open before your finger leaves the key.".into(),
            now_ru: "Пишу персональный узел на Rust без внешних крейтов. Разбираю WAL-режим SQLite, бюджеты рендера и дешёвые способы отдавать HTML за единицы миллисекунд.".into(),
            now_en: "Building a personal node in Rust with zero external crates. Digging into SQLite WAL mode, render budgets, and cheap ways to ship HTML in single-digit milliseconds.".into(),
            location: "Warsaw, PL · UTC+1".into(),
            email: "me@rudolfzinovev.me".into(),
            github: "https://github.com/RUdolf1517".into(),
            telegram: "https://t.me/rudolfzinovev".into(),
            matrix: "@rudolf:rudolfzinovev.me".into(),
            site_url: "https://rudolfzinovev.me".into(),
        };
        self.save_profile(&profile)?;
        for seed in SEEDS {
            self.create_article(&NewArticle {
                translation_key: Some(seed.key),
                lang: seed.lang,
                slug: seed.slug,
                title: seed.title,
                summary: seed.summary,
                body_md: seed.body,
                tags: seed.tags,
                status: "published",
                featured: seed.featured,
                published_at: Some(seed.published_at),
            })?;
        }
        Ok(true)
    }

    pub fn article_count(&self) -> Result<i64, String> {
        Ok(self
            .db
            .query_one("SELECT COUNT(*) FROM articles", &[])?
            .map(|r| r[0].as_i64())
            .unwrap_or(0))
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_num(n: i64) -> String {
    n.to_string()
}

struct Seed {
    key: &'static str,
    lang: &'static str,
    slug: &'static str,
    title: &'static str,
    summary: &'static str,
    tags: &'static str,
    body: &'static str,
    featured: bool,
    published_at: i64,
}

const DAY: i64 = 86_400;
const BASE: i64 = 1_762_000_000; // 2025-11-14 — fixed, deterministic seed timestamps

const SEEDS: &[Seed] = &[
    Seed {
        key: "zero-crate-http",
        lang: "ru",
        slug: "http-server-bez-krejtov",
        title: "HTTP/1.1 на голом Rust и libc",
        summary: "Как собрать честный многопоточный сервер без единого внешнего крейта: парсинг запроса, keep-alive, ETag, gzip и бюджет ответа.",
        tags: "rust, http, производительность",
        featured: true,
        published_at: BASE,
        body: "# Зачем отказываться от фреймворков\n\nКогда весь узел — это несколько маршрутов и база SQLite, любой веб-фреймворк приносит больше дерева зависимостей, чем кода в вашем приложении. Здесь нет `async`, нет runtime и нет `Cargo.lock` на триста пакетов — только `std::net` и системные библиотеки.\n\n## Анатомия запроса\n\nМинимальный парсер HTTP/1.1 читает строку запроса, заголовки и тело. Всё, что нужно для сайта:\n\n- метод и путь с query-строкой;\n- `Cookie`, `If-None-Match`, `Accept-Encoding`, `User-Agent`;\n- `Content-Length` для форм и `Origin` для CSRF-проверки.\n\n```rust\nlet mut buf = [0u8; 16 * 1024];\nlet n = stream.read(&mut buf)?;\nlet head = std::str::from_utf8(&buf[..n])?;\nlet mut lines = head.split(\"\\r\\n\");\nlet request_line = lines.next().unwrap_or_default();\n```\n\n## Что даёт keep-alive\n\nБраузер открывает соединение и переиспользует его для стилей, скрипта, шрифта и иконки. При `Connection: keep-alive` и таймауте в 10 секунд мы экономим три TCP-рукопожатия на каждый визит — это десятки миллисекунд на мобильной сети.\n\n| Оптимизация | Экономия |\n| --- | --- |\n| keep-alive | 20–60 мс |\n| gzip для HTML | 70–80% байт |\n| ETag + 304 | весь HTML-ответ |\n| инлайн CSS | один RTT |\n\n## Честный бюджет\n\n> Страница должна собраться быстрее, чем браузер успеет нарисовать спиннер.\n\nМы измеряем рендер в микросекундах и пишем это значение прямо в подвал страницы. Если число выросло — значит, шаблон потяжелел, и это видно без профилировщика.",
    },
    Seed {
        key: "zero-crate-http",
        lang: "en",
        slug: "http-server-without-crates",
        title: "HTTP/1.1 on bare Rust and libc",
        summary: "Building an honest multi-threaded server with zero external crates: request parsing, keep-alive, ETag, gzip and a response budget.",
        tags: "rust, http, performance",
        featured: true,
        published_at: BASE,
        body: "# Why drop the frameworks\n\nWhen the whole node is a handful of routes plus a SQLite file, any web framework brings more dependency tree than application code. No `async`, no runtime, no three-hundred-package `Cargo.lock` — just `std::net` and system libraries.\n\n## Anatomy of a request\n\nA minimal HTTP/1.1 parser reads the request line, headers and body. Everything this site needs:\n\n- method and path with query string;\n- `Cookie`, `If-None-Match`, `Accept-Encoding`, `User-Agent`;\n- `Content-Length` for forms and `Origin` for CSRF checks.\n\n```rust\nlet mut buf = [0u8; 16 * 1024];\nlet n = stream.read(&mut buf)?;\nlet head = std::str::from_utf8(&buf[..n])?;\nlet mut lines = head.split(\"\\r\\n\");\nlet request_line = lines.next().unwrap_or_default();\n```\n\n## What keep-alive buys you\n\nThe browser opens a connection and reuses it for styles, script, font and icon. With `Connection: keep-alive` and a 10 second idle timeout we skip three TCP handshakes per visit — tens of milliseconds on a mobile network.\n\n| Optimization | Saving |\n| --- | --- |\n| keep-alive | 20–60 ms |\n| gzip for HTML | 70–80% bytes |\n| ETag + 304 | the entire HTML body |\n| inline CSS | one RTT |\n\n## An honest budget\n\n> A page should assemble faster than the browser can draw a spinner.\n\nWe measure render time in microseconds and print it in the page footer. When the number grows, the template got heavier — and you see it without a profiler.",
    },
    Seed {
        key: "sqlite-without-orm",
        lang: "ru",
        slug: "sqlite-bez-orm",
        title: "SQLite без ORM: WAL, подготовленные выражения и честные типы",
        summary: "Один файл базы, четыре прагмы и тонкая обёртка над C API. Как получить надёжное хранилище без миграционных фреймворков.",
        tags: "sqlite, rust, хранение данных",
        featured: false,
        published_at: BASE - 9 * DAY,
        body: "# Один файл вместо кластера\n\nДля персонального сайта база — это файл на диске. `journal_mode = WAL` позволяет читателям не блокировать писателя, а `synchronous = NORMAL` убирает лишний `fsync` на каждую транзакцию, сохраняя устойчивость к падению процесса.\n\n## Прагмы, которые действительно важны\n\n```sql\nPRAGMA journal_mode = WAL;\nPRAGMA synchronous = NORMAL;\nPRAGMA foreign_keys = ON;\nPRAGMA busy_timeout = 5000;\n```\n\n- `busy_timeout` превращает редкие конфликты блокировок в короткое ожидание вместо ошибки;\n- `foreign_keys` включается отдельно для каждого соединения — об этом легко забыть;\n- `cache_size` в килобайтах с минусом задаёт размер страничного кеша.\n\n## Обёртка над C API\n\nОбёртка занимает четыреста строк: открытие, подготовка, связывание параметров, шаг, извлечение строк. Параметры передаются через `sqlite3_bind_text` с деструктором `SQLITE_TRANSIENT`, чтобы SQLite сам скопировал строку и не зависел от времени жизни Rust-значения.\n\n| Слой | Строк кода |\n| --- | --- |\n| FFI-объявления | ~60 |\n| Обёртка соединения | ~300 |\n| Схема и запросы | ~400 |\n\n> Осторожность с `SQLITE_TRANSIENT` важнее, чем кажется: это классический источник порчи данных при FFI.\n\n## Миграции без фреймворка\n\nСхема создаётся через `CREATE TABLE IF NOT EXISTS`, а версия хранится в таблице `meta`. Для персонального сайта этого достаточно: изменения применяются идемпотентно при старте.",
    },
    Seed {
        key: "sqlite-without-orm",
        lang: "en",
        slug: "sqlite-without-an-orm",
        title: "SQLite without an ORM: WAL, prepared statements, honest types",
        summary: "One database file, four pragmas and a thin wrapper over the C API. Reliable storage without a migration framework.",
        tags: "sqlite, rust, storage",
        featured: false,
        published_at: BASE - 9 * DAY,
        body: "# One file instead of a cluster\n\nFor a personal site the database is a file on disk. `journal_mode = WAL` lets readers proceed while a writer works, and `synchronous = NORMAL` drops a redundant `fsync` per transaction while staying crash-safe for the process.\n\n## The pragmas that actually matter\n\n```sql\nPRAGMA journal_mode = WAL;\nPRAGMA synchronous = NORMAL;\nPRAGMA foreign_keys = ON;\nPRAGMA busy_timeout = 5000;\n```\n\n- `busy_timeout` turns rare lock conflicts into a short wait instead of an error;\n- `foreign_keys` is per connection — easy to forget;\n- a negative `cache_size` is kilobytes of page cache.\n\n## Wrapping the C API\n\nThe wrapper is four hundred lines: open, prepare, bind, step, extract. Text parameters go through `sqlite3_bind_text` with the `SQLITE_TRANSIENT` destructor so SQLite copies the string and never depends on the lifetime of a Rust value.\n\n| Layer | Lines of code |\n| --- | --- |\n| FFI declarations | ~60 |\n| Connection wrapper | ~300 |\n| Schema and queries | ~400 |\n\n> Care with `SQLITE_TRANSIENT` matters more than it looks: it is the classic FFI data-corruption bug.\n\n## Migrations without a framework\n\nThe schema is created with `CREATE TABLE IF NOT EXISTS` and the version lives in a `meta` table. For a personal site that is enough: changes apply idempotently at boot.",
    },
    Seed {
        key: "render-budget",
        lang: "ru",
        slug: "budzhet-14-kilobajt",
        title: "Бюджет 14 килобайт: SSR-шаблоны, которые быстрее фреймворка",
        summary: "Почему серверный рендер вручную даёт лучший LCP, как считать бюджет страницы в байтах и зачем вообще нужен JavaScript.",
        tags: "производительность, ssr, дизайн",
        featured: false,
        published_at: BASE - 21 * DAY,
        body: "# Ноль внешних запросов\n\nСамый быстрый сторонний ресурс — тот, которого нет. Весь CSS инлайнится в `<head>`, шрифты — системные, аналитика — собственный счётчик на сервере. Браузер делает ровно один запрос за документом.\n\n## Из чего складывается страница\n\n| Компонент | Бюджет |\n| --- | --- |\n| HTML + инлайн CSS | ≤ 14 КБ gzip |\n| JS | ≤ 2 КБ gzip |\n| шрифты | 0 |\n| аналитика | 0 |\n\n## Три темы, один документ\n\nТема переключается атрибутом на `<html>`. Светлая «Архивная бумага» и тёмный «Обсидиан» — это два набора CSS-переменных, а не две таблицы стилей.\n\n```css\n:root[data-theme=\"obsidian\"] {\n  --bg: #0d0f11;\n  --ink: #e7e2d8;\n  --accent: #d8a24a;\n}\n```\n\n> Дизайн без картинок — это дисциплина типографики, а не аскетизм.\n\n## Прогрессивное улучшение\n\nJavaScript не обязателен: навигация, поиск и формы работают как обычные ссылки и `POST`-запросы. Скрипт только добавляет переключение темы без перезагрузки, копирование кода и живой счётчик. Если он не загрузится — сайт останется полностью рабочим.",
    },
    Seed {
        key: "render-budget",
        lang: "en",
        slug: "the-14-kilobyte-budget",
        title: "The 14 kilobyte budget: SSR templates faster than a framework",
        summary: "Why hand-written server rendering wins on LCP, how to keep a page budget in bytes, and why JavaScript is optional.",
        tags: "performance, ssr, design",
        featured: false,
        published_at: BASE - 21 * DAY,
        body: "# Zero external requests\n\nThe fastest third-party asset is the one you never ship. All CSS is inlined in `<head>`, fonts are system fonts, analytics is a server-side counter. The browser makes exactly one request for the document.\n\n## What a page is made of\n\n| Component | Budget |\n| --- | --- |\n| HTML + inline CSS | ≤ 14 KB gzip |\n| JS | ≤ 2 KB gzip |\n| fonts | 0 |\n| analytics | 0 |\n\n## Three themes, one document\n\nThe theme switches by an attribute on `<html>`. Light \"Archival Paper\" and dark \"Obsidian\" are two sets of CSS variables, not two stylesheets.\n\n```css\n:root[data-theme=\"obsidian\"] {\n  --bg: #0d0f11;\n  --ink: #e7e2d8;\n  --accent: #d8a24a;\n}\n```\n\n> Design without images is typographic discipline, not asceticism.\n\n## Progressive enhancement\n\nJavaScript is optional: navigation, search and forms work as plain links and `POST` requests. The script only adds theme switching without reloads, code copying and a live counter. If it fails to load, the site stays fully usable.",
    },
    Seed {
        key: "vps-bottleneck",
        lang: "ru",
        slug: "uzkoe-mesto-za-5-evro",
        title: "Узкое место на сервере за пять евро",
        summary: "Что реально ограничивает персональный сайт: не CPU, не память и не Rust, а TLS-рукопожатия, холодный кеш страниц и размер ответа.",
        tags: "инфраструктура, tls, производительность",
        featured: false,
        published_at: BASE - 34 * DAY,
        body: "# Что мы измеряем\n\nОдин vCPU и гигабайт памяти выдерживают тысячи запросов в секунду к статическому HTML. Значит, узкое место — не код приложения.\n\n## Порядок величины задержек\n\n- TLS-рукопожатие без session resumption: 40–120 мс;\n- открытие нового TCP-соединения: 20–60 мс;\n- рендер страницы: 200–800 **микросекунд**;\n- чтение статьи из SQLite: 30–90 микросекунд.\n\n| Источник задержки | Доля |\n| --- | --- |\n| сеть и TLS | ~85% |\n| шаблонизация | ~1% |\n| база | < 1% |\n\n> Оптимизировать рендер дальше имеет смысл только после того, как исчезли лишние сетевые круги.\n\n## Практический вывод\n\n1. Терминируйте TLS на обратном прокси с HTTP/2 и OCSP stapling.\n2. Отдавайте `Cache-Control: public, max-age=...` для статики и `ETag` для HTML.\n3. Держите ответ в одном пакете — до 14 КБ gzip.\n4. Логи пишите в SQLite, а не в stdout через пайп в journald.\n\n## Резервное копирование\n\nОдин файл базы, ежедневный `sqlite3 .backup` в отдельный каталог, ротация на семь дней. Этого достаточно, чтобы восстановить узел за минуту.",
    },
    Seed {
        key: "vps-bottleneck",
        lang: "en",
        slug: "the-bottleneck-on-a-five-euro-box",
        title: "The bottleneck on a five euro box",
        summary: "What actually limits a personal site: not CPU, not memory and not Rust, but TLS handshakes, cold page cache and response size.",
        tags: "infrastructure, tls, performance",
        featured: false,
        published_at: BASE - 34 * DAY,
        body: "# What we measure\n\nOne vCPU and a gigabyte of RAM will serve thousands of requests per second of static HTML. So the bottleneck is not the application code.\n\n## Orders of magnitude\n\n- TLS handshake without session resumption: 40–120 ms;\n- new TCP connection: 20–60 ms;\n- page render: 200–800 **microseconds**;\n- reading an article from SQLite: 30–90 microseconds.\n\n| Source of latency | Share |\n| --- | --- |\n| network and TLS | ~85% |\n| templating | ~1% |\n| database | < 1% |\n\n> Optimizing the render further only makes sense after the extra network round trips are gone.\n\n## Practical conclusions\n\n1. Terminate TLS at a reverse proxy with HTTP/2 and OCSP stapling.\n2. Send `Cache-Control: public, max-age=...` for static assets and `ETag` for HTML.\n3. Keep the response in a single packet — 14 KB gzip or less.\n4. Write logs to SQLite, not to stdout piped into journald.\n\n## Backups\n\nOne database file, a nightly `sqlite3 .backup` into a separate directory, seven day rotation. Enough to restore the node in a minute.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> Store {
        let path = std::env::temp_dir().join(format!("rz-test-{name}-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).expect("open store");
        store.seed_if_empty().expect("seed");
        store
    }

    #[test]
    fn seeds_bilingual_pairs_and_filters() {
        let store = temp_db("seed");
        assert_eq!(store.article_count().unwrap(), 8);
        let ru = store
            .list_articles(&ArticleFilter { lang: Some("ru"), ..Default::default() })
            .unwrap();
        assert_eq!(ru.len(), 4);
        let tag = store
            .list_articles(&ArticleFilter { lang: Some("en"), tag: Some("sqlite"), ..Default::default() })
            .unwrap();
        assert_eq!(tag.len(), 1);
        let found = store
            .list_articles(&ArticleFilter { lang: Some("ru"), query: Some("WAL"), ..Default::default() })
            .unwrap();
        assert!(!found.is_empty());
    }

    #[test]
    fn owner_auth_and_sessions() {
        let store = temp_db("auth");
        let generated = store.bootstrap_owner("owner", None).unwrap();
        let pw = generated.expect("generated password");
        assert!(store.authenticate("owner", "nope").unwrap().is_none());
        let uid = store.authenticate("owner", &pw).unwrap().expect("auth ok");
        let (token, csrf) = store.create_session(uid, "iphash", "test-agent").unwrap();
        let session = store.session_by_token(&token).unwrap().expect("session");
        assert_eq!(session.csrf, csrf);
        assert!(store.session_by_token("deadbeef").unwrap().is_none());
        store.revoke_session(&session.token_hash).unwrap();
        assert!(store.session_by_token(&token).unwrap().is_none());
    }

    #[test]
    fn profile_roundtrip_and_backup_json() {
        let store = temp_db("profile");
        let mut p = store.profile().unwrap();
        p.owner_name = "Test \"Quoted\" Name".into();
        store.save_profile(&p).unwrap();
        assert_eq!(store.profile().unwrap().owner_name, "Test \"Quoted\" Name");
        let json = store.export_json().unwrap();
        assert!(json.contains("\\\"Quoted\\\""));
        assert!(json.contains("\"articles\": ["));
        assert!(json.trim_end().ends_with('}'));
    }

    #[test]
    fn telemetry_aggregates() {
        let store = temp_db("telemetry");
        for i in 0..10 {
            store
                .record_view(&ViewRecord {
                    ts: now_unix() - i,
                    path: "/".into(),
                    lang: "ru".into(),
                    status_code: 200,
                    render_us: 200 + i,
                    bytes: 9000,
                    referrer: "ref".into(),
                    ua: "ua".into(),
                    is_bot: i % 5 == 0,
                    visitor_hash: "vh".into(),
                })
                .unwrap();
        }
        let t = store.telemetry().unwrap();
        assert_eq!(t.total_views, 10);
        assert_eq!(t.views_today, 10);
        assert_eq!(t.uniques_today, 1);
        assert!(t.p95_render_us >= t.avg_render_us);
        assert_eq!(t.daily.len(), 14);
    }
}
