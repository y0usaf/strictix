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
    parse, AstNode, AttrName, Binding, Expr, LambdaParam, Root, StringPart, SyntaxKind, SyntaxNode,
    TextRange,
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

    let generators = match &options.generators {
        Some(generators) => generators.clone(),
        None => {
            let root = std::env::current_dir()
                .map_err(|e| format!("cannot read the current directory: {e}"))?;
            format!(
                "(builtins.getFlake {}).inputs.nix-generators.lib",
                merge::nix_string(&root.display().to_string())
            )
        }
    };
    let mut imported: BTreeMap<PathBuf, Imported> = BTreeMap::new();
    let mut owners: BTreeMap<PathBuf, &PathBuf> = BTreeMap::new();
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
            Ok(converted) => {
                owners.insert(target.clone(), &first.module);
                imported.insert(target.clone(), converted);
            }
            Err(reason) => report.kept.push((
                target.clone(),
                format!("{}:{}: {reason}", first.module.display(), first.line),
            )),
        }
    }
    let languages: BTreeSet<&str> = imported
        .values()
        .filter_map(Imported::tree_language)
        .collect();
    let attrs = generator_attrs(&generators, &languages)?;
    let mut calls: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (target, converted) in &imported {
        let Some(owner) = owners.get(target) else {
            continue;
        };
        let Some((_, source, tree)) = modules.iter().find(|(module, _, _)| module == *owner) else {
            continue;
        };
        let model = SemanticModel::new(source, tree);
        let site = &sites[target][0];
        let known = converted
            .tree_language()
            .and_then(|language| attrs.get(language));
        let extra: Vec<String> = converted
            .names()
            .iter()
            .filter(|name| {
                known.is_none_or(|known| !known.contains(*name))
                    && model.resolve_lexical(name, site.range.start()).is_some()
            })
            .cloned()
            .collect();
        calls.insert(target.clone(), converted.call(&extra));
    }
    for (target, reason) in check(&generators, &calls)? {
        calls.remove(&target);
        report.kept.push((target, reason));
    }

    let mut spliced: BTreeSet<PathBuf> = BTreeSet::new();
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
            match formal_spot(tree) {
                Ok(at) => Some(at),
                Err(reason) => {
                    for (target, found) in &by_target {
                        report.kept.push((
                            (*target).clone(),
                            format!("{}:{}: {reason}", module.display(), found[0].line),
                        ));
                    }
                    continue;
                }
            }
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
            spliced.insert(target.clone());
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
    for target in &spliced {
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
    report.inlined = spliced
        .iter()
        .map(|target| {
            (
                imported
                    .get(target)
                    .map_or_else(String::new, |converted| converted.language().to_owned()),
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
        if let Some(Expr::String(string)) = Expr::cast(node) {
            let parts: Vec<StringPart<'_>> = string.parts().collect();
            if let [StringPart::Interp(interp)] = parts.as_slice() {
                if let Some(absolute) = interp.expr().and_then(|inner| path_at(inner, source, dir))
                {
                    let range = interp
                        .expr()
                        .map(|inner| unparen(inner).content_range())
                        .unwrap_or_else(|| interp.syntax().content_range());
                    found.push(Found {
                        key: absolute,
                        range,
                        place: Place::File,
                        line: line_of(source, range.start()),
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
            for item in paths_in(binding.syntax()) {
                if let Some(absolute) = path_at(item, source, dir) {
                    found.push(Found {
                        key: absolute,
                        range: item.content_range(),
                        place: Place::File,
                        line: line_of(source, item.content_range().start()),
                    });
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

/// Every path under a node, so a `src` or `patches` value that nests or
/// concatenates lists keeps every file it names.
fn paths_in(node: &SyntaxNode) -> Vec<Expr<'_>> {
    node.descendants()
        .flat_map(|child| child.child_tokens())
        .filter(|token| token.kind() == SyntaxKind::Path)
        .map(Expr::Path)
        .collect()
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

/// The names a tree leaves to the set it is applied to: its outer lambda's
/// formals, which the importers write from the names the source leaves free.
fn formals_of(tree: &str) -> Option<Vec<String>> {
    let parsed = parse(tree);
    if parsed.error_nodes().next().is_some() {
        return None;
    }
    let Expr::Lambda(lambda) = unparen(Root::cast(&parsed)?.expr()?) else {
        return None;
    };
    let LambdaParam::Formals(formals, _) = lambda.param() else {
        return None;
    };
    Some(
        formals
            .params()
            .map(|param| param.name.text(tree).to_owned())
            .collect(),
    )
}

/// What a file becomes: a generator syntax tree with the constructors it
/// names, or a value a value generator prints.
enum Imported {
    Tree {
        language: &'static str,
        tree: String,
        names: Vec<String>,
    },
    Value {
        language: &'static str,
        generator: &'static str,
        value: String,
    },
}

impl Imported {
    fn language(&self) -> &'static str {
        match self {
            Self::Tree { language, .. } | Self::Value { language, .. } => language,
        }
    }

    /// The call that prints the file. `extra` are names the tree reads that
    /// the generator set does not hold and the module does.
    fn call(&self, extra: &[String]) -> String {
        match self {
            Self::Tree {
                language,
                tree,
                names,
            } => {
                let set = if extra.is_empty() {
                    format!("generators.{language}")
                } else {
                    format!(
                        "(generators.{language} // {{ inherit {}; }})",
                        extra.join(" ")
                    )
                };
                let applied = if strictix_import::bound(language) {
                    format!("({tree}) {set}")
                } else {
                    let formals = if names.is_empty() {
                        "_".to_owned()
                    } else {
                        let names: String =
                            names.iter().map(|name| format!("  {name},\n")).collect();
                        format!("{{\n{names}  ...\n}}")
                    };
                    format!("({formals}: {tree}) {set}")
                };
                format!("generators.{language}.render ({applied})")
            }
            Self::Value {
                generator, value, ..
            } => format!("generators.{generator} {value}"),
        }
    }

    /// The language whose generator set holds this tree, if it is one.
    fn tree_language(&self) -> Option<&'static str> {
        match self {
            Self::Tree { language, .. } => Some(language),
            Self::Value { .. } => None,
        }
    }

    /// The identifiers the tree reads that the generator set must supply.
    fn names(&self) -> &[String] {
        match self {
            Self::Tree { names, .. } => names,
            Self::Value { .. } => &[],
        }
    }
}

fn convert(target: &Path) -> Result<Imported, String> {
    let bytes = std::fs::read(target).map_err(|e| format!("cannot read: {e}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| "the file is not UTF-8, so no generator prints it".to_owned())?;
    let extension = target
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "css" {
        return Ok(Imported::Value {
            language: "css",
            generator: "cssSource",
            value: strictix_import::css(&text)?,
        });
    }
    let Some(language) = language(&extension) else {
        return Err(format!("no generator converts .{extension}"));
    };
    let tree = strictix_import::expression(language, &text, &[])?;
    let names = formals_of(&tree).unwrap_or_else(|| strictix_import::constructors(&tree));
    Ok(Imported::Tree {
        language,
        tree,
        names,
    })
}

/// Evaluate an expression that is too large for an argument, through a
/// temporary file.
fn eval_expression(expression: &str) -> Result<(bool, String, String), String> {
    let mut path = std::env::temp_dir();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    path.push(format!(
        "strictix-import-{}-{unique}.nix",
        std::process::id()
    ));
    std::fs::write(&path, expression)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--file"])
        .arg(&path)
        .output()
        .map_err(|e| format!("running nix eval: {e}"));
    let _ = std::fs::remove_file(&path);
    let output = output?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

/// The attributes each generator set holds, so a tree's other formals can
/// be taken from the module instead.
fn generator_attrs(
    generators: &str,
    languages: &BTreeSet<&str>,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    if languages.is_empty() {
        return Ok(BTreeMap::new());
    }
    let fields: Vec<String> = languages
        .iter()
        .map(|language| format!("{language} = builtins.attrNames generators.{language};"))
        .collect();
    let expression = format!(
        "let generators = {generators}; in {{ {} }}",
        fields.join(" ")
    );
    let (ok, stdout, stderr) = eval_expression(&expression)?;
    if !ok {
        return Err(format!("reading the generator sets failed:\n{stderr}"));
    }
    let json =
        JsonValue::parse(&stdout).map_err(|e| format!("parsing nix eval output: {}", e.message))?;
    let mut attrs = BTreeMap::new();
    for (language, names) in json
        .as_object()
        .ok_or("the generator sets are not an object")?
    {
        let names = names
            .as_array()
            .ok_or("a generator's attributes are not a list")?
            .iter()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect();
        attrs.insert(language.clone(), names);
    }
    Ok(attrs)
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
    let (ok, stdout, _) = eval_expression(&expression)?;
    let targets: Vec<&PathBuf> = calls.keys().collect();
    if ok {
        let json = JsonValue::parse(&stdout)
            .map_err(|e| format!("parsing nix eval output: {}", e.message))?;
        let results = json.as_array().ok_or("the render check is not a list")?;
        let mut failures = Vec::new();
        for (index, result) in results.iter().enumerate() {
            if result.as_bool() != Some(true) {
                let target = targets.get(index).ok_or("the render check is short")?;
                let code = calls.get(*target).ok_or("the render check is long")?;
                failures.push((
                    (*target).clone(),
                    format!(
                        "the generator does not print this file byte for byte ({})",
                        first_difference(generators, target, code)?
                    ),
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
        let (ok, stdout, stderr) = eval_expression(&expression)?;
        if !ok {
            failures.push((
                target.clone(),
                format!("the generator cannot print it: {}", last_error(&stderr)),
            ));
            continue;
        }
        if stdout.trim() != "true" {
            failures.push((
                target.clone(),
                format!(
                    "the generator does not print this file byte for byte ({})",
                    first_difference(generators, target, code)?
                ),
            ));
        }
    }
    Ok(failures)
}

/// Where the generator's text leaves the file: both are printed and
/// compared line by line.
fn first_difference(generators: &str, target: &Path, code: &str) -> Result<String, String> {
    let expression = format!("let generators = {generators}; in ({code})");
    let (ok, stdout, stderr) = eval_expression(&expression)?;
    if !ok {
        return Ok(format!("it does not evaluate: {}", last_error(&stderr)));
    }
    let rendered =
        JsonValue::parse(&stdout).map_err(|e| format!("parsing nix eval output: {}", e.message))?;
    let rendered = rendered.as_str().unwrap_or_default();
    let original = std::fs::read_to_string(target).map_err(|e| format!("cannot read: {e}"))?;
    for (index, (printed, written)) in rendered.lines().zip(original.lines()).enumerate() {
        if printed != written {
            return Ok(format!(
                "line {}: prints `{}` where the file has `{}`",
                index + 1,
                cut(printed),
                cut(written)
            ));
        }
    }
    Ok(
        match rendered.lines().count().cmp(&original.lines().count()) {
            std::cmp::Ordering::Less => "it prints fewer lines".to_owned(),
            std::cmp::Ordering::Greater => "it prints more lines".to_owned(),
            std::cmp::Ordering::Equal => "the last line has no newline".to_owned(),
        },
    )
}

fn cut(line: &str) -> String {
    let trimmed = line.trim_end();
    if trimmed.chars().count() > 60 {
        format!("{}…", trimmed.chars().take(60).collect::<String>())
    } else {
        trimmed.to_owned()
    }
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
        .rfind(|line| line.starts_with("error:"))
        .or_else(|| stderr.lines().map(str::trim).rfind(|line| !line.is_empty()))
        .unwrap_or("nix could not evaluate it")
        .to_owned()
}
