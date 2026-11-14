//! Server-side HTML templates. No template engine, no runtime, no external
//! requests: CSS and JS are inlined into the document, icons are data URIs.

use crate::i18n::{self, Lang, Theme, t};
use crate::markdown::{html_escape, render_markdown};
use crate::models::{Article, BackupStats, Profile, SessionInfo, Telemetry};

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
    pub theme: Theme,
    pub path: &'a str,
    pub profile: &'a Profile,
    pub owner: bool,
}

impl Chrome<'_> {
    fn t(&self) -> &'static i18n::T {
        t(self.lang)
    }

    fn role(&self) -> &str {
        match self.lang {
            Lang::Ru => &self.profile.role_ru,
            Lang::En => &self.profile.role_en,
        }
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
        self.path == href || (key == "articles" && self.path.contains("/articles"))
    }
}

fn head(chrome: &Chrome<'_>, title: &str, description: &str, noindex: bool) -> String {
    let tr = chrome.t();
    let mut h = String::with_capacity(4096);
    h.push_str("<!doctype html>\n<html lang=\"");
    h.push_str(chrome.lang.code());
    h.push_str("\" data-theme=\"");
    h.push_str(chrome.theme.code());
    h.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n");
    w!(h, "<title>{}</title>\n", esc(title));
    w!(h, "<meta name=\"description\" content=\"{}\">\n", esc(description));
    w!(h, "<meta name=\"theme-color\" content=\"{}\">\n", if chrome.theme == Theme::Obsidian { "#0b0d0f" } else { "#efeade" });
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

fn header(chrome: &Chrome<'_>) -> String {
    let tr = chrome.t();
    let mut h = String::new();
    h.push_str("<a class=\"skip\" href=\"#main\">");
    h.push_str(tr.skip_link);
    h.push_str("</a>\n<header class=\"masthead\">\n<div class=\"masthead-inner\">\n");
    w!(
        h,
        "<a class=\"brand\" href=\"{}\"><span class=\"brand-mark\">RZ</span><span class=\"brand-text\"><strong>{}</strong><small>{}</small></span></a>\n",
        chrome.href("home"),
        esc(&chrome.profile.owner_name),
        esc(chrome.role())
    );
    h.push_str("<nav class=\"nav\" aria-label=\"Primary\">");
    for (key, label) in [
        ("articles", tr.nav_articles),
        ("about", tr.nav_about),
        ("tags", tr.all_tags),
    ] {
        let href = if key == "tags" {
            format!("{}/tags", chrome.lang.prefix())
        } else {
            chrome.href(key)
        };
        w!(
            h,
            "<a href=\"{}\"{}><span class=\"nav-idx\">{:02}</span>{}</a>",
            href,
            if chrome.is_active(key) { " aria-current=\"page\"" } else { "" },
            match key { "articles" => 1, "about" => 2, _ => 3 },
            label
        );
    }
    h.push_str("</nav>\n<div class=\"head-tools\">");
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
    w!(
        h,
        "<button class=\"tool\" type=\"button\" data-theme-toggle title=\"{}\" aria-label=\"{}\"><span data-theme-label>{}</span></button>",
        tr.theme_toggle,
        tr.theme_toggle,
        match chrome.theme {
            Theme::Obsidian => tr.theme_light,
            Theme::Paper => tr.theme_dark,
        }
    );
    if chrome.owner {
        w!(h, "<a class=\"tool tool-accent\" href=\"/cabinet\">[{}]</a>", tr.nav_cabinet);
    }
    h.push_str("<a class=\"tool\" href=\"/feed.xml\" title=\"RSS\">RSS</a>");
    h.push_str("</div>\n</div>\n");
    // Instrument ticker
    h.push_str("<div class=\"ticker\" aria-hidden=\"true\"><div class=\"ticker-inner\">");
    w!(
        h,
        "<span class=\"tick tick-live\"><i></i>{}</span>",
        esc(&chrome.profile.location)
    );
    h.push_str("<span class=\"tick\">UTC <b data-clock>--:--:--</b></span>");
    w!(
        h,
        "<span class=\"tick\">{} <b data-render-us>—</b> µs</span>",
        tr.hero_render
    );
    w!(
        h,
        "<span class=\"tick\">CACHE <b>ETAG / 304</b></span>"
    );
    w!(
        h,
        "<span class=\"tick tick-right\">RZ-NONCOMMERCIAL-1.0</span></div></div>\n",
    );
    h.push_str("</header>\n");
    h
}

fn footer(chrome: &Chrome<'_>) -> String {
    let tr = chrome.t();
    let mut h = String::new();
    h.push_str("<footer class=\"foot\">\n<div class=\"foot-inner\">");
    w!(
        h,
        "<div class=\"foot-col\"><span class=\"label\">NODE</span><b>{}</b><span>{}</span></div>",
        esc(&chrome.profile.owner_name),
        esc(&chrome.profile.location)
    );
    w!(
        h,
        "<div class=\"foot-col\"><span class=\"label\">BUILD</span><b>{}</b><span>{}</span></div>",
        env!("CARGO_PKG_VERSION"),
        tr.footer_built
    );
    w!(
        h,
        "<div class=\"foot-col\"><span class=\"label\">LICENSE</span><b>{}</b><span>{}</span></div>",
        "RZ-NONCOMMERCIAL-1.0",
        tr.footer_license
    );
    w!(
        h,
        "<div class=\"foot-col\"><span class=\"label\">{}</span><b><span data-render-us>—</span> µs</b><span>server-timing</span></div>",
        tr.hero_render
    );
    h.push_str("</div><div class=\"foot-base\">");
    w!(
        h,
        "<span>© {} {} · {}</span>",
        i18n::format_date_utc(i18n::now_unix()).get(..4).unwrap_or("2025"),
        esc(&chrome.profile.owner_name),
        tr.footer_built
    );
    w!(
        h,
        "<span class=\"foot-links\"><a href=\"{}\">{}</a> · <a href=\"/feed.xml\">RSS</a> · <a href=\"/sitemap.xml\">SITEMAP</a> · <a href=\"/login\">{}</a></span>",
        chrome.href("about"),
        tr.nav_about,
        tr.footer_admin
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

// ------------------------------------------------------------------- home

fn article_row(chrome: &Chrome<'_>, idx: usize, a: &Article) -> String {
    let tr = chrome.t();
    let mut h = String::new();
    w!(
        h,
        "<li class=\"row\"><span class=\"row-idx\">{:02}</span><a class=\"row-link\" href=\"{}/articles/{}\">",
        idx,
        a.lang_enum().prefix(),
        esc(&a.slug)
    );
    w!(
        h,
        "<span class=\"row-title\">{}</span><span class=\"row-sum\">{}</span>",
        esc(&a.title),
        esc(&a.summary)
    );
    w!(
        h,
        "<span class=\"row-meta\"><time datetime=\"{}\">{}</time><span class=\"dot\">·</span>{} {}</span>",
        a.date_iso(),
        a.date_iso(),
        a.reading_time(),
        tr.min_short
    );
    h.push_str("</a><span class=\"row-tags\">");
    for (tag, slug) in a.tag_list().iter().zip(a.tag_slugs()) {
        w!(
            h,
            "<a class=\"chip\" href=\"{}/tags/{}\">{}</a>",
            chrome.lang.prefix(),
            slug,
            esc(tag)
        );
    }
    h.push_str("</span></li>\n");
    h
}

pub fn home(
    chrome: &Chrome<'_>,
    latests: &[Article],
    tags: &[(String, String, i64)],
    article_total: i64,
) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"hero\">\n<div class=\"hero-copy\">\n");
    w!(b, "<p class=\"kicker\"><span class=\"kicker-mark\">◆</span>{}</p>\n", tr.hero_kicker);
    // Split the owner name for an oversized two-line display.
    let mut parts = chrome.profile.owner_name.splitn(2, ' ');
    let first = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    b.push_str("<h1 class=\"display\">");
    w!(b, "<span>{}</span>", esc(first));
    if !rest.is_empty() {
        w!(b, "<span class=\"display-alt\">{}</span>", esc(rest));
    }
    b.push_str("</h1>\n");
    w!(b, "<p class=\"lede\">{}</p>\n", esc(&match chrome.lang {
        Lang::Ru => &chrome.profile.manifesto_ru,
        Lang::En => &chrome.profile.manifesto_en,
    }));
    b.push_str("<div class=\"cta\">");
    w!(b, "<a class=\"btn btn-solid\" href=\"{}\">{}</a>", chrome.href("articles"), tr.hero_cta_articles);
    w!(b, "<a class=\"btn\" href=\"{}\">{}</a>", chrome.href("about"), tr.hero_cta_about);
    b.push_str("</div>\n</div>\n");
    // Instrument panel on the right: spec sheet, not decoration.
    b.push_str("<aside class=\"spec\">\n");
    w!(b, "<div class=\"spec-head\"><span>SPEC // NODE</span><span class=\"spec-live\">LIVE</span></div>\n");
    b.push_str("<dl class=\"spec-list\">");
    for (k, v) in [
        ("STACK", "RUST / C-FFI"),
        ("STORE", "SQLITE · WAL"),
        ("TRANSPORT", "HTTP/1.1 · GZIP"),
        ("BROWSER JS", "< 2 KB"),
        ("EXTERNAL REQ", "0"),
    ] {
        w!(b, "<div><dt>{}</dt><dd>{}</dd></div>", k, v);
    }
    w!(
        b,
        "<div><dt>ENTRIES</dt><dd>{:02}</dd></div>",
        article_total
    );
    w!(b, "<div><dt>LICENSE</dt><dd>NON-COMMERCIAL</dd></div>");
    b.push_str("</dl>\n");
    w!(
        b,
        "<p class=\"spec-note\">{} · {} µs budget</p>\n",
        esc(tr.hero_status),
        800
    );
    b.push_str("</aside>\n</section>\n");

    // Search + section head
    b.push_str("<section class=\"section\">\n<div class=\"section-head\">\n");
    w!(b, "<h2 class=\"h2\"><span class=\"h2-mark\">01</span>{}</h2>", tr.latests);
    w!(b, "<p class=\"section-sub\">{}</p>", tr.latests_sub);
    b.push_str("</div>\n");
    b.push_str(&search_box(chrome, None));
    if latests.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<ol class=\"rows\">");
        for (i, a) in latests.iter().enumerate() {
            b.push_str(&article_row(chrome, i + 1, a));
        }
        b.push_str("</ol>");
    }
    w!(
        b,
        "<p class=\"more\"><a class=\"btn btn-link\" href=\"{}\">{} →</a></p>",
        chrome.href("articles"),
        tr.all_articles
    );
    b.push_str("</section>\n");

    if !tags.is_empty() {
        b.push_str("<section class=\"section section-tags\">\n");
        w!(b, "<h2 class=\"h2\"><span class=\"h2-mark\">02</span>{}</h2>", tr.all_tags);
        b.push_str("<div class=\"chips\">");
        for (tag, slug, count) in tags {
            w!(
                b,
                "<a class=\"chip chip-lg\" href=\"{}/tags/{}\">{} <span class=\"chip-n\">{}</span></a>",
                chrome.lang.prefix(),
                slug,
                esc(tag),
                count
            );
        }
        b.push_str("</div></section>\n");
    }

    let title = format!("{} — {}", chrome.profile.owner_name, tr.site_role);
    layout(chrome, &title, &chrome.profile.manifesto_en, &b)
}

