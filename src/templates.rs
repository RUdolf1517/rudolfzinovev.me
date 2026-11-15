//! Server-side HTML templates. No template engine, no runtime, no external
//! requests: CSS and JS are inlined into the document, icons are data URIs.

use crate::i18n::{self, Lang, t};
use crate::models::Kind;
use crate::markdown::{html_escape, render_markdown};
use crate::models::{
    Article, BackupStats, LogKind, Profile, ProjectNote, RepoLink, SessionInfo, Telemetry,
};

pub const CSS: &str = include_str!("../assets/style.css");
pub const JS: &str = include_str!("../assets/app.js");
/// SHA-256 of the inlined script, computed once, used as a CSP source hash.
pub fn script_csp_hash() -> &'static str {
    use std::sync::OnceLock;
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let digest = crate::crypto::sha256_hex(JS.as_bytes());
        let bytes = crate::crypto::hex_decode(&digest).unwrap_or_default();
        crate::crypto::base64_encode(&bytes)
    })
}

pub const FAVICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" fill="#0b0d0f"/><rect x="4" y="4" width="24" height="24" fill="none" stroke="#e0a458" stroke-width="1.5"/><path d="M10 9h7a5 5 0 0 1 0 10h-7z" fill="none" stroke="#e9e4d9" stroke-width="2"/><path d="M10 9v14" stroke="#e9e4d9" stroke-width="2"/><circle cx="24" cy="23" r="2" fill="#c2542d"/></svg>"##;

pub fn esc(s: &str) -> String {
    html_escape(s)
}

macro_rules! w {
    ($out:expr, $($arg:tt)*) => {
        $out.push_str(&format!($($arg)*))
    };
}

/// `1 234` with a thin space every three digits.
fn fmt_num(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 && ch.is_ascii_digit() && s.len() > 3 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

pub struct Chrome<'a> {
    pub lang: Lang,
    pub path: &'a str,
    pub profile: &'a Profile,
    pub owner: bool,
}

/// Everything a project page needs besides the article itself: the GitHub
/// binding with its cached facts, the automatic timeline and the owner's own
/// journal. These are three separate things on purpose — they are never mixed.
pub struct ProjectView<'a> {
    pub link: Option<&'a RepoLink>,
    pub events: &'a [crate::github::GhEvent],
    pub notes: &'a [ProjectNote],
}

impl Chrome<'_> {
    fn t(&self) -> &'static i18n::T {
        t(self.lang)
    }

    /// Same path in the other language, swapping the `/en` prefix.
    fn alt_lang_path(&self, target: Lang) -> String {
        let path = self.path;
        match target {
            Lang::En => {
                if path == "/" {
                    "/en".to_string()
                } else {
                    format!("/en{path}")
                }
            }
            Lang::Ru => {
                if path == "/en" {
                    "/".to_string()
                } else if let Some(rest) = path.strip_prefix("/en") {
                    if rest.is_empty() { "/".to_string() } else { rest.to_string() }
                } else {
                    path.to_string()
                }
            }
        }
    }

    fn href(&self, key: &str) -> String {
        let prefix = self.lang.prefix();
        match key {
            "home" => {
                if prefix.is_empty() { "/".into() } else { prefix.into() }
            }
            other => format!("{prefix}/{other}"),
        }
    }

    fn is_active(&self, key: &str) -> bool {
        let href = self.href(key);
        // A detail page keeps its section lit in the running head.
        self.path == href
            || matches!(key, "notes" | "projects" | "about")
                && self.path.starts_with(&format!("{href}/"))
    }
}

fn head(chrome: &Chrome<'_>, title: &str, description: &str, noindex: bool) -> String {
    let tr = chrome.t();
    let mut h = String::with_capacity(4096);
    h.push_str("<!doctype html>\n<html lang=\"");
    h.push_str(chrome.lang.code());
    h.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n");
    w!(h, "<title>{}</title>\n", esc(title));
    w!(h, "<meta name=\"description\" content=\"{}\">\n", esc(description));
    h.push_str("<meta name=\"theme-color\" content=\"#fcfbf7\">\n");
    if noindex {
        h.push_str("<meta name=\"robots\" content=\"noindex,nofollow\">\n");
    }
    w!(h, "<link rel=\"icon\" href=\"data:image/svg+xml,{}\">\n", encode_uri_component(FAVICON));
    w!(
        h,
        "<link rel=\"alternate\" type=\"application/rss+xml\" title=\"RSS\" href=\"{}feed.xml\">\n",
        if chrome.lang == Lang::Ru { "/" } else { "/en/" }
    );
    w!(h, "<link rel=\"canonical\" href=\"{}{}\">\n", esc(&chrome.profile.site_url), esc(chrome.path));
    h.push_str("<style>");
    h.push_str(CSS);
    h.push_str("</style>\n</head>\n<body class=\"page\">\n");
    let _ = tr;
    h
}

fn encode_uri_component(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    for b in svg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Running head: name, folio number for the current section, minimal tools.
/// Deliberately not a navbar — the site should read as one continuous document.
fn header(chrome: &Chrome<'_>) -> String {
    let tr = chrome.t();
    let mut h = String::new();
    h.push_str("<a class=\"skip\" href=\"#main\">");
    h.push_str(tr.skip_link);
    h.push_str("</a>\n<header class=\"run\">\n<div class=\"run-inner\">");
    h.push_str("<span class=\"run-mark\" aria-hidden=\"true\">❦</span>");
    w!(
        h,
        "<a class=\"run-name\" href=\"{}\">{}</a>",
        chrome.href("home"),
        esc(&chrome.profile.owner_name)
    );
    h.push_str("<nav class=\"run-nav\" aria-label=\"Primary\">");
    for (key, label) in [
        ("projects", tr.nav_projects),
        ("notes", tr.nav_notes),
        ("about", tr.nav_about),
    ] {
        w!(
            h,
            "<a href=\"{}\"{}>{}</a>",
            chrome.href(key),
            if chrome.is_active(key) { " aria-current=\"page\"" } else { "" },
            label
        );
    }
    h.push_str("</nav>\n<span class=\"run-tools\">");
    let alt = match chrome.lang {
        Lang::Ru => Lang::En,
        Lang::En => Lang::Ru,
    };
    w!(
        h,
        "<a class=\"tool\" href=\"{}\" hreflang=\"{}\" title=\"{}\">{}</a>",
        esc(&chrome.alt_lang_path(alt)),
        alt.code(),
        tr.lang_switch,
        alt.label()
    );
    if chrome.owner {
        w!(h, "<a class=\"tool tool-accent\" href=\"/cabinet\">{}</a>", tr.nav_cabinet);
    }
    h.push_str("</span></div></header>\n");
    h
}

fn footer(chrome: &Chrome<'_>) -> String {
    let p = chrome.profile;
    let mut h = String::new();
    h.push_str("<footer class=\"foot\">\n<div class=\"foot-inner\">");
    h.push_str("<span class=\"foot-mark\" aria-hidden=\"true\">· · ·</span>");
    w!(
        h,
        "<p class=\"foot-links\"><a href=\"{}\">{}</a><a href=\"/feed.xml\">RSS</a><a href=\"/sitemap.xml\">Sitemap</a><a href=\"{}\">GitHub</a><a href=\"/login\">{}</a></p>",
        chrome.href("about"),
        chrome.t().nav_about,
        esc(&p.github),
        chrome.t().footer_admin
    );
    w!(
        h,
        "<p class=\"foot-note\">© {} {} · {} · RZ-NONCOMMERCIAL-1.0</p>",
        i18n::format_date_utc(i18n::now_unix()).get(..4).unwrap_or("2025"),
        esc(&p.owner_name),
        esc(&p.location)
    );
    h.push_str("</div>\n</footer>\n");
    w!(h, "<script>{}</script>\n", JS);
    h.push_str("</body>\n</html>\n");
    h
}

/// Standard public document with masthead, footer and the inlined script.
fn layout(chrome: &Chrome<'_>, title: &str, description: &str, body: &str) -> String {
    let mut h = head(chrome, title, description, false);
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell\">\n");
    h.push_str(body);
    h.push_str("\n</main>\n");
    h.push_str(&footer(chrome));
    h
}

fn search_box(chrome: &Chrome<'_>, section: &str, query: Option<&str>) -> String {
    let tr = chrome.t();
    format!(
        "<form class=\"search\" action=\"{}/{}\" method=\"get\" role=\"search\">\
         <div class=\"search-field\">\
         <input id=\"q\" name=\"q\" type=\"search\" value=\"{}\" placeholder=\"{}\" aria-label=\"{}\" autocomplete=\"off\" data-search-input>\
         <button type=\"submit\">{}</button></div></form>",
        chrome.lang.prefix(),
        section,
        esc(query.unwrap_or("")),
        tr.search_placeholder,
        tr.search_label,
        tr.search_submit
    )
}

// ------------------------------------------------------------------- home

/// Year of an entry's date, used to break the ledger into catalogue blocks.
fn year_of(a: &Article) -> String {
    a.date_iso().get(..4).unwrap_or("").to_string()
}

/// Ledger markup for a list of entries, with a year rule between blocks.
fn ledger(chrome: &Chrome<'_>, articles: &[Article]) -> String {
    let mut h = String::with_capacity(articles.len() * 320);
    h.push_str("<ol class=\"ledger\">");
    let mut last_year: Option<String> = None;
    for a in articles {
        let y = year_of(a);
        if last_year.as_deref() != Some(y.as_str()) {
            w!(h, "<li class=\"year-break\">{}</li>", esc(&y));
            last_year = Some(y);
        }
        h.push_str(&article_row(chrome, a));
    }
    h.push_str("</ol>");
    h
}

