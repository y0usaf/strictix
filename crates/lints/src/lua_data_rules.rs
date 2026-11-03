use std::collections::HashSet;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{AstNode, Binding, Expr, IndStringExpr, StringPart, TextRange};

use crate::config_file_rules::{file_texts, string_text, wrapped};
use crate::config_text_generator::{attr_name, nix_string, render, Node, SENTINEL};
use crate::schema::{callee_name, unwrap_parens};

const PREFIXES: [&str; 5] = ["opt", "o", "g", "wo", "bo"];
const MAX_EXACT: i64 = 1 << 53;
const SYMBOLS: [&str; 10] = ["...", "..", "==", "~=", "<=", ">=", "::", "//", "<<", ">>"];

enum Piece<'a> {
    Text {
        at: usize,
        source: u32,
    },
    Interp {
        at: usize,
        range: TextRange,
        full: &'a str,
    },
}

impl Piece<'_> {
    fn at(&self) -> usize {
        match self {
            Self::Text { at, .. } | Self::Interp { at, .. } => *at,
        }
    }
}

struct LuaText<'a> {
    text: String,
    pieces: Vec<Piece<'a>>,
    start: u32,
}

impl<'a> LuaText<'a> {
    fn new(string: IndStringExpr<'a>, source: &'a str) -> Self {
        let mut lua = LuaText {
            text: String::new(),
            pieces: Vec::new(),
            start: string.syntax().range().start(),
        };
        for part in string.parts() {
            let at = lua.text.len();
            match part {
                StringPart::Content(token) => {
                    lua.pieces.push(Piece::Text {
                        at,
                        source: token.range().start(),
                    });
                    lua.text.push_str(token.text(source));
                }
                StringPart::Interp(interp) => {
                    lua.pieces.push(Piece::Interp {
                        at,
                        range: interp.syntax().range(),
                        full: interp.syntax().text(source),
                    });
                    lua.text.push(SENTINEL);
                }
            }
        }
        lua
    }

    fn piece(&self, offset: usize, end: bool) -> &Piece<'a> {
        let index = self.pieces.partition_point(|p| {
            if end {
                p.at() < offset
            } else {
                p.at() <= offset
            }
        });
        &self.pieces[index - 1]
    }

    fn source_range(&self, start: usize, end: usize) -> TextRange {
        let from = match *self.piece(start, false) {
            Piece::Text { at, source } => source + (start - at) as u32,
            Piece::Interp { range, .. } => range.start(),
        };
        let to = match *self.piece(end, true) {
            Piece::Text { at, source } => source + (end - at) as u32,
            Piece::Interp { range, .. } => range.end(),
        };
        TextRange::new(from, to)
    }

    fn interp(&self, offset: usize) -> &'a str {
        match *self.piece(offset, false) {
            Piece::Interp { full, .. } => full,
            Piece::Text { .. } => "",
        }
    }

    fn keeps_indentation(&self, start: usize, end: usize) -> bool {
        let mut all = usize::MAX;
        let mut kept = usize::MAX;
        let mut offset = 0;
        for line in self.text.split('\n') {
            let indent = line.len() - line.trim_start_matches(' ').len();
            if indent < line.len() {
                all = all.min(indent);
                if offset <= start || offset >= end {
                    kept = kept.min(indent);
                }
            }
            offset += line.len() + 1;
        }
        all == kept
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    Str,
    Long,
    Num,
    Interp,
    Comment,
    Sym,
}

#[derive(Clone, Copy)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
    line: usize,
    end_line: usize,
}

fn long_open(text: &str, i: usize) -> Option<(usize, usize)> {
    let rest = text[i..].strip_prefix('[')?;
    let level = rest.len() - rest.trim_start_matches('=').len();
    rest[level..]
        .starts_with('[')
        .then_some((level, i + level + 2))
}

fn long_close(text: &str, from: usize, level: usize) -> Option<usize> {
    let close = format!("]{}]", "=".repeat(level));
    text[from..].find(&close).map(|at| from + at + close.len())
}