fn search_box(chrome: &Chrome<'_>, query: Option<&str>) -> String {
    let tr = chrome.t();
    format!(
        "<form class=\"search\" action=\"{}/articles\" method=\"get\" role=\"search\">\
         <label class=\"label\" for=\"q\">{}</label>\
         <div class=\"search-field\"><span class=\"prompt\">$</span>\
         <input id=\"q\" name=\"q\" type=\"search\" value=\"{}\" placeholder=\"{}\" autocomplete=\"off\" data-search-input>\
         <button class=\"btn btn-mini\" type=\"submit\">{}</button></div></form>",
        chrome.lang.prefix(),
        tr.search_label,
        esc(query.unwrap_or("")),
        tr.search_placeholder,
        tr.search_submit
    )
}

// --------------------------------------------------------------- articles

pub fn articles_index(
    chrome: &Chrome<'_>,
    articles: &[Article],
    tags: &[(String, String, i64)],
    active_tag: Option<&str>,
    query: Option<&str>,
) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"section section-first\">\n<div class=\"section-head\">\n");
    let heading = if query.is_some() {
        tr.search_results
    } else {
        tr.all_articles
    };
    w!(b, "<h2 class=\"h2\"><span class=\"h2-mark\">01</span>{}</h2>", heading);
    w!(
        b,
        "<p class=\"section-sub\">{} {}</p>",
        articles.len(),
        if chrome.lang == Lang::Ru { "записей в индексе" } else { "entries in the index" }
    );
    b.push_str("</div>\n");
    b.push_str(&search_box(chrome, query));

    if !tags.is_empty() {
        b.push_str("<div class=\"chips chips-filter\">");
        w!(
            b,
            "<a class=\"chip{}\" href=\"{}/articles\">{}</a>",
            if active_tag.is_none() { " chip-on" } else { "" },
            chrome.lang.prefix(),
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

    if articles.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", if query.is_some() { tr.search_empty } else { tr.empty_state });
    } else {
        b.push_str("<ol class=\"rows\">");
        for (i, a) in articles.iter().enumerate() {
            b.push_str(&article_row(chrome, i + 1, a));
        }
        b.push_str("</ol>");
        if query.is_some() {
            w!(
                b,
                "<p class=\"more\"><a class=\"btn btn-link\" href=\"{}/articles\">{} →</a></p>",
                chrome.lang.prefix(),
                tr.clear
            );
        }
    }
    b.push_str("</section>");
    let title = if let Some(q) = query {
        format!("{} «{}» — {}", tr.search_results, q, chrome.profile.owner_name)
    } else {
        format!("{} — {}", tr.nav_articles, chrome.profile.owner_name)
    };
    layout(chrome, &title, tr.latests_sub, &b)
}

