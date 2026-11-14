//! Fast, XSS-safe Markdown-to-HTML compiler written in pure Rust.
//! Supports headings with automatic anchor IDs & TOC extraction, fenced code blocks
//! with line numbers & language badges, tables, blockquotes, lists, callouts,
//! inline code, bold, italic, strikethrough, and sanitized links.

#[derive(Debug, Clone)]
pub struct TocItem {
    pub level: u8,
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct RenderedMarkdown {
    pub html: String,
    pub toc: Vec<TocItem>,
}

pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn slugify_anchor(s: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                slug.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "sec".to_string()
    } else {
        slug
    }
}

/// Words-per-minute estimate that accounts for Russian's longer average word.
pub fn estimate_reading_minutes(md: &str) -> i64 {
    let words = md.split_whitespace().count() as i64;
    ((words + 149) / 150).max(1)
}

pub fn render_markdown(input: &str) -> RenderedMarkdown {
    let mut html = String::with_capacity(input.len() * 3 / 2 + 128);
    let mut toc = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;
    let mut sec_counter = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end();

        // Blank line
        if trimmed.trim().is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block ```lang
        if let Some(fence_rest) = trimmed.trim_start().strip_prefix("```") {
            let lang = fence_rest.trim();
            let lang_label = if lang.is_empty() { "TEXT" } else { lang };
            i += 1;
            let mut code_lines = Vec::new();
            while i < lines.len() {
                if lines[i].trim_start().starts_with("```") {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            html.push_str("<figure class=\"code-frame\">");
            html.push_str("<figcaption class=\"code-bar\"><span class=\"code-lang\">[");
            html.push_str(&html_escape(&lang_label.to_uppercase()));
            html.push_str("]</span><span class=\"code-meta\">");
            html.push_str(&format!("{} LN", code_lines.len()));
            html.push_str("</span><button type=\"button\" class=\"code-copy\" data-copy-btn hidden>COPY</button></figcaption>");
            html.push_str("<pre><code>");
            for (idx, cl) in code_lines.iter().enumerate() {
                if idx > 0 {
                    html.push('\n');
                }
                html.push_str(&html_escape(cl));
            }
            html.push_str("</code></pre></figure>\n");
            continue;
        }

        // Horizontal rule
        let t = trimmed.trim();
        if t == "---" || t == "***" || t == "___" {
            html.push_str("<hr class=\"rule-divider\" />\n");
            i += 1;
            continue;
        }

        // Headings # .. ####
        if t.starts_with('#') {
            let hashes = t.chars().take_while(|&c| c == '#').count();
            if (1..=4).contains(&hashes)
                && t[hashes..].starts_with(' ')
            {
                let raw_title = t[hashes..].trim();
                sec_counter += 1;
                let base_slug = slugify_anchor(raw_title);
                let id = format!("s{sec_counter}-{base_slug}");
                let level = hashes as u8;
                if (2..=3).contains(&level) {
                    toc.push(TocItem {
                        level,
                        id: id.clone(),
                        title: strip_inline_markers(raw_title),
                    });
                }
                let tag = match level {
                    1 => "h2", // Keep H1 reserved for article title
                    2 => "h2",
                    3 => "h3",
                    _ => "h4",
                };
                html.push_str(&format!(
                    "<{tag} id=\"{id}\" class=\"md-h md-h{level}\"><a class=\"h-anchor\" href=\"#{id}\" aria-hidden=\"true\">§</a>{}</{tag}>\n",
                    render_inline(raw_title)
                ));
                i += 1;
                continue;
            }
        }

        // Blockquote >
        if t.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() {
                let ql = lines[i].trim_start();
                if let Some(rest) = ql.strip_prefix('>') {
                    quote_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
                    i += 1;
                } else {
                    break;
                }
            }
            let joined = quote_lines.join("\n");
            let inner = render_markdown(&joined);
            html.push_str("<blockquote class=\"md-quote\">\n");
            html.push_str(&inner.html);
            html.push_str("</blockquote>\n");
            continue;
        }

        // Pipe Table
        if t.starts_with('|')
            && i + 1 < lines.len()
            && is_table_separator(lines[i + 1].trim())
        {
            let headers = split_table_row(t);
            i += 2; // skip header + separator
            html.push_str("<div class=\"table-scroll\"><table class=\"md-table\"><thead><tr>");
            for h in &headers {
                html.push_str("<th>");
                html.push_str(&render_inline(h));
                html.push_str("</th>");
            }
            html.push_str("</tr></thead><tbody>");
            while i < lines.len() && lines[i].trim().starts_with('|') {
                let cells = split_table_row(lines[i].trim());
                html.push_str("<tr>");
                for col_idx in 0..headers.len() {
                    let cell = cells.get(col_idx).copied().unwrap_or("");
                    html.push_str("<td>");
                    html.push_str(&render_inline(cell));
                    html.push_str("</td>");
                }
                html.push_str("</tr>");
                i += 1;
            }
            html.push_str("</tbody></table></div>\n");
            continue;
        }

        // Unordered list (- or *)
        if is_ul_item(t) {
            html.push_str("<ul class=\"md-list\">\n");
            while i < lines.len() {
                let lt = lines[i].trim();
                if let Some(item_text) = strip_ul_marker(lt) {
                    html.push_str("<li>");
                    html.push_str(&render_inline(item_text));
                    html.push_str("</li>\n");
                    i += 1;
                } else {
                    break;
                }
            }
            html.push_str("</ul>\n");
            continue;
        }

        // Ordered list (1. 2. ...)
        if strip_ol_marker(t).is_some() {
            html.push_str("<ol class=\"md-list md-ol\">\n");
            while i < lines.len() {
                let lt = lines[i].trim();
                if let Some(item_text) = strip_ol_marker(lt) {
                    html.push_str("<li>");
                    html.push_str(&render_inline(item_text));
                    html.push_str("</li>\n");
                    i += 1;
                } else {
                    break;
                }
            }
            html.push_str("</ol>\n");
            continue;
        }

        // Paragraph: collect contiguous normal lines
        let mut para = Vec::new();
        while i < lines.len() {
            let pl = lines[i].trim();
            if pl.is_empty()
                || pl.starts_with("```")
                || pl.starts_with('#')
                || pl.starts_with('>')
                || pl == "---"
                || pl == "***"
                || is_ul_item(pl)
                || strip_ol_marker(pl).is_some()
                || (pl.starts_with('|')
                    && i + 1 < lines.len()
                    && is_table_separator(lines[i + 1].trim()))
            {
                break;
            }
            para.push(pl);
            i += 1;
        }
        if !para.is_empty() {
            html.push_str("<p>");
            for (idx, pl) in para.iter().enumerate() {
                if idx > 0 {
                    html.push(' ');
                }
                html.push_str(&render_inline(pl));
            }
            html.push_str("</p>\n");
        } else {
            // Safety fallback to guarantee progress
            html.push_str("<p>");
            html.push_str(&render_inline(t));
            html.push_str("</p>\n");
            i += 1;
        }
    }

    RenderedMarkdown { html, toc }
}