/// One ledger line: catalogue number, title, dotted leader, date.
fn article_row(chrome: &Chrome<'_>, a: &Article) -> String {
    let tr = chrome.t();
    let mut h = String::new();
    h.push_str("<li class=\"entry\"><a href=\"");
    h.push_str(&entry_path(a.lang_enum().prefix(), a.section(), &a.slug));
    h.push_str("\">");
    w!(h, "<span class=\"entry-line\"><span class=\"entry-no\">{}</span>", catalogue_no(a.id));
    w!(h, "<span class=\"entry-title\">{}</span>", esc(&a.title));
    h.push_str("<span class=\"entry-dots\" aria-hidden=\"true\"></span>");
    w!(
        h,
        "<time class=\"entry-date\" datetime=\"{}\">{}</time>",
        a.date_iso(),
        a.date_iso()
    );
    h.push_str("</span>");
    if !a.summary.is_empty() {
        w!(h, "<span class=\"entry-sum\">{}</span>", esc(&a.summary));
    }
    h.push_str("<span class=\"entry-foot\">");
    w!(h, "<span>{} {}</span>", a.reading_time(), tr.min_short);
    for tag in a.tag_list() {
        w!(h, "<span class=\"tag\">{}</span>", esc(&tag));
    }
    h.push_str("</span></a></li>\n");
    h
}

/// Public URL of one entry: `/notes/<slug>`, `/en/projects/<slug>`, …
pub fn entry_path(prefix: &str, section: &str, slug: &str) -> String {
    format!("{prefix}/{section}/{}", esc(slug))
}

/// Stable catalogue number for an entry — the site's quiet claim to being an
/// archive rather than a feed. Two digits, padded, never reused.
pub fn catalogue_no(id: i64) -> String {
    format!("{id:02}")
}

/// Sequential index for a filtered list, used by the contents list.
fn index_no(i: usize) -> String {
    format!("{:02}", i + 1)
}

// --------------------------------------------------------------- articles

/// `/notes`, `/projects` and tag filters: one catalogue, filtered.
pub fn section_index(
    chrome: &Chrome<'_>,
    kind: Kind,
    entries: &[Article],
    tags: &[(String, String, i64)],
    active_tag: Option<&str>,
    query: Option<&str>,
) -> String {
    let tr = chrome.t();
    let section = kind.section();
    let (noun, all_label, intro) = match (kind, chrome.lang) {
        (Kind::Note, Lang::Ru) => ("заметка", tr.all_articles, tr.notes_intro),
        (Kind::Note, Lang::En) => ("note", tr.all_articles, tr.notes_intro),
        (Kind::Project, Lang::Ru) => ("проект", tr.all_projects, tr.projects_intro),
        (Kind::Project, Lang::En) => ("project", tr.all_projects, tr.projects_intro),
    };
    let mut b = String::new();
    b.push_str("<section class=\"section section-first\">\n");
    let heading = if query.is_some() {
        tr.search_results
    } else {
        all_label
    };
    w!(b, "<h2 class=\"sec-label\">{}</h2>\n", heading);
    w!(
        b,
        "<p class=\"sec-note\">{} {} · {}</p>\n",
        entries.len(),
        match (kind, chrome.lang) {
            (Kind::Note, Lang::Ru) => i18n::ru_plural(entries.len() as i64, "заметка", "заметки", "заметок"),
            (Kind::Project, Lang::Ru) => i18n::ru_plural(entries.len() as i64, "проект", "проекта", "проектов"),
            (_, Lang::En) => {
                if entries.len() == 1 { noun } else { "PLURAL" }
            }
        }
        .replace(
            "PLURAL",
            if kind == Kind::Note { "notes" } else { "projects" }
        )
        .as_str(),
        intro
    );
    b.push_str(&search_box(chrome, section, query));

    if !tags.is_empty() {
        b.push_str("<div class=\"chips\">");
        w!(
            b,
            "<a class=\"chip{}\" href=\"{}/{}\">{}</a>",
            if active_tag.is_none() { " chip-on" } else { "" },
            chrome.lang.prefix(),
            section,
            tr.all_tags
        );
        for (tag, slug, count) in tags {
            let on = active_tag == Some(slug.as_str());
            w!(
                b,
                "<a class=\"chip{}\" href=\"{}/tags/{}\">{} <span class=\"chip-n\">{}</span></a>",
                if on { " chip-on" } else { "" },
                chrome.lang.prefix(),
                slug,
                esc(tag),
                count
            );
        }
        b.push_str("</div>");
    }

    if entries.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", if query.is_some() { tr.search_empty } else { tr.empty_state });
    } else {
        b.push_str(&ledger(chrome, entries));
        if query.is_some() {
            w!(
                b,
                "<p class=\"more\"><a href=\"{}/{}\">{}</a></p>",
                chrome.lang.prefix(),
                section,
                tr.clear
            );
        }
    }
    b.push_str("</section>");
    let title = if let Some(q) = query {
        format!("{} «{}» — {}", tr.search_results, q, chrome.profile.owner_name)
    } else {
        format!("{} — {}", all_label, chrome.profile.owner_name)
    };
    layout(chrome, &title, intro, &b)
}

// ------------------------------------------------------------------- home

pub fn home(
    chrome: &Chrome<'_>,
    notes: &[Article],
    projects: &[Article],
    notes_total: i64,
    projects_total: i64,
) -> String {
    let tr = chrome.t();
    let p = chrome.profile;
    let (statement, role) = match chrome.lang {
        Lang::Ru => (&p.manifesto_ru, &p.role_ru),
        Lang::En => (&p.manifesto_en, &p.role_en),
    };
    let mut b = String::new();

    // Title page: the whole first screen is one statement.
    b.push_str("<section class=\"titlepage\">\n");
    w!(b, "<p class=\"tp-kicker\">{}</p>\n", esc(role));
    let (first, rest) = split_name(&p.owner_name);
    b.push_str("<h1 class=\"tp-name\">");
    w!(b, "{}", esc(&first));
    if !rest.is_empty() {
        w!(b, "<em>{}</em>", esc(&rest));
    }
    b.push_str("</h1>\n<div class=\"tp-rule\"></div>\n");
    w!(b, "<p class=\"tp-statement\">{}</p>\n", esc(statement));

    let (notes_word, projects_word) = match chrome.lang {
        Lang::Ru => (
            i18n::ru_plural(notes_total, "заметка", "заметки", "заметок"),
            i18n::ru_plural(projects_total, "проект", "проекта", "проектов"),
        ),
        Lang::En => (
            if notes_total == 1 { "note" } else { "notes" },
            if projects_total == 1 { "project" } else { "projects" },
        ),
    };
    w!(
        b,
        "<p class=\"tp-meta\"><span>{} {}</span><span>{} {}</span><span>{}</span></p>\n",
        notes_total,
        notes_word,
        projects_total,
        projects_word,
        esc(&p.location)
    );
    b.push_str("<p class=\"tp-links\">");
    w!(
        b,
        "<a href=\"{}\">{}</a><a href=\"{}\">{}</a>",
        chrome.href("projects"),
        tr.hero_cta_projects,
        chrome.href("notes"),
        tr.hero_cta_notes
    );
    b.push_str("</p>\n</section>\n");

    if !projects.is_empty() {
        b.push_str("<section class=\"section\">\n");
        w!(b, "<h2 class=\"sec-label\">{}</h2>\n", tr.nav_projects);
        w!(b, "<p class=\"sec-note\">{}</p>\n", tr.projects_intro);
        b.push_str(&ledger(chrome, projects));
        w!(
            b,
            "<p class=\"more\"><a href=\"{}\">{} →</a></p>",
            chrome.href("projects"),
            tr.all_projects
        );
        b.push_str("</section>\n");
    }

    b.push_str("<section class=\"section\">\n");
    w!(b, "<h2 class=\"sec-label\">{}</h2>\n", tr.latests);
    if notes.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str(&ledger(chrome, notes));
    }
    w!(
        b,
        "<p class=\"more\"><a href=\"{}\">{} →</a></p>",
        chrome.href("notes"),
        tr.all_articles
    );
    b.push_str("</section>\n");

    let title = format!("{} — {}", p.owner_name, tr.site_role);
    layout(chrome, &title, statement, &b)
}

/// Split a full name so the surname can be set on its own line, quieter.
fn split_name(name: &str) -> (String, String) {
    match name.split_once(' ') {
        Some((first, rest)) => (first.to_string(), rest.to_string()),
        None => (name.to_string(), String::new()),
    }
}

// --------------------------------------------------------------- sections

#[allow(dead_code)]
pub fn entry_page(
    chrome: &Chrome<'_>,
    article: &Article,
    other: Option<&Article>,
    related: &[Article],
) -> String {
    entry_page_with(chrome, article, other, related, None)
}