pub fn article_page(
    chrome: &Chrome<'_>,
    article: &Article,
    other: Option<&Article>,
    related: &[Article],
) -> String {
    let tr = chrome.t();
    let rendered = render_markdown(&article.body_md);
    let mut b = String::new();
    b.push_str("<article class=\"article\">\n");
    w!(
        b,
        "<nav class=\"crumbs\" aria-label=\"Breadcrumb\"><a href=\"{}/articles\">← {}</a></nav>",
        chrome.lang.prefix(),
        tr.nav_articles
    );
    b.push_str("<header class=\"article-head\">\n");
    b.push_str("<div class=\"article-tags\">");
    if article.featured {
        w!(b, "<span class=\"chip chip-hot\">{}</span>", tr.featured);
    }
    for (tag, slug) in article.tag_list().iter().zip(article.tag_slugs()) {
        w!(
            b,
            "<a class=\"chip\" href=\"{}/tags/{}\">{}</a>",
            chrome.lang.prefix(),
            slug,
            esc(tag)
        );
    }
    b.push_str("</div>\n");
    w!(b, "<h1 class=\"article-title\">{}</h1>\n", esc(&article.title));
    w!(b, "<p class=\"article-sum\">{}</p>\n", esc(&article.summary));
    b.push_str("<div class=\"article-meta\">");
    w!(
        b,
        "<span><span class=\"label\">{}</span><time datetime=\"{}\">{}</time></span>",
        tr.published_on,
        article.date_iso(),
        article.date_iso()
    );
    w!(b, "<span><span class=\"label\">READ</span>{} {}</span>", article.reading_time(), tr.min_short);
    if article.updated_at > article.published_at + 60 {
        w!(
            b,
            "<span><span class=\"label\">{}</span>{}</span>",
            tr.updated_on,
            article.edit_date_iso()
        );
    }
    if let Some(other) = other {
        w!(
            b,
            "<a class=\"lang-swap\" href=\"{}/articles/{}\" hreflang=\"{}\">{}</a>",
            other.lang_enum().prefix(),
            esc(&other.slug),
            other.lang,
            tr.read_other_lang
        );
    }
    b.push_str("</div>\n</header>\n");

    if !rendered.toc.is_empty() {
        b.push_str("<nav class=\"toc\" aria-label=\"Contents\"><span class=\"label\">");
        b.push_str(tr.toc);
        b.push_str("</span><ol>");
        for item in &rendered.toc {
            w!(
                b,
                "<li class=\"toc-l{}\"><a href=\"#{}\"><span class=\"toc-n\">{}</span>{}</a></li>",
                item.level,
                esc(&item.id),
                if item.level == 3 { "·" } else { "→" },
                esc(&item.title)
            );
        }
        b.push_str("</ol></nav>\n");
    }

    b.push_str("<div class=\"prose\">");
    b.push_str(&rendered.html);
    b.push_str("</div>\n");

    b.push_str("<footer class=\"article-foot\">");
    w!(
        b,
        "<span class=\"label\">EOF</span><span>{} {} · {} {}</span>",
        article.reading_time(),
        tr.min_short,
        article.body_md.split_whitespace().count(),
        if chrome.lang == Lang::Ru { "слов" } else { "words" }
    );
    w!(b, "<a href=\"{}/articles\">{}</a>", chrome.lang.prefix(), tr.back_to_articles);
    b.push_str("</footer>\n</article>\n");

    if !related.is_empty() {
        b.push_str("<section class=\"section related\">\n");
        w!(b, "<h2 class=\"h2\"><span class=\"h2-mark\">02</span>{}</h2>", tr.related);
        b.push_str("<ol class=\"rows\">");
        for (i, a) in related.iter().enumerate() {
            b.push_str(&article_row(chrome, i + 1, a));
        }
        b.push_str("</ol></section>\n");
    }

    let desc = if article.summary.is_empty() {
        crate::http::truncate_chars(&article.body_md.replace('#', "").replace('\n', " "), 155)
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

pub fn about(chrome: &Chrome<'_>, articles: i64, tags: &[(String, String, i64)]) -> String {
    let tr = chrome.t();
    let p = chrome.profile;
    let (manifesto, now, role) = match chrome.lang {
        Lang::Ru => (&p.manifesto_ru, &p.now_ru, &p.role_ru),
        Lang::En => (&p.manifesto_en, &p.now_en, &p.role_en),
    };
    let mut b = String::new();
    b.push_str("<section class=\"section section-first\">\n<div class=\"about-grid\">\n<div>\n");
    w!(b, "<h1 class=\"display display-sm\"><span>{}</span></h1>", tr.about_title);
    w!(b, "<p class=\"lede\">{}</p>", esc(role));
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
    b.push_str("</div>\n<aside class=\"spec spec-tall\">\n");
    w!(b, "<div class=\"spec-head\"><span>DOSSIER</span><span class=\"spec-live\">VERIFIED</span></div>\n");
    b.push_str("<dl class=\"spec-list\">");
    w!(b, "<div><dt>NAME</dt><dd>{}</dd></div>", esc(&p.owner_name));
    w!(b, "<div><dt>HANDLE</dt><dd>{}</dd></div>", esc(&p.handle));
    w!(b, "<div><dt>LOC</dt><dd>{}</dd></div>", esc(&p.location));
    w!(b, "<div><dt>ENTRIES</dt><dd>{:02}</dd></div>", articles);
    w!(b, "<div><dt>TAGS</dt><dd>{:02}</dd></div>", tags.len());
    w!(b, "<div><dt>LICENSE</dt><dd>RZ-NC-1.0</dd></div>");
    b.push_str("</dl>\n<div class=\"spec-body\">");
    w!(b, "<span class=\"label\">{}</span><p>{}</p>", tr.about_stack, tr.about_stack_text);
    w!(b, "<span class=\"label\">{}</span><p>{}</p>", tr.about_license, tr.about_license_text);
    b.push_str("</div>\n</aside>\n</div>\n");
    b.push_str("<div class=\"contact\">");
    w!(b, "<h2 class=\"h2\"><span class=\"h2-mark\">02</span>{}</h2>", tr.about_contact);
    b.push_str("<ul class=\"contact-list\">");
    for (label, value, href) in [
        ("MAIL", p.email.clone(), format!("mailto:{}", p.email)),
        ("GITHUB", p.github.clone(), p.github.clone()),
        ("TELEGRAM", p.telegram.clone(), p.telegram.clone()),
        ("MATRIX", p.matrix.clone(), String::new()),
    ] {
        if value.trim().is_empty() {
            continue;
        }
        if href.starts_with("http") || href.starts_with("mailto") {
            w!(b, "<li><span class=\"label\">{}</span><a href=\"{}\" rel=\"me noopener\">{}</a></li>", label, esc(&href), esc(&value));
        } else {
            w!(b, "<li><span class=\"label\">{}</span><code>{}</code></li>", label, esc(&value));
        }
    }
    b.push_str("</ul></div>\n</section>");
    let title = format!("{} — {}", tr.about_title, p.owner_name);
    layout(chrome, &title, manifesto, &b)
}

// ------------------------------------------------------------------ login

pub fn login(chrome: &Chrome<'_>, error: Option<&str>, csrf: &str, rate_limited: bool) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"auth\">\n<div class=\"auth-card\">\n");
    w!(b, "<div class=\"spec-head\"><span>AUTH // OWNER</span><span class=\"spec-live\">{}</span></div>\n", if rate_limited { "LOCKED" } else { "READY" });
    w!(b, "<h1 class=\"auth-title\">{}</h1>", tr.login_title);
    w!(b, "<p class=\"auth-hint\">{}</p>", tr.login_hint);
    if let Some(msg) = error {
        w!(b, "<p class=\"alert\" role=\"alert\">{}</p>", esc(msg));
    }
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
        "<p class=\"auth-foot\"><a href=\"{}\">← {}</a><span>PBKDF2 · 210K · SHA-256</span></p>",
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
        ("overview", "Обзор / Overview", "00"),
        ("articles", "Записи / Entries", "01"),
        ("new", "Новая / New", "02"),
        ("profile", "Профиль / Profile", "03"),
        ("telemetry", "Метрики / Metrics", "04"),
        ("security", "Доступ / Access", "05"),
        ("backup", "Бэкап / Backup", "06"),
    ];
    for (key, label, num) in items {
        let href = match key {
            "overview" => "/cabinet".to_string(),
            "new" => "/cabinet/articles/new".to_string(),
            other => format!("/cabinet/{other}"),
        };
        w!(
            h,
            "<a class=\"side-link{}\" href=\"{}\"><span>{num}</span>{label}</a>",
            if active == key { " side-on" } else { "" },
            href
        );
    }
    h.push_str("</nav>");
    h
}

pub fn cabinet_shell(chrome: &Chrome<'_>, active: &str, title: &str, body: &str, flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut h = head(
        chrome,
        &format!("{title} — {}", tr.cabinet),
        "Owner cabinet",
        true,
    );
    h.push_str(&header(chrome));
    h.push_str("<main id=\"main\" class=\"shell cabinet\">\n");
    h.push_str("<div class=\"cab-top\">");
    w!(
        h,
        "<div><span class=\"label\">{}</span><h1 class=\"cab-title\">{}</h1></div>",
        tr.cabinet,
        esc(title)
    );
    w!(
        h,
        "<div class=\"cab-top-tools\"><span class=\"pill\">{} · {}</span>",
        tr.cabinet_owner,
        esc(&chrome.profile.handle)
    );
    h.push_str("<form method=\"post\" action=\"/logout\" class=\"inline\"><button class=\"btn btn-mini\" type=\"submit\">");
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
    b.push_str(&stat_card(tr.cabinet_articles, &format!("{:02}", stats.articles), "total"));
    b.push_str(&stat_card("PUBLISHED", &format!("{:02}", stats.published), "live"));
    b.push_str(&stat_card("DRAFTS", &format!("{:02}", stats.drafts), "wip"));
    b.push_str(&stat_card("VIEWS", &fmt_num(stats.views), "logged"));
    b.push_str("</div>\n");

    b.push_str("<section class=\"panel\">\n<div class=\"panel-head\"><span>RENDER</span><span>µs</span></div>");
    b.push_str("<div class=\"stats stats-3\">");
    b.push_str(&stat_card("AVG", &format!("{}", telemetry.avg_render_us), "microseconds"));
    b.push_str(&stat_card("P95", &format!("{}", telemetry.p95_render_us), "microseconds"));
    b.push_str(&stat_card("AVG BYTES", &fmt_num(telemetry.avg_bytes), "bytes"));
    b.push_str("</div></section>\n");

    b.push_str("<section class=\"panel\">\n<div class=\"panel-head\"><span>RECENT</span>");
    w!(b, "<a href=\"/cabinet/articles/new\">+ {}</a></div>\n", tr.cabinet_new_article);
    if recent.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<table class=\"tbl\"><thead><tr><th>ID</th><th>LANG</th><th>TITLE</th><th>STATUS</th><th>UPDATED</th><th></th></tr></thead><tbody>");
        for a in recent {
            w!(
                b,
                "<tr><td class=\"mono\">{:03}</td><td class=\"mono up\">{}</td><td><a href=\"/cabinet/articles/{}\">{}</a></td><td><span class=\"tag tag-{}\">{}</span></td><td class=\"mono\">{}</td><td class=\"mono\"><a href=\"{}/articles/{}\" target=\"_blank\" rel=\"noopener\">VIEW</a></td></tr>",
                a.id,
                a.lang,
                a.id,
                esc(&a.title),
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
    b.push_str("<section class=\"panel\">\n<div class=\"panel-head\"><span>ENTRIES</span>");
    w!(b, "<a href=\"/cabinet/articles/new\">+ {}</a></div>\n", tr.cabinet_new_article);
    if articles.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<table class=\"tbl\"><thead><tr><th>ID</th><th>LANG</th><th>TITLE</th><th>STATUS</th><th>FEAT</th><th>VIEWS</th><th>PUBLISHED</th><th>OPS</th></tr></thead><tbody>");
        for a in articles {
            w!(
                b,
                "<tr><td class=\"mono\">{:03}</td><td class=\"mono up\">{}</td><td><a href=\"/cabinet/articles/{}\">{}</a><br><span class=\"mono dim\">{}/articles/{}</span></td><td><span class=\"tag tag-{}\">{}</span></td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td>",
                a.id,
                a.lang,
                a.id,
                esc(&a.title),
                a.lang_enum().prefix(),
                esc(&a.slug),
                a.status,
                a.status,
                if a.featured { "★" } else { "·" },
                a.views,
                if a.published_at > 0 { a.date_iso() } else { "—".to_string() }
            );
            b.push_str("<td class=\"ops\">");
            let next_status = if a.is_published() { "draft" } else { "published" };
            w!(
                b,
                "<form method=\"post\" action=\"/cabinet/articles/{}/status\" class=\"inline\"><input type=\"hidden\" name=\"status\" value=\"{}\"><button class=\"btn btn-mini\" type=\"submit\">{}</button></form>",
                a.id,
                next_status,
                if a.is_published() { tr.cabinet_unpublish } else { tr.cabinet_publish }
            );
            w!(
                b,
                "<form method=\"post\" action=\"/cabinet/articles/{}/delete\" class=\"inline\" data-confirm=\"{}\"><button class=\"btn btn-mini btn-danger\" type=\"submit\">{}</button></form>",
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
    flash: Option<&str>,
    errors: &[String],
) -> String {
    let tr = chrome.t();
    let (action, id, lang, slug, title, summary, body, tag_str, status, featured, published_at) = match article {
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
        ),
    };
    let mut b = String::new();
    if !errors.is_empty() {
        b.push_str("<div class=\"alert-block\" role=\"alert\">");
        for e in errors {
            w!(b, "<p class=\"alert\">{}</p>", esc(e));
        }
        b.push_str("</div>");
    }
    w!(
        b,
        "<section class=\"panel\">\n<div class=\"panel-head\"><span>{}</span><span class=\"mono\">{}</span></div>\n",
        if id > 0 { "EDIT" } else { "NEW" },
        if id > 0 { format!("#{id:03}") } else { "#---".to_string() }
    );
    w!(b, "<form method=\"post\" action=\"{}\" class=\"form editor\">", action);
    if id > 0 {
        w!(b, "<input type=\"hidden\" name=\"id\" value=\"{id}\">");
        w!(b, "<input type=\"hidden\" name=\"published_at\" value=\"{published_at}\">");
    }
    b.push_str("<div class=\"form-grid\">");
    w!(
        b,
        "<label class=\"field field-wide\"><span class=\"label\">{}</span><input name=\"title\" type=\"text\" required value=\"{}\" maxlength=\"180\"></label>",
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
        "<label class=\"field field-wide\"><span class=\"label\">{}</span><input name=\"summary\" type=\"text\" value=\"{}\" maxlength=\"300\"></label>",
        tr.field_summary,
        esc(&summary)
    );
    w!(
        b,
        "<label class=\"field field-wide\"><span class=\"label\">{}</span><input name=\"tags\" type=\"text\" value=\"{}\" placeholder=\"{}\" maxlength=\"200\"></label>",
        tr.field_tags,
        esc(&tag_str),
        tr.field_tags_hint
    );
    if !tags.is_empty() {
        b.push_str("<div class=\"chips chips-hint\">");
        for (tag, _, _) in tags.iter().take(12) {
            w!(b, "<button class=\"chip\" type=\"button\" data-tag=\"{}\">{}</button>", esc(tag), esc(tag));
        }
        b.push_str("</div>");
    }
    w!(
        b,
        "<label class=\"field field-full\"><span class=\"label\">{} <span class=\"hint\">{}</span></span><textarea name=\"body_md\" rows=\"22\" spellcheck=\"false\" data-editor>{}</textarea><span class=\"field-foot\"><span data-count>0</span> chars · <span data-read>0</span> min</span></label>",
        tr.field_body,
        tr.field_body_hint,
        esc(&body)
    );
    b.push_str("</div><div class=\"form-actions\">");
    w!(
        b,
        "<label class=\"switch\"><input type=\"checkbox\" name=\"featured\" value=\"1\"{}><span>FEATURED</span></label>",
        if featured { " checked" } else { "" }
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
    b.push_str("</div></div></form></section>");
    if let Some(a) = article {
        b.push_str("<section class=\"panel panel-flat\">");
        w!(
            b,
            "<div class=\"panel-head\"><span>PREVIEW</span><a href=\"{}/articles/{}\" target=\"_blank\" rel=\"noopener\">OPEN ↗</a></div>\n<div class=\"prose prose-preview\">{}</div></section>",
            a.lang_enum().prefix(),
            esc(&a.slug),
            render_markdown(&a.body_md).html
        );
    }
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
    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>PROFILE</span><span>SITE MANIFEST</span></div>");
    b.push_str("<form method=\"post\" action=\"/cabinet/profile\" class=\"form\">");
    b.push_str("<div class=\"form-grid\">");
    let text_fields = [
        ("owner_name", "OWNER NAME", &p.owner_name, "field"),
        ("handle", "HANDLE", &p.handle, "field"),
        ("location", "LOCATION", &p.location, "field"),
        ("email", "EMAIL", &p.email, "field"),
        ("github", "GITHUB", &p.github, "field"),
        ("telegram", "TELEGRAM", &p.telegram, "field"),
        ("matrix", "MATRIX", &p.matrix, "field"),
        ("site_url", "SITE URL", &p.site_url, "field"),
        ("role_ru", "ROLE (RU)", &p.role_ru, "field field-wide"),
        ("role_en", "ROLE (EN)", &p.role_en, "field field-wide"),
    ];
    for (name, label, value, class) in text_fields {
        w!(
            b,
            "<label class=\"{}\"><span class=\"label\">{}</span><input name=\"{}\" type=\"text\" value=\"{}\"></label>",
            class,
            label,
            name,
            esc(value)
        );
    }
    for (name, label, value) in [
        ("manifesto_ru", "MANIFESTO (RU)", &p.manifesto_ru),
        ("manifesto_en", "MANIFESTO (EN)", &p.manifesto_en),
        ("now_ru", "NOW (RU)", &p.now_ru),
        ("now_en", "NOW (EN)", &p.now_en),
    ] {
        w!(
            b,
            "<label class=\"field field-full\"><span class=\"label\">{}</span><textarea name=\"{}\" rows=\"4\">{}</textarea></label>",
            label,
            name,
            esc(value)
        );
    }
    b.push_str("</div><div class=\"form-actions\"><span class=\"hint\">manifesto → hero copy</span>");
    w!(b, "<button class=\"btn btn-solid\" type=\"submit\">{}</button>", tr.cabinet_save);
    b.push_str("</div></form></section>");
    cabinet_shell(chrome, "profile", tr.cabinet_profile, &b, flash)
}

pub fn cabinet_telemetry(chrome: &Chrome<'_>, tele: &Telemetry, flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<div class=\"stats\">");
    b.push_str(&stat_card("TOTAL", &fmt_num(tele.total_views), "requests"));
    b.push_str(&stat_card("24H", &fmt_num(tele.views_today), "requests"));
    b.push_str(&stat_card("UNIQUE 24H", &fmt_num(tele.uniques_today), "visitors"));
    b.push_str(&stat_card("BOTS", &format!("{}%", tele.bot_share), "share"));
    b.push_str("</div>\n");

    let max = tele.daily.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>14 DAYS</span><span>requests / day</span></div>");
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
        "<p class=\"panel-foot\">avg <b>{} µs</b> · p95 <b>{} µs</b> · avg payload <b>{} B</b></p>",
        tele.avg_render_us,
        tele.p95_render_us,
        tele.avg_bytes
    );
    b.push_str("</section>\n");

    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>TOP PATHS</span><span>count</span></div>");
    if tele.top_paths.is_empty() {
        w!(b, "<p class=\"empty\">{}</p>", tr.empty_state);
    } else {
        b.push_str("<table class=\"tbl\"><tbody>");
        for (path, n) in &tele.top_paths {
            w!(b, "<tr><td class=\"mono\">{}</td><td class=\"mono right\">{}</td></tr>", esc(path), n);
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
    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>SESSIONS</span><span>SHA-256 token hashes</span></div>");
    b.push_str("<table class=\"tbl\"><thead><tr><th>STATE</th><th>CREATED</th><th>LAST</th><th>EXPIRES</th><th>AGENT</th><th>OPS</th></tr></thead><tbody>");
    for s in sessions {
        w!(
            b,
            "<tr><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mono dim\">{}</td><td>",
            if s.current { "<span class=\"tag tag-current\">CURRENT</span>" } else { "<span class=\"tag\">ACTIVE</span>" },
            i18n::format_date_utc(s.created_at),
            i18n::format_datetime_utc(s.last_seen),
            i18n::format_date_utc(s.expires_at),
            esc(&crate::http::truncate_chars(&s.ua, 48))
        );
        if !s.current {
            w!(
                b,
                "<form method=\"post\" action=\"/cabinet/security/revoke\" class=\"inline\"><input type=\"hidden\" name=\"token\" value=\"{}\"><button class=\"btn btn-mini\" type=\"submit\">{}</button></form>",
                esc(&s.token_hash),
                tr.cabinet_revoke
            );
        } else {
            b.push_str("—");
        }
        b.push_str("</td></tr>");
    }
    b.push_str("</tbody></table>");
    w!(
        b,
        "<form method=\"post\" action=\"/cabinet/security/revoke-all\" class=\"panel-actions\"><button class=\"btn\" type=\"submit\">{}</button><span class=\"hint\">keeps this session</span></form>",
        tr.cabinet_revoke_others
    );
    b.push_str("</section>\n");

    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>PASSWORD</span><span>PBKDF2-HMAC-SHA256 · 210 000</span></div>");
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
    b.push_str("</div><div class=\"form-actions\"><span class=\"hint\">min 12 chars · all other sessions revoked</span>");
    w!(b, "<button class=\"btn btn-solid\" type=\"submit\">{}</button>", tr.cabinet_change_password);
    b.push_str("</div></form></section>");
    cabinet_shell(chrome, "security", tr.cabinet_security, &b, flash)
}

pub fn cabinet_backup(chrome: &Chrome<'_>, stats: &BackupStats, flash: Option<&str>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<div class=\"stats\">");
    b.push_str(&stat_card("ARTICLES", &format!("{}", stats.articles), "exported"));
    b.push_str(&stat_card("PUBLISHED", &format!("{}", stats.published), "live"));
    b.push_str(&stat_card("DRAFTS", &format!("{}", stats.drafts), "included"));
    b.push_str(&stat_card("VIEWS", &fmt_num(stats.views), "logged"));
    b.push_str("</div>\n");
    b.push_str("<section class=\"panel\"><div class=\"panel-head\"><span>EXPORT</span><span>application/json</span></div>");
    w!(b, "<p class=\"panel-note\">{}</p>", tr.cabinet_export_hint);
    w!(
        b,
        "<p><a class=\"btn btn-solid\" href=\"/cabinet/backup.json\" download=\"rudolfzinovev-backup.json\">{}</a></p>",
        tr.cabinet_download
    );
    b.push_str("<div class=\"code-frame\"><div class=\"code-bar\"><span class=\"code-lang\">[SHELL]</span><span class=\"code-meta\">BACKUP</span></div><pre><code>curl -sS -H \"Cookie: rz_session=$RZ_TOKEN\" \\\n  https://rudolfzinovev.me/cabinet/backup.json \\\n  &gt; backup-$(date +%F).json\nsqlite3 data/site.sqlite \".backup 'data/backup.sqlite'\"</code></pre></div>");
    b.push_str("</section>");
    cabinet_shell(chrome, "backup", tr.cabinet_backup, &b, flash)
}

// ----------------------------------------------------------------- errors

pub fn not_found(chrome: &Chrome<'_>) -> String {
    let tr = chrome.t();
    let mut b = String::new();
    b.push_str("<section class=\"err\">\n<div class=\"err-code\">404</div><div>");
    w!(b, "<h1 class=\"err-title\">{}</h1>", tr.not_found_title);
    w!(b, "<p class=\"err-text\">{}</p>", tr.not_found_text);
    w!(b, "<a class=\"btn btn-solid\" href=\"{}\">{}</a>", chrome.href("home"), tr.not_found_cta);
    b.push_str("</div></section>");
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
    b.push_str("<section class=\"err\">\n<div class=\"err-code\">500</div><div>");
    w!(b, "<h1 class=\"err-title\">{}</h1>", tr.server_error_title);
    w!(b, "<p class=\"err-text\">{}</p>", tr.server_error_text);
    w!(b, "<a class=\"btn btn-solid\" href=\"{}\">{}</a>", chrome.href("home"), tr.not_found_cta);
    b.push_str("</div></section>");
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
        w!(x, "<link>{base}/articles/{}</link>\n", esc(&a.slug));
        w!(x, "<guid isPermaLink=\"true\">{base}/articles/{}</guid>\n", esc(&a.slug));
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
    add(format!("{base}/articles"), None, "0.9");
    add(format!("{base}/en/articles"), None, "0.9");
    add(format!("{base}/about"), None, "0.6");
    add(format!("{base}/en/about"), None, "0.6");
    for a in articles {
        add(
            format!("{base}{}/articles/{}", a.lang_enum().prefix(), a.slug),
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
            theme: Theme::Obsidian,
            path: "/",
            profile: p,
            owner: false,
        }
    }

    #[test]
    fn home_renders_without_leaking_placeholders() {
        let html = home(&chrome(Lang::Ru), &[], &[], 8);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Rudolf"));
        assert!(html.contains("data-render-us"));
        assert!(!html.contains("@@RENDER_US@@"));
        assert!(html.contains("data-theme=\"obsidian\""));
    }

    #[test]
    fn article_page_escapes_and_switches_language() {
        let a = Article {
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
        let html = article_page(&chrome(Lang::Ru), &a, None, &[]);
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
