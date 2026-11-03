use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use strictix_core::json::JsonValue;
use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, BinExpr, Binding, Expr, LetExpr, ListExpr,
    NodeOrToken, Root, StringPart, SyntaxKind as K, SyntaxNode, TextRange,
};

pub struct MergeOptions {
    pub root: PathBuf,
    pub entry: String,
    pub out: String,
    pub expand: Vec<String>,
    pub dry_run: bool,
}

pub struct MergeReport {
    pub inlined: Vec<String>,
    pub kept: Vec<(String, String)>,
    pub rewritten: Vec<String>,
    pub keyed_modules: Vec<String>,
    pub expansions: Vec<(String, usize)>,
    pub partial_paths: Vec<String>,
    pub dropped_bindings: Vec<String>,
    pub untouched: usize,
    pub unreached: Vec<String>,
    pub out_lines: usize,
    pub formatted: bool,
    pub verbatim: Vec<String>,
}

enum Edit {
    Import { target: String, literal: String },
    Package { target: String, literal: String },
    Module { target: String, literal: String },
    Rebase { target: String, whole: bool },
    Expand(usize),
    Remove,
}

struct Site {
    range: TextRange,
    path: TextRange,
    edit: Edit,
}

struct Dropped {
    file: String,
    name: String,
    source: String,
    before: Option<String>,
}

struct Analysis {
    source: String,
    sites: Vec<Site>,
    strings: Vec<String>,
    idents: HashMap<String, usize>,
    expansions: Vec<String>,
    named: Vec<String>,
    module_values: Vec<String>,
}

impl Analysis {
    fn text(&self, range: TextRange) -> &str {
        &self.source[range.start() as usize..range.end() as usize]
    }
}

struct Analyzer<'a> {
    root: &'a Path,
    dir: &'a str,
    source: &'a str,
    expand: &'a [String],
    consumed: HashSet<u32>,
    partial: HashSet<u32>,
    listed: HashMap<String, usize>,
    let_values: Vec<(String, String, TextRange, String, String)>,
    out: Analysis,
    error: Option<String>,
}