/// The project page adds three blocks below the body: repository facts, the
/// automatic GitHub timeline, and the owner's journal. `view` is `None` for
/// notes and for projects that have neither a repository nor a journal.
pub fn entry_page_with(
    chrome: &Chrome<'_>,
    article: &Article,
    other: Option<&Article>,
    related: &[Article],
    view: Option<&ProjectView<'_>>,
) -> String {
    let tr = chrome.t();
    let rendered = render_markdown(&article.body_md);
    let mut b = String::new();
    b.push_str("<div class=\"progress\" aria-hidden=\"true\"></div>");
    b.push_str("<article class=\"article\">\n");
    let (back_label, index_label) = if article.is_project() {
        (tr.nav_projects, tr.back_to_projects)
    } else {
        (tr.nav_notes, tr.back_to_articles)
    };
    w!(
        b,
        "<nav class=\"crumbs\" aria-label=\"Breadcrumb\"><a href=\"{}/{}\">← {}</a></nav>",
        chrome.lang.prefix(),
        article.section(),
        back_label
    );

    // One catalogue line: entry number · date · reading time · revision.
    b.push_str("<p class=\"chapter-no\">");
    w!(
        b,
        "<span class=\"no\">{}{}</span>",
        match (article.is_project(), chrome.lang) {
            (true, Lang::Ru) => "проект",
            (true, Lang::En) => "project",
            (false, Lang::Ru) => "запись",
            (false, Lang::En) => "entry",
        },
        format!(" {}", catalogue_no(article.id))
    );
    w!(b, "<span class=\"sep\" aria-hidden=\"true\">·</span>");
    w!(
        b,
        "<time datetime=\"{}\">{}</time>",
        article.date_iso(),
        article.date_iso()
    );
    w!(b, "<span class=\"sep\" aria-hidden=\"true\">·</span>");
    w!(b, "<span>{} {}</span>", article.reading_time(), tr.min_short);
    if article.updated_at > article.published_at + 60 {
        w!(b, "<span class=\"sep\" aria-hidden=\"true\">·</span>");
        w!(b, "<span>{} {}</span>", tr.updated_on, article.edit_date_iso());
    }
    b.push_str("</p>\n");

    w!(b, "<h1 class=\"article-title\">{}</h1>\n", esc(&article.title));
    if !article.summary.is_empty() {
        w!(b, "<p class=\"article-sum\">{}</p>\n", esc(&article.summary));
    }

    // Everything below the title reads inside one column. The tags line used to
    // sit outside these wrappers while the body sat inside them, so under the
    // two-column grid at >=1180px the tags took the narrow margin track and the
    // body jumped into the second column. The wrapper is opened before the tags
    // and closed in the same place as before.
    b.push_str("<div class=\"chapter-flow\">");
    b.push_str("<div class=\"chapter-tags\">");
    for (tag, slug) in article.tag_list().iter().zip(article.tag_slugs()) {
        w!(
            b,
            "<a href=\"{}/tags/{}\">{}</a>",
            chrome.lang.prefix(),
            slug,
            esc(tag)
        );
    }
    if let Some(other) = other {
        w!(
            b,
            "<a class=\"lang-swap\" href=\"{}/{}/{}\" hreflang=\"{}\">{}</a>",
            other.lang_enum().prefix(),
            other.section(),
            esc(&other.slug),
            other.lang,
            tr.read_other_lang
        );
    }
    b.push_str("</div>\n");

    b.push_str("<div class=\"prose\">");
    b.push_str(&rendered.html);
    b.push_str("</div>\n");
    w!(b, "<p class=\"finis\" aria-hidden=\"true\">❦</p>");
    if let Some(view) = view {
        b.push_str(&project_sections(chrome, article, view));
    }

    b.push_str("<footer class=\"article-foot\">");
    w!(
        b,
        "<span>{} {}</span>",
        article.body_md.split_whitespace().count(),
        if chrome.lang == Lang::Ru { "слов" } else { "words" }
    );
    w!(
        b,
        "<span>{} {}</span>",
        catalogue_no(article.id),
        if chrome.lang == Lang::Ru { "в каталоге" } else { "in the catalogue" }
    );
    w!(b, "<a href=\"{}/{}\">{}</a>", chrome.lang.prefix(), article.section(), index_label);
    b.push_str("</footer>\n");

    // Contents wait in the margin, not in the way of the first paragraph.
    if !rendered.toc.is_empty() {
        b.push_str("<aside class=\"chapter-aside\">");
        b.push_str("<div class=\"toc\">");
        w!(b, "<h2 class=\"sec-label\">{}</h2>", tr.toc);
        b.push_str("<ol>");
        for (i, item) in rendered.toc.iter().enumerate() {
            w!(
                b,
                "<li class=\"toc-l{}\"><a href=\"#{}\"><span class=\"t\">{}</span><span class=\"d\" aria-hidden=\"true\"></span><span class=\"n\">{}</span></a></li>",
                item.level,
                esc(&item.id),
                esc(&item.title),
                index_no(i)
            );
        }
        b.push_str("</ol></div></aside>\n");
    }
    // Close the column that was opened before the tags line.
    b.push_str("</div>\n");
    b.push_str("</article>\n");
    b.push_str("<footer class=\"article-foot\">");
    w!(
        b,
        "<span>{} {}</span>",
        article.body_md.split_whitespace().count(),
        if chrome.lang == Lang::Ru { "слов" } else { "words" }
    );
    w!(
        b,
        "<span>{} {}</span>",
        catalogue_no(article.id),
        if chrome.lang == Lang::Ru { "в каталоге" } else { "in the catalogue" }
    );
    w!(b, "<a href=\"{}/articles\">{}</a>", chrome.lang.prefix(), tr.back_to_articles);
    b.push_str("</footer>\n");
    if !related.is_empty() {
        b.push_str("<section class=\"section\">\n");
        w!(b, "<h2 class=\"sec-label\">{}</h2>", tr.related);
        b.push_str(&ledger(chrome, related));
        b.push_str("</section>\n");
    }

    let desc = if article.summary.is_empty() {
        crate::util::truncate_chars(&article.body_md.replace('#', "").replace('\n', " "), 155)
    } else {
        article.summary.clone()
    };
    let mut h = head(chrome, &format!("{} — {}", article.title, chrome.profile.owner_name), &desc, false);
    h.push_str("<meta property=\"og:type\" content=\"article\">\n");
    w!(h, "<meta property=\"og:title\" content=\"{}\">\n", esc(&article.title));
    w!(h, "<meta property=\"article:published_time\" content=\"{}\">\n", i18n::format_rfc2822_utc(article.published_at));
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell\">");
    h.push_str(&b);
    h.push_str("</main>\n");
    h.push_str(&footer(chrome));
    h
}

// ------------------------------------------------------------------ about

pub fn about(chrome: &Chrome<'_>, notes: i64, projects: i64) -> String {
    let tr = chrome.t();
    let p = chrome.profile;
    let (manifesto, now, role) = match chrome.lang {
        Lang::Ru => (&p.manifesto_ru, &p.now_ru, &p.role_ru),
        Lang::En => (&p.manifesto_en, &p.now_en, &p.role_en),
    };
    let mut b = String::new();
    b.push_str("<section class=\"about\">\n");
    w!(b, "<p class=\"tp-kicker\">{}</p>", tr.about_title);
    w!(b, "<h1 class=\"article-title\">{}</h1>\n", esc(&p.owner_name));
    w!(b, "<p class=\"article-sum\">{}</p>\n", esc(role));

    w!(
        b,
        "<div class=\"manifesto\"><span class=\"label\">{}</span><p>{}</p></div>",
        tr.about_manifesto,
        esc(manifesto)
    );
    w!(
        b,
        "<div class=\"manifesto\"><span class=\"label\">{}</span><p>{}</p></div>",
        tr.about_now,
        esc(now)
    );
    w!(
        b,
        "<div class=\"manifesto\"><span class=\"label\">{}</span><p>{}</p></div>",
        tr.about_stack,
        esc(tr.about_stack_text)
    );

    b.push_str("<div class=\"section\">");
    w!(b, "<h2 class=\"sec-label\">{}</h2>", tr.about_contact);
    b.push_str("<ul class=\"contact-list\">");
    for (label, value, href) in [
        ("Email", p.email.clone(), format!("mailto:{}", p.email)),
        ("GitHub", p.github.clone(), p.github.clone()),
        ("Telegram", p.telegram.clone(), p.telegram.clone()),
        ("Matrix", p.matrix.clone(), String::new()),
        ("Location", p.location.clone(), String::new()),
    ] {
        if value.trim().is_empty() {
            continue;
        }
        if href.starts_with("http") || href.starts_with("mailto") {
            w!(b, "<li><span class=\"label\">{}</span><a href=\"{}\" rel=\"me noopener\">{}</a></li>", label, esc(&href), esc(&value));
        } else {
            w!(b, "<li><span class=\"label\">{}</span><span>{}</span></li>", label, esc(&value));
        }
    }
    b.push_str("</ul></div>\n");

    b.push_str("<div class=\"section\">");
    w!(b, "<h2 class=\"sec-label\">{}</h2>", tr.about_license);
    w!(
        b,
        "<div class=\"manifesto\"><p>{} · {} {}, {} {}</p></div>",
        tr.about_license_text,
        notes,
        match chrome.lang {
            Lang::Ru => i18n::ru_plural(notes, "заметка", "заметки", "заметок"),
            Lang::En => if notes == 1 { "note" } else { "notes" },
        },
        projects,
        match chrome.lang {
            Lang::Ru => i18n::ru_plural(projects, "проект", "проекта", "проектов"),
            Lang::En => if projects == 1 { "project" } else { "projects" },
        }
    );
    b.push_str("</div>\n</section>");
    let title = format!("{} — {}", tr.about_title, p.owner_name);
    layout(chrome, &title, manifesto, &b)
}

// ------------------------------------------------------------------ login

pub fn login(chrome: &Chrome<'_>, error: Option<&str>, csrf: &str, rate_limited: bool) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"auth\">\n");
    w!(b, "<h1 class=\"auth-title\">{}</h1>", tr.login_title);
    w!(b, "<p class=\"auth-hint\">{}</p>", tr.login_hint);
    if let Some(msg) = error {
        w!(b, "<p class=\"alert\" role=\"alert\">{}</p>", esc(msg));
    }
    let _ = rate_limited;
    w!(b, "<form method=\"post\" action=\"/login\" class=\"form\">");
    w!(b, "<input type=\"hidden\" name=\"csrf\" value=\"{}\">", esc(csrf));
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"username\" type=\"text\" autocomplete=\"username\" required autofocus value=\"\"></label>",
        tr.login_user
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>",
        tr.login_password
    );
    w!(b, "<button class=\"btn btn-solid btn-wide\" type=\"submit\">{}</button>", tr.login_submit);
    b.push_str("</form>\n");
    w!(
        b,
        "<p class=\"auth-foot\"><a href=\"{}\">← {}</a></p>",
        chrome.href("home"),
        tr.not_found_cta
    );
    b.push_str("</div></section>");
    let mut h = head(chrome, &format!("{} — {}", tr.login_title, chrome.profile.owner_name), tr.login_hint, true);
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell\">");
    h.push_str(&b);
    h.push_str("</main>\n");
    h.push_str(&footer(chrome));
    h
}

// ---------------------------------------------------------------- cabinet

fn cabinet_nav(active: &str) -> String {
    let mut h = String::from("<nav class=\"side\" aria-label=\"Cabinet\">");
    let items = [
        ("overview", "Overview"),
        ("articles", "Entries"),
        ("new", "New"),
        ("profile", "Profile"),
        ("telemetry", "Metrics"),
        ("security", "Access"),
        ("backup", "Backup"),
    ];
    for (key, label) in items {
        let href = match key {
            "overview" => "/cabinet".to_string(),
            "new" => "/cabinet/articles/new".to_string(),
            other => format!("/cabinet/{other}"),
        };
        w!(
            h,
            "<a class=\"side-link{}\" href=\"{}\">{label}</a>",
            if active == key { " side-on" } else { "" },
            href
        );
    }
    h.push_str("</nav>");
    h
}

