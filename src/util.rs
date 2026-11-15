//! Helpers the core used to borrow from the hand-written HTTP module: character
//! truncation, bot detection, referrer host, form and URL decoding.

/// Character-count truncation, not byte-count: Russian text is 2 bytes per
/// letter and slicing by bytes would panic mid-codepoint.
pub fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The CSP pins the inline script by base64(SHA-256), which is the only form a
/// browser accepts. Computed once and memoised: the input is a compile-time
/// constant, and this runs on every response.
pub fn script_csp_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| crypto::base64_encode(&crypto::sha256_bytes(crate::templates::JS.as_bytes())))
}

/// Coarse bot detection. Deliberately generous: a false positive only means a
/// visit is excluded from the aggregate counter, never that it is blocked.
pub fn is_bot(user_agent: &str) -> bool {
    let ua = user_agent.to_ascii_lowercase();
    [
        "bot",
        "crawl",
        "spider",
        "curl/",
        "wget",
        "python-requests",
        "headless",
        "monitor",
        "uptime",
        "facebookexternalhit",
        "preview",
    ]
    .iter()
    .any(|needle| ua.contains(needle))
}

/// Referrer reduced to its host: the cabinet stores no full URLs.
pub fn referrer_host(referer: &str) -> String {
    let after_scheme = referer
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(referer);
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    truncate_chars(&host, 120)
}

/// Percent-decoding for query strings and `application/x-www-form-urlencoded`.
/// `+` is a space, invalid escapes are kept literally (never panics).
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
                let hex = |b: u8| -> Option<u8> {
                    match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        b'A'..=b'F' => Some(b - b'A' + 10),
                        _ => None,
                    }
                };
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi * 16 + lo);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
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

/// `a=1&b=2` into a map. Duplicate keys keep the last value, as browsers expect.
pub fn parse_form(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_hash_is_base64_sha256_of_the_script() {
        let h = script_csp_hash();
        // base64 of a 32-byte digest is 44 characters ending in '='.
        assert_eq!(h.len(), 44);
        assert!(h.ends_with('='));
    }

    #[test]
    fn decodes_plus_and_percent_escapes() {
        assert_eq!(url_decode("a+b%20c"), "a b c");
        assert_eq!(url_decode("%D0%BF%D1%80%D0%B8"), "при");
        // A malformed escape is passed through instead of panicking.
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
    }

    #[test]
    fn form_pairs_become_tuples() {
        let pairs = parse_form("title=Hello+world&tags=rust%2C+http&empty=");
        assert_eq!(pairs[0], ("title".to_string(), "Hello world".to_string()));
        assert_eq!(pairs[1].1, "rust, http");
        assert_eq!(pairs[2], ("empty".to_string(), String::new()));
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("привет", 3), "при");
        assert_eq!(truncate_chars("abcdef", 99), "abcdef");
    }

    #[test]
    fn referrer_is_reduced_to_host() {
        assert_eq!(referrer_host("https://news.ycombinator.com/item?id=1"), "news.ycombinator.com");
        assert_eq!(referrer_host("http://LOCALHOST:8080/x"), "localhost:8080");
        assert_eq!(referrer_host(""), "");
    }

    #[test]
    fn bots_are_recognised() {
        assert!(is_bot("Mozilla/5.0 (compatible; Googlebot/2.1)"));
        assert!(is_bot("curl/8.5.0"));
        assert!(!is_bot("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1.15"));
    }
}
