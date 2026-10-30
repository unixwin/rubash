//! A hand-rolled minimal JSON subset for the DAP frontend
//! (debugger design section 13; PLAN section 3: the repo adds
//! no serde dependency, so the adapter's JSON layer is this
//! subset, written by hand).
//!
//! The subset is the whole of JSON as DAP uses it: objects,
//! arrays, strings (the full escape set, including `\uXXXX`
//! with surrogate pairs), numbers, booleans, and null. Objects
//! keep insertion order (a `Vec` of pairs): DAP messages are
//! small, and a transcript reads in the order the adapter
//! built it. Numbers parse as integers when the literal is
//! integral — DAP's own fields (seq, line, ids) are integers —
//! and as floats otherwise; the adapter only ever *emits*
//! integers, strings, booleans, arrays, and objects.

use std::fmt;

/// One JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`
    Null,
    /// `true` / `false`
    Bool(bool),
    /// An integral number.
    Int(i64),
    /// A non-integral number.
    Float(f64),
    /// A string.
    Str(String),
    /// An array of values.
    Array(Vec<Json>),
    /// An object, keys in insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Build an object from key/value pairs.
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Object(
            pairs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// One field of an object value; `None` for non-objects and
    /// absent keys.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string payload, when this value is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    /// The integer payload, when this value is a number
    /// (a float with an integral value also reads).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(value) => Some(*value),
            Json::Float(value) if value.fract() == 0.0 && value.abs() < 9.0e15 => {
                Some(*value as i64)
            }
            _ => None,
        }
    }

    /// The boolean payload, when this value is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The elements, when this value is an array.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Parse a complete JSON document. Trailing input after the
    /// value is an error: a framed DAP body is exactly one value.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        parser.skip_ws();
        let value = parser.value()?;
        parser.skip_ws();
        if parser.pos != parser.bytes.len() {
            return Err(format!("trailing input at byte {}", parser.pos));
        }
        Ok(value)
    }

    /// Serialize compactly (no insignificant whitespace).
    pub fn write_to(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Int(value) => out.push_str(&value.to_string()),
            Json::Float(value) => {
                if value.is_finite() {
                    out.push_str(&value.to_string());
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(text) => write_json_string(text, out),
            Json::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.write_to(out);
                }
                out.push(']');
            }
            Json::Object(pairs) => {
                out.push('{');
                for (index, (key, value)) in pairs.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_json_string(key, out);
                    out.push(':');
                    value.write_to(out);
                }
                out.push('}');
            }
        }
    }

    /// Serialize to a fresh string.
    pub fn stringify(&self) -> String {
        let mut out = String::new();
        self.write_to(&mut out);
        out
    }
}

impl From<&str> for Json {
    fn from(text: &str) -> Json {
        Json::Str(text.to_string())
    }
}

impl From<String> for Json {
    fn from(text: String) -> Json {
        Json::Str(text)
    }
}