pub fn cabinet_shell(chrome: &Chrome<'_>, active: &str, title: &str, body: &str, flash: Option<&str>) -> String {
    let tr = chrome.t();
    // `true` is noindex: the cabinet must never end up in a search result.
    let mut h = head(
        chrome,
        &format!("{title} — {}", tr.cabinet),
        "Owner cabinet",
        true,
    );
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell cabinet\">\n");
    h.push_str("<div class=\"cab-top\">");
    w!(h, "<h1 class=\"cab-title\">{}</h1>", esc(title));
    h.push_str("<div class=\"cab-top-tools\">");
    h.push_str("<form method=\"post\" action=\"/logout\" class=\"inline\"><button class=\"tool\" type=\"submit\">");
    h.push_str(tr.cabinet_logout);
    h.push_str("</button></form></div></div>\n");
    if let Some(msg) = flash {
        w!(h, "<p class=\"flash\" role=\"status\">{}</p>", esc(msg));
    }
    h.push_str("<div class=\"cab-grid\">");
    h.push_str(&cabinet_nav(active));
    h.push_str("<div class=\"cab-main\">");
    h.push_str(body);
    h.push_str("</div></div>\n</main>\n");
    h.push_str(&footer(chrome));
    h
}

fn stat_card(label: &str, value: &str, note: &str) -> String {
    format!(
        "<div class=\"stat\"><span class=\"label\">{}</span><b>{}</b><span class=\"stat-note\">{}</span></div>",
        esc(label),
        esc(value),
        esc(note)
    )
}

pub fn cabinet_overview(
    chrome: &Chrome<'_>,
    stats: &BackupStats,
    telemetry: &Telemetry,
    recent: &[Article],
    flash: Option<&str>,
) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<div class=\"stats\">");
    b.push_str(&stat_card(tr.cabinet_articles, &stats.articles.to_string(), "total"));
    b.push_str(&stat_card("Published", &stats.published.to_string(), "live"));
    b.push_str(&stat_card("Drafts", &stats.drafts.to_string(), "unpublished"));
    b.push_str(&stat_card("Views", &fmt_num(stats.views), "logged"));
    b.push_str("</div>\n");

    b.push_str("<section class=\"panel\">\n<div class=\"panel-head\"><span>Render time</span><span>microseconds</span></div>");
    b.push_str("<div class=\"stats\">");
    b.push_str(&stat_card("Average", &telemetry.avg_render_us.to_string(), "per request"));
    b.push_str(&stat_card("p95", &telemetry.p95_render_us.to_string(), "per request"));
    b.push_str(&stat_card("Payload", &fmt_num(telemetry.avg_bytes), "average bytes"));
    b.push_str("</div></section>\n");

    b.push_str("<section class=\"panel\">\n<div class=\"panel-head\"><span>Recent</span>");
    w!(b, "<a href=\"/cabinet/articles/new\">{}</a></div>\n", tr.cabinet_new_article);
    if recent.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<table class=\"tbl\"><thead><tr><th>Title</th><th>Language</th><th>Status</th><th>Updated</th><th></th></tr></thead><tbody>");
        for a in recent {
            w!(
                b,
                "<tr><td><a href=\"/cabinet/articles/{}\">{}</a></td><td class=\"dim\">{}</td><td><span class=\"tag tag-{}\">{}</span></td><td class=\"dim\">{}</td><td class=\"right\"><a class=\"dim\" href=\"{}/articles/{}\" target=\"_blank\" rel=\"noopener\">View</a></td></tr>",
                a.id,
                esc(&a.title),
                a.lang,
                a.status,
                a.status,
                a.edit_date_iso(),
                a.lang_enum().prefix(),
                esc(&a.slug)
            );
        }
        b.push_str("</tbody></table>");
    }
    b.push_str("</section>");
    cabinet_shell(chrome, "overview", "Overview", &b, flash)
}

pub fn cabinet_articles(chrome: &Chrome<'_>, articles: &[Article], flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"panel\">\n<div class=\"panel-head\"><span>Entries</span>");
    w!(b, "<a href=\"/cabinet/articles/new\">{}</a></div>\n", tr.cabinet_new_article);
    if articles.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<table class=\"tbl\"><thead><tr><th>Title</th><th>Kind</th><th>Language</th><th>Status</th><th>Views</th><th>Published</th><th></th></tr></thead><tbody>");
        for a in articles {
            w!(
                b,
                "<tr><td><a href=\"/cabinet/articles/{}\">{}</a></td><td class=\"dim\">{}</td><td class=\"dim\">{}</td><td><span class=\"tag tag-{}\">{}</span></td><td class=\"dim\">{}</td><td class=\"dim\">{}</td><td class=\"ops right\">",
                a.id,
                esc(&a.title),
                a.kind,
                a.lang,
                a.status,
                a.status,
                a.views,
                if a.published_at > 0 { a.date_iso() } else { "—".to_string() }
            );
            let next_status = if a.is_published() { "draft" } else { "published" };
            w!(
                b,
                "<form method=\"post\" action=\"/cabinet/articles/{}/status\" class=\"inline\"><input type=\"hidden\" name=\"status\" value=\"{}\"><button class=\"link-btn\" type=\"submit\">{}</button></form>",
                a.id,
                next_status,
                if a.is_published() { tr.cabinet_unpublish } else { tr.cabinet_publish }
            );
            if a.is_project() {
                w!(
                    b,
                    "<a class=\"link-btn\" href=\"/cabinet/projects/{}\">github</a>",
                    a.id
                );
            }
            w!(
                b,
                "<form method=\"post\" action=\"/cabinet/articles/{}/delete\" class=\"inline\" data-confirm=\"{}\"><button class=\"link-btn danger\" type=\"submit\">{}</button></form>",
                a.id,
                tr.cabinet_confirm_delete,
                tr.cabinet_delete
            );
            b.push_str("</td></tr>");
        }
        b.push_str("</tbody></table>");
    }
    b.push_str("</section>");
    cabinet_shell(chrome, "articles", tr.cabinet_articles, &b, flash)
}

pub fn cabinet_editor(
    chrome: &Chrome<'_>,
    article: Option<&Article>,
    tags: &[(String, String, i64)],
    project_tags: &[(String, String, i64)],
    flash: Option<&str>,
    errors: &[String],
) -> String {
    let tr = chrome.t();
    let (action, id, lang, slug, title, summary, body, tag_str, status, featured, published_at, key, kind) = match article {
        Some(a) => (
            format!("/cabinet/articles/{}", a.id),
            a.id,
            a.lang.clone(),
            a.slug.clone(),
            a.title.clone(),
            a.summary.clone(),
            a.body_md.clone(),
            a.tags.clone(),
            a.status.clone(),
            a.featured,
            a.published_at,
            a.translation_key.clone(),
            if a.is_project() { "project".to_string() } else { "note".to_string() },
        ),
        None => (
            "/cabinet/articles/new".to_string(),
            0,
            chrome.lang.code().to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "draft".to_string(),
            false,
            0,
            String::new(),
            "note".to_string(),
        ),
    };
    let mut b = String::new();
    if !errors.is_empty() {
        b.push_str("<div class=\"alert\" role=\"alert\">");
        for e in errors {
            w!(b, "<p class=\"alert\">{}</p>", esc(e));
        }
        b.push_str("</div>");
    }
    w!(b, "<form method=\"post\" action=\"{}\" class=\"form editor\">", action);
    if id > 0 {
        w!(b, "<input type=\"hidden\" name=\"id\" value=\"{id}\">");
        w!(b, "<input type=\"hidden\" name=\"published_at\" value=\"{published_at}\">");
    }
    b.push_str("<div class=\"form-grid\">");
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{}</span><input name=\"title\" type=\"text\" required value=\"{}\" maxlength=\"180\"></label>",
        tr.field_title,
        esc(&title)
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"slug\" type=\"text\" value=\"{}\" placeholder=\"auto-from-title\" maxlength=\"120\" pattern=\"[a-z0-9а-я\\-]*\"></label>",
        tr.field_slug,
        esc(&slug)
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><select name=\"lang\">{}</select></label>",
        tr.field_lang,
        ["ru", "en"]
            .iter()
            .map(|l| format!(
                "<option value=\"{l}\"{}>{l}</option>",
                if *l == lang { " selected" } else { "" }
            ))
            .collect::<String>()
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><select name=\"kind\"><option value=\"note\"{}>{}</option><option value=\"project\"{}>{}</option></select></label>",
        tr.field_kind,
        if kind == "note" { " selected" } else { "" },
        tr.kind_note,
        if kind == "project" { " selected" } else { "" },
        tr.kind_project
    );
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{}</span><input name=\"summary\" type=\"text\" value=\"{}\" maxlength=\"300\"></label>",
        tr.field_summary,
        esc(&summary)
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"tags\" type=\"text\" value=\"{}\" placeholder=\"{}\" maxlength=\"200\"></label>",
        tr.field_tags,
        esc(&tag_str),
        tr.field_tags_hint
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"translation_key\" type=\"text\" value=\"{}\" maxlength=\"64\" placeholder=\"RU ↔ EN\"></label>",
        tr.field_translation_key,
        esc(&key)
    );
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{}</span><textarea name=\"body_md\" rows=\"22\" spellcheck=\"false\" data-editor>{}</textarea><span class=\"field-foot\"><span data-count>0</span> characters · <span data-read>0</span> min read</span></label>",
        tr.field_body,
        esc(&body)
    );
    b.push_str("</div><div class=\"form-actions\">");
    w!(
        b,
        "<label class=\"switch\"><input type=\"checkbox\" name=\"featured\" value=\"1\"{}><span>{}</span></label>",
        if featured { " checked" } else { "" },
        tr.featured
    );
    b.push_str("<div class=\"form-actions-right\">");
    w!(
        b,
        "<select name=\"status\" class=\"select-inline\"><option value=\"draft\"{}>{}</option><option value=\"published\"{}>{}</option></select>",
        if status == "draft" { " selected" } else { "" },
        tr.cabinet_draft,
        if status == "published" { " selected" } else { "" },
        tr.cabinet_publish
    );
    w!(b, "<button class=\"btn btn-solid\" type=\"submit\">{}</button>", tr.cabinet_save);
    b.push_str("</div></div></form>");
    if let Some(a) = article {
        if a.is_project() {
            w!(
                b,
                "<p class=\"panel-foot\"><a href=\"/cabinet/projects/{}\">GitHub и журнал проекта →</a></p>",
                a.id
            );
        }
        b.push_str("<section class=\"panel\">");
        w!(
            b,
            "<div class=\"panel-head\"><span>Preview</span><a href=\"{}/{}/{}\" target=\"_blank\" rel=\"noopener\">Open</a></div>\n<div class=\"prose\">{}</div></section>",
            a.lang_enum().prefix(),
            a.section(),
            esc(&a.slug),
            render_markdown(&a.body_md).html
        );
    }
    let _ = (tags, project_tags);
    cabinet_shell(
        chrome,
        if id > 0 { "articles" } else { "new" },
        if id > 0 { tr.cabinet_editor } else { tr.cabinet_new_article },
        &b,
        flash,
    )
}

