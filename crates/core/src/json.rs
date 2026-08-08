//! A minimal, dependency-free JSON parser (M8).
//!
//! strictix's blessed dependencies are serde + toml, and only inside the
//! CLI crate. The core crate therefore parses options.json (nixpkgs
//! schema data) by hand. This module implements the full JSON grammar:
//! objects, arrays, strings with escapes (including \uXXXX surrogate
//! pairs), numbers, true/false/null, and arbitrary whitespace.
//! Trailing non-whitespace content is rejected.
//!
//! The parser never panics: every malformed input yields a
//! [JsonError] carrying a message and the byte offset where the
//! problem was detected.

/// A parsed JSON value.
///
/// Objects keep their entries in insertion order as a Vec of key/value
/// pairs; duplicate keys are kept (both entries survive), per the
/// contract.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    /// The JSON literal null.
    Null,
    /// The JSON literals true and false.
    Bool(bool),
    /// A JSON number, stored as an f64.
    Number(f64),
    /// A JSON string, decoded: escapes and \uXXXX sequences are
    /// resolved to their UTF-8 text.
    String(String),
    /// A JSON array.
    Array(Vec<JsonValue>),
    /// A JSON object; entries in source order.
    Object(Vec<(String, JsonValue)>),
}

/// A JSON parse failure: a human-readable message plus the byte offset
/// in the input where the error was detected.
#[derive(Debug, PartialEq, Eq)]
pub struct JsonError {
    /// What went wrong.
    pub message: String,
    /// Byte offset into the input where the error was detected.
    pub offset: usize,
}

impl JsonValue {
    /// Parse a complete JSON document from input.
    ///
    /// Leading and trailing whitespace (space, tab, LF, CR) is skipped;
    /// any non-whitespace content after the first value is an error.
    pub fn parse(input: &str) -> Result<JsonValue, JsonError> {
        let mut parser = Parser { input, pos: 0 };
        parser.skip_ws();
        let value = parser.value()?;
        parser.skip_ws();
        if parser.peek_byte().is_some() {
            return Err(parser.error("trailing content"));
        }
        Ok(value)
    }

    /// Object lookup: the first entry whose key equals key, or None
    /// when this is not an object or the key is absent.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// This value as a string, if it is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// This value as a bool, if it is one.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// This value as an array slice, if it is one.
    #[must_use]
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }

    /// This value as an object's entries in insertion order, if it is
    /// one.
    #[must_use]
    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(entries) => Some(entries),
            _ => None,
        }
    }
}

