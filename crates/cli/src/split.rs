use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, Binding, Expr, HasAttrExpr, ListExpr,
    NodeOrToken, ParenExpr, Root, SelectExpr, StringPart, SyntaxKind as K, SyntaxNode, SyntaxToken,
    TextRange, UnaryExpr,
};

use crate::merge::{
    binding_name, binding_value_range, format_files, is_call_package, is_relative, join, let_of,
    module_literal, parent, relative_literal,
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

struct Manifest {
    entry: String,
    keyer: Option<String>,
    files: HashMap<String, String>,
    redirected: Vec<String>,
    overrides: HashMap<String, HashMap<String, String>>,
    expansions: Vec<(String, Vec<String>)>,
    removed: Vec<(String, Option<String>)>,
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

impl Manifest {
    fn read(value: &Value) -> Result<Manifest, String> {
        let field = |key: &str| value.get(key).and_then(Value::string).map(str::to_string);
        let entry = field("entry").ok_or("the merge manifest has no `entry` string")?;
        let files = value
            .get("files")
            .map(string_map)
            .ok_or("the merge manifest has no `files` set")?;
        Ok(Manifest {
            entry,
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
            expansions: value
                .get("expansions")
                .map(Value::list)
                .unwrap_or_default()
                .iter()
                .filter_map(|e| {
                    let source = e.get("source")?.string()?.to_string();
                    Some((source, strings(e.get("modules"))))
                })
                .collect(),
            removed: value
                .get("removed")
                .map(Value::list)
                .unwrap_or_default()
                .iter()
                .filter_map(|r| {
                    let source = r.get("source")?.string()?.to_string();
                    let before = r.get("before").and_then(Value::string).map(str::to_string);
                    Some((source, before))
                })
                .collect(),
        })
    }
}

struct Splitter<'a> {
    source: &'a str,
    manifest: &'a Manifest,
    out_dir: &'a str,
    dir: &'a str,
    overrides: Option<&'a HashMap<String, String>>,
    edits: Vec<(TextRange, String)>,
    handled: HashSet<u32>,
    skip: HashSet<u32>,
    error: Option<String>,
}

impl<'a> Splitter<'a> {
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

    fn replace(&mut self, token: &SyntaxToken, text: String) {
        self.handled.insert(token.range().start());
        self.edits.push((token.range(), text));
    }

    fn path(&mut self, token: &SyntaxToken) {
        let text = token.text(self.source);
        if !is_relative(text) {
            return;
        }
        match join(self.out_dir, text) {
            Some(resolved) => {
                let restored = self.restore(relative_literal(self.dir, &resolved));
                self.edits.push((token.range(), restored));
            }
            None => {
                self.error
                    .get_or_insert_with(|| format!("path {text} escapes the root"));
            }
        }
    }

    fn visit(&mut self, node: &'a SyntaxNode) -> bool {
        match node.kind() {
            K::ParenExpr => {
                if let Some(target) = self.keyed(node) {
                    let text = self.module(target);
                    self.edits.push((node.content_range(), text));
                    return true;
                }
            }
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
            Expr::Let(e) => self.walk(e.syntax()),
            Expr::With(e) => self.walk(e.syntax()),
            Expr::Assert(e) => self.walk(e.syntax()),
            Expr::If(e) => self.walk(e.syntax()),
            Expr::Attrset(e) => self.walk(e.syntax()),
            Expr::RecAttrset(e) => self.walk(e.syntax()),
            Expr::List(e) => self.walk(e.syntax()),
            Expr::Lambda(e) => self.walk(e.syntax()),
            Expr::Apply(e) => self.walk(e.syntax()),
            Expr::Unary(e) => self.walk(e.syntax()),
            Expr::Bin(e) => self.walk(e.syntax()),
            Expr::Select(e) => self.walk(e.syntax()),
            Expr::HasAttr(e) => self.walk(e.syntax()),
            Expr::String(e) => self.walk(e.syntax()),
            Expr::IndString(e) => self.walk(e.syntax()),
            Expr::Paren(e) => self.walk(e.syntax()),
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
        for (source, modules) in &self.manifest.expansions {
            let best = candidates
                .iter()
                .enumerate()
                .map(|(i, (_, items))| (i, items.iter().filter(|m| modules.contains(m)).count()))
                .filter(|(_, overlap)| *overlap > 0)
                .max_by_key(|(i, overlap)| (*overlap, std::cmp::Reverse(*i)));
            let Some((index, _)) = best else {
                notes.push(format!(
                    "`{source}`: no module list in the entry matches it; not restored"
                ));
                continue;
            };
            let (range, items) = candidates.remove(index);
            for item in items.iter().filter(|item| !modules.contains(item)) {
                notes.push(format!(
                    "{item} was added to the list `{source}` replaced; it is included only if that call discovers it"
                ));
            }
            for module in modules.iter().filter(|module| !items.contains(module)) {
                notes.push(format!(
                    "{module} was removed from the list `{source}` replaced; that call will discover it again"
                ));
            }
            self.skip.insert(range.start());
            self.edits.push((range, source.clone()));
            restored += 1;
        }
        restored
    }

    fn restore_removed(&mut self, value: Expr<'a>) -> Result<(), String> {
        if self.manifest.removed.is_empty() {
            return Ok(());
        }
        let bindings: Vec<Binding> = let_of(value)
            .and_then(|(_, let_expr)| let_expr.bindings())
            .ok_or("the entry is no longer `[ARG:] let ... in BODY`; removed bindings cannot be restored")?
            .items()
            .filter_map(|item| match item {
                AttrItem::Binding(binding) => Some(binding),
                AttrItem::Inherit(_) => None,
            })
            .collect();
        let last = bindings.last().ok_or("the entry let has no bindings")?;
        for (source, before) in &self.manifest.removed {
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
            self.edits.push((TextRange::new(at, at), text));
        }
        Ok(())
    }

    fn render(&mut self, range: TextRange) -> String {
        self.edits.sort_by_key(|(r, _)| (r.start(), r.end()));
        let mut out = String::new();
        let mut cursor = range.start() as usize;
        for (r, text) in &self.edits {
            out.push_str(&self.source[cursor..r.start() as usize]);
            out.push_str(text);
            cursor = r.end() as usize;
        }
        out.push_str(&self.source[cursor..range.end() as usize]);
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
        _ => return None,
    })
}

fn indent(source: &str, at: u32) -> &str {
    let before = &source[..at as usize];
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[start..];
    &line[..line.len() - line.trim_start().len()]
}

fn manifest_name<'s>(body: Expr<'_>, source: &'s str) -> Option<&'s str> {
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
    Some(token.text(source))
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

fn restore_bindings(
    bindings: &[(&str, Binding<'_>)],
    name: &str,
    manifest: &Manifest,
    source: &str,
    out_dir: &str,
    notes: &mut Vec<String>,
) -> Result<(Vec<(String, String)>, usize), String> {
    let mut restored = Vec::new();
    let mut expansions = 0;
    for (binding_name, binding) in bindings {
        if *binding_name == name || manifest.keyer.as_deref() == Some(*binding_name) {
            continue;
        }
        let path = manifest.files.get(*binding_name).ok_or_else(|| {
            format!("binding {binding_name} has no path in {name}.files; add one to split it out")
        })?;
        let (Some(value), Some(range)) = (binding.value(), binding_value_range(*binding)) else {
            return Err(format!("binding {binding_name} has no value"));
        };
        let mut splitter = Splitter {
            source,
            manifest,
            out_dir,
            dir: parent(path),
            overrides: manifest.overrides.get(path),
            edits: Vec::new(),
            handled: HashSet::new(),
            skip: HashSet::new(),
            error: None,
        };
        if *path == manifest.entry {
            expansions = splitter.restore_expansions(value, notes);
            splitter.restore_removed(value)?;
        }
        splitter.walk_value(value);
        if let Some(error) = splitter.error {
            return Err(format!("{binding_name}: {error}"));
        }
        let text = splitter.render(range);
        restored.push((path.clone(), format!("{}\n", text.trim())));
    }
    Ok((restored, expansions))
}

pub fn split(options: &SplitOptions) -> Result<SplitReport, String> {
    let root = std::fs::canonicalize(&options.root)
        .map_err(|e| format!("{}: {e}", options.root.display()))?;
    let merged = join("", &options.merged).ok_or("merged file escapes the root")?;
    let source =
        std::fs::read_to_string(root.join(&merged)).map_err(|e| format!("{merged}: {e}"))?;
    let tree = parse(&source);
    if tree.error_nodes().next().is_some() {
        return Err(format!("{merged}: parse error"));
    }
    let Some(Expr::Let(top)) = Root::cast(&tree).and_then(|root| root.expr()) else {
        return Err(format!("{merged}: expected a top-level `let ... in ...`"));
    };
    let name = top
        .body()
        .and_then(|body| manifest_name(body, &source))
        .ok_or_else(|| format!("{merged}: expected the body `builtins.seq MANIFEST ENTRY`"))?;
    let bindings: Vec<(&str, Binding)> = top
        .bindings()
        .into_iter()
        .flat_map(|b| b.items())
        .filter_map(|item| match item {
            AttrItem::Binding(binding) => Some((binding_name(binding, &source)?, binding)),
            AttrItem::Inherit(_) => None,
        })
        .collect();
    let manifest_value = bindings
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, b)| b.value())
        .ok_or_else(|| format!("{merged}: no binding {name}"))?;
    let manifest = Manifest::read(&literal(manifest_value, &source)?)?;
    let out_dir = parent(&merged);

    let mut notes = Vec::new();
    let (restored, expansions) =
        restore_bindings(&bindings, name, &manifest, &source, out_dir, &mut notes)?;
    if !restored.iter().any(|(path, _)| *path == manifest.entry) {
        return Err(format!(
            "{merged}: no binding holds the entry {}",
            manifest.entry
        ));
    }
    if let Some((path, _)) = restored.iter().find(|(path, _)| root.join(path).exists()) {
        return Err(format!("{path} already exists; split would overwrite it"));
    }

    let mut redirected = Vec::new();
    for file in &manifest.redirected {
        let text = std::fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
        match redirect(
            &text,
            file,
            &merged,
            &manifest.entry,
            manifest.overrides.get(file),
        ) {
            Some(text) => redirected.push((file.clone(), text)),
            None => notes.push(format!("{file} no longer refers to {merged}")),
        }
    }

    if let Some((path, _)) = restored
        .iter()
        .find(|(path, _)| std::fs::symlink_metadata(root.join(path)).is_ok())
    {
        return Err(format!("{path} already exists; refusing to overwrite it"));
    }
    if !options.dry_run {
        for (path, text) in restored.iter().chain(&redirected) {
            let full = root.join(path);
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            let write = if restored.iter().any(|(p, _)| p == path) {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&full)
            } else {
                std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&full)
            };
            write
                .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
                .map_err(|e| format!("writing {path}: {e}"))?;
        }
        std::fs::remove_file(root.join(&merged)).map_err(|e| format!("removing {merged}: {e}"))?;
        let touched: Vec<String> = restored
            .iter()
            .chain(&redirected)
            .map(|(path, _)| path.clone())
            .collect();
        format_files(&root, &touched)?;
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