pub(crate) fn join(dir: &str, literal: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for segment in literal.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

pub(crate) fn parent(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

pub(crate) fn relative_literal(from_dir: &str, target: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let rest = to[common..].join("/");
    let ups = from.len() - common;
    match (ups, rest.is_empty()) {
        (0, true) => "./.".to_string(),
        (0, false) => format!("./{rest}"),
        (_, true) => "../".repeat(ups).trim_end_matches('/').to_string(),
        (_, false) => format!("{}{rest}", "../".repeat(ups)),
    }
}

pub(crate) fn module_literal(from_dir: &str, target: &str) -> String {
    let dir = if target == "default.nix" {
        ""
    } else {
        target.strip_suffix("/default.nix").unwrap_or(target)
    };
    relative_literal(from_dir, dir)
}

fn is_nix(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|ext| ext == "nix")
}

fn nix_target(root: &Path, rel: &str) -> Option<String> {
    let full = root.join(rel);
    if is_nix(rel) && full.is_file() {
        Some(rel.to_string())
    } else if full.join("default.nix").is_file() {
        Some(if rel.is_empty() {
            "default.nix".to_string()
        } else {
            format!("{rel}/default.nix")
        })
    } else {
        None
    }
}

pub(crate) fn is_relative(literal: &str) -> bool {
    literal.starts_with("./") || literal.starts_with("../")
}

pub(crate) fn is_call_package(func: Expr<'_>, source: &str) -> bool {
    let text = func.content_text(source);
    matches!(func, Expr::Ident(_)) && text == "callPackage"
        || matches!(func, Expr::Select(_)) && text.ends_with(".callPackage")
}

pub(crate) fn binding_name<'s>(binding: Binding<'_>, source: &'s str) -> Option<&'s str> {
    let path = binding.attrpath()?;
    let mut elements = path.elements();
    let (Some(AttrName::Ident(token)), None) = (elements.next(), elements.next()) else {
        return None;
    };
    Some(token.text(source))
}

pub(crate) fn binding_value_range(binding: Binding<'_>) -> Option<TextRange> {
    let children = binding.syntax().children();
    let start = children
        .iter()
        .find(|c| c.kind() == K::Assign)?
        .range()
        .end();
    let end = children
        .iter()
        .rev()
        .find(|c| c.kind() == K::Semicolon)?
        .range()
        .start();
    Some(TextRange::new(start, end))
}

impl<'a> Analyzer<'a> {
    fn text(&self, range: TextRange) -> &'a str {
        &self.source[range.start() as usize..range.end() as usize]
    }

    fn resolve(&mut self, literal: &str) -> Option<String> {
        let joined = join(self.dir, literal);
        if joined.is_none() && self.error.is_none() {
            self.error = Some(format!("path {literal} escapes the merge root"));
        }
        joined
    }

    fn target_of(&mut self, expr: Option<Expr<'a>>) -> Option<(TextRange, String, String)> {
        let Some(Expr::Path(token)) = expr else {
            return None;
        };
        let literal = self.text(token.range());
        if !is_relative(literal) {
            return None;
        }
        let rel = self.resolve(literal)?;
        nix_target(self.root, &rel).map(|target| (token.range(), target, rel))
    }

    fn push(&mut self, range: TextRange, path: TextRange, edit: Edit) {
        self.consumed.insert(path.start());
        self.out.sites.push(Site { range, path, edit });
    }

    fn visit_apply(&mut self, node: &'a SyntaxNode) -> bool {
        let Some(apply) = ApplyExpr::cast(node) else {
            return false;
        };
        let Some(func) = apply.func() else {
            return false;
        };
        let func_text = func.content_text(self.source);
        if matches!(func, Expr::Ident(_)) && self.expand.iter().any(|name| name == func_text) {
            let range = node.content_range();
            let index = self.out.expansions.len();
            self.out.expansions.push(self.text(range).to_string());
            self.out.sites.push(Site {
                range,
                path: range,
                edit: Edit::Expand(index),
            });
            return true;
        }
        let is_import = matches!(func, Expr::Ident(_)) && func_text == "import"
            || matches!(func, Expr::Select(_)) && func_text == "builtins.import";
        let is_package = is_call_package(func, self.source);
        if !(is_import || is_package) {
            return false;
        }
        let Some((path, target, literal)) = self.target_of(apply.arg()) else {
            return false;
        };
        if is_import {
            self.push(node.content_range(), path, Edit::Import { target, literal });
            true
        } else {
            self.push(path, path, Edit::Package { target, literal });
            false
        }
    }

    fn visit_list(&mut self, node: &'a SyntaxNode) {
        let Some(list) = ListExpr::cast(node) else {
            return;
        };
        for item in list.items() {
            if let Expr::Ident(token) = item {
                *self
                    .listed
                    .entry(self.text(token.range()).to_string())
                    .or_default() += 1;
            }
            if let Some((path, target, literal)) = self.target_of(Some(item)) {
                self.push(path, path, Edit::Module { target, literal });
            }
        }
    }

    fn visit_binding(&mut self, node: &'a SyntaxNode, in_let: bool) {
        let Some(binding) = Binding::cast(node) else {
            return;
        };
        let Some(attrpath) = binding.attrpath() else {
            return;
        };
        let Some((path, target, literal)) = self.target_of(binding.value()) else {
            return;
        };
        let attr = self.text(attrpath.syntax().content_range()).to_string();
        let module_attr = attrpath.elements().any(|element| {
            let name = match element {
                AttrName::Ident(token) => self.text(token.range()),
                _ => return false,
            };
            name == "module" || name.ends_with("Modules") || name == "modules"
        });
        if module_attr {
            self.out.module_values.push(attr);
            self.push(path, path, Edit::Module { target, literal });
        } else if in_let {
            if let Some(name) = binding_name(binding, self.source) {
                self.let_values
                    .push((name.to_string(), attr, path, target, literal));
            }
        }
    }

    fn settle_let_values(&mut self) {
        for (name, attr, path, target, literal) in std::mem::take(&mut self.let_values) {
            let listed = self.listed.get(&name).copied().unwrap_or(0);
            if listed == 0 || self.out.idents.get(&name) != Some(&(listed + 1)) {
                continue;
            }
            self.out.sites.retain(|site| site.range != path);
            self.out.module_values.push(attr);
            self.out.sites.push(Site {
                range: path,
                path,
                edit: Edit::Module { target, literal },
            });
        }
    }

    fn visit_bin(&mut self, node: &'a SyntaxNode) {
        let Some(bin) = BinExpr::cast(node) else {
            return;
        };
        if bin.op() != Some(K::Plus) {
            return;
        }
        let (Some(Expr::Path(path)), Some(rhs)) = (bin.lhs(), bin.rhs()) else {
            return;
        };
        let last = match rhs {
            Expr::String(s) => s.parts().last(),
            Expr::IndString(s) => s.parts().last(),
            _ => return,
        };
        if let Some(StringPart::Content(token)) = last {
            if !is_nix(self.text(token.range())) {
                self.partial.insert(path.range().start());
            }
        }
    }

    fn visit_path(&mut self, range: TextRange) {
        if self.consumed.contains(&range.start()) {
            return;
        }
        let literal = self.text(range);
        if !is_relative(literal) {
            return;
        }
        if let Some(target) = self.resolve(literal) {
            let whole = !self.partial.contains(&range.start());
            self.out.sites.push(Site {
                range,
                path: range,
                edit: Edit::Rebase { target, whole },
            });
        }
    }

    fn walk(&mut self, node: &'a SyntaxNode) {
        match node.kind() {
            K::ApplyExpr => {
                if self.visit_apply(node) {
                    return;
                }
            }
            K::ListExpr => self.visit_list(node),
            K::BinExpr => self.visit_bin(node),
            _ => {}
        }
        if matches!(
            node.kind(),
            K::LetBindings | K::AttrsetExpr | K::RecAttrsetExpr
        ) {
            for child in node.child_nodes().filter(|n| n.kind() == K::Binding) {
                self.visit_binding(child, node.kind() == K::LetBindings);
            }
        }
        for child in node.children() {
            match child {
                NodeOrToken::Node(n) => self.walk(n),
                NodeOrToken::Token(t) => match t.kind() {
                    K::Ident => {
                        *self
                            .out
                            .idents
                            .entry(self.text(t.range()).to_string())
                            .or_default() += 1;
                    }
                    K::StringContent => self.out.strings.push(self.text(t.range()).to_string()),
                    K::Path => self.visit_path(t.range()),
                    _ => {}
                },
            }
        }
    }
}