/// Byte-cursor parser over a &str. pos is always on a UTF-8
/// character boundary, so byte offsets reported in errors match the
/// input.
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl Parser<'_> {
    /// The byte at pos, if any.
    fn peek_byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    /// An error reported at the current position.
    fn error(&self, message: &str) -> JsonError {
        JsonError {
            message: message.to_owned(),
            offset: self.pos,
        }
    }

    /// Skip spaces, tabs, line feeds and carriage returns.
    fn skip_ws(&mut self) {
        while matches!(self.peek_byte(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// Parse the value starting at pos (any kind).
    fn value(&mut self) -> Result<JsonValue, JsonError> {
        match self.peek_byte() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b't') => self.literal(b"true", JsonValue::Bool(true)),
            Some(b'f') => self.literal(b"false", JsonValue::Bool(false)),
            Some(b'n') => self.literal(b"null", JsonValue::Null),
            Some(b'-') | Some(b'.') | Some(b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    /// Match a fixed keyword literal (true, false, null).
    fn literal(&mut self, word: &[u8], value: JsonValue) -> Result<JsonValue, JsonError> {
        if self.input.as_bytes()[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected character"))
        }
    }

    /// Parse a number: -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?.
    fn number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.pos;
        if self.peek_byte() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek_byte() {
            // Leading zero: must not be followed by another digit ("01").
            Some(b'0') => {
                self.pos += 1;
                if matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                    return Err(self.error("invalid number"));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            // Bare '.' or '-' followed by non-digit.
            _ => return Err(self.error("invalid number")),
        }
        if self.peek_byte() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number"));
            }
            while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number"));
            }
            while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = &self.input[start..self.pos];
        // Reject NaN/Infinity (and overflow-to-infinity like "1e999"):
        // a JSON number must be finite.
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(JsonValue::Number(n)),
            _ => Err(self.error("invalid number")),
        }
    }

    /// Parse a double-quoted string, returning its decoded content.
    fn string(&mut self) -> Result<String, JsonError> {
        debug_assert_eq!(self.peek_byte(), Some(b'"'));
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek_byte() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                // Unescaped control characters are invalid JSON.
                b if b < 0x20 => {
                    return Err(self.error("unexpected control character in string"));
                }
                // Raw content: copy a full (possibly multi-byte) char.
                _ => {
                    let ch = self.input[self.pos..]
                        .chars()
                        .next()
                        .expect("pos is on a char boundary");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    /// Decode the escape sequence whose backslash sits at pos.
    fn escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        match self.peek_byte() {
            Some(b'"') => {
                out.push('"');
                self.pos += 1;
            }
            Some(b'\\') => {
                out.push('\\');
                self.pos += 1;
            }
            Some(b'/') => {
                out.push('/');
                self.pos += 1;
            }
            Some(b'b') => {
                out.push('\u{0008}');
                self.pos += 1;
            }
            Some(b'f') => {
                out.push('\u{000C}');
                self.pos += 1;
            }
            Some(b'n') => {
                out.push('\n');
                self.pos += 1;
            }
            Some(b'r') => {
                out.push('\r');
                self.pos += 1;
            }
            Some(b't') => {
                out.push('\t');
                self.pos += 1;
            }
            Some(b'u') => {
                self.pos += 1;
                self.unicode_escape(out)?;
            }
            Some(_) => return Err(self.error("invalid escape")),
            None => return Err(self.error("unterminated string")),
        }
        Ok(())
    }

    /// Decode a \uXXXX escape (the u has already been consumed),
    /// combining surrogate pairs.
    fn unicode_escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let code_point = self.hex4()?;
        match code_point {
            // High surrogate: must be immediately followed by \u + low.
            0xD800..=0xDBFF => {
                if self.peek_byte() == Some(b'\\') {
                    self.pos += 1;
                    if self.peek_byte() == Some(b'u') {
                        self.pos += 1;
                        let low = self.hex4()?;
                        if (0xDC00..=0xDFFF).contains(&low) {
                            let combined =
                                0x1_0000 + ((code_point - 0xD800) << 10) + (low - 0xDC00);
                            match char::from_u32(combined) {
                                Some(ch) => {
                                    out.push(ch);
                                    return Ok(());
                                }
                                // Unreachable: a valid pair is always a scalar.
                                None => return Err(self.error("invalid escape")),
                            }
                        }
                    }
                }
                Err(self.error("unpaired surrogate"))
            }
            // Lone low surrogate.
            0xDC00..=0xDFFF => Err(self.error("unpaired surrogate")),
            _ => match char::from_u32(code_point) {
                Some(ch) => {
                    out.push(ch);
                    Ok(())
                }
                // Unreachable: 4 hex digits are at most 0xFFFF and
                // surrogates are handled above.
                None => Err(self.error("invalid escape")),
            },
        }
    }

    /// Read four hex digits as a u32 (e.g. the body of a \uXXXX).
    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(byte) = self.peek_byte() else {
                return Err(self.error("unterminated string"));
            };
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a' + 10),
                b'A'..=b'F' => u32::from(byte - b'A' + 10),
                _ => return Err(self.error("invalid escape")),
            };
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    /// Parse an object; the opening brace sits at pos.
    fn object(&mut self) -> Result<JsonValue, JsonError> {
        debug_assert_eq!(self.peek_byte(), Some(b'{'));
        self.pos += 1;
        self.skip_ws();
        let mut entries = Vec::new();
        if self.peek_byte() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            match self.peek_byte() {
                Some(b'"') => {}
                Some(_) => return Err(self.error("expected string key")),
                None => return Err(self.error("unterminated object")),
            }
            let key = self.string()?;
            self.skip_ws();
            match self.peek_byte() {
                Some(b':') => self.pos += 1,
                Some(_) => return Err(self.error("expected ':'")),
                None => return Err(self.error("unterminated object")),
            }
            self.skip_ws();
            let value = self.value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.peek_byte() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(entries));
                }
                Some(_) => return Err(self.error("expected ',' or '}'")),
                None => return Err(self.error("unterminated object")),
            }
        }
    }

    /// Parse an array; the opening bracket sits at pos.
    fn array(&mut self) -> Result<JsonValue, JsonError> {
        debug_assert_eq!(self.peek_byte(), Some(b'['));
        self.pos += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek_byte() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            if self.peek_byte().is_none() {
                return Err(self.error("unterminated array"));
            }
            let value = self.value()?;
            items.push(value);
            self.skip_ws();
            match self.peek_byte() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                Some(_) => return Err(self.error("expected ',' or ']'")),
                None => return Err(self.error("unterminated array")),
            }
        }
    }
}