fn lex(text: &str) -> Option<Vec<Token>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let (mut i, mut line) = (0, 0);
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let kind = if text[i..].starts_with("--") {
            i = match long_open(text, i + 2) {
                Some((level, from)) => long_close(text, from, level)?,
                None => text[i..].find('\n').map_or(bytes.len(), |at| i + at),
            };
            Kind::Comment
        } else if let Some((level, from)) = long_open(text, i) {
            i = long_close(text, from, level)?;
            Kind::Long
        } else if c == b'"' || c == b'\'' {
            i += 1;
            loop {
                match *bytes.get(i)? {
                    b'\\' => i += 2,
                    b'\n' => return None,
                    b if b == c => break,
                    _ => i += 1,
                }
            }
            i += 1;
            Kind::Str
        } else if c.is_ascii_digit()
            || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            while bytes.get(i).is_some_and(|&b| {
                b.is_ascii_alphanumeric()
                    || b == b'.'
                    || (matches!(b, b'+' | b'-')
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P'))
            }) {
                i += 1;
            }
            Kind::Num
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while bytes
                .get(i)
                .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
            {
                i += 1;
            }
            Kind::Name
        } else if c == SENTINEL as u8 {
            i += 1;
            Kind::Interp
        } else {
            i += SYMBOLS
                .iter()
                .find(|s| text[i..].starts_with(*s))
                .map_or_else(
                    || text[i..].chars().next().map_or(1, char::len_utf8),
                    |s| s.len(),
                );
            Kind::Sym
        };
        let end_line = line + text[start..i].matches('\n').count();
        tokens.push(Token {
            kind,
            start,
            end: i,
            line,
            end_line,
        });
        line = end_line;
    }
    Some(tokens)
}

fn lua_string(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = raw[1..raw.len() - 1].chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            match chars.next()? {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                c @ ('\\' | '"' | '\'') => c,
                _ => return None,
            }
        } else {
            c
        });
    }
    Some(out)
}

fn lua_number(raw: &str, negative: bool) -> Option<String> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let literal = if digits(raw) {
        raw.parse::<i64>()
            .ok()
            .filter(|v| *v <= MAX_EXACT)?
            .to_string()
    } else {
        let (whole, fraction) = raw.split_once('.')?;
        if !digits(whole) || !digits(fraction) || (whole.len() > 1 && whole.starts_with('0')) {
            return None;
        }
        raw.to_owned()
    };
    if !negative {
        return Some(literal);
    }
    (!literal.bytes().all(|b| matches!(b, b'0' | b'.'))).then(|| format!("-{literal}"))
}

enum Key {
    Pos,
    Name(Option<String>),
    Code,
}