pub fn cabinet_profile(chrome: &Chrome<'_>, p: &Profile, flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<form method=\"post\" action=\"/cabinet/profile\" class=\"form\">");
    b.push_str("<div class=\"form-grid\">");
    let text_fields = [
        ("owner_name", "Name", &p.owner_name),
        ("handle", "Handle", &p.handle),
        ("location", "Location", &p.location),
        ("email", "Email", &p.email),
        ("github", "GitHub", &p.github),
        ("telegram", "Telegram", &p.telegram),
        ("matrix", "Matrix", &p.matrix),
        ("site_url", "Site URL", &p.site_url),
        ("role_ru", "Role (RU)", &p.role_ru),
        ("role_en", "Role (EN)", &p.role_en),
    ];
    for (name, label, value) in text_fields {
        w!(
            b,
            "<label class=\"field\"><span class=\"label\">{}</span><input name=\"{}\" type=\"text\" value=\"{}\"></label>",
            label,
            name,
            esc(value)
        );
    }
    for (name, label, value) in [
        ("manifesto_ru", "Manifesto (RU)", &p.manifesto_ru),
        ("manifesto_en", "Manifesto (EN)", &p.manifesto_en),
        ("now_ru", "Now (RU)", &p.now_ru),
        ("now_en", "Now (EN)", &p.now_en),
    ] {
        w!(
            b,
            "<label class=\"field field-full\"><span class=\"label\">{}</span><textarea name=\"{}\" rows=\"3\">{}</textarea></label>",
            label,
            name,
            esc(value)
        );
    }
    b.push_str("</div><div class=\"form-actions\">");
    w!(b, "<button class=\"btn btn-solid\" type=\"submit\">{}</button>", tr.cabinet_save);
    b.push_str("</div></form>");
    cabinet_shell(chrome, "profile", tr.cabinet_profile, &b, flash)
}

pub fn cabinet_telemetry(chrome: &Chrome<'_>, tele: &Telemetry, flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<div class=\"stats\">");
    b.push_str(&stat_card("Views", &fmt_num(tele.total_views), "all time"));
    b.push_str(&stat_card("24 hours", &fmt_num(tele.views_today), "requests"));
    b.push_str(&stat_card("Visitors", &fmt_num(tele.uniques_today), "unique, 24h"));
    b.push_str(&stat_card("Bots", &format!("{}%", tele.bot_share), "of traffic"));
    b.push_str("</div>\n");

    let max = tele.daily.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>Last 14 days</span><span>requests</span></div>");
    b.push_str("<div class=\"bars\">");
    for (day, n) in &tele.daily {
        let height = ((*n as f64 / max as f64) * 100.0).round() as i64;
        w!(
            b,
            "<div class=\"bar\" title=\"{} — {}\"><div class=\"bar-fill\" style=\"height:{}%\"></div><span>{}</span></div>",
            day,
            n,
            height.max(if *n > 0 { 4 } else { 1 }),
            day.get(8..).unwrap_or(day)
        );
    }
    b.push_str("</div>");
    w!(
        b,
        "<p class=\"panel-foot\">Average render <b>{} µs</b> · p95 <b>{} µs</b> · average payload <b>{} B</b></p>",
        tele.avg_render_us,
        tele.p95_render_us,
        tele.avg_bytes
    );
    b.push_str("</section>\n");

    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>Top paths</span><span>requests</span></div>");
    if tele.top_paths.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<table class=\"tbl\"><tbody>");
        for (path, n) in &tele.top_paths {
            w!(b, "<tr><td class=\"dim\">{}</td><td class=\"dim right\">{}</td></tr>", esc(path), n);
        }
        b.push_str("</tbody></table>");
    }
    b.push_str("</section>");
    cabinet_shell(chrome, "telemetry", tr.cabinet_telemetry, &b, flash)
}

pub fn cabinet_security(
    chrome: &Chrome<'_>,
    sessions: &[SessionInfo],
    flash: Option<&str>,
    error: Option<&str>,
) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    if let Some(e) = error {
        w!(b, "<p class=\"alert\" role=\"alert\">{}</p>", esc(e));
    }
    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>Sessions</span><span>hashed tokens</span></div>");
    b.push_str("<table class=\"tbl\"><thead><tr><th>State</th><th>Last active</th><th>Expires</th><th>Device</th><th></th></tr></thead><tbody>");
    for s in sessions {
        w!(
            b,
            "<tr><td>{}</td><td class=\"dim\">{}</td><td class=\"dim\">{}</td><td class=\"dim\">{}</td><td class=\"right\">",
            if s.current { "<span class=\"tag tag-current\">This device</span>" } else { "<span class=\"tag\">Active</span>" },
            i18n::format_datetime_utc(s.last_seen),
            i18n::format_date_utc(s.expires_at),
            esc(&crate::util::truncate_chars(&s.ua, 40))
        );
        if !s.current {
            w!(
                b,
                "<form method=\"post\" action=\"/cabinet/security/revoke\" class=\"inline\"><input type=\"hidden\" name=\"token\" value=\"{}\"><button class=\"link-btn\" type=\"submit\">{}</button></form>",
                esc(&s.token_hash),
                tr.cabinet_revoke
            );
        }
        b.push_str("</td></tr>");
    }
    b.push_str("</tbody></table>");
    w!(
        b,
        "<form method=\"post\" action=\"/cabinet/security/revoke-all\" class=\"panel-actions\"><button class=\"link-btn\" type=\"submit\">{}</button></form>",
        tr.cabinet_revoke_others
    );
    b.push_str("</section>\n");

    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>Password</span><span>PBKDF2-HMAC-SHA256</span></div>");
    b.push_str("<form method=\"post\" action=\"/cabinet/security/password\" class=\"form\">");
    b.push_str("<div class=\"form-grid\">");
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"current\" type=\"password\" autocomplete=\"current-password\" required></label>",
        tr.cabinet_current_password
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"new\" type=\"password\" autocomplete=\"new-password\" minlength=\"12\" required></label>",
        tr.cabinet_new_password
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"repeat\" type=\"password\" autocomplete=\"new-password\" minlength=\"12\" required></label>",
        tr.cabinet_repeat_password
    );
    b.push_str("</div><div class=\"form-actions\">");
    w!(b, "<button class=\"btn btn-solid\" type=\"submit\">{}</button>", tr.cabinet_change_password);
    b.push_str("</div></form></section>");
    cabinet_shell(chrome, "security", tr.cabinet_security, &b, flash)
}

pub fn cabinet_backup(chrome: &Chrome<'_>, stats: &BackupStats, flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<div class=\"stats\">");
    b.push_str(&stat_card("Entries", &stats.articles.to_string(), "exported"));
    b.push_str(&stat_card("Published", &stats.published.to_string(), "live"));
    b.push_str(&stat_card("Drafts", &stats.drafts.to_string(), "included"));
    b.push_str(&stat_card("Views", &fmt_num(stats.views), "logged"));
    b.push_str("</div>\n");
    b.push_str("<section class=\"panel\">");
    w!(b, "<p class=\"panel-note\">{}</p>", tr.cabinet_export_hint);
    w!(
        b,
        "<p><a class=\"btn\" href=\"/cabinet/backup.json\" download=\"rudolfzinovev-backup.json\">{}</a></p>",
        tr.cabinet_download
    );
    b.push_str("</section>");
    cabinet_shell(chrome, "backup", tr.cabinet_backup, &b, flash)
}

// ----------------------------------------------------------------- errors

pub fn not_found(chrome: &Chrome<'_>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"err\">");
    w!(b, "<p class=\"err-code\">404</p>");
    w!(b, "<h1 class=\"err-title\">{}</h1>", tr.not_found_title);
    w!(b, "<p class=\"err-text\">{}</p>", tr.not_found_text);
    w!(b, "<a class=\"back\" href=\"{}\">{} →</a>", chrome.href("home"), tr.not_found_cta);
    b.push_str("</section>");
    let mut h = head(chrome, &format!("404 — {}", chrome.profile.owner_name), tr.not_found_text, true);
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell\">");
    h.push_str(&b);
    h.push_str("</main>\n");
    h.push_str(&footer(chrome));
    h
}

pub fn server_error(chrome: &Chrome<'_>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"err\">");
    w!(b, "<p class=\"err-code\">500</p>");
    w!(b, "<h1 class=\"err-title\">{}</h1>", tr.server_error_title);
    w!(b, "<p class=\"err-text\">{}</p>", tr.server_error_text);
    w!(b, "<a class=\"back\" href=\"{}\">{} →</a>", chrome.href("home"), tr.not_found_cta);
    b.push_str("</section>");
    let mut h = head(chrome, &format!("500 — {}", chrome.profile.owner_name), tr.server_error_text, true);
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell\">");
    h.push_str(&b);
    h.push_str("</main>\n");
    h.push_str(&footer(chrome));
    h
}

// ------------------------------------------------------------------- feeds

