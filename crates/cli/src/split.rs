use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, Binding, Expr, HasAttrExpr, LetExpr, ListExpr,
    NodeOrToken, ParenExpr, Root, SelectExpr, StringPart, SyntaxKind as K, SyntaxNode, SyntaxToken,
    TextRange, UnaryExpr,
};

use crate::merge::{
    binding_name, binding_value_range, format_files, formatted_texts, is_call_package, is_relative,
    join, let_of, module_literal, outputs_binding, parent, relative_literal,
};

pub struct SplitOptions {
    pub root: PathBuf,
    pub merged: String,
    pub dry_run: bool,
}

pub struct SplitReport {
    pub restored: Vec<String>,
    pub redirected: Vec<String>,
    pub expansions: usize,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy)]
struct Span {
    text: TextRange,
    merged: TextRange,
    copied: bool,
}

#[derive(Default)]
pub(crate) struct Mapped {
    pub(crate) text: String,
    spans: Vec<Span>,
}

fn offset(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

impl Mapped {
    fn inserted(text: String, origin: TextRange) -> Mapped {
        let mut out = Mapped::default();
        out.insert(&text, origin);
        out
    }

    fn push(&mut self, text: &str, merged: TextRange, copied: bool) {
        if text.is_empty() {
            return;
        }
        let start = offset(self.text.len());
        self.text.push_str(text);
        self.spans.push(Span {
            text: TextRange::new(start, offset(self.text.len())),
            merged,
            copied,
        });
    }

    fn copy(&mut self, source: &str, range: TextRange) {
        self.push(
            &source[range.start() as usize..range.end() as usize],
            range,
            true,
        );
    }

    fn insert(&mut self, text: &str, origin: TextRange) {
        self.push(text, origin, false);
    }

    fn append(&mut self, other: Mapped) {
        let shift = offset(self.text.len());
        self.text.push_str(&other.text);
        self.spans.extend(other.spans.into_iter().map(|span| Span {
            text: TextRange::new(span.text.start() + shift, span.text.end() + shift),
            ..span
        }));
    }

    fn file_text(self) -> Mapped {
        let lead = offset(self.text.len() - self.text.trim_start().len());
        let end = offset(self.text.trim_end().len()).max(lead);
        let mut out = Mapped {
            text: self.text[lead as usize..end as usize].to_string(),
            spans: Vec::new(),
        };
        for span in self.spans {
            let start = span.text.start().max(lead);
            let stop = span.text.end().min(end);
            if start >= stop {
                continue;
            }
            let merged = if span.copied {
                let from = span.merged.start() + (start - span.text.start());
                TextRange::new(from, from + (stop - start))
            } else {
                span.merged
            };
            out.spans.push(Span {
                text: TextRange::new(start - lead, stop - lead),
                merged,
                copied: span.copied,
            });
        }
        let tail = out.spans.last().map_or(0, |span| span.merged.end());
        out.insert("\n", TextRange::new(tail, tail));
        out
    }

    fn point(&self, at: u32, end: bool) -> u32 {
        let index = self.spans.partition_point(|span| {
            if end {
                span.text.end() < at
            } else {
                span.text.end() <= at
            }
        });
        let Some(span) = self.spans.get(index).or(self.spans.last()) else {
            return 0;
        };
        if span.copied {
            let within = at.clamp(span.text.start(), span.text.end()) - span.text.start();
            span.merged.start() + within
        } else if end {
            span.merged.end()
        } else {
            span.merged.start()
        }
    }

    pub(crate) fn range(&self, range: TextRange) -> TextRange {
        let start = self.point(range.start(), false);
        if range.end() <= range.start() {
            return TextRange::new(start, start);
        }
        TextRange::new(start, self.point(range.end(), true).max(start))
    }

    pub(crate) fn exact(&self, range: TextRange) -> Option<TextRange> {
        let index = self
            .spans
            .partition_point(|span| span.text.end() < range.start());
        self.spans[index..]
            .iter()
            .take_while(|span| span.text.start() <= range.start())
            .find(|span| span.copied && range.end() <= span.text.end())
            .map(|span| {
                let start = span.merged.start() + (range.start() - span.text.start());
                TextRange::new(start, start + (range.end() - range.start()))
            })
    }
}

enum Value {
    Str(String),
    List(Vec<Value>),
    Attrs(Vec<(String, Value)>),
}

impl Value {
    fn get(&self, key: &str) -> Option<&Value> {
        self.attrs().iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn attrs(&self) -> &[(String, Value)] {
        match self {
            Value::Attrs(items) => items,
            _ => &[],
        }
    }

    fn list(&self) -> &[Value] {
        match self {
            Value::List(items) => items,
            _ => &[],
        }
    }

    fn string(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn string_value<'a>(parts: impl Iterator<Item = StringPart<'a>>, source: &str) -> Option<String> {
    let mut out = String::new();
    for part in parts {
        let StringPart::Content(token) = part else {
            return None;
        };
        out.push_str(&unescape(token.text(source)));
    }
    Some(out)
}

fn literal(expr: Expr<'_>, source: &str) -> Result<Value, String> {
    let not_data = || format!("`{}` is not literal data", expr.content_text(source));
    match expr {
        Expr::String(s) => string_value(s.parts(), source)
            .map(Value::Str)
            .ok_or_else(not_data),
        Expr::List(list) => list
            .items()
            .map(|item| literal(item, source))
            .collect::<Result<_, _>>()
            .map(Value::List),
        Expr::Attrset(set) => {
            let mut items = Vec::new();
            for item in set.items() {
                let AttrItem::Binding(binding) = item else {
                    return Err(not_data());
                };
                let key = attr_key(binding, source).ok_or_else(not_data)?;
                let value = literal(binding.value().ok_or_else(not_data)?, source)?;
                items.push((key, value));
            }
            Ok(Value::Attrs(items))
        }
        Expr::Paren(paren) => literal(paren.expr().ok_or_else(not_data)?, source),
        _ => Err(not_data()),
    }
}

fn attr_key(binding: Binding<'_>, source: &str) -> Option<String> {
    let path = binding.attrpath()?;
    let mut elements = path.elements();
    match (elements.next(), elements.next()) {
        (Some(AttrName::Ident(token)), None) => Some(token.text(source).to_string()),
        (Some(AttrName::Str(s)), None) => string_value(s.parts(), source),
        _ => None,
    }
}

struct Expansion {
    file: String,
    source: String,
    modules: Vec<String>,
}

struct Removed {
    file: String,
    source: String,
    before: Option<String>,
}

struct Manifest {
    entry: String,
    out: Option<String>,
    keyer: Option<String>,
    files: HashMap<String, String>,
    redirected: Vec<String>,
    overrides: HashMap<String, HashMap<String, String>>,
    expansions: Vec<Expansion>,
    removed: Vec<Removed>,
    module_values: Vec<(String, String)>,
    verbatim: HashMap<String, String>,
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .map(Value::list)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::string)
        .map(str::to_string)
        .collect()
}

fn string_map(value: &Value) -> HashMap<String, String> {
    value
        .attrs()
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), v.string()?.to_string())))
        .collect()
}