fn analyze(root: &Path, rel: &str, expand: &[String]) -> Result<Analysis, String> {
    let source = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    let tree = parse(&source);
    if tree.error_nodes().next().is_some() {
        return Err(format!("{rel}: parse error"));
    }
    let bound = root_let_names(&tree, &source);
    let expand: Vec<String> = expand
        .iter()
        .filter(|name| bound.contains(name.as_str()))
        .cloned()
        .collect();
    let mut analyzer = Analyzer {
        root,
        dir: parent(rel),
        source: &source,
        expand: &expand,
        consumed: HashSet::new(),
        partial: HashSet::new(),
        listed: HashMap::new(),
        let_values: Vec::new(),
        out: Analysis {
            source: String::new(),
            sites: Vec::new(),
            strings: Vec::new(),
            idents: HashMap::new(),
            expansions: Vec::new(),
            named: Vec::new(),
            module_values: Vec::new(),
        },
        error: None,
    };
    analyzer.walk(&tree);
    analyzer.settle_let_values();
    if let Some(error) = analyzer.error {
        return Err(format!("{rel}: {error}"));
    }
    let mut out = analyzer.out;
    out.sites.sort_by_key(|site| site.range.start());
    out.source = source;
    Ok(out)
}

fn collect_nix(root: &Path, rel: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
    let dir = root.join(rel);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect_nix(root, &child, out)?;
        } else if kind.is_file() && is_nix(&name) {
            out.insert(child);
        }
    }
    Ok(())
}

pub(crate) fn let_of(expr: Expr<'_>) -> Option<(bool, LetExpr<'_>)> {
    let (is_lambda, inner) = match expr {
        Expr::Lambda(lambda) => (true, lambda.body()?),
        other => (false, other),
    };
    let Expr::Let(let_expr) = inner else {
        return None;
    };
    Some((is_lambda, let_expr))
}

fn entry_let(tree: &SyntaxNode) -> Option<(bool, LetExpr<'_>)> {
    let_of(Root::cast(tree)?.expr()?)
}

fn root_let_names<'s>(tree: &SyntaxNode, source: &'s str) -> HashSet<&'s str> {
    entry_let(tree)
        .and_then(|(_, let_expr)| let_expr.bindings())
        .into_iter()
        .flat_map(|bindings| bindings.items())
        .filter_map(|item| match item {
            AttrItem::Binding(binding) => binding_name(binding, source),
            AttrItem::Inherit(_) => None,
        })
        .collect()
}

pub(crate) fn outputs_binding<'a>(tree: &'a SyntaxNode, source: &str) -> Option<Binding<'a>> {
    let Expr::Attrset(set) = Root::cast(tree)?.expr()? else {
        return None;
    };
    set.items().find_map(|item| match item {
        AttrItem::Binding(binding) if binding_name(binding, source) == Some("outputs") => {
            Some(binding)
        }
        _ => None,
    })
}

fn flake_outputs(source: &str) -> Option<TextRange> {
    let tree = parse(source);
    match outputs_binding(&tree, source)?.value()? {
        Expr::Lambda(lambda) => Some(lambda.body()?.content_range()),
        value => Some(value.content_range()),
    }
}

fn line_start(source: &str, at: usize) -> usize {
    let before = &source[..at];
    let trimmed = before.trim_end_matches([' ', '\t']);
    if trimmed.ends_with('\n') {
        trimmed.len() - 1
    } else {
        at
    }
}

fn drop_dead_expanders(file: &str, analysis: &mut Analysis, names: &[String]) -> Vec<Dropped> {
    let tree = parse(&analysis.source);
    let Some(bindings) = entry_let(&tree).and_then(|(_, let_expr)| let_expr.bindings()) else {
        return Vec::new();
    };
    let items: Vec<Binding> = bindings
        .items()
        .filter_map(|item| match item {
            AttrItem::Binding(binding) => Some(binding),
            AttrItem::Inherit(_) => None,
        })
        .collect();
    let mut dropped = Vec::new();
    for (index, binding) in items.iter().enumerate() {
        let Some(name) = binding_name(*binding, &analysis.source) else {
            continue;
        };
        if !names.iter().any(|n| n == name) || analysis.idents.get(name) != Some(&1) {
            continue;
        }
        let content = binding.syntax().content_range();
        let start = line_start(&analysis.source, content.start() as usize);
        let range = TextRange::new(
            u32::try_from(start).unwrap_or(content.start()),
            content.end(),
        );
        let before = items[index + 1..]
            .iter()
            .find_map(|next| binding_name(*next, &analysis.source))
            .map(str::to_string);
        dropped.push(Dropped {
            file: file.to_string(),
            name: name.to_string(),
            source: analysis.text(content).to_string(),
            before,
        });
        analysis
            .sites
            .retain(|site| site.range.end() <= range.start() || site.range.start() >= range.end());
        analysis.sites.push(Site {
            range,
            path: range,
            edit: Edit::Remove,
        });
    }
    analysis.sites.sort_by_key(|site| site.range.start());
    dropped
}