pub fn feed_xml(lang: Lang, profile: &Profile, articles: &[Article]) -> String {
    let (site_title, site_desc) = match lang {
        Lang::Ru => (
            profile.owner_name.clone(),
            format!("{} — технические заметки", profile.role_ru),
        ),
        Lang::En => (
            profile.owner_name.clone(),
            format!("{} — technical notes", profile.role_en),
        ),
    };
    let base = format!("{}{}", profile.site_url.trim_end_matches('/'), lang.prefix());
    let mut x = String::with_capacity(8192);
    x.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n<channel>\n");
    w!(x, "<title>{}</title>\n", esc(&site_title));
    w!(x, "<link>{base}/</link>\n");
    w!(x, "<description>{}</description>\n", esc(&site_desc));
    x.push_str("<language>");
    x.push_str(lang.code());
    x.push_str("</language>\n");
    w!(
        x,
        "<atom:link href=\"{base}/feed.xml\" rel=\"self\" type=\"application/rss+xml\"/>\n"
    );
    if let Some(first) = articles.first() {
        w!(x, "<lastBuildDate>{}</lastBuildDate>\n", i18n::format_rfc2822_utc(first.updated_at));
    }
    for a in articles {
        w!(x, "<item>\n<title>{}</title>\n", esc(&a.title));
        w!(x, "<link>{base}{}</link>\n", entry_path(a.lang_enum().prefix(), a.section(), &a.slug));
        w!(x, "<guid isPermaLink=\"true\">{base}/{}</guid>\n", entry_path(a.lang_enum().prefix(), a.section(), &a.slug));
        w!(x, "<pubDate>{}</pubDate>\n", i18n::format_rfc2822_utc(a.published_at));
        w!(x, "<description>{}</description>\n", esc(&a.summary));
        for tag in a.tag_list() {
            w!(x, "<category>{}</category>\n", esc(&tag));
        }
        x.push_str("</item>\n");
    }
    x.push_str("</channel>\n</rss>\n");
    x
}

pub fn sitemap_xml(profile: &Profile, articles: &[Article]) -> String {
    let base = profile.site_url.trim_end_matches('/');
    let mut x = String::with_capacity(4096);
    x.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    let mut add = |loc: String, lastmod: Option<String>, priority: &str| {
        x.push_str("<url>");
        w!(x, "<loc>{}</loc>", esc(&loc));
        if let Some(m) = lastmod {
            w!(x, "<lastmod>{m}</lastmod>");
        }
        w!(x, "<priority>{priority}</priority></url>\n");
    };
    add(format!("{base}/"), None, "1.0");
    add(format!("{base}/en"), None, "1.0");
    add(format!("{base}/notes"), None, "0.9");
    add(format!("{base}/en/notes"), None, "0.9");
    add(format!("{base}/projects"), None, "0.9");
    add(format!("{base}/en/projects"), None, "0.9");
    add(format!("{base}/about"), None, "0.6");
    add(format!("{base}/en/about"), None, "0.6");
    for a in articles {
        add(
            format!("{base}{}", entry_path(a.lang_enum().prefix(), a.section(), &a.slug)),
            Some(i18n::format_date_utc(a.updated_at)),
            "0.8",
        );
    }
    x.push_str("</urlset>\n");
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome(lang: Lang) -> Chrome<'static> {
        static P: std::sync::OnceLock<Profile> = std::sync::OnceLock::new();
        let p = P.get_or_init(|| Profile {
            owner_name: "Rudolf Zinovev".into(),
            handle: "rudolf".into(),
            role_ru: "инженер".into(),
            role_en: "engineer".into(),
            manifesto_ru: "Манифест".into(),
            manifesto_en: "Manifesto".into(),
            now_ru: "Сейчас".into(),
            now_en: "Now".into(),
            location: "Warsaw".into(),
            email: "me@example.com".into(),
            github: "https://github.com/x".into(),
            telegram: "https://t.me/x".into(),
            matrix: "@x:y".into(),
            site_url: "https://rudolfzinovev.me".into(),
        });
        Chrome {
            lang,
            path: "/",
            profile: p,
            owner: false,
        }
    }

    #[test]
    fn home_renders_without_leaking_placeholders() {
        let html = home(&chrome(Lang::Ru), &[], &[], 8, 2);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Rudolf"));
        // No decorative instrument blocks and no per-request markers survive.
        assert!(!html.contains("@@RENDER_US@@"));
        assert!(!html.contains("data-render-us"));
        assert!(!html.contains("ticker"));
        assert!(!html.contains("SPEC //"));
        // No theme machinery at all: one paper, no switches.
        assert!(!html.contains("data-theme"));
        assert!(!html.contains("theme-toggle"));
        assert!(!html.contains("rz_theme"));
    }

    fn project_article() -> Article {
        Article {
            kind: "project".into(),
            id: 42,
            translation_key: "k".into(),
            lang: "ru".into(),
            slug: "tzim".into(),
            title: "tzim".into(),
            summary: "sum".into(),
            body_md: "## Head\n\nbody".into(),
            tags: "rust".into(),
            status: "published".into(),
            featured: false,
            reading_minutes: 3,
            published_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            created_at: 1_700_000_000,
            views: 0,
        }
    }

    /// The whole point of the feature: generated facts and the owner's own
    /// notes may sit on one page but must never be interleaved or styled alike.
    #[test]
    fn project_page_keeps_github_and_journal_apart() {
        let a = project_article();
        let link = RepoLink {
            repo: "RUdolf1517/tzim".into(),
            stars: 7,
            language: "Rust".into(),
            release_tag: "v0.16.1".into(),
            release_url: "https://github.com/RUdolf1517/tzim/releases/tag/v0.16.1".into(),
            release_at: 1_700_000_050,
            pushed_at: 1_700_000_100,
            fetched_at: 1_700_000_200,
            ..Default::default()
        };
        let events = vec![crate::github::GhEvent {
            kind: "commit".into(),
            title: "brew: update formula".into(),
            detail: "f605c2e · Rudolf".into(),
            url: "https://github.com/RUdolf1517/tzim/commit/f605c2e".into(),
            at: 1_700_000_100,
        }];
        let notes = vec![ProjectNote {
            id: 9,
            project_id: 42,
            lang: "ru".into(),
            kind: "decision".into(),
            title: "Почему отказались от zopfli".into(),
            body_md: "Компромисс между временем и размером.".into(),
            log_date: 1_700_000_150,
            public: true,
            created_at: 0,
            updated_at: 0,
        }];
        let view = ProjectView { link: Some(&link), events: &events, notes: &notes };
        let html = entry_page_with(&chrome(Lang::Ru), &a, None, &[], Some(&view));

        let repo_at = html.find("class=\"repo\"").expect("repository block");
        let journal_at = html.find("class=\"journal\"").expect("journal block");
        assert!(repo_at < journal_at, "the feed comes first, the notebook after");

        // Different visual languages, asserted on the markup itself.
        assert!(html.contains("<ol class=\"feed\">"));
        assert!(html.contains("feed-kind"));
        assert!(html.contains("journal-kind"));
        assert!(html.contains("js-decision"));

        // The generated event is inside the feed, the owner's note is not.
        let feed = &html[repo_at..journal_at];
        assert!(feed.contains("brew: update formula"));
        assert!(!feed.contains("Почему отказались от zopfli"));
        let journal = &html[journal_at..];
        assert!(journal.contains("Почему отказались от zopfli"));
        assert!(!journal.contains("brew: update formula"));
        assert!(journal.contains("решение"), "the kind is named in words");
    }

    #[test]
    fn journal_hides_private_entries_from_visitors() {
        let a = project_article();
        let mut note = ProjectNote {
            id: 1,
            project_id: 42,
            lang: "ru".into(),
            kind: "note".into(),
            title: "черновик".into(),
            body_md: "секрет".into(),
            log_date: 1_700_000_000,
            public: false,
            created_at: 0,
            updated_at: 0,
        };
        let notes = vec![note.clone()];
        let view = ProjectView { link: None, events: &[], notes: &notes };
        let visitor = entry_page_with(&chrome(Lang::Ru), &a, None, &[], Some(&view));
        assert!(!visitor.contains("секрет"), "a private entry never reaches a visitor");

        // The owner sees it, marked as a draft.
        let mut owner_chrome = chrome(Lang::Ru);
        owner_chrome.owner = true;
        let owner = entry_page_with(&owner_chrome, &a, None, &[], Some(&view));
        assert!(owner.contains("секрет"));
        assert!(owner.contains("черновик"));

        note.public = true;
        let notes = vec![note];
        let view = ProjectView { link: None, events: &[], notes: &notes };
        let visitor = entry_page_with(&chrome(Lang::Ru), &a, None, &[], Some(&view));
        assert!(visitor.contains("секрет"));
    }

    #[test]
    fn article_page_escapes_and_switches_language() {
        let a = Article {
            kind: "note".into(),
            id: 1,
            translation_key: "k".into(),
            lang: "ru".into(),
            slug: "test".into(),
            title: "<script>alert(1)</script>".into(),
            summary: "sum".into(),
            body_md: "## Head\n\nbody".into(),
            tags: "rust".into(),
            status: "published".into(),
            featured: false,
            reading_minutes: 2,
            published_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            created_at: 1_700_000_000,
            views: 3,
        };
        let html = entry_page(&chrome(Lang::Ru), &a, None, &[]);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("s1-head"));
    }

    #[test]
    fn feed_is_wellformed_ish() {
        let c = chrome(Lang::En);
        let xml = feed_xml(Lang::En, c.profile, &[]);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<link>https://rudolfzinovev.me/en/</link>"));
    }

    #[test]
    fn alt_lang_paths() {
        let c = chrome(Lang::Ru);
        assert_eq!(c.alt_lang_path(Lang::En), "/en");
        let mut c2 = chrome(Lang::En);
        c2.path = "/en/articles/x";
        assert_eq!(c2.alt_lang_path(Lang::Ru), "/articles/x");
    }
}

// ------------------------------------------------------- project sections

