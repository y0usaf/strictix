//! `import`: turn the files a module reads into the generator data that
//! prints them, inline at the option that uses them.
//!
//! A module that points at a source file (`text = builtins.readFile
//! ./x.lisp`, `environment.etc."x.css".source = ./x.css`, a `src` or
//! `patches` list) is rewritten in place: the path becomes the finix
//! generator call whose syntax tree, or CSS value, holds the file's data,
//! and the source file is deleted. Nothing is written beside the module.
//!
//! Every conversion is checked before anything is written: the generator
//! call is evaluated with nix and must render the original file byte for
//! byte. A file the generator cannot print back exactly is left where it
//! is and reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use strictix_core::json::JsonValue;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    parse, AstNode, AttrName, Binding, Expr, LambdaParam, ListExpr, Root, StringPart, SyntaxKind,
    SyntaxNode, TextRange,
};

use crate::merge;
use crate::walk;

/// Where a module points at a file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// The whole expression is the file's text (`builtins.readFile ./x`).
    Text,
    /// The expression is the file itself (a `source`, a `src` entry, a
    /// file entry's value).
    File,
}

struct Site {
    module: PathBuf,
    range: TextRange,
    place: Place,
    line: usize,
    /// Whether `generators` is in scope where the file is named.
    bound: bool,
}

pub struct Options {
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub generators: Option<String>,
    pub dry_run: bool,
}

#[derive(Default)]
pub struct Report {
    pub inlined: Vec<(String, PathBuf)>,
    pub deleted: Vec<PathBuf>,
    pub kept: Vec<(PathBuf, String)>,
    pub modules: Vec<PathBuf>,
}

pub fn run(options: &Options) -> Result<Report, String> {
    let paths = if options.paths.is_empty() {
        vec![options.root.clone()]
    } else {
        options.paths.clone()
    };
    let (files, _, _) = walk::collect_files(&paths, None)?;
    let mut report = Report::default();
    let mut sites: BTreeMap<PathBuf, Vec<Site>> = BTreeMap::new();
    let mut modules: Vec<(PathBuf, String, SyntaxNode)> = Vec::new();
    for module in &files {
        let source = std::fs::read_to_string(module)
            .map_err(|e| format!("cannot read {}: {e}", module.display()))?;
        let tree = parse(&source);
        if tree.error_nodes().next().is_some() {
            continue;
        }
        let dir = module.parent().unwrap_or(Path::new("")).to_path_buf();
        let model = SemanticModel::new(&source, &tree);
        for site in find(&source, &tree, &dir) {
            sites.entry(site.key).or_default().push(Site {
                module: module.clone(),
                range: site.range,
                place: site.place,
                line: site.line,
                bound: model.is_bound("generators", site.range.start()),
            });
        }
        modules.push((module.clone(), source, tree));
    }

    let generators = options.generators.clone().unwrap_or_else(|| {
        format!(
            "(builtins.getFlake (toString {})).inputs.nix-generators.lib",
            merge::nix_string(&options.root.display().to_string())
        )
    });
    let mut calls: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut languages: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (target, found) in &sites {
        let mut users: BTreeSet<&PathBuf> = BTreeSet::new();
        for site in found {
            users.insert(&site.module);
        }
        let first = &found[0];
        if users.len() > 1 {
            let names: Vec<String> = users
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            report.kept.push((
                target.clone(),
                format!(
                    "{}:{}: read by several modules ({}), so it needs one owner",
                    first.module.display(),
                    first.line,
                    names.join(", ")
                ),
            ));
            continue;
        }
        match convert(target) {
            Ok((code, language)) => {
                calls.insert(target.clone(), code);
                languages.insert(target.clone(), language);
            }
            Err(reason) => report.kept.push((
                target.clone(),
                format!("{}:{}: {reason}", first.module.display(), first.line),
            )),
        }
    }
    for (target, reason) in check(&generators, &calls)? {
        calls.remove(&target);
        report.kept.push((target, reason));
    }

    let mut rewritten: Vec<(PathBuf, String)> = Vec::new();
    for (module, source, tree) in &modules {
        let local: Vec<(&PathBuf, &Site)> = sites
            .iter()
            .filter(|(target, _)| calls.contains_key(*target))
            .flat_map(|(target, found)| {
                found
                    .iter()
                    .filter(|site| &site.module == module)
                    .map(move |site| (target, site))
            })
            .collect();
        if local.is_empty() {
            continue;
        }
        let mut by_target: BTreeMap<&PathBuf, Vec<&Site>> = BTreeMap::new();
        for (target, site) in &local {
            by_target.entry(*target).or_default().push(*site);
        }
        let needs_generators = local.iter().any(|(_, site)| !site.bound);
        let formal = if needs_generators {
            Some(formal_spot(tree)?)
        } else {
            None
        };
        let body = let_spot(tree);
        let mut hoisted: Vec<(String, String)> = Vec::new();
        let mut edits: Vec<(TextRange, String)> = Vec::new();
        for (target, found) in by_target {
            let code = &calls[target];
            let hoisted_name = if found.len() > 1 && body.is_some() {
                let name = binding_name(source, target);
                hoisted.push((name.clone(), code.clone()));
                Some(name)
            } else {
                None
            };
            for site in found {
                let replacement = match (hoisted_name.as_deref(), site.place) {
                    (Some(name), Place::Text) => name.to_owned(),
                    (Some(name), Place::File) => format!("({})", named_file(target, name)),
                    (None, Place::Text) => format!("({code})"),
                    (None, Place::File) => format!("({})", named_file(target, code)),
                };
                edits.push((site.range, replacement));
            }
        }
        if let Some(at) = formal {
            edits.push((TextRange::new(at, at), "generators, ".to_owned()));
        }
        if let (Some(body), false) = (body, hoisted.is_empty()) {
            let bindings: String = hoisted
                .iter()
                .map(|(name, value)| format!("  {name} = {value};\n"))
                .collect();
            edits.push((TextRange::new(body, body), format!("let\n{bindings}in\n")));
        }
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start()));
        let mut text = source.clone();
        for (range, replacement) in edits {
            text.replace_range(range.start() as usize..range.end() as usize, &replacement);
        }

        rewritten.push((module.clone(), text));
    }

    for (module, text) in &rewritten {
        if parse(text).error_nodes().next().is_some() {
            return Err(format!(
                "the rewrite of {} does not parse; nothing was written",
                module.display()
            ));
        }
    }
    if options.dry_run {
        for (module, text) in &rewritten {
            let original = std::fs::read_to_string(module)
                .map_err(|e| format!("cannot read {}: {e}", module.display()))?;
            print!("{}", crate::render::diff(&original, text, false));
        }
        return Ok(report);
    }
    for (module, text) in rewritten {
        std::fs::write(&module, &text)
            .map_err(|e| format!("cannot write {}: {e}", module.display()))?;
        report.modules.push(module);
    }
    for target in calls.keys() {
        std::fs::remove_file(target)
            .map_err(|e| format!("cannot delete {}: {e}", target.display()))?;
        report.deleted.push(target.clone());
    }
    let paths: Vec<String> = report
        .modules
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let _ = merge::format_files(&options.root, &paths);
    report.inlined = calls
        .keys()
        .map(|target| {
            (
                languages.get(target).cloned().unwrap_or_default(),
                target.clone(),
            )
        })
        .collect();
    Ok(report)
}

