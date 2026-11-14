//! Domain models shared by the store, the renderer and the cabinet.

use crate::markdown;
use crate::sqlite::SqlVal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    pub fn code(&self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    pub fn other(&self) -> Lang {
        match self {
            Lang::Ru => Lang::En,
            Lang::En => Lang::Ru,
        }
    }

    pub fn from_code(s: &str) -> Option<Lang> {
        match s {
            "ru" => Some(Lang::Ru),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    /// Prefix used by every public route: "" for Russian, "/en" for English.
    pub fn prefix(&self) -> &'static str {
        match self {
            Lang::Ru => "",
            Lang::En => "/en",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Lang::Ru => "RU",
            Lang::En => "EN",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Article {
    pub id: i64,
    pub translation_key: String,
    pub lang: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub body_md: String,
    pub tags: String,
    pub status: String,
    pub featured: bool,
    pub reading_minutes: i64,
    pub published_at: i64,
    pub updated_at: i64,
    pub created_at: i64,
    pub views: i64,
}

impl Article {
    pub fn from_row(row: &[SqlVal]) -> Option<Article> {
        if row.len() < 15 {
            return None;
        }
        Some(Article {
            id: row[0].as_i64(),
            translation_key: row[1].as_str().to_string(),
            lang: row[2].as_str().to_string(),
            slug: row[3].as_str().to_string(),
            title: row[4].as_str().to_string(),
            summary: row[5].as_str().to_string(),
            body_md: row[6].as_str().to_string(),
            tags: row[7].as_str().to_string(),
            status: row[8].as_str().to_string(),
            featured: row[9].as_i64() != 0,
            reading_minutes: row[10].as_i64(),
            published_at: row[11].as_i64(),
            updated_at: row[12].as_i64(),
            created_at: row[13].as_i64(),
            views: row[14].as_i64(),
        })
    }

    pub fn lang_enum(&self) -> Lang {
        Lang::from_code(&self.lang).unwrap_or(Lang::Ru)
    }

    pub fn is_published(&self) -> bool {
        self.status == "published"
    }

    pub fn tag_list(&self) -> Vec<String> {
        self.tags
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    }

    pub fn tag_slugs(&self) -> Vec<String> {
        self.tag_list().iter().map(|t| slug_of(t)).collect()
    }

    pub fn has_tag_slug(&self, slug: &str) -> bool {
        self.tag_slugs().iter().any(|t| t == slug)
    }

    pub fn reading_time(&self) -> i64 {
        if self.reading_minutes > 0 {
            self.reading_minutes
        } else {
            markdown::estimate_reading_minutes(&self.body_md)
        }
    }

    pub fn date_iso(&self) -> String {
        super::i18n::format_date_utc(self.published_at)
    }

    pub fn edit_date_iso(&self) -> String {
        super::i18n::format_date_utc(self.updated_at)
    }
}

pub fn slug_of(input: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            for low in ch.to_lowercase() {
                out.push(low);
            }
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "n-a".to_string()
    } else {
        out
    }
}

#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub owner_name: String,
    pub handle: String,
    pub role_ru: String,
    pub role_en: String,
    pub manifesto_ru: String,
    pub manifesto_en: String,
    pub now_ru: String,
    pub now_en: String,
    pub location: String,
    pub email: String,
    pub github: String,
    pub telegram: String,
    pub matrix: String,
    pub site_url: String,
  }

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionInfo {
    pub token_hash: String,
    pub csrf: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_seen: i64,
    pub ip_hash: String,
    pub ua: String,
    pub current: bool,
}

#[derive(Debug, Clone)]
pub struct ViewRecord {
    pub ts: i64,
    pub path: String,
    pub lang: String,
    pub status_code: i64,
    pub render_us: i64,
    pub bytes: i64,
    pub referrer: String,
    pub ua: String,
    pub is_bot: bool,
    /// Day-scoped SHA-256 of IP + user agent — never the raw address.
    pub visitor_hash: String,
}

#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    pub total_views: i64,
    pub views_today: i64,
    pub uniques_today: i64,
    pub bot_share: i64,
    pub avg_render_us: i64,
    pub p95_render_us: i64,
    pub avg_bytes: i64,
    pub top_paths: Vec<(String, i64)>,
    pub daily: Vec<(String, i64)>,
}

#[derive(Debug, Clone, Copy)]
pub struct BackupStats {
    pub articles: i64,
    pub published: i64,
    pub drafts: i64,
    pub views: i64,
}

pub fn detect_lang(headers: &str, cookie_lang: Option<&str>) -> Lang {
    if let Some(l) = cookie_lang.and_then(Lang::from_code) {
        return l;
    }
    let lower = headers.to_ascii_lowercase();
    if let Some(pos) = lower.find("accept-language:") {
        let tail = &lower[pos + 16..];
        let line = tail.split(['\n', '\r']).next().unwrap_or("");
        let ru_pos = line.find("ru").unwrap_or(usize::MAX);
        let en_pos = line.find("en").unwrap_or(usize::MAX);
        if ru_pos < en_pos {
            return Lang::Ru;
        }
        if en_pos != usize::MAX {
            return Lang::En;
        }
    }
    Lang::Ru
}