/// The two histories, kept visually apart:
///   · «Из репозитория» — facts, machine-generated, set in the mono voice;
///   · «Журнал» — the owner's own notes, decisions and milestones, set in the
///     serif voice with a margin mark. They are never interleaved.
fn project_sections(chrome: &Chrome<'_>, article: &Article, view: &ProjectView<'_>) -> String {
    let ru = chrome.lang == Lang::Ru;
    let mut b = String::new();

    if let Some(link) = view.link.filter(|l| l.is_linked()) {
        b.push_str("<section class=\"repo\" aria-labelledby=\"repo-h\">");
        w!(
            b,
            "<h2 class=\"sec-label\" id=\"repo-h\">{}</h2>",
            if ru { "Из репозитория" } else { "From the repository" }
        );
        b.push_str("<div class=\"repo-head\">");
        let repo_url = if link.html_url.is_empty() {
            format!("https://github.com/{}", link.repo)
        } else {
            link.html_url.clone()
        };
        w!(
            b,
            "<a class=\"repo-name\" href=\"{}\" rel=\"noopener\" target=\"_blank\">{}</a>",
            esc(&repo_url),
            esc(&link.repo)
        );

        // Facts, in the typewriter voice: numbers are facts, not prose.
        b.push_str("<p class=\"repo-facts\">");
        let mut facts: Vec<String> = Vec::new();
        if link.stars > 0 {
            facts.push(format!("★ {}", fmt_num(link.stars)));
        }
        if link.forks > 0 {
            facts.push(format!("⑂ {}", fmt_num(link.forks)));
        }
        if !link.language.is_empty() {
            facts.push(esc(&link.language));
        }
        if !link.license.is_empty() {
            facts.push(esc(&link.license));
        }
        if link.archived {
            facts.push(if ru { "архив".into() } else { "archived".to_string() });
        }
        if link.pushed_at > 0 {
            facts.push(format!(
                "{} {}",
                if ru { "обновлён" } else { "updated" },
                crate::i18n::format_date_utc(link.pushed_at)
            ));
        }
        b.push_str(&facts.join(" · "));
        b.push_str("</p>");

        if !link.description.is_empty() {
            w!(b, "<p class=\"repo-desc\">{}</p>", esc(&link.description));
        }

        if !link.release_tag.is_empty() {
            b.push_str("<p class=\"repo-release\">");
            w!(
                b,
                "<span class=\"repo-release-label\">{}</span> ",
                if ru { "последний релиз" } else { "latest release" }
            );
            w!(
                b,
                "<a href=\"{}\" rel=\"noopener\" target=\"_blank\">{}</a>",
                esc(&link.release_url),
                esc(&link.release_tag)
            );
            if link.release_at > 0 {
                w!(
                    b,
                    " <span class=\"dim\">· {}</span>",
                    crate::i18n::format_date_utc(link.release_at)
                );
            }
            b.push_str("</p>");
        }

        // Issues stay opt-in and only ever appear as a counter.
        if link.issues_enabled() && link.open_issues > 0 {
            w!(
                b,
                "<p class=\"repo-facts\">{} <a href=\"https://github.com/{}/issues\" rel=\"noopener\" target=\"_blank\">{}</a></p>",
                if ru { "открытых issues:" } else { "open issues:" },
                esc(&link.repo),
                fmt_num(link.open_issues)
            );
        }
        b.push_str("</div>");

        // Automatic history: releases, commits, README — machine text, mono,
        // dated, never written by hand.
        if !view.events.is_empty() {
            b.push_str("<ol class=\"feed\">");
            for e in view.events {
                let (label, mark) = match e.kind.as_str() {
                    "release" => (
                        if ru { "релиз" } else { "release" },
                        if ru { "версия" } else { "version" },
                    ),
                    "readme" => (
                        if ru { "README" } else { "README" },
                        if ru { "документация" } else { "docs" },
                    ),
                    _ => (
                        if ru { "коммит" } else { "commit" },
                        if ru { "код" } else { "code" },
                    ),
                };
                w!(
                    b,
                    "<li class=\"feed-item feed-{}\"><time datetime=\"{}\">{}</time>",
                    esc(&e.kind),
                    crate::i18n::format_date_utc(e.at),
                    crate::i18n::format_date_utc(e.at)
                );
                w!(b, "<span class=\"feed-kind\">{}</span>", label);
                if e.url.is_empty() {
                    w!(b, "<span class=\"feed-title\">{}</span>", esc(&e.title));
                } else {
                    w!(
                        b,
                        "<a class=\"feed-title\" href=\"{}\" rel=\"noopener\" target=\"_blank\">{}</a>",
                        esc(&e.url),
                        esc(&e.title)
                    );
                }
                if !e.detail.is_empty() && e.detail != e.title {
                    w!(b, "<span class=\"feed-detail\">{}</span>", esc(&e.detail));
                }
                b.push_str("</li>");
                let _ = mark;
            }
            b.push_str("</ol>");
        }

        if link.fetched_at > 0 {
            w!(
                b,
                "<p class=\"repo-stamp\">{} {}</p>",
                if ru { "снимок" } else { "snapshot" },
                crate::i18n::format_date_utc(link.fetched_at)
            );
        }
        if !link.last_error.is_empty() {
            w!(
                b,
                "<p class=\"repo-stamp\">{}</p>",
                if ru {
                    "GitHub сейчас недоступен — показан последний снимок."
                } else {
                    "GitHub is unreachable right now — showing the last snapshot."
                }
            );
        }
        if chrome.owner {
            w!(
                b,
                "<p class=\"repo-stamp\"><a href=\"/cabinet/projects/{}\">{}</a></p>",
                article.id,
                if ru { "настроить привязку →" } else { "configure this link →" }
            );
        }
        b.push_str("</section>");
    }

    // The owner's journal. Deliberately a different shape from the feed above:
    // no dates in a column, no mono typeface, a margin mark per kind.
    if !view.notes.is_empty() {
        b.push_str("<section class=\"journal\" aria-labelledby=\"journal-h\">");
        w!(
            b,
            "<h2 class=\"sec-label\" id=\"journal-h\">{}</h2>",
            if ru { "Журнал проекта" } else { "Project journal" }
        );
        for kind in LogKind::all() {
            let group: Vec<&ProjectNote> = view
                .notes
                .iter()
                .filter(|n| n.kind_enum() == kind)
                .filter(|n| n.public || chrome.owner)
                .collect();
            if group.is_empty() {
                continue;
            }
            w!(
                b,
                "<h3 class=\"journal-kind\"><span class=\"mark\" aria-hidden=\"true\">{}</span>{}</h3>",
                kind.mark(),
                kind.label(chrome.lang)
            );
            for note in group {
                w!(
                    b,
                    "<article class=\"journal-item js-{}\" id=\"n{}\">",
                    kind.code(),
                    note.id
                );
                b.push_str("<header class=\"journal-head\">");
                w!(
                    b,
                    "<time class=\"journal-date\" datetime=\"{}\">{}</time>",
                    note.date_machine(),
                    note.date_iso()
                );
                w!(b, "<h4 class=\"journal-title\">{}</h4>", esc(&note.title));
                if !note.public {
                    w!(
                        b,
                        "<span class=\"journal-private\">{}</span>",
                        if ru { "черновик" } else { "private" }
                    );
                }
                if chrome.owner {
                    w!(
                        b,
                        "<a class=\"journal-edit\" href=\"/cabinet/projects/{}/notes/{}\">{}</a>",
                        note.project_id,
                        note.id,
                        if ru { "править" } else { "edit" }
                    );
                }
                b.push_str("</header>");
                let rendered = render_markdown(&note.body_md);
                w!(b, "<div class=\"prose journal-body\">{}</div>", rendered.html);
                b.push_str("</article>");
            }
        }
        if chrome.owner {
            w!(
                b,
                "<p class=\"journal-add\"><a href=\"/cabinet/projects/{}/notes/new\">{}</a></p>",
                article.id,
                if ru { "+ добавить запись" } else { "+ add a note" }
            );
        }
        b.push_str("</section>");
    } else if chrome.owner {
        w!(
            b,
            "<section class=\"journal\"><p class=\"journal-add\"><a href=\"/cabinet/projects/{}/notes/new\">{}</a></p></section>",
            article.id,
            if ru { "+ добавить запись в журнал" } else { "+ add a journal entry" }
        );
    }

    b
}

// ------------------------------------------------- cabinet: project binding