enum Value<'a> {
    Str(Option<String>),
    Num(Option<String>),
    Bool(bool),
    Inline(&'a str),
    Table(Vec<(Key, Value<'a>)>),
    Code,
}

impl Value<'_> {
    fn is_data(&self) -> bool {
        match self {
            Self::Code => false,
            Self::Table(fields) => fields
                .iter()
                .all(|(key, value)| !matches!(key, Key::Code) && value.is_data()),
            _ => true,
        }
    }

    fn nix(&self, item: bool) -> Option<Node> {
        let operand = |text: String| if item { format!("({text})") } else { text };
        Some(match self {
            Self::Str(text) => Node::Raw(nix_string(text.as_deref()?, &mut std::iter::empty())?),
            Self::Num(literal) => {
                let literal = literal.clone()?;
                Node::Raw(if literal.starts_with('-') {
                    operand(literal)
                } else {
                    literal
                })
            }
            Self::Bool(value) => Node::Raw(value.to_string()),
            Self::Inline(full) => {
                Node::Raw(operand(format!("lib.generators.mkLuaInline \"{full}\"")))
            }
            Self::Table(fields) if fields.iter().all(|(key, _)| matches!(key, Key::Pos)) => {
                Node::List(
                    fields
                        .iter()
                        .map(|(_, value)| value.nix(true))
                        .collect::<Option<_>>()?,
                )
            }
            Self::Table(fields) => {
                let mut seen = HashSet::new();
                let mut items = Vec::new();
                for (key, value) in fields {
                    let Key::Name(Some(name)) = key else {
                        return None;
                    };
                    let derivation = name == "type"
                        && matches!(value, Self::Str(Some(text)) if text == "derivation");
                    if name == "_type" || derivation || !seen.insert(name.as_str()) {
                        return None;
                    }
                    items.push((attr_name(name)?, value.nix(false)?));
                }
                Node::Set(items)
            }
            Self::Code => return None,
        })
    }
}

struct Parser<'t, 'a> {
    lua: &'t LuaText<'a>,
    tokens: Vec<Token>,
    comments: Vec<usize>,
}

impl<'a> Parser<'_, 'a> {
    fn text(&self, i: usize) -> Option<&str> {
        self.tokens.get(i).map(|t| &self.lua.text[t.start..t.end])
    }

    fn sym(&self, i: usize, sym: &str) -> bool {
        self.tokens.get(i).is_some_and(|t| t.kind == Kind::Sym) && self.text(i) == Some(sym)
    }

    fn name(&self, i: usize) -> Option<&str> {
        self.tokens
            .get(i)
            .filter(|t| t.kind == Kind::Name)
            .and_then(|_| self.text(i))
    }

    fn commented(&self, start: usize, end: usize) -> bool {
        let first = self.comments.partition_point(|&at| at < start);
        self.comments.get(first).is_some_and(|&at| at < end)
    }

    fn primary(&self, i: usize) -> Option<(Value<'a>, usize)> {
        let token = self.tokens.get(i)?;
        let text = self.text(i)?;
        Some(match token.kind {
            Kind::Str => (Value::Str(lua_string(text)), i + 1),
            Kind::Num => (Value::Num(lua_number(text, false)), i + 1),
            Kind::Long => (Value::Str(None), i + 1),
            Kind::Interp => (Value::Inline(self.lua.interp(token.start)), i + 1),
            Kind::Name if matches!(text, "true" | "false") => (Value::Bool(text == "true"), i + 1),
            Kind::Sym if text == "-" && self.tokens.get(i + 1)?.kind == Kind::Num => {
                (Value::Num(lua_number(self.text(i + 1)?, true)), i + 2)
            }
            Kind::Sym if text == "{" => self.table(i)?,
            _ => (Value::Code, i),
        })
    }

    fn skip(&self, mut i: usize) -> Option<usize> {
        let mut depth = 0usize;
        loop {
            let token = self.tokens.get(i)?;
            let text = self.text(i)?;
            let (open, close) = match token.kind {
                Kind::Sym => (
                    matches!(text, "(" | "[" | "{"),
                    matches!(text, ")" | "]" | "}"),
                ),
                Kind::Name => (
                    matches!(text, "function" | "if" | "do" | "repeat"),
                    matches!(text, "end" | "until"),
                ),
                _ => (false, false),
            };
            if depth == 0 && (close || matches!(text, "," | ";") && token.kind == Kind::Sym) {
                return Some(i);
            }
            if open {
                depth += 1;
            } else if close {
                depth -= 1;
            }
            i += 1;
        }
    }

    fn field(&self, i: usize) -> Option<(Value<'a>, usize)> {
        let (value, next) = self.primary(i)?;
        if !matches!(value, Value::Code) && [",", ";", "}"].iter().any(|s| self.sym(next, s)) {
            return Some((value, next));
        }
        Some((Value::Code, self.skip(i)?))
    }

    fn table(&self, open: usize) -> Option<(Value<'a>, usize)> {
        let mut i = open + 1;
        let mut fields = Vec::new();
        loop {
            if self.sym(i, "}") {
                return Some((Value::Table(fields), i + 1));
            }
            let key = if self.sym(i, "[") {
                let close = self.skip(i + 1)?;
                if !self.sym(close, "]") || !self.sym(close + 1, "=") {
                    return None;
                }
                let key = match (close == i + 2, self.tokens.get(i + 1)?.kind) {
                    (true, Kind::Str) => Key::Name(lua_string(self.text(i + 1)?)),
                    (true, Kind::Num) => Key::Name(None),
                    _ => Key::Code,
                };
                i = close + 2;
                key
            } else if let (Some(name), true) = (self.name(i), self.sym(i + 1, "=")) {
                i += 2;
                Key::Name(Some(name.to_owned()))
            } else {
                Key::Pos
            };
            let (value, next) = self.field(i)?;
            fields.push((key, value));
            i = next;
            if self.sym(i, ",") || self.sym(i, ";") {
                i += 1;
            } else if !self.sym(i, "}") {
                return None;
            }
        }
    }

    fn assignment(&self, i: usize) -> Option<(&str, &str, Value<'a>, usize)> {
        let token = self.tokens.get(i)?;
        let first = i == 0 || (self.tokens[i - 1].end_line < token.line && !self.sym(i - 1, ","));
        if !first || self.name(i)? != "vim" || !self.sym(i + 1, ".") || !self.sym(i + 3, ".") {
            return None;
        }
        let prefix = self.name(i + 2).filter(|p| PREFIXES.contains(p))?;
        let name = self.name(i + 4)?;
        if !self.sym(i + 5, "=") {
            return None;
        }
        let (value, mut end) = self.primary(i + 6)?;
        if !value.is_data() {
            return None;
        }
        if self.sym(end, ";") && self.tokens[end].line == self.tokens[end - 1].end_line {
            end += 1;
        }
        let ends = self.tokens.get(end).is_none_or(|next| {
            next.line > self.tokens[end - 1].end_line
                && (next.kind != Kind::Sym || self.text(end) == Some("::"))
        });
        ends.then_some((prefix, name, value, end))
    }
}