fn evaluate_expansions(
    root: &Path,
    file: &str,
    analysis: &Analysis,
) -> Result<Vec<Vec<String>>, String> {
    if analysis.expansions.is_empty() {
        return Ok(Vec::new());
    }
    let tree = parse(&analysis.source);
    let (is_lambda, body) = entry_let(&tree)
        .and_then(|(is_lambda, let_expr)| Some((is_lambda, let_expr.body()?.content_range())))
        .ok_or_else(|| format!("{file}: --expand needs `[ARG:] let ... in BODY`"))?;
    let mut calls = String::new();
    for call in &analysis.expansions {
        let _ = writeln!(calls, "(map toString ({call}))");
    }
    let source = &analysis.source;
    let scoped = format!(
        "{}[\n{calls}]{}",
        &source[..body.start() as usize],
        &source[body.end() as usize..]
    );
    let flake_ref = if root.join(".git").exists() {
        format!("git+file://{}", root.display())
    } else {
        format!("path:{}", root.display())
    };
    let expr = if is_lambda {
        format!(
            "let flake = builtins.getFlake \"{flake_ref}\"; in ({scoped}) (flake.inputs // {{ self = flake; }})"
        )
    } else {
        scoped
    };
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", &expr])
        .current_dir(root.join(parent(file)))
        .output()
        .map_err(|e| format!("running nix eval: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "evaluating the expansions in {file} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let json = JsonValue::parse(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("parsing nix eval output: {}", e.message))?;
    let prefix = format!("{}/", root.display());
    let lists = json.as_array().ok_or("expansion result is not a list")?;
    lists
        .iter()
        .map(|list| {
            list.as_array()
                .ok_or_else(|| "expansion result is not a list of lists".to_string())?
                .iter()
                .map(|item| {
                    let path = item.as_str().ok_or("expansion element is not a path")?;
                    let rel = path
                        .strip_prefix(&prefix)
                        .ok_or_else(|| format!("expanded path {path} is outside the merge root"))?;
                    nix_target(root, rel)
                        .ok_or_else(|| format!("expanded path {path} is not a .nix module"))
                })
                .collect()
        })
        .collect()
}

fn formatter_system(root: &Path) -> Result<Option<String>, String> {
    if !root.join("flake.nix").is_file() {
        return Ok(None);
    }
    let flake = nix_string(&format!("path:{}", root.display()));
    let probe = Command::new("nix")
        .args([
            "eval",
            "--impure",
            "--raw",
            "--expr",
            &format!(
                "let flake = builtins.getFlake {flake}; system = builtins.currentSystem; in if flake ? formatter && flake.formatter ? ${{system}} then system else \"\""
            ),
        ])
        .output();
    match probe {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("running nix eval: {e}")),
        Ok(output) if !output.status.success() => Err(format!(
            "evaluating the flake's formatter failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )),
        Ok(output) => {
            let system = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok((!system.is_empty()).then_some(system))
        }
    }
}

fn run_formatter(root: &Path, system: &str, files: &[String]) -> Result<(), String> {
    let output = Command::new("nix")
        .arg("run")
        .arg(format!("path:{}#formatter.{system}", root.display()))
        .arg("--")
        .args(files)
        .current_dir(root)
        .output()
        .map_err(|e| format!("running the flake formatter: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "formatting {} failed:\n{}",
            files.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub(crate) fn format_files(root: &Path, files: &[String]) -> Result<bool, String> {
    if files.is_empty() {
        return Ok(false);
    }
    let Some(system) = formatter_system(root)? else {
        return Ok(false);
    };
    run_formatter(root, &system, files)?;
    Ok(true)
}

pub(crate) fn formatted_texts(
    root: &Path,
    texts: &[(&str, &str)],
) -> Result<Option<Vec<String>>, String> {
    if texts.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(system) = formatter_system(root)? else {
        return Ok(None);
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("strictix-format-{}-{nanos}", std::process::id()));
    let result = (|| {
        let mut paths = Vec::new();
        for (rel, text) in texts {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            paths.push(path.display().to_string());
        }
        run_formatter(root, &system, &paths)?;
        paths
            .iter()
            .map(|path| std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}")))
            .collect::<Result<Vec<_>, _>>()
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.map(Some)
}

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "assert", "with", "let", "in", "rec", "inherit", "or",
];

fn mangle(rel: &str) -> String {
    let stem = rel.strip_suffix(".nix").unwrap_or(rel);
    let stem = stem.strip_suffix("/default").unwrap_or(stem);
    let mut name: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    if KEYWORDS.contains(&name.as_str()) {
        name.push('_');
    }
    name
}

fn string_refs(analysis: &Analysis, files: &BTreeSet<String>) -> Vec<String> {
    files
        .iter()
        .filter(|file| {
            let needle = format!("/{file}");
            analysis
                .strings
                .iter()
                .any(|s| s.contains(&needle) || s.as_str() == file.as_str())
        })
        .cloned()
        .collect()
}

fn path_refs(analysis: &Analysis, file: &str, files: &BTreeSet<String>) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for site in &analysis.sites {
        let Edit::Rebase { target, whole } = &site.edit else {
            continue;
        };
        let literal = analysis.text(site.path);
        if files.contains(target) {
            found.push((
                target.clone(),
                format!("named by the path `{literal}` in {file} outside import, callPackage and module lists, so the file has to exist"),
            ));
        } else if *whole {
            let prefix = if target.is_empty() {
                String::new()
            } else {
                format!("{target}/")
            };
            found.extend(files.iter().filter(|f| f.starts_with(&prefix)).map(|f| {
                (
                    f.clone(),
                    format!("inside the directory `{literal}` that {file} names by path, so the directory has to keep it"),
                )
            }));
        }
    }
    found
}

fn inline_targets(analysis: &Analysis) -> impl Iterator<Item = &String> {
    analysis.sites.iter().filter_map(|site| match &site.edit {
        Edit::Import { target, .. }
        | Edit::Package { target, .. }
        | Edit::Module { target, .. } => Some(target),
        _ => None,
    })
}

const KEYER_BODY: &str = "key: module:
    if builtins.isFunction module
    then {
      __functionArgs = builtins.functionArgs module;
      __functor = _: args: module args // {inherit key;};
    }
    else module // {inherit key;}";

fn fresh_name(base: &str, taken: &HashSet<&String>, used: &mut HashSet<String>) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while taken.contains(&name) || used.contains(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    used.insert(name.clone());
    name
}

pub(crate) fn nix_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

type Expanded = HashMap<String, Vec<Vec<String>>>;

struct Renderer<'a> {
    names: &'a HashMap<String, String>,
    keyed: &'a BTreeSet<String>,
    keyer: &'a str,
    expanded: &'a Expanded,
    out_dir: &'a str,
}

impl Renderer<'_> {
    fn module_element(&self, target: &str) -> String {
        match self.names.get(target) {
            None => relative_literal(self.out_dir, target),
            Some(name) if self.keyed.contains(target) => {
                format!("({} \"{target}\" {name})", self.keyer)
            }
            Some(name) => name.clone(),
        }
    }

    fn render(&self, file: &str, analysis: &Analysis) -> String {
        let end = u32::try_from(analysis.source.len()).unwrap_or(u32::MAX);
        self.render_range(file, analysis, TextRange::new(0, end))
    }

    fn render_range(&self, file: &str, analysis: &Analysis, range: TextRange) -> String {
        let source = &analysis.source;
        let mut out = String::with_capacity(source.len());
        let mut cursor = range.start() as usize;
        for site in &analysis.sites {
            if site.range.start() < range.start() || site.range.end() > range.end() {
                continue;
            }
            out.push_str(&source[cursor..site.range.start() as usize]);
            match &site.edit {
                Edit::Import { target, literal } => {
                    if let Some(name) = self.names.get(target) {
                        out.push_str(name);
                    } else {
                        out.push_str(
                            &source[site.range.start() as usize..site.path.start() as usize],
                        );
                        out.push_str(&relative_literal(self.out_dir, literal));
                        out.push_str(&source[site.path.end() as usize..site.range.end() as usize]);
                    }
                }
                Edit::Package { target, literal } => match self.names.get(target) {
                    Some(name) => out.push_str(name),
                    None => out.push_str(&relative_literal(self.out_dir, literal)),
                },
                Edit::Module { target, literal } => {
                    if self.names.contains_key(target) {
                        out.push_str(&self.module_element(target));
                    } else {
                        out.push_str(&relative_literal(self.out_dir, literal));
                    }
                }
                Edit::Rebase { target, .. } => {
                    out.push_str(&relative_literal(self.out_dir, target));
                }
                Edit::Remove => {}
                Edit::Expand(index) => {
                    out.push('[');
                    for target in &self.expanded[file][*index] {
                        out.push('\n');
                        out.push_str(&self.module_element(target));
                    }
                    out.push_str("\n]");
                }
            }
            cursor = site.range.end() as usize;
        }
        out.push_str(&source[cursor..range.end() as usize]);
        out
    }
}

fn overrides(
    analysis: &Analysis,
    file: &str,
    names: &HashMap<String, String>,
) -> BTreeMap<String, String> {
    let dir = parent(file);
    let mut out = BTreeMap::new();
    for site in &analysis.sites {
        let (canonical, original) = match &site.edit {
            Edit::Import { target, .. } if names.contains_key(target) => (
                format!("import {}", module_literal(dir, target)),
                analysis.text(site.range),
            ),
            Edit::Package { target, .. } | Edit::Module { target, .. }
                if names.contains_key(target) =>
            {
                (module_literal(dir, target), analysis.text(site.path))
            }
            Edit::Import { literal, .. }
            | Edit::Package { literal, .. }
            | Edit::Module { literal, .. } => {
                (relative_literal(dir, literal), analysis.text(site.path))
            }
            Edit::Rebase { target, .. } => {
                (relative_literal(dir, target), analysis.text(site.path))
            }
            Edit::Expand(_) | Edit::Remove => continue,
        };
        if canonical != original {
            out.entry(canonical).or_insert_with(|| original.to_string());
        }
    }
    out
}

fn redirect_entry(
    analysis: &Analysis,
    rel: &str,
    entry: &str,
    out_file: &str,
) -> Option<(String, Option<String>)> {
    let source = &analysis.source;
    let mut text = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut changed = false;
    let mut original = None;
    for site in &analysis.sites {
        let (Edit::Import { target, .. }
        | Edit::Package { target, .. }
        | Edit::Module { target, .. }) = &site.edit
        else {
            continue;
        };
        if target != entry {
            continue;
        }
        let literal = analysis.text(site.path);
        if literal != module_literal(parent(rel), entry) {
            original = Some(literal.to_string());
        }
        text.push_str(&source[cursor..site.path.start() as usize]);
        text.push_str(&relative_literal(parent(rel), out_file));
        cursor = site.path.end() as usize;
        changed = true;
    }
    text.push_str(&source[cursor..]);
    changed.then_some((text, original))
}

fn next_targets<'a>(file: &str, analysis: &'a Analysis, expanded: &'a Expanded) -> Vec<&'a String> {
    inline_targets(analysis)
        .chain(&analysis.named)
        .chain(expanded.get(file).into_iter().flatten().flatten())
        .collect()
}

fn expand_reachable(
    root: &Path,
    entry: &str,
    analyses: &HashMap<String, Analysis>,
) -> Result<Expanded, String> {
    let mut expanded = Expanded::new();
    let mut order = vec![entry.to_string()];
    let mut seen: HashSet<String> = order.iter().cloned().collect();
    let mut index = 0;
    while index < order.len() {
        let current = order[index].clone();
        index += 1;
        let Some(analysis) = analyses.get(&current) else {
            continue;
        };
        let lists = evaluate_expansions(root, &current, analysis)?;
        if !lists.is_empty() {
            expanded.insert(current.clone(), lists);
        }
        for target in next_targets(&current, analysis, &expanded) {
            if seen.insert(target.clone()) {
                order.push(target.clone());
            }
        }
    }
    Ok(expanded)
}

fn reach(
    entry: &str,
    analyses: &HashMap<String, Analysis>,
    expanded: &Expanded,
    kept: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut order = vec![entry.to_string()];
    let mut seen: HashSet<String> = order.iter().cloned().collect();
    let mut index = 0;
    while index < order.len() {
        let current = order[index].clone();
        index += 1;
        let Some(analysis) = analyses.get(&current) else {
            continue;
        };
        for target in next_targets(&current, analysis, expanded) {
            if !kept.contains_key(target)
                && analyses.contains_key(target)
                && seen.insert(target.clone())
            {
                order.push(target.clone());
            }
        }
    }
    order
}

fn partition(
    entry: &str,
    files: &BTreeSet<String>,
    analyses: &HashMap<String, Analysis>,
    expanded: &Expanded,
) -> Result<(Vec<String>, BTreeMap<String, String>), String> {
    let mut kept: BTreeMap<String, String> = BTreeMap::new();
    for file in files {
        for (target, reason) in path_refs(&analyses[file], file, files) {
            kept.entry(target).or_insert(reason);
        }
    }
    loop {
        let inlined = reach(entry, analyses, expanded, &kept);
        let set: HashSet<&String> = inlined.iter().collect();
        let mut added = Vec::new();
        for file in files.iter().filter(|file| !set.contains(file)) {
            for target in inline_targets(&analyses[file]) {
                if target != entry && set.contains(target) && !kept.contains_key(target) {
                    added.push((
                        target.clone(),
                        format!("imported by {file}, which stays a file"),
                    ));
                }
            }
        }
        if added.is_empty() {
            if let Some(reason) = kept.get(entry) {
                return Err(format!("entry {entry} cannot be merged: {reason}"));
            }
            return Ok((inlined, kept));
        }
        for (target, reason) in added {
            kept.entry(target).or_insert(reason);
        }
    }
}

struct Manifest<'a> {
    entry: &'a str,
    out: &'a str,
    keyer: Option<&'a str>,
    names: &'a HashMap<String, String>,
    overrides: BTreeMap<String, BTreeMap<String, String>>,
    redirected: Vec<String>,
    expansions: Vec<(&'a str, &'a str, &'a [String])>,
    dropped: &'a [Dropped],
    module_values: Vec<(&'a str, &'a str)>,
    verbatim: Vec<(&'a str, &'a str)>,
}

impl Manifest<'_> {
    fn render(&self, name: &str) -> String {
        let mut out = format!(
            "  {name} = {{\n    entry = {};\n    out = {};\n",
            nix_string(self.entry),
            nix_string(self.out)
        );
        if let Some(keyer) = self.keyer {
            let _ = writeln!(out, "    keyer = {};", nix_string(keyer));
        }
        let mut files: Vec<(&String, &String)> =
            self.names.iter().map(|(file, name)| (name, file)).collect();
        files.sort();
        out.push_str("    files = {\n");
        for (name, file) in files {
            let _ = writeln!(out, "      {name} = {};", nix_string(file));
        }
        out.push_str("    };\n");
        if !self.redirected.is_empty() {
            out.push_str("    redirected = [");
            for file in &self.redirected {
                let _ = write!(out, " {}", nix_string(file));
            }
            out.push_str(" ];\n");
        }
        if !self.overrides.is_empty() {
            out.push_str("    overrides = {\n");
            for (file, map) in &self.overrides {
                let _ = writeln!(out, "      {} = {{", nix_string(file));
                for (canonical, original) in map {
                    let _ = writeln!(
                        out,
                        "        {} = {};",
                        nix_string(canonical),
                        nix_string(original)
                    );
                }
                out.push_str("      };\n");
            }
            out.push_str("    };\n");
        }
        if !self.expansions.is_empty() {
            out.push_str("    expansions = [\n");
            for (file, source, modules) in &self.expansions {
                let _ = writeln!(
                    out,
                    "      {{\n        file = {};\n        source = {};",
                    nix_string(file),
                    nix_string(source)
                );
                out.push_str("        modules = [\n");
                for module in *modules {
                    let _ = writeln!(out, "          {}", nix_string(module));
                }
                out.push_str("        ];\n      }\n");
            }
            out.push_str("    ];\n");
        }
        if !self.dropped.is_empty() {
            out.push_str("    removed = [\n");
            for dropped in self.dropped {
                let _ = writeln!(
                    out,
                    "      {{\n        file = {};\n        source = {};",
                    nix_string(&dropped.file),
                    nix_string(&dropped.source)
                );
                if let Some(before) = &dropped.before {
                    let _ = writeln!(out, "        before = {};", nix_string(before));
                }
                out.push_str("      }\n");
            }
            out.push_str("    ];\n");
        }
        if !self.verbatim.is_empty() {
            out.push_str("    verbatim = {\n");
            for (file, text) in &self.verbatim {
                let _ = writeln!(out, "      {} = {};", nix_string(file), nix_string(text));
            }
            out.push_str("    };\n");
        }
        if !self.module_values.is_empty() {
            out.push_str("    module_values = [\n");
            for (file, attr) in &self.module_values {
                let _ = writeln!(
                    out,
                    "      {{\n        file = {};\n        attr = {};\n      }}",
                    nix_string(file),
                    nix_string(attr)
                );
            }
            out.push_str("    ];\n");
        }
        out.push_str("  };\n");
        out
    }
}

fn keyed_modules(
    inlined: &[String],
    analyses: &HashMap<String, Analysis>,
    expanded: &Expanded,
) -> BTreeSet<String> {
    let set: HashSet<&String> = inlined.iter().collect();
    let mut uses: HashMap<&String, usize> = HashMap::new();
    for file in inlined {
        for site in &analyses[file].sites {
            if let Edit::Module { target, .. } = &site.edit {
                *uses.entry(target).or_default() += 1;
            }
        }
        for target in expanded.get(file).into_iter().flatten().flatten() {
            *uses.entry(target).or_default() += 1;
        }
    }
    uses.into_iter()
        .filter(|(target, count)| *count > 1 && set.contains(target))
        .map(|(target, _)| target.clone())
        .collect()
}

fn partial_paths(inlined: &[String], analyses: &HashMap<String, Analysis>) -> Vec<String> {
    inlined
        .iter()
        .flat_map(|file| {
            analyses[file]
                .sites
                .iter()
                .filter_map(move |site| match &site.edit {
                    Edit::Rebase {
                        whole: false,
                        target,
                    } => Some(format!("{file}: {target}")),
                    _ => None,
                })
        })
        .collect()
}

type Loaded = (BTreeSet<String>, HashMap<String, Analysis>);

fn load(root: &Path, entry: &str, out_file: &str, expand: &[String]) -> Result<Loaded, String> {
    let mut files = BTreeSet::new();
    collect_nix(root, "", &mut files)?;
    if out_file != entry {
        files.remove(out_file);
    }
    if !files.contains(entry) {
        return Err(format!("entry {entry} is not a .nix file under the root"));
    }
    let mut analyses = HashMap::new();
    for file in &files {
        analyses.insert(file.clone(), analyze(root, file, expand)?);
    }
    for analysis in analyses.values_mut() {
        analysis.named = string_refs(analysis, &files);
    }
    Ok((files, analyses))
}

fn bind_names(
    bound: &[&String],
    inlined: &[String],
    analyses: &HashMap<String, Analysis>,
) -> (HashMap<String, String>, String, String) {
    let taken: HashSet<&String> = inlined
        .iter()
        .flat_map(|f| analyses[f].idents.keys())
        .collect();
    let mut names = HashMap::new();
    let mut used = HashSet::new();
    for file in bound {
        names.insert(
            (*file).clone(),
            fresh_name(&mangle(file), &taken, &mut used),
        );
    }
    let keyer = fresh_name("merge_keyed_module", &taken, &mut used);
    let manifest = fresh_name("merge_manifest", &taken, &mut used);
    (names, keyer, manifest)
}

pub fn merge(options: &MergeOptions) -> Result<MergeReport, String> {
    let root = std::fs::canonicalize(&options.root)
        .map_err(|e| format!("{}: {e}", options.root.display()))?;
    let entry = join("", &options.entry).ok_or("entry escapes the merge root")?;
    let out_file = join("", &options.out).ok_or("output escapes the merge root")?;
    let out_dir = parent(&out_file).to_string();
    let in_place = out_file == entry;
    if !in_place && std::fs::symlink_metadata(root.join(&out_file)).is_ok() {
        return Err(format!(
            "{out_file} already exists; refusing to overwrite it"
        ));
    }

    let (files, mut analyses) = load(&root, &entry, &out_file, &options.expand)?;
    let expanded = expand_reachable(&root, &entry, &analyses)?;
    let mut dropped = Vec::new();
    let mut expanding: Vec<&String> = expanded.keys().collect();
    expanding.sort();
    for file in expanding {
        if let Some(analysis) = analyses.get_mut(file) {
            dropped.extend(drop_dead_expanders(file, analysis, &options.expand));
        }
    }
    let (inlined, kept) = partition(&entry, &files, &analyses, &expanded)?;
    let inlined_set: HashSet<&String> = inlined.iter().collect();
    dropped.retain(|d| inlined_set.contains(&d.file));

    let shell = flake_outputs(&analyses[&entry].source);
    let bound: Vec<&String> = inlined
        .iter()
        .filter(|file| shell.is_none() || **file != entry)
        .collect();
    let (names, keyer_name, manifest_name) = bind_names(&bound, &inlined, &analyses);
    let keyed = keyed_modules(&inlined, &analyses, &expanded);

    let renderer = Renderer {
        names: &names,
        keyed: &keyed,
        keyer: &keyer_name,
        expanded: &expanded,
        out_dir: &out_dir,
    };
    let mut bindings = String::new();
    if !keyed.is_empty() {
        let _ = writeln!(bindings, "  {keyer_name} = {KEYER_BODY};");
    }
    let mut sorted = bound.clone();
    sorted.sort_by_key(|f| &names[*f]);
    for file in sorted {
        let body = renderer.render(file, &analyses[file]);
        let _ = write!(bindings, "  {} =\n{}\n  ;\n", names[file], body.trim());
    }

    let mut rewritten = Vec::new();
    let mut manifest_expansions = Vec::new();
    for file in &inlined {
        let analysis = &analyses[file];
        for (source, modules) in analysis
            .expansions
            .iter()
            .zip(expanded.get(file).into_iter().flatten())
        {
            manifest_expansions.push((file.as_str(), source.as_str(), modules.as_slice()));
        }
    }
    let mut manifest = Manifest {
        entry: &entry,
        out: &out_file,
        keyer: (!keyed.is_empty()).then_some(keyer_name.as_str()),
        names: &names,
        overrides: inlined
            .iter()
            .map(|file| (file.clone(), overrides(&analyses[file], file, &names)))
            .filter(|(_, map)| !map.is_empty())
            .collect(),
        redirected: Vec::new(),
        expansions: manifest_expansions,
        dropped: &dropped,
        module_values: inlined
            .iter()
            .flat_map(|file| {
                analyses[file]
                    .module_values
                    .iter()
                    .map(move |attr| (file.as_str(), attr.as_str()))
            })
            .collect(),
        verbatim: Vec::new(),
    };
    if !options.dry_run {
        let texts: Vec<(&str, &str)> = inlined
            .iter()
            .map(|file| (file.as_str(), analyses[file].source.as_str()))
            .collect();
        if let Some(formatted) = formatted_texts(&root, &texts)? {
            manifest.verbatim = texts
                .into_iter()
                .zip(formatted)
                .filter(|((_, text), formatted)| text != formatted)
                .map(|(pair, _)| pair)
                .collect();
        }
    }
    if !in_place {
        for file in files.iter().filter(|file| !inlined_set.contains(file)) {
            if let Some((text, original)) = redirect_entry(&analyses[file], file, &entry, &out_file)
            {
                if let Some(original) = original {
                    let canonical = module_literal(parent(file), &entry);
                    manifest
                        .overrides
                        .insert(file.clone(), BTreeMap::from([(canonical, original)]));
                }
                manifest.redirected.push(file.clone());
                rewritten.push((file.clone(), text));
            }
        }
    }
    bindings.push_str(&manifest.render(&manifest_name));
    let merged = match shell {
        None => format!(
            "let\n{bindings}in\n  builtins.seq {manifest_name} {}\n",
            names[&entry]
        ),
        Some(core) => {
            let analysis = &analyses[&entry];
            let end = u32::try_from(analysis.source.len()).unwrap_or(u32::MAX);
            format!(
                "{}let\n{bindings}in\n  builtins.seq {manifest_name} ({}){}",
                renderer.render_range(&entry, analysis, TextRange::new(0, core.start())),
                renderer.render_range(&entry, analysis, core),
                renderer.render_range(&entry, analysis, TextRange::new(core.end(), end)),
            )
        }
    };

    let (out_lines, formatted) = if options.dry_run {
        (merged.lines().count(), false)
    } else {
        let deleted: Vec<&String> = inlined.iter().filter(|f| **f != out_file).collect();
        write_merge(&root, &out_file, in_place, &merged, &rewritten, &deleted)?
    };

    let verbatim = manifest
        .verbatim
        .iter()
        .map(|(file, _)| (*file).to_string())
        .collect();
    let unreached = files
        .iter()
        .filter(|file| !inlined_set.contains(file) && !kept.contains_key(*file))
        .cloned()
        .collect();
    Ok(MergeReport {
        untouched: files.len() - inlined.len(),
        unreached,
        partial_paths: partial_paths(&inlined, &analyses),
        kept: kept.into_iter().collect(),
        rewritten: rewritten.into_iter().map(|(f, _)| f).collect(),
        keyed_modules: keyed.into_iter().collect(),
        expansions: manifest
            .expansions
            .iter()
            .map(|(file, call, targets)| (format!("{file}: {call}"), targets.len()))
            .collect(),
        dropped_bindings: dropped
            .into_iter()
            .map(|d| format!("{}: {}", d.file, d.name))
            .collect(),
        inlined,
        out_lines,
        formatted,
        verbatim,
    })
}

fn write_merge(
    root: &Path,
    out_file: &str,
    in_place: bool,
    merged: &str,
    rewritten: &[(String, String)],
    deleted: &[&String],
) -> Result<(usize, bool), String> {
    let mut open = std::fs::OpenOptions::new();
    open.write(true);
    if in_place {
        open.truncate(true);
    } else {
        open.create_new(true);
    }
    open.open(root.join(out_file))
        .and_then(|mut file| std::io::Write::write_all(&mut file, merged.as_bytes()))
        .map_err(|e| format!("writing {out_file}: {e}"))?;
    for (file, text) in rewritten {
        std::fs::write(root.join(file), text).map_err(|e| format!("writing {file}: {e}"))?;
    }
    for file in deleted {
        remove_with_empty_parents(root, file)?;
    }
    let mut touched = vec![out_file.to_string()];
    touched.extend(rewritten.iter().map(|(file, _)| file.clone()));
    let formatted = format_files(root, &touched)?;
    let lines = std::fs::read_to_string(root.join(out_file))
        .map_err(|e| format!("reading {out_file}: {e}"))?
        .lines()
        .count();
    Ok((lines, formatted))
}

fn remove_with_empty_parents(root: &Path, rel: &str) -> Result<(), String> {
    std::fs::remove_file(root.join(rel)).map_err(|e| format!("removing {rel}: {e}"))?;
    let mut dir = parent(rel);
    while !dir.is_empty() {
        let full = root.join(dir);
        let empty = std::fs::read_dir(&full).is_ok_and(|mut d| d.next().is_none());
        if !empty {
            break;
        }
        std::fs::remove_dir(&full).map_err(|e| format!("removing {dir}: {e}"))?;
        dir = parent(dir);
    }
    Ok(())
}