struct Found {
    key: PathBuf,
    range: TextRange,
    place: Place,
    line: usize,
}

fn find(source: &str, tree: &strictix_syntax::SyntaxNode, dir: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    for node in tree.descendants() {
        if let Some(Expr::Apply(apply)) = Expr::cast(node) {
            let reads = apply
                .func()
                .map(|func| unparen(func))
                .and_then(|func| dotted(func, source))
                .is_some_and(|name| {
                    matches!(
                        name.as_str(),
                        "builtins.readFile" | "readFile" | "lib.fileContents" | "fileContents"
                    )
                });
            if reads {
                if let Some(absolute) = apply.arg().and_then(|arg| path_at(arg, source, dir)) {
                    found.push(Found {
                        key: absolute,
                        range: apply.syntax().content_range(),
                        place: Place::Text,
                        line: line_of(source, apply.syntax().content_range().start()),
                    });
                    continue;
                }
            }
        }
        let Some(binding) = Binding::cast(node) else {
            continue;
        };
        let Some(last) = last_segment(binding, source) else {
            continue;
        };
        let Some(value) = binding.value() else {
            continue;
        };
        if matches!(last.as_deref(), Some("src" | "srcs" | "patches")) {
            let mut value = unparen(value);
            while let Expr::Paren(paren) = value {
                let Some(inner) = paren.expr() else { break };
                value = inner;
            }
            match value {
                Expr::List(list) => {
                    for item in ListExpr::cast(list.syntax())
                        .into_iter()
                        .flat_map(|l| l.items())
                    {
                        if let Some(absolute) = path_at(item, source, dir) {
                            found.push(Found {
                                key: absolute,
                                range: item.content_range(),
                                place: Place::File,
                                line: line_of(source, item.content_range().start()),
                            });
                        }
                    }
                }
                single => {
                    if let Some(absolute) = path_at(single, source, dir) {
                        found.push(Found {
                            key: absolute,
                            range: single.content_range(),
                            place: Place::File,
                            line: line_of(source, single.content_range().start()),
                        });
                    }
                }
            }
            continue;
        }
        let last = last.as_deref().unwrap_or_default();
        if is_path_binding(last) {
            if let Some(absolute) = path_at(value, source, dir) {
                found.push(Found {
                    key: absolute,
                    range: value.content_range(),
                    place: Place::File,
                    line: line_of(source, value.content_range().start()),
                });
            }
        }
    }
    found
}