impl From<bool> for Json {
    fn from(value: bool) -> Json {
        Json::Bool(value)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Json {
        Json::Int(value)
    }
}

impl From<u32> for Json {
    fn from(value: u32) -> Json {
        Json::Int(i64::from(value))
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Json {
        Json::Int(value as i64)
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.stringify())
    }
}

/// Append `text` as a quoted, escaped JSON string.
fn write_json_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// A recursive-descent parser over the input bytes.
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while let Some(byte) = self.bytes.get(self.pos) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.keyword("true", Json::Bool(true)),
            Some(b'f') => self.keyword("false", Json::Bool(false)),
            Some(b'n') => self.keyword("null", Json::Null),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            _ => Err(format!("unexpected input at byte {}", self.pos)),
        }
    }

    fn keyword(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(pairs));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("object key must be a string at byte {}", self.pos));
            }
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("expected ':' at byte {}", self.pos));
            }
            self.pos += 1;
            self.skip_ws();
            let value = self.value()?;
            pairs.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(pairs));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err("unterminated string".to_string());
            };
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(esc) = self.peek() else {
                        return Err("unterminated escape".to_string());
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            if (0xD800..0xDC00).contains(&first) {
                                // A high surrogate must pair with a
                                // low one; anything else is replaced,
                                // as serde_json replaces it.
                                if self.peek() == Some(b'\\') {
                                    let save = self.pos;
                                    self.pos += 1;
                                    if self.peek() == Some(b'u') {
                                        self.pos += 1;
                                        let second = self.hex4()?;
                                        if (0xDC00..0xE000).contains(&second) {
                                            let combined = 0x10000u32
                                                + (((first as u32 - 0xD800) << 10)
                                                    | (second as u32 - 0xDC00));
                                            out.push(
                                                char::from_u32(combined).unwrap_or('\u{FFFD}'),
                                            );
                                            continue;
                                        }
                                    }
                                    self.pos = save;
                                }
                                out.push('\u{FFFD}');
                            } else {
                                out.push(char::from_u32(first as u32).unwrap_or('\u{FFFD}'));
                            }
                        }
                        _ => return Err(format!("invalid escape at byte {}", self.pos)),
                    }
                }
                _ => {
                    // A raw byte: strings arrive as UTF-8 (the
                    // framing layer decodes the body as UTF-8), so
                    // copy the whole scalar starting here.
                    let start = self.pos - 1;
                    let mut end = self.pos;
                    while end < self.bytes.len() && (self.bytes[end] & 0xC0) == 0x80 {
                        end += 1;
                    }
                    match std::str::from_utf8(&self.bytes[start..end]) {
                        Ok(text) => out.push_str(text),
                        Err(_) => return Err(format!("invalid UTF-8 in string at byte {start}")),
                    }
                    self.pos = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let mut value: u16 = 0;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err("short \\u escape".to_string());
            };
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a') + 10,
                b'A'..=b'F' => u16::from(byte - b'A') + 10,
                _ => return Err(format!("invalid hex digit at byte {}", self.pos)),
            };
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "invalid number".to_string())?;
        if let Ok(int) = text.parse::<i64>() {
            return Ok(Json::Int(int));
        }
        text.parse::<f64>()
            .map(Json::Float)
            .map_err(|_| format!("invalid number '{text}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn roundtrips_the_dap_shapes() {
        let text = r#"{"seq":3,"type":"request","command":"evaluate","arguments":{"expression":"${x:=1}","context":"hover","frameId":12},"flag":true,"none":null,"list":[1,2.5,"a\nb"]}"#;
        let value = Json::parse(text).expect("parses");
        assert_eq!(
            value.get("command").and_then(Json::as_str),
            Some("evaluate")
        );
        assert_eq!(value.get("seq").and_then(Json::as_i64), Some(3));
        assert_eq!(
            value
                .get("arguments")
                .and_then(|args| args.get("expression"))
                .and_then(Json::as_str),
            Some("${x:=1}")
        );
        assert_eq!(value.get("flag").and_then(Json::as_bool), Some(true));
        assert_eq!(value.get("none"), Some(&Json::Null));
        // Serialization is byte-stable for the shapes we parse:
        // integers stay integers, escapes normalize.
        let out = value.stringify();
        let reparsed = Json::parse(&out).expect("reparses");
        assert_eq!(reparsed, value);
    }

    #[test]
    fn strings_escape_and_unescape() {
        let value = Json::Str("a\"b\\c\nd\u{01}eé".to_string());
        let text = value.stringify();
        assert_eq!(text, "\"a\\\"b\\\\c\\nd\\u0001eé\"");
        assert_eq!(Json::parse(&text), Ok(value));
    }

    #[test]
    fn unicode_escapes_decode_including_surrogate_pairs() {
        assert_eq!(
            Json::parse(r#""\u0041\ud83d\ude00""#),
            Ok(Json::Str("A\u{1F600}".to_string()))
        );
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        for text in ["", "{", "[1,", "{\"a\":}", "tru", "\"open", "01x"] {
            assert!(Json::parse(text).is_err(), "input {text:?}");
        }
    }
}
