//! Hand-written lexer for the Nix expression language.
//!
//! Produces a flat token stream with trivia (whitespace, comments)
//! preserved, so concatenating all token texts reproduces the source
//! exactly. String interpolation is handled with a mode stack: entering
//! `${` pushes a normal-expression frame, and the matching `}` pops
//! back into string mode.

use crate::kind::SyntaxKind;

/// A single lexed token, as a span into the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub start: u32,
    pub len: u32,
}

impl Token {
    /// The source text this token covers.
    #[must_use]
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        let start = self.start as usize;
        &source[start..start + self.len as usize]
    }
}

/// Lex `source` into tokens. Always terminates; unterminated constructs
/// yield an [`SyntaxKind::Error`] token. The final token is
/// [`SyntaxKind::Eof`].
#[must_use]
pub fn lex(source: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(source);
    lexer.run();
    lexer.tokens
}

/// Lexer state: which syntactic context we are scanning in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Between expressions. The bottom of the stack is always Normal.
    Normal,
    /// Inside a `"..."` string.
    Str,
    /// Inside an `''...''` indented string.
    IndStr,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    /// Start of pending string content not yet flushed as a token.
    content_start: usize,
    tokens: Vec<Token>,
    /// Mode stack. Bottom is the top-level Normal frame; interpolation
    /// pushes Normal frames on top of string frames.
    modes: Vec<Mode>,
    /// Brace depth per Normal frame, parallel to `modes` entries that
    /// are Normal. Index i is the depth of the i-th Normal frame.
    braces: Vec<usize>,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'\'' | b'-')
}

fn is_path_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+')
}

