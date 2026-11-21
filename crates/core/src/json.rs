use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
        Null,
        Bool(bool),
        Number(f64),
        String(String),
        Array(Vec<JsonValue>),
        Object(Vec<(String, JsonValue)>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct JsonError {
        pub message: String,
        pub offset: usize,
}

impl JsonValue {
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

        #[must_use]
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

        #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s),
            _ => None,
        }
    }

        #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

        #[must_use]
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(entries) => Some(entries),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            JsonValue::Number(n) => Some(*n),
            _ => None,
        }
    }
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl Parser<'_> {
        fn peek_byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

        fn error(&self, message: &str) -> JsonError {
        JsonError {
            message: message.to_owned(),
            offset: self.pos,
        }
    }

        fn skip_ws(&mut self) {
        while matches!(self.peek_byte(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

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

        fn literal(&mut self, word: &[u8], value: JsonValue) -> Result<JsonValue, JsonError> {
        if self.input.as_bytes()[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected character"))
        }
    }

        fn number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.pos;
        if self.peek_byte() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek_byte() {
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
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(JsonValue::Number(n)),
            _ => Err(self.error("invalid number")),
        }
    }

        fn string(&mut self) -> Result<String, JsonError> {
        debug_assert_eq!(self.peek_byte(), Some(b'"'));
        self.pos += 1;
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
                b if b < 0x20 => {
                    return Err(self.error("unexpected control character in string"));
                }
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

        fn unicode_escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let code_point = self.hex4()?;
        match code_point {
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
                                None => return Err(self.error("invalid escape")),
                            }
                        }
                    }
                }
                Err(self.error("unpaired surrogate"))
            }
            0xDC00..=0xDFFF => Err(self.error("unpaired surrogate")),
            _ => match char::from_u32(code_point) {
                Some(ch) => {
                    out.push(ch);
                    Ok(())
                }
                None => Err(self.error("invalid escape")),
            },
        }
    }

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

impl JsonValue {
    fn write_compact(&self, out: &mut String) {
        match self {
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsonValue::Number(n) => out.push_str(&format_number(*n)),
            JsonValue::String(s) => write_json_string(s, out),
            JsonValue::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_compact(out);
                }
                out.push(']');
            }
            JsonValue::Object(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(k, out);
                    out.push(':');
                    v.write_compact(out);
                }
                out.push('}');
            }
        }
    }
}

impl fmt::Display for JsonValue {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        self.write_compact(&mut out);
        formatter.write_str(&out)
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

pub fn write_json_str(s: &str, out: &mut impl std::io::Write) -> std::io::Result<()> {
    out.write_all(b"\"")?;
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        let esc: &[u8] = match b {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            0x08 => b"\\b",
            0x0C => b"\\f",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0..0x20 => {
                if start < i {
                    out.write_all(&bytes[start..i])?;
                }
                write!(out, "\\u{:04x}", b)?;
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        if start < i {
            out.write_all(&bytes[start..i])?;
        }
        out.write_all(esc)?;
        start = i + 1;
    }
    if start < bytes.len() {
        out.write_all(&bytes[start..])?;
    }
    out.write_all(b"\"")
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        let esc = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            0x08 => "\\b",
            0x0C => "\\f",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0..0x20 => {
                if start < i {
                    out.push_str(&s[start..i]);
                }
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", b);
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        if start < i {
            out.push_str(&s[start..i]);
        }
        out.push_str(esc);
        start = i + 1;
    }
    if start < s.len() {
        out.push_str(&s[start..]);
    }
    out.push('"');
}