fn is_lua(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".lua")
}

fn ind_string<'a>(value: Expr<'a>, source: &str) -> Option<IndStringExpr<'a>> {
    let mut value = value;
    while let Some(items) = wrapped(value, source) {
        let [item] = items.as_slice() else {
            return None;
        };
        value = *item;
    }
    match unwrap_parens(value) {
        Expr::IndString(string) => Some(string),
        _ => None,
    }
}

fn write_text<'a>(expr: Expr<'a>, source: &str) -> Option<Expr<'a>> {
    let Expr::Apply(apply) = expr else {
        return None;
    };
    let Expr::Apply(inner) = unwrap_parens(apply.func()?) else {
        return None;
    };
    if callee_name(inner.func()?, source)? != "writeText" {
        return None;
    }
    let Expr::String(name) = unwrap_parens(inner.arg()?) else {
        return None;
    };
    is_lua(&string_text(name, source)).then_some(apply.arg()?)
}

struct Finding {
    message: String,
    start: usize,
    end: usize,
    generated: Option<String>,
}

fn indent_of(source: &str, offset: u32) -> &str {
    let start = offset as usize;
    let line = &source[source[..start].rfind('\n').map_or(0, |i| i + 1)..start];
    &line[..line.len() - line.trim_start().len()]
}

fn assignment_runs(parser: &Parser<'_, '_>, source: &str, out: &mut Vec<Finding>) {
    let mut i = 0;
    while i < parser.tokens.len() {
        let Some((prefix, name, value, end)) = parser.assignment(i) else {
            i += 1;
            continue;
        };
        let mut run = vec![(name, value)];
        let mut next = end;
        while let Some((other, name, value, end)) = parser.assignment(next) {
            let gap = parser.tokens[next - 1].end;
            if other != prefix || parser.commented(gap, parser.tokens[next].start) {
                break;
            }
            run.push((name, value));
            next = end;
        }
        if run.len() >= 3 {
            let (start, stop) = (parser.tokens[i].start, parser.tokens[next - 1].end);
            let range = parser.lua.source_range(start, stop);
            let mut seen = HashSet::new();
            let generated = run
                .iter()
                .map(|(name, value)| {
                    seen.insert(*name)
                        .then(|| Some((attr_name(name)?, value.nix(false)?)))
                        .flatten()
                })
                .collect::<Option<Vec<_>>>()
                .map(|items| {
                    let mut out = String::from("for k, v in pairs(${lib.generators.toLua { } ");
                    render(
                        &Node::Set(items),
                        indent_of(source, range.start()),
                        0,
                        &mut out,
                    );
                    out.push_str(&format!("}}) do vim.{prefix}[k] = v end"));
                    out
                });
            out.push(Finding {
                message: format!(
                    "{} consecutive vim.{prefix}.* assignments are hand-written Lua data; generate them from a Nix attrset with lib.generators.toLua",
                    run.len()
                ),
                start,
                end: stop,
                generated,
                });
        }
        i = next;
    }
}

