//! A minimal JSON reader, written because the project links zero crates and the
//! GitHub API is the only JSON we ever parse.
//!
//! Supports the whole JSON grammar (objects, arrays, strings with escapes and
//! `\uXXXX` surrogate pairs, numbers, booleans, null) and nothing else: no
//! serialisation, no schema, no lifetimes beyond the parsed tree.

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Field lookup on an object; `None` for any other shape.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Convenience: `obj.get(k).and_then(Json::as_str).unwrap_or("")`.
    pub fn str_at(&self, key: &str) -> String {
        self.get(key)
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string()
    }

    pub fn int_at(&self, key: &str) -> i64 {
        self.get(key).and_then(Json::as_i64).unwrap_or(0)
    }
}

pub fn parse(input: &str) -> Result<Json, String> {
    let mut p = Parser {
        src: input.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let value = p.value(0)?;
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(format!("trailing data at byte {}", p.pos));
    }
    Ok(value)
}

const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", byte as char, self.pos))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err("json nested too deeply".to_string());
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            Some(other) => Err(format!("unexpected byte {} at {}", other as char, self.pos)),
            None => Err("unexpected end of json".to_string()),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        let end = self.pos + word.len();
        if end <= self.src.len() && &self.src[self.pos..end] == word.as_bytes() {
            self.pos = end;
            Ok(value)
        } else {
            Err(format!("bad literal at byte {}", self.pos))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            pairs.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(Json::Obj(pairs)),
                _ => return Err(format!("bad object at byte {}", self.pos)),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            let value = self.value(depth + 1)?;
            items.push(value);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(Json::Arr(items)),
                _ => return Err(format!("bad array at byte {}", self.pos)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let b = self.bump().ok_or_else(|| "unterminated string".to_string())?;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.bump().ok_or_else(|| "bad escape".to_string())?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // Surrogate pair: a low half must follow.
                                if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                                    return Err("lone high surrogate".to_string());
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err("bad low surrogate".to_string());
                                }
                                let code = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                char::from_u32(code).unwrap_or('\u{fffd}')
                            } else {
                                char::from_u32(hi).unwrap_or('\u{fffd}')
                            };
                            out.push(ch);
                        }
                        other => return Err(format!("bad escape \\{}", other as char)),
                    }
                }
                // Multi-byte UTF-8 arrives as raw bytes; copy the whole sequence.
                b if b < 0x80 => out.push(b as char),
                b => {
                    let tail = utf8_tail_len(b);
                    let start = self.pos - 1;
                    let end = (start + 1 + tail).min(self.src.len());
                    let chunk = &self.src[start..end];
                    out.push_str(std::str::from_utf8(chunk).map_err(|e| e.to_string())?);
                    self.pos = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.bump().ok_or_else(|| "short \\u escape".to_string())?;
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return Err("bad hex digit".to_string()),
            };
            v = v * 16 + digit;
        }
        Ok(v)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'.' || b == b'e' || b == b'E' || b == b'+' || b == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).map_err(|e| e.to_string())?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number {text:?}"))
    }
}

fn utf8_tail_len(lead: u8) -> usize {
    match lead {
        0xC0..=0xDF => 1,
        0xE0..=0xEF => 2,
        0xF0..=0xF7 => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_objects_arrays_and_scalars() {
        let doc = parse(r#"{"a":1,"b":[true,false,null],"c":{"d":"x"}}"#).unwrap();
        assert_eq!(doc.int_at("a"), 1);
        assert_eq!(doc.get("b").unwrap().as_arr().unwrap().len(), 3);
        assert_eq!(doc.get("c").unwrap().str_at("d"), "x");
    }

    #[test]
    fn decodes_escapes_and_cyrillic() {
        let doc = parse(r#"{"s":"line\nquote\" tab\t \u00e9 \u0416"}"#).unwrap();
        assert_eq!(doc.str_at("s"), "line\nquote\" tab\t é Ж");
    }

    #[test]
    fn decodes_surrogate_pairs() {
        let doc = parse(r#""\ud83d\ude00""#).unwrap();
        assert_eq!(doc.as_str().unwrap(), "😀");
    }

    #[test]
    fn rejects_trailing_and_broken_input() {
        assert!(parse(r#"{"a":1} x"#).is_err());
        assert!(parse(r#"{"a":}"#).is_err());
        assert!(parse(r#"{"a":"unterminated"#).is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn handles_utf8_passthrough_in_strings() {
        let doc = parse("{\"t\":\"Цимцум — contraction\"}").unwrap();
        assert_eq!(doc.str_at("t"), "Цимцум — contraction");
    }
}