/// Where a project's GitHub link is configured. Separate from the article
/// editor on purpose: the editor is about writing, this is about plumbing.
pub fn cabinet_project(
    chrome: &Chrome<'_>,
    article: &Article,
    link: Option<&RepoLink>,
    events: &[crate::github::GhEvent],
    counts: (i64, i64, i64),
    notes: &[ProjectNote],
    flash: Option<&str>,
    error: Option<&str>,
) -> String {
    let ru = chrome.lang == Lang::Ru;
    let t = |rus: &'static str, eng: &'static str| if ru { rus } else { eng };
    let mut b = String::new();
    if let Some(err) = error {
        w!(b, "<p class=\"alert\" role=\"alert\">{}</p>", esc(err));
    }
    let linked = link.filter(|l| l.is_linked());
    let date = crate::i18n::format_date_dots;

    // ---------------------------------------------------------- repository
    b.push_str("<section class=\"panel\">");
    w!(
        b,
        "<div class=\"panel-head\"><span>{}</span><a href=\"/cabinet/articles/{}\">{} →</a></div>",
        esc(&article.title),
        article.id,
        t("запись", "entry")
    );

    if let Some(link) = linked {
        b.push_str("<table class=\"tbl\"><tbody>");
        let row = |k: &str, v: String| {
            format!("<tr><th class=\"dim\">{}</th><td>{}</td></tr>", esc(k), v)
        };
        b.push_str(&row(t("Репозиторий", "Repository"), esc(&link.repo)));
        if !link.description.is_empty() {
            b.push_str(&row(t("Описание", "Description"), esc(&link.description)));
        }
        b.push_str(&row(
            t("Сведения", "Facts"),
            format!(
                "★ {} · ⑂ {} · {} · {}",
                link.stars, link.forks, esc(&link.language), esc(&link.license)
            ),
        ));
        if !link.release_tag.is_empty() {
            b.push_str(&row(
                t("Последний релиз", "Latest release"),
                format!(
                    "{} <span class=\"dim\">· {}</span>",
                    esc(&link.release_tag),
                    if link.release_at > 0 {
                        date(link.release_at)
                    } else {
                        String::new()
                    }
                ),
            ));
        }
        if link.pushed_at > 0 {
            b.push_str(&row(
                t("Последний коммит", "Last commit"),
                date(link.pushed_at),
            ));
        }
        b.push_str(&row(
            t("Обновлено", "Last refresh"),
            if link.fetched_at > 0 {
                format!(
                    "{} <span class=\"dim\">· {}</span>",
                    date(link.fetched_at),
                    t("обновляется само", "refreshes on its own")
                )
            } else {
                t("ещё не загружалось", "never fetched").to_string()
            },
        ));
        if link.show_issues {
            b.push_str(&row(
                t("Открытых issues", "Open issues"),
                link.open_issues.to_string(),
            ));
        }
        if !link.last_error.is_empty() {
            b.push_str(&row(
                t("Последняя ошибка", "Last error"),
                format!("<span class=\"danger\">{}</span>", esc(&link.last_error)),
            ));
        }
        b.push_str(&row(
            t("Автоматическая история", "Automatic history"),
            format!(
                "{} <span class=\"dim\">· {}</span>",
                match events.len() {
                    0 => t("пока пусто", "nothing cached yet").to_string(),
                    n => format!("{} {}", n, t("событий в кэше", "events cached")),
                },
                t("коммиты, релизы, README", "commits, releases, README")
            ),
        ));
        b.push_str("</tbody></table>");

        b.push_str("<div class=\"panel-actions\">");
        w!(
            b,
            "<form method=\"post\" action=\"/cabinet/projects/{}/refresh\" class=\"inline\"><button class=\"btn\" type=\"submit\">{}</button></form>",
            article.id,
            t("Обновить сейчас", "Refresh now")
        );
        w!(
            b,
            "<form method=\"post\" action=\"/cabinet/projects/{}/unlink\" class=\"inline\" data-confirm=\"{}\"><button class=\"link-btn danger\" type=\"submit\">{}</button></form>",
            article.id,
            t("Забыть сохранённые данные об этом репозитории?", "Forget the cached snapshot for this repository?"),
            t("Отвязать", "Unlink")
        );
        b.push_str("</div>");
    } else {
        w!(
            b,
            "<p class=\"panel-note\">{}</p>",
            t(
                "Репозиторий пока не привязан. Вставь <code>owner/name</code> или ссылку на GitHub.",
                "No repository bound yet. Paste <code>owner/name</code> or a GitHub URL."
            )
        );
    }
    b.push_str("</section>");

    // ------------------------------------------------------- binding form
    b.push_str("<section class=\"panel\">");
    w!(
        b,
        "<div class=\"panel-head\"><span>{}</span><span class=\"dim\">{}</span></div>",
        t("GitHub-репозиторий", "GitHub repository"),
        t("только публичные", "public repos only")
    );
    w!(
        b,
        "<form method=\"post\" action=\"/cabinet/projects/{}/link\" class=\"form\">",
        article.id
    );
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{}</span><input name=\"repo\" type=\"text\" value=\"{}\" placeholder=\"RUdolf1517/tzim\" maxlength=\"140\"></label>",
        t("Репозиторий", "Repository"),
        esc(linked.map(|l| l.repo.as_str()).unwrap_or(""))
    );
    w!(
        b,
        "<label class=\"switch\"><input type=\"checkbox\" name=\"show_issues\" value=\"1\"{}><span>{}</span></label>",
        if linked.map(|l| l.show_issues).unwrap_or(false) {
            " checked"
        } else {
            ""
        },
        t("Показывать число открытых issues", "Show open issues count")
    );
    b.push_str("<div class=\"form-actions\"><div class=\"form-actions-right\">");
    w!(
        b,
        "<button class=\"btn btn-solid\" type=\"submit\">{}</button>",
        t("Сохранить", "Save link")
    );
    b.push_str("</div></div></form>");
    w!(
        b,
        "<p class=\"panel-foot\">{}</p>",
        t(
            "Коммиты, релизы и изменения README загружаются автоматически и кэшируются. Issues и pull requests выключены, пока не отмечено выше.",
            "Commits, releases and README changes are fetched automatically and cached. Issues and pull requests stay off unless enabled above."
        )
    );
    b.push_str("</section>");

    // ------------------------------------------------------------- journal
    b.push_str("<section class=\"panel\">");
    w!(
        b,
        "<div class=\"panel-head\"><span>{}</span><a href=\"/cabinet/projects/{}/notes/new\">+ {}</a></div>",
        t("Личные заметки", "Journal"),
        article.id,
        t("добавить", "add")
    );
    w!(
        b,
        "<p class=\"panel-note\">{}: {} · {} · {}. {}</p>",
        t("Написаны тобой и никогда не смешиваются с GitHub", "Written by you and never mixed with GitHub"),
        format!("{} {}", counts.0, t("наблюдений", "observations")),
        format!("{} {}", counts.1, t("решений", "decisions")),
        format!("{} {}", counts.2, t("этапов", "milestones")),
        t(
            "Отдельный блок на странице проекта: свой шрифт, своя колонка на полях, свои метки N / D / M.",
            "A separate block on the project page: its own type, its own margin column, its own N / D / M marks."
        )
    );

    if notes.is_empty() {
        w!(
            b,
            "<p class=\"panel-note\">{}</p>",
            t("Заметок пока нет.", "No entries yet.")
        );
    } else {
        b.push_str("<table class=\"tbl\"><tbody>");
        for n in notes {
            let hidden = if n.public {
                String::new()
            } else {
                format!(
                    " <span class=\"dim\">· {}</span>",
                    t("черновик, виден только тебе", "draft, visible to you only")
                )
            };
            w!(
                b,
                "<tr><th class=\"dim mark-col\">{}</th><td><a href=\"/cabinet/projects/{}/notes/{}\">{}</a>\
                 <span class=\"dim\">· {} · {}</span>{}</td></tr>",
                format!(
                    "{} {}",
                    LogKind::from_code(&n.kind).mark(),
                    date(n.log_date)
                ),
                n.project_id,
                n.id,
                esc(&n.title),
                LogKind::from_code(&n.kind).label(chrome.lang),
                n.lang,
                hidden
            );
        }
        b.push_str("</tbody></table>");
    }
    b.push_str("</section>");

    cabinet_shell(
        chrome,
        "projects",
        t("Проект · GitHub", "Project · GitHub"),
        &b,
        flash,
    )
}

/// Create or edit one journal entry.
pub fn cabinet_project_note(
    chrome: &Chrome<'_>,
    project: &Article,
    note: Option<&ProjectNote>,
    flash: Option<&str>,
    errors: &[String],
) -> String {
    let ru = chrome.lang == Lang::Ru;
    let t = |rus: &'static str, eng: &'static str| if ru { rus } else { eng };
    let mut b = String::new();
    if !errors.is_empty() {
        b.push_str("<div class=\"alert\" role=\"alert\">");
        for e in errors {
            w!(b, "<p class=\"alert\">{}</p>", esc(e));
        }
        b.push_str("</div>");
    }
    // A note belongs to the project in the URL, whatever the form says.
    let (action, kind, lang, title, body, date, public) = match note {
        Some(n) => (
            format!("/cabinet/projects/{}/notes/{}", n.project_id, n.id),
            n.kind.clone(),
            n.lang.clone(),
            n.title.clone(),
            n.body_md.clone(),
            crate::i18n::format_date_dots(n.log_date),
            n.public,
        ),
        None => (
            format!("/cabinet/projects/{}/notes/new", project.id),
            "note".to_string(),
            chrome.lang.code().to_string(),
            String::new(),
            String::new(),
            crate::i18n::format_date_dots(crate::i18n::now_unix()),
            true,
        ),
    };

    w!(
        b,
        "<section class=\"panel\"><div class=\"panel-head\"><span>{}</span><a href=\"/cabinet/projects/{}\">← {}</a></div>",
        esc(&project.title),
        project.id,
        t("к проекту", "project")
    );
    w!(
        b,
        "<p class=\"panel-note\">{}</p>",
        t(
            "Запись в личном журнале проекта. На странице проекта она стоит отдельным блоком под историей GitHub — с другими шрифтами и метками N / D / M.",
            "An entry in the project's own journal. On the project page it sits in its own block under the GitHub history, with its own type and N / D / M marks."
        )
    );
    w!(b, "<form method=\"post\" action=\"{}\" class=\"form editor\">", action);
    b.push_str("<div class=\"form-grid\">");
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{}</span><input name=\"title\" type=\"text\" required value=\"{}\" maxlength=\"180\" placeholder=\"{}\"></label>",
        t("Заголовок", "Title"),
        esc(&title),
        t("Optimization strategy", "Optimization strategy")
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><select name=\"kind\">{}</select></label>",
        t("Тип", "Type"),
        LogKind::all()
            .iter()
            .map(|k| format!(
                "<option value=\"{}\"{}>{}</option>",
                k.code(),
                if k.code() == kind { " selected" } else { "" },
                k.label(chrome.lang)
            ))
            .collect::<String>()
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><select name=\"lang\"><option value=\"ru\"{}>ru</option><option value=\"en\"{}>en</option></select></label>",
        t("Язык", "Language"),
        if lang == "ru" { " selected" } else { "" },
        if lang == "en" { " selected" } else { "" }
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><input name=\"date\" type=\"date\" value=\"{}\"></label>",
        t("Дата", "Date"),
        date
    );
    w!(
        b,
        "<label class=\"field\"><span class=\"label\">{}</span><select name=\"public\"><option value=\"1\"{}>{}</option><option value=\"0\"{}>{}</option></select></label>",
        t("Видимость", "Visibility"),
        if public { " selected" } else { "" },
        t("публичная", "public"),
        if public { "" } else { " selected" },
        t("черновик", "private")
    );
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{}</span><textarea name=\"body_md\" rows=\"16\" spellcheck=\"false\" data-editor>{}</textarea></label>",
        t("Содержание (Markdown)", "Body (Markdown)"),
        esc(&body)
    );
    b.push_str("</div><div class=\"form-actions\"><div class=\"form-actions-right\">");
    w!(
        b,
        "<button class=\"btn btn-solid\" type=\"submit\">{}</button>",
        t("Сохранить", "Save")
    );
    b.push_str("</div></div></form></section>");

    if let Some(n) = note {
        w!(
            b,
            "<section class=\"panel\"><div class=\"panel-head\"><span>{}</span><span class=\"dim\">{} {}</span></div><div class=\"prose\">{}</div></section>",
            t("Предпросмотр", "Preview"),
            LogKind::from_code(&n.kind).mark(),
            crate::i18n::format_date_dots(n.log_date),
            render_markdown(&n.body_md).html
        );
        w!(
            b,
            "<form method=\"post\" action=\"/cabinet/projects/{}/notes/{}/delete\" class=\"inline\" data-confirm=\"{}\"><button class=\"link-btn danger\" type=\"submit\">{}</button></form>",
            n.project_id,
            n.id,
            t("Удалить эту запись?", "Delete this entry?"),
            t("Удалить запись", "Delete entry")
        );
    }

    cabinet_shell(chrome, "projects", t("Заметка журнала", "Journal entry"), &b, flash)
}