fn records<'v>(value: &'v Value, key: &str) -> &'v [Value] {
    value.get(key).map(Value::list).unwrap_or_default()
}

impl Manifest {
    fn read(value: &Value) -> Result<Manifest, String> {
        let field = |key: &str| value.get(key).and_then(Value::string).map(str::to_string);
        let entry = field("entry").ok_or("the merge manifest has no `entry` string")?;
        let files = value
            .get("files")
            .map(string_map)
            .ok_or("the merge manifest has no `files` set")?;
        let file_of = |record: &Value| {
            record
                .get("file")
                .and_then(Value::string)
                .unwrap_or(&entry)
                .to_string()
        };
        Ok(Manifest {
            out: field("out"),
            keyer: field("keyer"),
            files,
            redirected: strings(value.get("redirected")),
            overrides: value
                .get("overrides")
                .map(Value::attrs)
                .unwrap_or_default()
                .iter()
                .map(|(file, map)| (file.clone(), string_map(map)))
                .collect(),
            expansions: records(value, "expansions")
                .iter()
                .filter_map(|e| {
                    Some(Expansion {
                        file: file_of(e),
                        source: e.get("source")?.string()?.to_string(),
                        modules: strings(e.get("modules")),
                    })
                })
                .collect(),
            removed: records(value, "removed")
                .iter()
                .filter_map(|r| {
                    Some(Removed {
                        file: file_of(r),
                        source: r.get("source")?.string()?.to_string(),
                        before: r.get("before").and_then(Value::string).map(str::to_string),
                    })
                })
                .collect(),
            module_values: records(value, "module_values")
                .iter()
                .filter_map(|r| Some((file_of(r), r.get("attr")?.string()?.to_string())))
                .collect(),
            verbatim: value.get("verbatim").map(string_map).unwrap_or_default(),
            entry,
        })
    }
}