fn is_ul_item(s: &str) -> bool {
    strip_ul_marker(s).is_some()
}

fn strip_ul_marker(s: &str) -> Option<&str> {
    s.strip_prefix("- ").or_else(|| s.strip_prefix("* "))
}

fn strip_ol_marker(s: &str) -> Option<&str> {
    let dot = s.find(". ")?;
    if dot == 0 || dot > 6 {
        return None;
    }
    if s[..dot].chars().all(|c| c.is_ascii_digit()) {
        Some(&s[dot + 2..])
    } else {
        None
    }
}

fn is_table_separator(s: &str) -> bool {
    if !s.starts_with('|') {
        return false;
    }
    let cells = split_table_row(s);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')
        })
}

fn split_table_row(s: &str) -> Vec<&str> {
    let inner = s.strip_prefix('|').unwrap_or(s);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(|c| c.trim()).collect()
}

fn strip_inline_markers(s: &str) -> String {
    s.replace('`', "").replace("**", "").replace('*', "")
}

fn sanitize_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with('/')
        || lower.starts_with('#')
    {
        Some(html_escape(trimmed))
    } else {
        None
    }
}

pub fn render_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Inline code `...`
        if chars[i] == '`' {
            if let Some(end) = (i + 1..len).find(|&k| chars[k] == '`') {
                let code: String = chars[i + 1..end].iter().collect();
                out.push_str("<code>");
                out.push_str(&html_escape(&code));
                out.push_str("</code>");
                i = end + 1;
                continue;
            }
        }

        // Link [label](url)
        if chars[i] == '[' {
            if let Some(close_bracket) = (i + 1..len).find(|&k| chars[k] == ']')
                && close_bracket + 1 < len
                && chars[close_bracket + 1] == '('
                && let Some(close_paren) = (close_bracket + 2..len).find(|&k| chars[k] == ')')
            {
                let label: String = chars[i + 1..close_bracket].iter().collect();
                let url: String = chars[close_bracket + 2..close_paren].iter().collect();
                if let Some(safe_url) = sanitize_url(&url) {
                    let external = safe_url.starts_with("http://") || safe_url.starts_with("https://");
                    if external {
                        out.push_str(&format!(
                            "<a href=\"{safe_url}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                            render_inline(&label)
                        ));
                    } else {
                        out.push_str(&format!(
                            "<a href=\"{safe_url}\">{}</a>",
                            render_inline(&label)
                        ));
                    }
                    i = close_paren + 1;
                    continue;
                }
            }
        }

        // Bold **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = (i + 2..len.saturating_sub(1))
                .find(|&k| chars[k] == '*' && chars[k + 1] == '*')
            {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<strong>");
                out.push_str(&render_inline(&inner));
                out.push_str("</strong>");
                i = end + 2;
                continue;
            }
        }

        // Strikethrough ~~...~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(end) = (i + 2..len.saturating_sub(1))
                .find(|&k| chars[k] == '~' && chars[k + 1] == '~')
            {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<del>");
                out.push_str(&render_inline(&inner));
                out.push_str("</del>");
                i = end + 2;
                continue;
            }
        }

        // Italic *...*
        if chars[i] == '*' {
            if let Some(end) = (i + 1..len).find(|&k| chars[k] == '*')
                && end > i + 1
            {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str("<em>");
                out.push_str(&render_inline(&inner));
                out.push_str("</em>");
                i = end + 1;
                continue;
            }
        }

        // Normal escaped character
        match chars[i] {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings_code_and_escapes_xss() {
        let md = "## Architecture\n\nHello **world** `<script>alert(1)</script>` and <img src=x onerror=alert(1)>.\n\n```rust\nfn main() {}\n```";
        let res = render_markdown(md);
        assert_eq!(res.toc.len(), 1);
        assert_eq!(res.toc[0].title, "Architecture");
        assert!(res.html.contains("<strong>world</strong>"));
        assert!(!res.html.contains("<img src=x"));
        assert!(res.html.contains("&lt;img src=x"));
        assert!(res.html.contains("[RUST]"));
    }
}