fn setup_tables(parser: &Parser<'_, '_>, source: &str, out: &mut Vec<Finding>) {
    for i in 0..parser.tokens.len() {
        let call = parser.name(i) == Some("require")
            && !(i > 0 && (parser.sym(i - 1, ".") || parser.sym(i - 1, ":")))
            && parser.sym(i + 1, "(")
            && parser
                .tokens
                .get(i + 2)
                .is_some_and(|t| t.kind == Kind::Str)
            && parser.sym(i + 3, ")")
            && parser.sym(i + 4, ".")
            && parser.name(i + 5) == Some("setup");
        if !call {
            continue;
        }
        let paren = parser.sym(i + 6, "(");
        let open = if paren { i + 7 } else { i + 6 };
        if !parser.sym(open, "{") {
            continue;
        }
        let Some((value, after)) = parser.table(open) else {
            continue;
        };
        if matches!(&value, Value::Table(fields) if fields.is_empty())
            || (paren && !parser.sym(after, ")"))
        {
            continue;
        }
        let (start, stop) = (parser.tokens[open].start, parser.tokens[after - 1].end);
        let module = parser.text(i + 2).unwrap_or_default();
        let generated = value
            .is_data()
            .then(|| value.nix(false))
            .flatten()
            .map(|node| {
                let range = parser.lua.source_range(start, stop);
                let mut out = String::from("${lib.generators.toLua { } ");
                render(&node, indent_of(source, range.start()), 0, &mut out);
                out.push('}');
                out
            });
        out.push(Finding {
            message: format!(
                "require({module}).setup table is hand-written Lua data; generate it from a Nix value with lib.generators.toLua"
            ),
            start,
            end: stop,
            generated,
        });
    }
}

pub struct HandWrittenLuaData;

impl HandWrittenLuaData {
    fn check_text(
        &self,
        model: &SemanticModel,
        string: IndStringExpr<'_>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let source = model.source();
        let lua = LuaText::new(string, source);
        let Some(all) = lex(&lua.text) else {
            return;
        };
        let (comments, tokens): (Vec<Token>, Vec<Token>) =
            all.into_iter().partition(|t| t.kind == Kind::Comment);
        let parser = Parser {
            lua: &lua,
            tokens,
            comments: comments.iter().map(|t| t.start).collect(),
        };
        let mut findings = Vec::new();
        assignment_runs(&parser, source, &mut findings);
        setup_tables(&parser, source, &mut findings);
        let lib = model.is_bound("lib", lua.start);
        for finding in findings {
            let range = lua.source_range(finding.start, finding.end);
            let text = &source[range.start() as usize..range.end() as usize];
            let exact = lib
                && !text.contains("''")
                && !text.contains("$$")
                && !parser.commented(finding.start, finding.end)
                && lua.keeps_indentation(finding.start, finding.end);
            let diagnostic = Diagnostic::new(self.code(), self.severity(), finding.message, range);
            diags.push(match finding.generated.filter(|_| exact) {
                Some(replacement) => diagnostic.with_fix(
                    Fix::new("generate the Lua data with lib.generators.toLua")
                        .edit(range, replacement),
                ),
                None => diagnostic,
            });
        }
    }
}

impl Rule for HandWrittenLuaData {
    fn code(&self) -> &'static str {
        "hand-written-lua-data"
    }

    fn name(&self) -> &'static str {
        "Hand-written Lua data"
    }

    fn description(&self) -> &'static str {
        "Flags Lua data spelled by hand inside a `.lua` file's text (a file entry's `text`, or `pkgs.writeText \"*.lua\"`): runs of three or more `vim.opt`/`vim.o`/`vim.g`/`vim.wo`/`vim.bo` assignments of literal values (strings, numbers, booleans, tables of literals) with only blank lines between them, and the table constructor passed to `require(\"X\").setup`. Such data is a Nix value; generate it with `lib.generators.toLua`. The fix rewrites an assignment run as `for k, v in pairs(${lib.generators.toLua { } { ... }}) do vim.opt[k] = v end` (assignment order is not kept) and a setup table as `${lib.generators.toLua { } { ... }}`, keeping whole-value interpolations as `lib.generators.mkLuaInline`. It is withheld without `lib` in scope, for setup tables holding functions, variables or calls, tables mixing array and hash parts, non-string keys, duplicate names, Lua escapes other than `\\n \\t \\r \\\\ \\\" \\'`, long strings, floats in exponent or hex form, numbers beyond 2^53, interpolations inside strings or keys, comments, `''`/`$$` escapes, or when the edit would change the string's indentation."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            if let Some(binding) = Binding::cast(node) {
                let Some((name, texts)) = file_texts(binding, source) else {
                    continue;
                };
                if !is_lua(&name) {
                    continue;
                }
                for (text, _) in texts {
                    if let Some(string) = ind_string(text, source) {
                        self.check_text(model, string, diags);
                    }
                }
            } else if let Some(text) = Expr::cast(node).and_then(|expr| write_text(expr, source)) {
                if let Some(string) = ind_string(text, source) {
                    self.check_text(model, string, diags);
                }
            }
        }
    }
}