struct Splitter<'a> {
    source: &'a str,
    manifest: &'a Manifest,
    out_dir: &'a str,
    file: &'a str,
    dir: &'a str,
    overrides: Option<&'a HashMap<String, String>>,
    edits: Vec<(TextRange, Mapped)>,
    handled: HashSet<u32>,
    skip: HashSet<u32>,
    hole: Option<TextRange>,
    error: Option<String>,
}

impl<'a> Splitter<'a> {
    fn new(source: &'a str, manifest: &'a Manifest, out_dir: &'a str, file: &'a str) -> Self {
        Splitter {
            source,
            manifest,
            out_dir,
            file,
            dir: parent(file),
            overrides: manifest.overrides.get(file),
            edits: Vec::new(),
            handled: HashSet::new(),
            skip: HashSet::new(),
            hole: None,
            error: None,
        }
    }

    fn restore(&self, canonical: String) -> String {
        self.overrides
            .and_then(|map| map.get(&canonical))
            .cloned()
            .unwrap_or(canonical)
    }

    fn target(&self, token: &SyntaxToken) -> Option<&'a str> {
        let manifest = self.manifest;
        (token.kind() == K::Ident)
            .then(|| manifest.files.get(token.text(self.source)))
            .flatten()
            .map(String::as_str)
    }

    fn module(&self, target: &str) -> String {
        self.restore(module_literal(self.dir, target))
    }

    fn import(&self, target: &str, paren: bool) -> String {
        let text = self.restore(format!("import {}", module_literal(self.dir, target)));
        if paren {
            format!("({text})")
        } else {
            text
        }
    }

    fn keyed(&self, node: &SyntaxNode) -> Option<&'a str> {
        let keyer = self.manifest.keyer.as_deref()?;
        let Expr::Apply(outer) = ParenExpr::cast(node)?.expr()? else {
            return None;
        };
        let Expr::Apply(inner) = outer.func()? else {
            return None;
        };
        let Expr::Ident(func) = inner.func()? else {
            return None;
        };
        if func.text(self.source) != keyer || !matches!(inner.arg()?, Expr::String(_)) {
            return None;
        }
        let Expr::Ident(name) = outer.arg()? else {
            return None;
        };
        self.target(name)
    }

    fn element(&self, item: Expr<'_>) -> Option<String> {
        match item {
            Expr::Ident(token) => self.target(token).map(str::to_string),
            Expr::Paren(paren) => self.keyed(paren.syntax()).map(str::to_string),
            Expr::Path(token) => {
                let text = token.text(self.source);
                is_relative(text)
                    .then(|| join(self.out_dir, text))
                    .flatten()
            }
            _ => None,
        }
    }

    fn edit(&mut self, range: TextRange, text: String) {
        self.edits.push((range, Mapped::inserted(text, range)));
    }

    fn replace(&mut self, token: &SyntaxToken, text: String) {
        self.handled.insert(token.range().start());
        self.edit(token.range(), text);
    }

    fn path(&mut self, token: &SyntaxToken) {
        let text = token.text(self.source);
        if !is_relative(text) {
            return;
        }
        match join(self.out_dir, text) {
            Some(resolved) => {
                let restored = self.restore(relative_literal(self.dir, &resolved));
                self.edit(token.range(), restored);
            }
            None => {
                self.error
                    .get_or_insert_with(|| format!("path {text} escapes the root"));
            }
        }
    }

    fn module_value(&mut self, node: &'a SyntaxNode) {
        let Some(binding) = Binding::cast(node) else {
            return;
        };
        let (Some(path), Some(Expr::Ident(token))) = (binding.attrpath(), binding.value()) else {
            return;
        };
        let range = path.syntax().content_range();
        let attr = &self.source[range.start() as usize..range.end() as usize];
        let listed = self
            .manifest
            .module_values
            .iter()
            .any(|(file, name)| file == self.file && name == attr);
        if listed {
            if let Some(target) = self.target(token) {
                let text = self.module(target);
                self.replace(token, text);
            }
        }
    }

    fn visit(&mut self, node: &'a SyntaxNode) -> bool {
        match node.kind() {
            K::ParenExpr => {
                if let Some(target) = self.keyed(node) {
                    let text = self.module(target);
                    self.edit(node.content_range(), text);
                    return true;
                }
            }
            K::Binding => self.module_value(node),
            K::ListExpr => {
                if self.skip.contains(&node.content_range().start()) {
                    return true;
                }
                for item in ListExpr::cast(node).into_iter().flat_map(|l| l.items()) {
                    if let Expr::Ident(token) = item {
                        if let Some(target) = self.target(token) {
                            let text = self.module(target);
                            self.replace(token, text);
                        }
                    }
                }
            }
            K::ApplyExpr => {
                let Some(apply) = ApplyExpr::cast(node) else {
                    return false;
                };
                if let (Some(func), Some(Expr::Ident(token))) = (apply.func(), apply.arg()) {
                    if let Some(target) = self.target(token) {
                        let text = if is_call_package(func, self.source) {
                            self.module(target)
                        } else {
                            self.import(target, true)
                        };
                        self.replace(token, text);
                    }
                }
            }
            K::SelectExpr | K::HasAttrExpr | K::UnaryExpr => {
                let operand = SelectExpr::cast(node)
                    .and_then(|e| e.base())
                    .or_else(|| HasAttrExpr::cast(node).and_then(|e| e.base()))
                    .or_else(|| UnaryExpr::cast(node).and_then(|e| e.operand()));
                if let Some(Expr::Ident(token)) = operand {
                    if let Some(target) = self.target(token) {
                        let text = self.import(target, true);
                        self.replace(token, text);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn walk(&mut self, node: &'a SyntaxNode) {
        if self.hole == Some(node.content_range()) && node.kind() == K::LetExpr {
            return;
        }
        if self.visit(node) {
            return;
        }
        let mut param = node.kind() == K::LambdaExpr;
        let mut default = false;
        for child in node.children() {
            match child {
                NodeOrToken::Node(n) => self.walk(n),
                NodeOrToken::Token(t) => match t.kind() {
                    K::Colon => param = false,
                    K::Question => default = true,
                    K::Comma => default = false,
                    K::Ident => {
                        let binder = param
                            || node.kind() == K::Formals && !default
                            || matches!(node.kind(), K::Attrpath | K::InheritStmt);
                        if !binder && !self.handled.contains(&t.range().start()) {
                            if let Some(target) = self.target(t) {
                                let text = self.import(target, false);
                                self.replace(t, text);
                            }
                        }
                    }
                    K::Path => self.path(t),
                    _ => {}
                },
            }
        }
    }

    fn walk_value(&mut self, value: Expr<'a>) {
        match value {
            Expr::Ident(token) => {
                if let Some(target) = self.target(token) {
                    let text = self.import(target, false);
                    self.replace(token, text);
                }
            }
            Expr::Path(token) => self.path(token),
            Expr::Int(_) | Expr::Float(_) | Expr::SearchPath(_) | Expr::Uri(_) => {}
            other => {
                if let Some(node) = expr_node(other) {
                    self.walk(node);
                }
            }
        }
    }

    fn restore_expansions(&mut self, value: Expr<'a>, notes: &mut Vec<String>) -> usize {
        let Some(node) = expr_node(value) else {
            return 0;
        };
        let mut candidates: Vec<(TextRange, Vec<String>)> = node
            .descendants()
            .filter_map(ListExpr::cast)
            .filter_map(|list| {
                let items: Option<Vec<String>> =
                    list.items().map(|item| self.element(item)).collect();
                items
                    .filter(|items| !items.is_empty())
                    .map(|items| (list.syntax().content_range(), items))
            })
            .collect();
        let mut restored = 0;
        let (manifest, file) = (self.manifest, self.file);
        for expansion in manifest.expansions.iter().filter(|e| e.file == file) {
            let (source, modules) = (&expansion.source, &expansion.modules);
            let best = candidates
                .iter()
                .enumerate()
                .map(|(i, (_, items))| (i, items.iter().filter(|m| modules.contains(m)).count()))
                .filter(|(_, overlap)| *overlap > 0)
                .max_by_key(|(i, overlap)| (*overlap, std::cmp::Reverse(*i)));
            let Some((index, _)) = best else {
                notes.push(format!(
                    "`{source}` in {}: no module list matches it; not restored",
                    self.file
                ));
                continue;
            };
            let (range, items) = candidates.remove(index);
            if items != *modules {
                notes.push(format!(
                    "the list `{source}` produced in {} was edited (items added, removed or reordered); kept as an explicit list instead of restoring the call",
                    self.file
                ));
                continue;
            }
            self.skip.insert(range.start());
            self.edit(range, source.clone());
            restored += 1;
        }
        restored
    }

    fn restore_removed(&mut self, value: Expr<'a>) -> Result<(), String> {
        let manifest = self.manifest;
        let removed: Vec<&Removed> = manifest
            .removed
            .iter()
            .filter(|r| r.file == self.file)
            .collect();
        if removed.is_empty() {
            return Ok(());
        }
        let bindings: Vec<Binding> = let_of(value)
            .and_then(|(_, let_expr)| let_expr.bindings())
            .ok_or_else(|| {
                format!(
                    "{} is no longer `[ARG:] let ... in BODY`; removed bindings cannot be restored",
                    self.file
                )
            })?
            .items()
            .filter_map(|item| match item {
                AttrItem::Binding(binding) => Some(binding),
                AttrItem::Inherit(_) => None,
            })
            .collect();
        let last = bindings
            .last()
            .ok_or_else(|| format!("the let of {} has no bindings", self.file))?;
        for Removed { source, before, .. } in removed {
            let next = before.as_deref().and_then(|name| {
                bindings
                    .iter()
                    .find(|b| binding_name(**b, self.source) == Some(name))
            });
            let (at, text) = if let Some(binding) = next {
                let start = binding.syntax().content_range().start();
                (start, format!("{source}\n{}", indent(self.source, start)))
            } else {
                let end = last.syntax().content_range().end();
                let start = last.syntax().content_range().start();
                (end, format!("\n{}{source}", indent(self.source, start)))
            };
            self.edit(TextRange::new(at, at), text);
        }
        Ok(())
    }

    fn render(&mut self, range: TextRange) -> Mapped {
        let mut edits = std::mem::take(&mut self.edits);
        edits.sort_by_key(|(r, _)| (r.start(), r.end()));
        let mut out = Mapped::default();
        let mut cursor = range.start();
        for (r, piece) in edits {
            out.copy(self.source, TextRange::new(cursor, r.start()));
            out.append(piece);
            cursor = r.end();
        }
        out.copy(self.source, TextRange::new(cursor, range.end()));
        out
    }
}

fn expr_node(expr: Expr<'_>) -> Option<&SyntaxNode> {
    Some(match expr {
        Expr::Let(e) => e.syntax(),
        Expr::Lambda(e) => e.syntax(),
        Expr::Attrset(e) => e.syntax(),
        Expr::RecAttrset(e) => e.syntax(),
        Expr::List(e) => e.syntax(),
        Expr::Apply(e) => e.syntax(),
        Expr::Paren(e) => e.syntax(),
        Expr::With(e) => e.syntax(),
        Expr::Assert(e) => e.syntax(),
        Expr::If(e) => e.syntax(),
        Expr::Bin(e) => e.syntax(),
        Expr::Select(e) => e.syntax(),
        Expr::HasAttr(e) => e.syntax(),
        Expr::Unary(e) => e.syntax(),
        Expr::String(e) => e.syntax(),
        Expr::IndString(e) => e.syntax(),
        Expr::Ident(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Path(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_) => return None,
    })
}

fn indent(source: &str, at: u32) -> &str {
    let before = &source[..at as usize];
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[start..];
    &line[..line.len() - line.trim_start().len()]
}

fn seq_parts<'a>(body: Expr<'a>, source: &'a str) -> Option<(&'a str, Expr<'a>)> {
    let mut body = body;
    while let Expr::Paren(paren) = body {
        body = paren.expr()?;
    }
    let Expr::Apply(outer) = body else {
        return None;
    };
    let Expr::Apply(inner) = outer.func()? else {
        return None;
    };
    if inner.func()?.content_text(source) != "builtins.seq" {
        return None;
    }
    let Expr::Ident(token) = inner.arg()? else {
        return None;
    };
    Some((token.text(source), outer.arg()?))
}

fn merge_let<'a>(tree: &'a SyntaxNode, source: &str) -> Option<(LetExpr<'a>, bool)> {
    match Root::cast(tree)?.expr()? {
        Expr::Let(top) => Some((top, false)),
        Expr::Attrset(_) => match outputs_binding(tree, source)?.value()? {
            Expr::Let(top) => Some((top, true)),
            Expr::Lambda(lambda) => match lambda.body()? {
                Expr::Let(top) => Some((top, true)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn redirect(
    source: &str,
    file: &str,
    merged: &str,
    entry: &str,
    overrides: Option<&HashMap<String, String>>,
) -> Option<String> {
    let tree = parse(source);
    let mut edits = Vec::new();
    let mut stack = vec![&tree];
    while let Some(node) = stack.pop() {
        for child in node.children() {
            match child {
                NodeOrToken::Node(n) => stack.push(n),
                NodeOrToken::Token(t) if t.kind() == K::Path => {
                    let text = t.text(source);
                    if is_relative(text) && join(parent(file), text).as_deref() == Some(merged) {
                        let canonical = module_literal(parent(file), entry);
                        let restored = overrides
                            .and_then(|map| map.get(&canonical))
                            .cloned()
                            .unwrap_or(canonical);
                        edits.push((t.range(), restored));
                    }
                }
                NodeOrToken::Token(_) => {}
            }
        }
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(r, _)| r.start());
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    for (range, text) in edits {
        out.push_str(&source[cursor..range.start() as usize]);
        out.push_str(&text);
        cursor = range.end() as usize;
    }
    out.push_str(&source[cursor..]);
    Some(out)
}

pub(crate) struct Restored {
    pub(crate) entry: String,
    pub(crate) out: Option<String>,
    pub(crate) files: Vec<(String, Mapped)>,
    redirected: Vec<String>,
    overrides: HashMap<String, HashMap<String, String>>,
    verbatim: HashMap<String, String>,
    expansions: usize,
    notes: Vec<String>,
}

fn restore_shell(
    tree: &SyntaxNode,
    top: LetExpr<'_>,
    body: Expr<'_>,
    source: &str,
    manifest: &Manifest,
    out_dir: &str,
) -> Result<Mapped, String> {
    let inner = match body {
        Expr::Paren(paren) => paren.expr().ok_or("the merged outputs body is empty")?,
        other => other,
    };
    let entry = manifest.entry.as_str();
    let mut value = Splitter::new(source, manifest, out_dir, entry);
    value.walk_value(inner);
    let restored = value.render(inner.content_range());
    if let Some(error) = value.error {
        return Err(format!("{entry}: {error}"));
    }
    let hole = top.syntax().content_range();
    let mut shell = Splitter::new(source, manifest, out_dir, entry);
    shell.hole = Some(hole);
    shell.walk(tree);
    if let Some(error) = shell.error {
        return Err(format!("{entry}: {error}"));
    }
    shell.edits.push((hole, restored));
    Ok(shell.render(TextRange::new(0, offset(source.len()))))
}

pub(crate) fn restore(source: &str, out_dir: Option<&str>) -> Result<Option<Restored>, String> {
    if !source.contains("builtins.seq") {
        return Ok(None);
    }
    let tree = parse(source);
    if tree.error_nodes().next().is_some() {
        return Ok(None);
    }
    let Some((top, shell)) = merge_let(&tree, source) else {
        return Ok(None);
    };
    let Some((name, body)) = top.body().and_then(|body| seq_parts(body, source)) else {
        return Ok(None);
    };
    let bindings: Vec<(&str, Binding)> = top
        .bindings()
        .into_iter()
        .flat_map(|b| b.items())
        .filter_map(|item| match item {
            AttrItem::Binding(binding) => Some((binding_name(binding, source)?, binding)),
            AttrItem::Inherit(_) => None,
        })
        .collect();
    let Some(manifest_value) = bindings
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, b)| b.value())
    else {
        return Ok(None);
    };
    let Ok(manifest) = literal(manifest_value, source).and_then(|value| Manifest::read(&value))
    else {
        return Ok(None);
    };
    let out_dir = out_dir
        .or_else(|| manifest.out.as_deref().map(parent))
        .unwrap_or("");

    let mut notes = Vec::new();
    let mut files = Vec::new();
    let mut expansions = 0;
    for (binding_name, binding) in &bindings {
        if *binding_name == name || manifest.keyer.as_deref() == Some(*binding_name) {
            continue;
        }
        let path = manifest.files.get(*binding_name).ok_or_else(|| {
            format!("binding {binding_name} has no path in {name}.files; add one to split it out")
        })?;
        let (Some(value), Some(range)) = (binding.value(), binding_value_range(*binding)) else {
            return Err(format!("binding {binding_name} has no value"));
        };
        let mut splitter = Splitter::new(source, &manifest, out_dir, path);
        let restored = splitter.restore_expansions(value, &mut notes);
        if restored > 0 {
            splitter.restore_removed(value)?;
        }
        expansions += restored;
        splitter.walk_value(value);
        if let Some(error) = splitter.error {
            return Err(format!("{binding_name}: {error}"));
        }
        files.push((path.clone(), splitter.render(range).file_text()));
    }
    if shell {
        let text = restore_shell(&tree, top, body, source, &manifest, out_dir)?;
        files.push((manifest.entry.clone(), text.file_text()));
    }
    if !files.iter().any(|(path, _)| *path == manifest.entry) {
        return Err(format!("no binding holds the entry {}", manifest.entry));
    }
    Ok(Some(Restored {
        entry: manifest.entry,
        out: manifest.out,
        files,
        redirected: manifest.redirected,
        overrides: manifest.overrides,
        verbatim: manifest.verbatim,
        expansions,
        notes,
    }))
}

fn restore_verbatim(
    root: &std::path::Path,
    restored: &[(String, String)],
    verbatim: &HashMap<String, String>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let originals: Vec<(&str, &str)> = restored
        .iter()
        .filter_map(|(path, _)| Some((path.as_str(), verbatim.get(path)?.as_str())))
        .collect();
    let Some(formatted) = formatted_texts(root, &originals)? else {
        return Ok(());
    };
    for ((path, original), formatted) in originals.into_iter().zip(formatted) {
        let current =
            std::fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        if current == formatted {
            std::fs::write(root.join(path), original).map_err(|e| format!("{path}: {e}"))?;
        } else {
            notes.push(format!(
                "{path} was edited in the merged file; written as the flake formatter prints it"
            ));
        }
    }
    Ok(())
}

pub fn split(options: &SplitOptions) -> Result<SplitReport, String> {
    let root = std::fs::canonicalize(&options.root)
        .map_err(|e| format!("{}: {e}", options.root.display()))?;
    let merged = join("", &options.merged).ok_or("merged file escapes the root")?;
    let source =
        std::fs::read_to_string(root.join(&merged)).map_err(|e| format!("{merged}: {e}"))?;
    if parse(&source).error_nodes().next().is_some() {
        return Err(format!("{merged}: parse error"));
    }
    let restored = restore(&source, Some(parent(&merged)))
        .map_err(|e| format!("{merged}: {e}"))?
        .ok_or_else(|| {
            format!("{merged}: expected `let ... in builtins.seq MANIFEST ENTRY`, or a flake whose outputs is one")
        })?;
    let Restored {
        entry,
        files,
        redirected: referrers,
        overrides,
        verbatim,
        expansions,
        mut notes,
        ..
    } = restored;

    let mut redirected = Vec::new();
    for file in &referrers {
        let text = std::fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
        match redirect(&text, file, &merged, &entry, overrides.get(file)) {
            Some(text) => redirected.push((file.clone(), text)),
            None => notes.push(format!("{file} no longer refers to {merged}")),
        }
    }

    let restored: Vec<(String, String)> = files
        .into_iter()
        .map(|(path, mapped)| (path, mapped.text))
        .collect();
    if let Some((path, _)) = restored
        .iter()
        .find(|(path, _)| *path != merged && std::fs::symlink_metadata(root.join(path)).is_ok())
    {
        return Err(format!("{path} already exists; refusing to overwrite it"));
    }
    if !options.dry_run {
        for (path, text) in restored.iter().chain(&redirected) {
            let full = root.join(path);
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            let mut open = std::fs::OpenOptions::new();
            open.write(true);
            if *path == merged || !restored.iter().any(|(p, _)| p == path) {
                open.truncate(true);
            } else {
                open.create_new(true);
            }
            open.open(&full)
                .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
                .map_err(|e| format!("writing {path}: {e}"))?;
        }
        if !restored.iter().any(|(path, _)| *path == merged) {
            std::fs::remove_file(root.join(&merged))
                .map_err(|e| format!("removing {merged}: {e}"))?;
        }
        let touched: Vec<String> = restored
            .iter()
            .chain(&redirected)
            .map(|(path, _)| path.clone())
            .collect();
        if format_files(&root, &touched)? {
            restore_verbatim(&root, &restored, &verbatim, &mut notes)?;
        }
    }

    let mut restored: Vec<String> = restored.into_iter().map(|(path, _)| path).collect();
    restored.sort();
    Ok(SplitReport {
        restored,
        redirected: redirected.into_iter().map(|(path, _)| path).collect(),
        expansions,
        notes,
    })
}