/// The binding's last attribute name when it is static text: a `source`
/// entry, a file entry (`\\"x.css\\"`) or a list of files.
fn last_segment(binding: Binding<'_>, source: &str) -> Option<Option<String>> {
    let path = binding.attrpath()?;
    let last = path.elements().last()?;
    Some(match last {
        AttrName::Ident(token) => Some(token.text(source).to_owned()),
        AttrName::Str(string) => {
            let mut text = String::new();
            for part in string.parts() {
                match part {
                    StringPart::Content(token) => text.push_str(token.text(source)),
                    StringPart::Interp(_) => return None,
                }
            }
            Some(text)
        }
        AttrName::Interp(_) => None,
    })
}

/// Whether some generator prints this file's kind.
fn convertible(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    extension == "css" || language(&extension).is_some()
}

/// A binding whose value is a file to keep: a `source`, a package's `src`
/// or `patches`, or an entry named like a file (`\"x.css\"`).
fn is_path_binding(name: &str) -> bool {
    matches!(name, "source" | "src" | "srcs" | "patches") || is_file_name(name)
}

fn is_file_name(name: &str) -> bool {
    let after = name.rsplit('/').next().unwrap_or(name);
    match after.rsplit_once('.') {
        Some((stem, extension)) => !stem.is_empty() && !extension.is_empty() && name != "source",
        None => false,
    }
}

fn path_at(expr: Expr<'_>, source: &str, dir: &Path) -> Option<PathBuf> {
    let Expr::Path(token) = unparen(expr) else {
        return None;
    };
    let text = token.text(source);
    if !(text.starts_with("./") || text.starts_with("../")) || text.contains("${") {
        return None;
    }
    let path = std::fs::canonicalize(dir.join(text)).ok()?;
    if !path.is_file() || !convertible(&path) {
        return None;
    }
    Some(path)
}

fn line_of(source: &str, offset: u32) -> usize {
    source[..offset as usize].matches('\n').count() + 1
}

fn unparen(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        match paren.expr() {
            Some(inner) => expr = inner,
            None => break,
        }
    }
    expr
}

fn dotted(expr: Expr<'_>, source: &str) -> Option<String> {
    match expr {
        Expr::Ident(token) => Some(token.text(source).to_owned()),
        Expr::Select(select) => {
            let mut name = dotted(select.base()?, source)?;
            for element in select.attrpath()?.elements() {
                match element {
                    AttrName::Ident(token) => {
                        name.push('.');
                        name.push_str(token.text(source));
                    }
                    _ => return None,
                }
            }
            Some(name)
        }
        _ => None,
    }
}

/// The generator call that prints `target`'s contents, plus its language.
fn convert(target: &Path) -> Result<(String, String), String> {
    let bytes = std::fs::read(target).map_err(|e| format!("cannot read: {e}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| "the file is not UTF-8, so no generator prints it".to_owned())?;
    let extension = target
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "css" {
        let value = strictix_import::css(&text)?;
        return Ok((format!("generators.cssSource {value}"), "css".to_owned()));
    }
    let Some(language) = language(&extension) else {
        return Err(format!("no generator converts .{extension}"));
    };
    let tree = strictix_import::expression(language, &text, &[])?;
    let applied = if strictix_import::bound(language) {
        format!("(({tree}) generators.{language})")
    } else {
        let names = strictix_import::constructors(&tree);
        let formals = if names.is_empty() {
            "_".to_owned()
        } else {
            let names: String = names.iter().map(|name| format!("  {name},\n")).collect();
            format!("{{\n{names}  ...\n}}")
        };
        format!("(({formals}: {tree}) generators.{language})")
    };
    Ok((
        format!("generators.{language}.render {applied}"),
        language.to_owned(),
    ))
}

fn language(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "glsl" | "frag" | "vert" | "vsh" | "fsh" => "glsl",
        "js" | "mjs" | "cjs" => "javascript",
        "lisp" | "lsp" | "cl" | "el" | "asd" => "lisp",
        "lua" => "lua",
        "luau" => "luau",
        "md" | "markdown" => "markdown",
        "patch" | "diff" => "patch",
        "py" => "python",
        "sh" | "bash" => "shell",
        "ts" | "tsx" | "mts" | "cts" => "ts",
        "vbs" | "vba" => "vbs",
        _ => return None,
    })
}