fn is_uri_char(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            b'%' | b'/'
                | b'?'
                | b':'
                | b'@'
                | b'&'
                | b'='
                | b'+'
                | b'$'
                | b','
                | b'-'
                | b'_'
                | b'.'
                | b'!'
                | b'~'
                | b'*'
                | b'\''
        )
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            src: source.as_bytes(),
            pos: 0,
            content_start: 0,
            tokens: Vec::new(),
            modes: vec![Mode::Normal],
            braces: vec![0],
        }
    }

    fn run(&mut self) {
        while !self.at_eof() {
            let before = self.pos;
            match self.mode() {
                Mode::Normal => self.normal(),
                Mode::Str => self.string(),
                Mode::IndStr => self.ind_string(),
            }
            debug_assert!(self.pos > before, "lexer made no progress");
        }
        if self.mode() != Mode::Normal {
            // Unterminated string: flush what we have, flag the error.
            self.flush_content();
            self.emit(SyntaxKind::Error, self.pos);
        }
        self.emit(SyntaxKind::Eof, self.pos);
    }

    // --- low-level helpers ---

    fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> u8 {
        self.peek_at(0)
    }

    fn peek_at(&self, offset: usize) -> u8 {
        self.src.get(self.pos + offset).copied().unwrap_or(0)
    }

    fn bump(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.src.len());
    }

    fn mode(&self) -> Mode {
        *self.modes.last().expect("mode stack never empty")
    }

    fn emit(&mut self, kind: SyntaxKind, start: usize) {
        self.tokens.push(Token {
            kind,
            start: start as u32,
            len: (self.pos - start) as u32,
        });
    }

    fn emit_len(&mut self, kind: SyntaxKind, len: usize) {
        let start = self.pos;
        self.bump(len);
        self.emit(kind, start);
    }

    /// Emit any accumulated string content as one token.
    fn flush_content(&mut self) {
        if self.pos > self.content_start {
            let start = self.content_start;
            self.content_start = self.pos;
            self.emit(SyntaxKind::StringContent, start);
        }
    }

    // --- normal mode ---

    fn normal(&mut self) {
        let c = self.peek();
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => self.whitespace(),
            b'#' => self.line_comment(),
            b'/' if self.peek_at(1) == b'*' => self.block_comment(),
            b'"' => {
                self.emit_len(SyntaxKind::StringStart, 1);
                self.modes.push(Mode::Str);
                self.content_start = self.pos;
            }
            b'\'' => {
                if self.peek_at(1) == b'\'' {
                    self.emit_len(SyntaxKind::IndStringStart, 2);
                    self.modes.push(Mode::IndStr);
                    self.content_start = self.pos;
                } else {
                    self.emit_len(SyntaxKind::Error, 1);
                }
            }
            b'0'..=b'9' => self.number(),
            b'<' => self.lt_or_search_path(),
            b'>' => {
                if self.peek_at(1) == b'=' {
                    self.emit_len(SyntaxKind::GtEq, 2);
                } else {
                    self.emit_len(SyntaxKind::Gt, 1);
                }
            }
            b'=' => {
                if self.peek_at(1) == b'=' {
                    self.emit_len(SyntaxKind::EqEq, 2);
                } else {
                    self.emit_len(SyntaxKind::Assign, 1);
                }
            }
            b'!' => {
                if self.peek_at(1) == b'=' {
                    self.emit_len(SyntaxKind::Neq, 2);
                } else {
                    self.emit_len(SyntaxKind::Bang, 1);
                }
            }
            b'&' => {
                if self.peek_at(1) == b'&' {
                    self.emit_len(SyntaxKind::AndAnd, 2);
                } else {
                    self.emit_len(SyntaxKind::Error, 1);
                }
            }
            b'|' => {
                if self.peek_at(1) == b'|' {
                    self.emit_len(SyntaxKind::OrOr, 2);
                } else {
                    self.emit_len(SyntaxKind::Error, 1);
                }
            }
            b'+' => {
                if self.peek_at(1) == b'+' {
                    self.emit_len(SyntaxKind::PlusPlus, 2);
                } else {
                    self.emit_len(SyntaxKind::Plus, 1);
                }
            }
            b'-' => {
                if self.peek_at(1) == b'>' {
                    self.emit_len(SyntaxKind::Arrow, 2);
                } else {
                    self.emit_len(SyntaxKind::Minus, 1);
                }
            }
            b'*' => self.emit_len(SyntaxKind::Star, 1),
            b'/' => self.slash_or_path(),
            b'.' => self.dot_or_path(),
            b'~' => {
                if self.peek_at(1) == b'/' {
                    self.path();
                } else {
                    self.emit_len(SyntaxKind::Error, 1);
                }
            }
            b'{' => {
                if let Some(depth) = self.braces.last_mut() {
                    *depth += 1;
                }
                self.emit_len(SyntaxKind::LBrace, 1);
            }
            b'}' => self.right_brace(),
            b'[' => self.emit_len(SyntaxKind::LBracket, 1),
            b']' => self.emit_len(SyntaxKind::RBracket, 1),
            b'(' => self.emit_len(SyntaxKind::LParen, 1),
            b')' => self.emit_len(SyntaxKind::RParen, 1),
            b';' => self.emit_len(SyntaxKind::Semicolon, 1),
            b',' => self.emit_len(SyntaxKind::Comma, 1),
            b'?' => self.emit_len(SyntaxKind::Question, 1),
            b'@' => self.emit_len(SyntaxKind::At, 1),
            b':' => self.emit_len(SyntaxKind::Colon, 1),
            b'$' if self.peek_at(1) == b'{' => {
                // Unquoted interpolation: valid in attrpaths (dynamic
                // segment) and in string-literal positions outside strings.
                self.emit_len(SyntaxKind::InterpStart, 2);
                self.modes.push(Mode::Normal);
                self.braces.push(0);
            }
            _ if is_ident_start(c) => self.ident_or_keyword(),
            _ => self.emit_len(SyntaxKind::Error, 1),
        }
    }

    fn whitespace(&mut self) {
        let start = self.pos;
        while matches!(self.peek(), b' ' | b'\t' | b'\r' | b'\n') {
            self.bump(1);
        }
        self.emit(SyntaxKind::Whitespace, start);
    }

    fn line_comment(&mut self) {
        let start = self.pos;
        while !self.at_eof() && self.peek() != b'\n' {
            self.bump(1);
        }
        self.emit(SyntaxKind::Comment, start);
    }

    fn block_comment(&mut self) {
        let start = self.pos;
        self.bump(2); // /*
        while !self.at_eof() {
            if self.peek() == b'*' && self.peek_at(1) == b'/' {
                self.bump(2);
                break;
            }
            self.bump(1);
        }
        self.emit(SyntaxKind::Comment, start);
    }

    fn number(&mut self) {
        let start = self.pos;
        while self.peek().is_ascii_digit() {
            self.bump(1);
        }
        // Nix floats require digits on both sides of the dot; an
        // exponent is only valid in float form. `1e3` is Int + Ident.
        let mut is_float = false;
        if self.peek() == b'.' && self.peek_at(1).is_ascii_digit() {
            self.bump(1);
            while self.peek().is_ascii_digit() {
                self.bump(1);
            }
            is_float = true;
        }
        if is_float && matches!(self.peek(), b'e' | b'E') {
            let save = self.pos;
            self.bump(1);
            if matches!(self.peek(), b'+' | b'-') {
                self.bump(1);
            }
            if self.peek().is_ascii_digit() {
                while self.peek().is_ascii_digit() {
                    self.bump(1);
                }
            } else {
                self.pos = save;
            }
        }
        self.emit(
            if is_float {
                SyntaxKind::Float
            } else {
                SyntaxKind::Int
            },
            start,
        );
    }

    fn ident_or_keyword(&mut self) {
        let start = self.pos;
        while is_ident_char(self.peek()) {
            self.bump(1);
        }
        // Nix quirk: `scheme:rest` with no space is a URI, so `x:1`
        // is a URI token, not a lambda. Lambda needs a space (or a
        // non-URI char) after the colon.
        if self.peek() == b':' && is_uri_char(self.peek_at(1)) {
            self.bump(1);
            while is_uri_char(self.peek()) {
                self.bump(1);
            }
            self.emit(SyntaxKind::Uri, start);
            return;
        }
        // Path continuation: `foo/bar`. Division requires spaces.
        if self.peek() == b'/' && is_path_char(self.peek_at(1)) {
            self.path_tail(start);
            return;
        }
        let kind = match &self.src[start..self.pos] {
            b"if" => SyntaxKind::KwIf,
            b"then" => SyntaxKind::KwThen,
            b"else" => SyntaxKind::KwElse,
            b"assert" => SyntaxKind::KwAssert,
            b"with" => SyntaxKind::KwWith,
            b"let" => SyntaxKind::KwLet,
            b"in" => SyntaxKind::KwIn,
            b"rec" => SyntaxKind::KwRec,
            b"inherit" => SyntaxKind::KwInherit,
            b"or" => SyntaxKind::KwOr,
            _ => SyntaxKind::Ident,
        };
        self.emit(kind, start);
    }

    fn lt_or_search_path(&mut self) {
        if self.peek_at(1) == b'=' {
            self.emit_len(SyntaxKind::LtEq, 2);
            return;
        }
        if is_ident_start(self.peek_at(1)) {
            // Try `<name>` or `<name/sub/path>`.
            let mut i = self.pos + 1;
            while i < self.src.len()
                && (is_ident_char(self.src[i]) || matches!(self.src[i], b'/' | b'.' | b'+'))
            {
                i += 1;
            }
            if i < self.src.len() && self.src[i] == b'>' {
                let start = self.pos;
                self.pos = i + 1;
                self.emit(SyntaxKind::SearchPath, start);
                return;
            }
        }
        self.emit_len(SyntaxKind::Lt, 1);
    }

    fn slash_or_path(&mut self) {
        if self.peek_at(1) == b'/' {
            self.emit_len(SyntaxKind::SlashSlash, 2);
        } else if is_path_char(self.peek_at(1)) {
            // Absolute path literal `/foo/bar`.
            self.path();
        } else {
            self.emit_len(SyntaxKind::Slash, 1);
        }
    }

    fn dot_or_path(&mut self) {
        if self.peek_at(1) == b'.' && self.peek_at(2) == b'.' {
            self.emit_len(SyntaxKind::Ellipsis, 3);
        } else if self.peek_at(1) == b'/' || (self.peek_at(1) == b'.' && self.peek_at(2) == b'/') {
            self.path();
        } else {
            self.emit_len(SyntaxKind::Dot, 1);
        }
    }

    /// Lex a path starting at the current position (`/`, `./`, `../`,
    /// `~/` prefixes).
    fn path(&mut self) {
        let start = self.pos;
        // The `~/` prefix consists of no path chars, so consume it
        // before the generic segment walker.
        if self.peek() == b'~' && self.peek_at(1) == b'/' {
            self.bump(2);
        }
        self.path_tail(start);
    }

    /// Continue a path from `start` (already consumed prefix), then
    /// emit a single Path token covering `start..pos`.
    fn path_tail(&mut self, start: usize) {
        // Consume the prefix characters (`./`, `../`, `~/`, `/`, or the
        // slash after an ident) and then path segments.
        loop {
            while is_path_char(self.peek()) {
                self.bump(1);
            }
            if self.peek() == b'/' && is_path_char(self.peek_at(1)) {
                self.bump(1);
            } else {
                break;
            }
        }
        self.emit(SyntaxKind::Path, start);
    }

    fn right_brace(&mut self) {
        let depth = self.braces.last_mut().expect("brace stack never empty");
        if *depth > 0 {
            *depth -= 1;
            self.emit_len(SyntaxKind::RBrace, 1);
        } else if self.modes.len() > 1 {
            // This `}` closes an interpolation.
            self.emit_len(SyntaxKind::InterpEnd, 1);
            self.modes.pop();
            self.braces.pop();
            self.content_start = self.pos;
        } else {
            // Top level: unmatched, but emit and let the parser report.
            self.emit_len(SyntaxKind::RBrace, 1);
        }
    }

    // --- string modes ---

    fn string(&mut self) {
        loop {
            if self.at_eof() {
                self.flush_content();
                return;
            }
            match self.peek() {
                b'\\' => self.bump(if self.peek_at(1) == 0 { 1 } else { 2 }),
                b'"' => {
                    self.flush_content();
                    self.emit_len(SyntaxKind::StringEnd, 1);
                    self.modes.pop();
                    return;
                }
                b'$' if self.peek_at(1) == b'{' => {
                    self.flush_content();
                    self.emit_len(SyntaxKind::InterpStart, 2);
                    self.modes.push(Mode::Normal);
                    self.braces.push(0);
                    return;
                }
                _ => self.bump(1),
            }
        }
    }

    fn ind_string(&mut self) {
        loop {
            if self.at_eof() {
                self.flush_content();
                return;
            }
            match self.peek() {
                b'$' if self.peek_at(1) == b'{' => {
                    self.flush_content();
                    self.emit_len(SyntaxKind::InterpStart, 2);
                    self.modes.push(Mode::Normal);
                    self.braces.push(0);
                    return;
                }
                b'\'' => {
                    if self.ind_quote() {
                        return;
                    }
                }
                _ => self.bump(1),
            }
        }
    }

    /// Handle a `'` inside an indented string: escapes, literal single
    /// quotes, or the closing `''`. Returns `true` if the string ended.
    fn ind_quote(&mut self) -> bool {
        if self.peek_at(1) != b'\'' {
            // A lone `'` is literal content.
            self.bump(1);
            return false;
        }
        match self.peek_at(2) {
            // `'''` escapes a literal `''`.
            b'\'' => self.bump(3),
            // `''$` escapes a literal `$`.
            b'$' => self.bump(3),
            // `''\n` style escapes.
            b'\\' => {
                self.bump(3);
                if !self.at_eof() {
                    self.bump(1);
                }
            }
            // Otherwise `''` closes the string.
            _ => {
                self.flush_content();
                self.emit_len(SyntaxKind::IndStringEnd, 2);
                self.modes.pop();
                self.content_start = self.pos;
                return true;
            }
        }
        false
    }
}
