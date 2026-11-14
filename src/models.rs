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

/// What a row in `articles` is: a personal note or a catalogue project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Note,
    Project,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Project => "project",
        }
    }

    pub fn section(self) -> &'static str {
        match self {
            Kind::Note => "notes",
            Kind::Project => "projects",
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
    /// `note` — a personal entry; `project` — a case in the project catalogue.
    pub kind: String,
}

impl Article {
    pub fn from_row(row: &[SqlVal]) -> Option<Article> {
        if row.len() < 16 {
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
            kind: row[15].as_str().to_string(),
        })
    }

    pub fn lang_enum(&self) -> Lang {
        Lang::from_code(&self.lang).unwrap_or(Lang::Ru)
    }

    pub fn is_published(&self) -> bool {
        self.status == "published"
    }

    pub fn is_project(&self) -> bool {
        self.kind == "project"
    }

    /// URL segment for this entry's kind.
    pub fn section(&self) -> &'static str {
        if self.is_project() {
            "projects"
        } else {
            "notes"
        }
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

// ------------------------------------------------------------- project log

/// What a project-log entry is. Deliberately separate from `Kind`: a `note`
/// article is a standalone essay, while these three belong to one project and
/// are shown under its GitHub timeline as the owner's own voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// An observation or thought about the work.
    Note,
    /// A technical decision, with the trade-off it settled.
    Decision,
    /// A stage that was reached.
    Milestone,
}

impl LogKind {
    pub fn code(self) -> &'static str {
        match self {
            LogKind::Note => "note",
            LogKind::Decision => "decision",
            LogKind::Milestone => "milestone",
        }
    }

    pub fn from_code(s: &str) -> LogKind {
        match s {
            "decision" => LogKind::Decision,
            "milestone" => LogKind::Milestone,
            _ => LogKind::Note,
        }
    }

    pub fn all() -> [LogKind; 3] {
        [LogKind::Note, LogKind::Decision, LogKind::Milestone]
    }

    /// Roman numeral used as the margin mark: a decision is not a note.
    pub fn mark(self) -> &'static str {
        match self {
            LogKind::Note => "N",
            LogKind::Decision => "D",
            LogKind::Milestone => "M",
        }
    }

    pub fn label(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (LogKind::Note, Lang::Ru) => "наблюдение",
            (LogKind::Note, Lang::En) => "observation",
            (LogKind::Decision, Lang::Ru) => "решение",
            (LogKind::Decision, Lang::En) => "decision",
            (LogKind::Milestone, Lang::Ru) => "этап",
            (LogKind::Milestone, Lang::En) => "milestone",
        }
    }
}

/// A note in a project's own journal. Owner-written, never generated.
#[derive(Debug, Clone)]
#[allow(dead_code)] // created_at/updated_at are kept for the backup export
pub struct ProjectNote {
    pub id: i64,
    /// The project entry this note belongs to (an `articles` row of kind `project`).
    pub project_id: i64,
    pub lang: String,
    pub kind: String,
    pub title: String,
    pub body_md: String,
    pub log_date: i64,
    pub public: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProjectNote {
    pub fn from_row(row: &[SqlVal]) -> Option<ProjectNote> {
        if row.len() < 10 {
            return None;
        }
        Some(ProjectNote {
            id: row[0].as_i64(),
            project_id: row[1].as_i64(),
            lang: row[2].as_str().to_string(),
            kind: row[3].as_str().to_string(),
            title: row[4].as_str().to_string(),
            body_md: row[5].as_str().to_string(),
            log_date: row[6].as_i64(),
            public: row[7].as_i64() != 0,
            created_at: row[8].as_i64(),
            updated_at: row[9].as_i64(),
        })
    }

    pub fn kind_enum(&self) -> LogKind {
        LogKind::from_code(&self.kind)
    }

    pub fn date_iso(&self) -> String {
        crate::i18n::format_date_utc(self.log_date)
    }
}

/// GitHub binding of a project plus the cached facts we show.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // a few facts are cached for the export, not shown on the page
pub struct RepoLink {
    /// `owner/name`, empty when the project has no repository.
    pub repo: String,
    pub html_url: String,
    pub description: String,
    pub stars: i64,
    pub forks: i64,
    pub language: String,
    pub license: String,
    pub homepage: String,
    pub open_issues: i64,
    pub archived: bool,
    pub release_tag: String,
    pub release_name: String,
    pub release_url: String,
    pub release_at: i64,
    pub pushed_at: i64,
    pub created_at: i64,
    pub fetched_at: i64,
    /// Last refresh failure: `network`, `not-found`, `rate-limited`, `api-error`.
    pub last_error: String,
    pub show_issues: bool,
}

impl RepoLink {
    pub fn from_row(row: &[SqlVal]) -> Option<RepoLink> {
        if row.len() < 20 {
            return None;
        }
        Some(RepoLink {
            repo: row[0].as_str().to_string(),
            html_url: row[1].as_str().to_string(),
            description: row[2].as_str().to_string(),
            stars: row[3].as_i64(),
            forks: row[4].as_i64(),
            language: row[5].as_str().to_string(),
            license: row[6].as_str().to_string(),
            homepage: row[7].as_str().to_string(),
            open_issues: row[8].as_i64(),
            archived: row[9].as_i64() != 0,
            release_tag: row[10].as_str().to_string(),
            release_name: row[11].as_str().to_string(),
            release_url: row[12].as_str().to_string(),
            release_at: row[13].as_i64(),
            pushed_at: row[14].as_i64(),
            created_at: row[15].as_i64(),
            fetched_at: row[16].as_i64(),
            last_error: row[17].as_str().to_string(),
            show_issues: row[18].as_i64() != 0,
            // row[19] is the project id, kept only by the SQL projection.
        })
    }

    pub fn is_linked(&self) -> bool {
        !self.repo.is_empty()
    }

    /// Issues are opt-in: a catalogue of projects should not look like a
    /// support desk unless the owner asked for it.
    pub fn issues_enabled(&self) -> bool {
        self.is_linked() && self.show_issues
    }
}