/// Where `generators` joins a module's formals: before `...` when the
/// formals are open, otherwise before the closing brace.
fn formal_spot(tree: &SyntaxNode) -> Result<u32, String> {
    let Some(root) = Root::cast(tree).and_then(|root| root.expr()) else {
        return Err("the file holds no expression".into());
    };
    let Some(Expr::Lambda(lambda)) = Some(unparen(root)) else {
        return Err(
            "the file does not take module arguments, so `generators` would be undefined".into(),
        );
    };
    let LambdaParam::Formals(formals, _) = lambda.param() else {
        return Err(
            "the file takes its arguments as one value, so `generators` would be undefined".into(),
        );
    };
    let tokens: Vec<_> = formals.syntax().child_tokens().collect();
    let spot = tokens
        .iter()
        .find(|token| token.kind() == SyntaxKind::Ellipsis)
        .or_else(|| {
            tokens
                .iter()
                .find(|token| token.kind() == SyntaxKind::RBrace)
        })
        .map(|token| token.range().start());
    spot.ok_or_else(|| "the formals list has no closing brace".to_owned())
}

/// Where the module's body starts: the value a `let` written inside the
/// module's arguments goes in front of, so the bindings can see them.
fn let_spot(tree: &SyntaxNode) -> Option<u32> {
    let root = Root::cast(tree)?.expr()?;
    match unparen(root) {
        Expr::Lambda(lambda) => lambda.body().map(|body| body.content_range().start()),
        _ => None,
    }
}

fn named_file(target: &Path, code: &str) -> String {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "generators.sourceFile {} ({code})",
        merge::nix_string(&name)
    )
}

/// A free `let` name for a file used more than once in one module.
fn binding_name(source: &str, target: &Path) -> String {
    let stem: String = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data".to_owned())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mut name = format!("source_{stem}");
    let mut suffix = 0;
    while source.contains(&name) {
        suffix += 1;
        name = format!("source_{stem}_{suffix}");
    }
    name
}

/// Every call must render its file byte for byte. A call that fails to
/// evaluate, or prints other bytes, is reported and left unconverted.
fn check(
    generators: &str,
    calls: &BTreeMap<PathBuf, String>,
) -> Result<Vec<(PathBuf, String)>, String> {
    if calls.is_empty() {
        return Ok(Vec::new());
    }
    let checks: Vec<String> = calls
        .iter()
        .map(|(target, code)| comparison(target, code))
        .collect();
    let expression = format!(
        "let generators = {generators}; in [\n{}\n]",
        checks.join("\n")
    );
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", &expression])
        .output()
        .map_err(|e| format!("running nix eval: {e}"))?;
    let targets: Vec<&PathBuf> = calls.keys().collect();
    if output.status.success() {
        let json = JsonValue::parse(&String::from_utf8_lossy(&output.stdout))
            .map_err(|e| format!("parsing nix eval output: {}", e.message))?;
        let results = json.as_array().ok_or("the render check is not a list")?;
        let mut failures = Vec::new();
        for (index, result) in results.iter().enumerate() {
            if result.as_bool() != Some(true) {
                let target = targets.get(index).ok_or("the render check is short")?;
                failures.push((
                    (*target).clone(),
                    "the generator does not print this file byte for byte".to_owned(),
                ));
            }
        }
        return Ok(failures);
    }
    let mut failures = Vec::new();
    for (target, code) in calls {
        let expression = format!(
            "let generators = {generators}; in {}",
            comparison(target, code)
        );
        let output = Command::new("nix")
            .args(["eval", "--impure", "--raw", "--expr", &expression])
            .output()
            .map_err(|e| format!("running nix eval: {e}"))?;
        let rendered = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || rendered != "true" {
            if std::env::var_os("STRICTIX_IMPORT_DEBUG").is_some() {
                eprintln!(
                    "--- {}\n{code}\n{}\n---",
                    target.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let reason = if output.status.success() {
                "the generator does not print this file byte for byte".to_owned()
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                format!("the generator cannot print it: {}", last_error(&stderr))
            };
            failures.push((target.clone(), reason));
        }
    }
    Ok(failures)
}

fn comparison(target: &Path, code: &str) -> String {
    format!(
        "(({code}) == builtins.readFile {})",
        merge::nix_string(&target.display().to_string())
    )
}

/// The error line of a nix-stack-trace, which ends every message.
fn last_error(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("nix could not evaluate it")
        .to_owned()
}
