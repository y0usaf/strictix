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
use std::io::Write as _;
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
    canon_module: PathBuf,
    range: TextRange,
    place: Place,
    line: usize,
    bound: bool,
}

struct PathRef {
    module: PathBuf,
    canon_module: PathBuf,
    range: TextRange,
    line: usize,
}

struct DirRef {
    dir: PathBuf,
    module: PathBuf,
    line: usize,
}

pub struct Options {
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub generators: Option<String>,
    pub dry_run: bool,
    /// Write each tree to its own `<file>.nix` beside the source and import
    /// it, instead of inlining it at the option.
    pub split: bool,
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
    let (files, _, _) = walk::collect_files_filtered(&paths, None, false)?;
    let mut scanned_roots: Vec<PathBuf> = Vec::new();
    for path in &paths {
        let root = if path.is_dir() {
            path.as_path()
        } else {
            module_dir(path)
        };
        let canonical = std::fs::canonicalize(root)
            .map_err(|e| format!("cannot resolve {}: {e}", root.display()))?;
        if !scanned_roots.contains(&canonical) {
            scanned_roots.push(canonical);
        }
    }
    let mut ref_roots: Vec<PathBuf> = Vec::new();
    if let Ok(canon_root) = std::fs::canonicalize(&options.root) {
        if canon_root.is_dir() {
            ref_roots.push(find_project_root(&canon_root).unwrap_or(canon_root));
        }
    }
    for root in &scanned_roots {
        let candidate = find_project_root(root).unwrap_or_else(|| root.clone());
        if !ref_roots.iter().any(|existing| candidate.starts_with(existing)) {
            ref_roots.retain(|existing| !existing.starts_with(&candidate));
            ref_roots.push(candidate);
        }
    }
    let (extra_ref_files, mut ref_scan_incomplete) = walk::collect_nix_files_best_effort(&ref_roots);

    let mut report = Report::default();
    let mut sites: BTreeMap<PathBuf, Vec<Site>> = BTreeMap::new();
    let mut all_refs: BTreeMap<PathBuf, Vec<PathRef>> = BTreeMap::new();
    let mut dynamic_dirs: Vec<DirRef> = Vec::new();
    let mut unparsed_files: Vec<(PathBuf, String)> = Vec::new();
    let mut modules: Vec<(PathBuf, PathBuf, String, SyntaxNode)> = Vec::new();
    let mut module_formals: BTreeMap<PathBuf, Result<u32, String>> = BTreeMap::new();
    let mut seen_modules: BTreeSet<PathBuf> = BTreeSet::new();

    for module in &files {
        let canon_module = std::fs::canonicalize(module)
            .map_err(|e| format!("cannot resolve {}: {e}", module.display()))?;
        if !seen_modules.insert(canon_module.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(module)
            .map_err(|e| format!("cannot read {}: {e}", module.display()))?;
        let dir = module_dir(module);
        let tree = parse(&source);
        let has_errors = tree.error_nodes().next().is_some();
        let dyn_dir_ranges = (!has_errors).then(|| dynamic_dir_ranges(&source, &tree));
        collect_path_refs(
            &source,
            module,
            &canon_module,
            dir,
            dyn_dir_ranges.as_ref(),
            &mut all_refs,
            &mut dynamic_dirs,
        );
        if has_errors {
            unparsed_files.push((module.clone(), source));
            continue;
        }
        let model = SemanticModel::new(&source, &tree);
        for site in find(&source, &tree, dir) {
            sites.entry(site.key).or_default().push(Site {
                module: module.clone(),
                canon_module: canon_module.clone(),
                range: site.range,
                place: site.place,
                line: site.line,
                bound: model.is_bound("generators", site.range.start()),
            });
        }
        module_formals.insert(canon_module.clone(), formal_spot(&tree));
        modules.push((module.clone(), canon_module, source, tree));
    }

    for extra_file in &extra_ref_files {
        let canon_file = match std::fs::canonicalize(extra_file) {
            Ok(path) => path,
            Err(e) => {
                if ref_scan_incomplete.is_none() {
                    ref_scan_incomplete =
                        Some(format!("cannot resolve {}: {e}", extra_file.display()));
                }
                continue;
            }
        };
        if !seen_modules.insert(canon_file.clone()) {
            continue;
        }
        let source = match std::fs::read_to_string(extra_file) {
            Ok(text) => text,
            Err(e) => {
                match std::fs::read(extra_file) {
                    Ok(bytes) => unparsed_files.push((
                        extra_file.clone(),
                        String::from_utf8_lossy(&bytes).into_owned(),
                    )),
                    Err(_) if ref_scan_incomplete.is_none() => {
                        ref_scan_incomplete =
                            Some(format!("cannot read {}: {e}", extra_file.display()));
                    }
                    Err(_) => {}
                }
                continue;
            }
        };
        let dir = module_dir(extra_file);
        let tree = parse(&source);
        let has_errors = tree.error_nodes().next().is_some();
        let dyn_dir_ranges = (!has_errors).then(|| dynamic_dir_ranges(&source, &tree));
        collect_path_refs(
            &source,
            extra_file,
            &canon_file,
            dir,
            dyn_dir_ranges.as_ref(),
            &mut all_refs,
            &mut dynamic_dirs,
        );
        if has_errors {
            unparsed_files.push((extra_file.clone(), source));
        }
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
    let mut calls: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut split: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for (target, found) in &sites {
        let first = &found[0];
        if !scanned_roots.iter().any(|root| target.starts_with(root)) {
            report.kept.push((
                target.clone(),
                format!(
                    "{}:{}: outside the scanned root",
                    first.module.display(),
                    first.line
                ),
            ));
            continue;
        }
        if let Some(unrecognized) = all_refs.get(target).and_then(|refs| {
            refs.iter().find(|r| {
                !found.iter().any(|site| {
                    site.canon_module == r.canon_module
                        && site.range.start() <= r.range.start()
                        && r.range.end() <= site.range.end()
                })
            })
        }) {
            report.kept.push((
                target.clone(),
                format!(
                    "{}:{}: referenced outside supported import sites",
                    unrecognized.module.display(),
                    unrecognized.line
                ),
            ));
            continue;
        }
        if let Some(dyn_ref) = dynamic_dirs.iter().find(|d| target.starts_with(&d.dir)) {
            report.kept.push((
                target.clone(),
                format!(
                    "{}:{}: may be referenced dynamically",
                    dyn_ref.module.display(),
                    dyn_ref.line
                ),
            ));
            continue;
        }
        if let Some(reason) = &ref_scan_incomplete {
            report.kept.push((
                target.clone(),
                format!("reference scan incomplete: {reason}"),
            ));
            continue;
        }
        if let Some(file_name) = target.file_name().and_then(|name| name.to_str()) {
            if let Some((broken, _)) = unparsed_files
                .iter()
                .find(|(_, source)| source.contains(file_name))
            {
                report.kept.push((
                    target.clone(),
                    format!(
                        "{}: may be referenced by a file with syntax errors",
                        broken.display()
                    ),
                ));
                continue;
            }
        }
        let mut users: BTreeSet<&PathBuf> = BTreeSet::new();
        for site in found {
            users.insert(&site.module);
        }
        if users.len() > 1 && !options.split {
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
        if options.split {
            let dest = split_path(target);
            if dest.exists() {
                report.kept.push((
                    target.clone(),
                    format!(
                        "{}:{}: {} already exists",
                        first.module.display(),
                        first.line,
                        dest.display()
                    ),
                ));
                continue;
            }
        }
        let blocked_formal = found.iter().find_map(|site| {
            if site.bound {
                return None;
            }
            match module_formals.get(&site.canon_module) {
                Some(Err(reason)) => Some((site, reason.as_str())),
                None => Some((site, "the file could not be analyzed")),
                Some(Ok(_)) => None,
            }
        });
        if let Some((site, reason)) = blocked_formal {
            report.kept.push((
                target.clone(),
                format!("{}:{}: {reason}", site.module.display(), site.line),
            ));
            continue;
        }
        match convert(target) {
            Ok(converted) => {
                calls.insert(target.clone(), converted.call());
                if options.split {
                    split.insert(target.clone(), split_path(target));
                }
                imported.insert(target.clone(), converted);
            }
            Err(reason) => report.kept.push((
                target.clone(),
                format!("{}:{}: {reason}", first.module.display(), first.line),
            )),
        }
    }
    for (target, reason) in check(&generators, &calls)? {
        calls.remove(&target);
        split.remove(&target);
        report.kept.push((target, reason));
    }

    let mut spliced: BTreeSet<PathBuf> = BTreeSet::new();
    let mut rewritten_counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut rewritten: Vec<(PathBuf, String, String)> = Vec::new();
    for (module, canon_module, source, tree) in &modules {
        let local: Vec<(&PathBuf, &Site)> = sites
            .iter()
            .filter(|(target, _)| calls.contains_key(*target))
            .flat_map(|(target, found)| {
                found
                    .iter()
                    .filter(|site| &site.canon_module == canon_module)
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
            match module_formals.get(canon_module) {
                Some(Ok(at)) => Some(*at),
                _ => continue,
            }
        } else {
            None
        };
        let body = let_spot(tree);
        let mut hoisted: Vec<(String, String)> = Vec::new();
        let mut edits: Vec<(TextRange, String)> = Vec::new();
        for (target, found) in by_target {
            let code = &calls[target];
            let hoisted_name = if found.len() > 1 && body.is_some() && !options.split {
                let name = binding_name(source, target);
                hoisted.push((name.clone(), code.clone()));
                Some(name)
            } else {
                None
            };
            spliced.insert(target.clone());
            *rewritten_counts.entry(target.clone()).or_default() += found.len();
            for site in found {
                let imported = split.get(target).map_or_else(
                    || code.clone(),
                    |path| format!("import {} {{inherit generators;}}", relative(module, path)),
                );
                let replacement = match (hoisted_name.as_deref(), site.place) {
                    (Some(name), Place::Text) => name.to_owned(),
                    (Some(name), Place::File) => format!("({})", named_file(target, name)),
                    (None, Place::Text) => format!("({imported})"),
                    (None, Place::File) => format!("({})", named_file(target, &imported)),
                };
                edits.push((site.range, replacement));
            }
        }
        if let Some(at) = formal {
            edits.push((TextRange::new(at, at), "generators, ".to_owned()));
        }
        if let (Some((at, into_let)), false) = (body, hoisted.is_empty()) {
            let bindings: String = hoisted
                .iter()
                .map(|(name, value)| format!("  {name} = {value};\n"))
                .collect();
            let text = if into_let {
                bindings
            } else {
                format!("let\n{bindings}in\n")
            };
            edits.push((TextRange::new(at, at), text));
        }
        edits.sort_by_key(|(range, _)| {
            (std::cmp::Reverse(range.start()), std::cmp::Reverse(range.end()))
        });
        let mut text = source.clone();
        for (range, replacement) in edits {
            text.replace_range(range.start() as usize..range.end() as usize, &replacement);
        }

        rewritten.push((module.clone(), source.clone(), text));
    }

    for target in split.keys() {
        if !spliced.contains(target) {
            continue;
        }
        let Some(code) = calls.get(target) else {
            continue;
        };
        let text = format!("{{generators}}: {code}\n");
        if parse(&text).error_nodes().next().is_some() {
            return Err(format!(
                "the tree written for {} does not parse; nothing was written",
                target.display()
            ));
        }
    }
    for (module, _, text) in &rewritten {
        if parse(text).error_nodes().next().is_some() {
            return Err(format!(
                "the rewrite of {} does not parse; nothing was written",
                module.display()
            ));
        }
    }
    if options.dry_run {
        for (target, path) in &split {
            if spliced.contains(target) {
                println!("{} would become {}", target.display(), path.display());
            }
        }
        for (_, original, text) in &rewritten {
            print!("{}", crate::render::diff(original, text, false));
        }
        return Ok(report);
    }
    let mut written_splits: Vec<&PathBuf> = Vec::new();
    for (target, path) in &split {
        if !spliced.contains(target) {
            continue;
        }
        if let Some(code) = calls.get(target) {
            let res = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .and_then(|mut file| {
                    written_splits.push(path);
                    file.write_all(format!("{{generators}}: {code}\n").as_bytes())
                });
            if let Err(e) = res {
                for created in written_splits {
                    let _ = std::fs::remove_file(created);
                }
                return Err(format!("cannot write {}: {e}", path.display()));
            }
        }
    }
    let mut written_modules: Vec<(PathBuf, String)> = Vec::new();
    for (module, original, text) in rewritten {
        if let Err(e) = std::fs::write(&module, &text) {
            let _ = std::fs::write(&module, &original);
            for (prev_module, prev_original) in written_modules.into_iter().rev() {
                let _ = std::fs::write(&prev_module, &prev_original);
            }
            for created in written_splits {
                let _ = std::fs::remove_file(created);
            }
            return Err(format!("cannot write {}: {e}", module.display()));
        }
        written_modules.push((module.clone(), original));
        report.modules.push(module);
    }
    for target in &spliced {
        let expected = sites.get(target).map_or(0, Vec::len);
        if rewritten_counts.get(target).copied() != Some(expected) {
            continue;
        }
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

fn find_project_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join("flake.nix").exists() || dir.join(".git").exists())
        .map(Path::to_path_buf)
}

fn module_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn mark_node_paths(node: &SyntaxNode, dynamic: &mut BTreeSet<(u32, u32)>) {
    for desc in node.descendants() {
        for token in desc.child_tokens() {
            if token.kind() == SyntaxKind::Path {
                let r = token.range();
                dynamic.insert((r.start(), r.end()));
            }
        }
    }
}

fn mark_expr_paths(expr: Expr<'_>, dynamic: &mut BTreeSet<(u32, u32)>) {
    match expr {
        Expr::Path(token) => {
            let r = token.range();
            dynamic.insert((r.start(), r.end()));
        }
        Expr::Ident(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_) => {}
        Expr::Let(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::With(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Assert(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::If(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Attrset(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::RecAttrset(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::List(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Lambda(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Apply(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Unary(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Bin(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Select(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::HasAttr(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::String(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::IndString(n) => mark_node_paths(n.syntax(), dynamic),
        Expr::Paren(n) => mark_node_paths(n.syntax(), dynamic),
    }
}

fn call_head(mut expr: Expr<'_>) -> Expr<'_> {
    loop {
        expr = unparen(expr);
        if let Expr::Apply(apply) = expr {
            match apply.func() {
                Some(func) => expr = func,
                None => return expr,
            }
        } else {
            return expr;
        }
    }
}

fn is_dynamic_dir_func(name: &str) -> bool {
    matches!(
        name,
        "readDir"
            | "builtins.readDir"
            | "listFilesRecursive"
            | "path.append"
            | "lib.path.append"
            | "cleanSource"
            | "lib.cleanSource"
            | "cleanSourceWith"
            | "lib.cleanSourceWith"
            | "filterSource"
            | "builtins.filterSource"
            | "builtins.path"
            | "sourceByRegex"
            | "toString"
            | "builtins.toString"
    ) || name.starts_with("lib.fileset.")
        || name.starts_with("fileset.")
        || name.starts_with("lib.sources.")
        || name.starts_with("sources.")
        || name.starts_with("lib.filesystem.")
        || name.contains(".lib.fileset.")
        || name.contains(".lib.sources.")
        || name.contains(".lib.filesystem.")
        || name.ends_with(".lib.cleanSource")
        || name.ends_with(".lib.cleanSourceWith")
}

fn dynamic_dir_ranges(source: &str, tree: &SyntaxNode) -> BTreeSet<(u32, u32)> {
    let mut dynamic = BTreeSet::new();
    for node in tree.descendants() {
        if node.kind() == SyntaxKind::InterpExpr {
            mark_node_paths(node, &mut dynamic);
        }
        if let Some(binding) = Binding::cast(node) {
            if let Some(Some(last)) = last_segment(binding, source) {
                if matches!(
                    last.as_str(),
                    "source" | "src" | "srcs" | "path" | "root" | "fileset" | "directory"
                ) {
                    if let Some(value) = binding.value() {
                        mark_expr_paths(value, &mut dynamic);
                    }
                }
            }
        }
        if let Some(Expr::Bin(bin)) = Expr::cast(node) {
            if bin.op() == Some(SyntaxKind::Plus) {
                if let Some(lhs) = bin.lhs() {
                    mark_expr_paths(lhs, &mut dynamic);
                }
            }
        }
        if let Some(Expr::Apply(apply)) = Expr::cast(node) {
            let is_dir_func = apply
                .func()
                .map(call_head)
                .and_then(|func| dotted(func, source))
                .is_some_and(|name| is_dynamic_dir_func(&name));
            if is_dir_func {
                if let Some(arg) = apply.arg() {
                    mark_expr_paths(arg, &mut dynamic);
                }
            }
        }
    }
    dynamic
}

fn resolve_prefix_dir(base_dir: &Path, prefix: &str, trailing_slash: bool) -> Option<PathBuf> {
    if prefix.starts_with("~/") {
        return None;
    }
    let dir_part = if trailing_slash || prefix.ends_with('/') {
        prefix
    } else {
        match prefix.rsplit_once('/') {
            Some(("", _)) => "/",
            Some((parent, _)) => parent,
            None => ".",
        }
    };
    std::fs::canonicalize(base_dir.join(dir_part))
        .ok()
        .filter(|path| path.is_dir())
}

fn collect_path_refs(
    source: &str,
    module: &Path,
    canon_module: &Path,
    dir: &Path,
    dyn_dir_ranges: Option<&BTreeSet<(u32, u32)>>,
    refs: &mut BTreeMap<PathBuf, Vec<PathRef>>,
    dynamic_dirs: &mut Vec<DirRef>,
) {
    let tokens = strictix_syntax::lex(source);
    for (idx, token) in tokens.iter().enumerate() {
        let token_end = token.start + token.len;
        if token.kind == SyntaxKind::Ident {
            if let (Some(slash), Some(interp)) = (tokens.get(idx + 1), tokens.get(idx + 2)) {
                if slash.kind == SyntaxKind::Slash
                    && interp.kind == SyntaxKind::InterpStart
                    && token_end == slash.start
                    && slash.start + slash.len == interp.start
                {
                    if let Some(resolved) = resolve_prefix_dir(dir, token.text(source), true) {
                        dynamic_dirs.push(DirRef {
                            dir: resolved,
                            module: module.to_path_buf(),
                            line: line_of(source, token.start),
                        });
                    }
                }
            }
            continue;
        }
        if token.kind != SyntaxKind::Path {
            continue;
        }
        let text = token.text(source);
        if text.starts_with("~/") {
            continue;
        }
        if let Some(pos) = text.find("${") {
            let prefix = &text[..pos];
            if let Some(resolved) = resolve_prefix_dir(dir, prefix, prefix.ends_with('/')) {
                dynamic_dirs.push(DirRef {
                    dir: resolved,
                    module: module.to_path_buf(),
                    line: line_of(source, token.start),
                });
            }
            continue;
        }
        let adjacent_interp = tokens
            .get(idx + 1)
            .is_some_and(|next| next.kind == SyntaxKind::InterpStart && token_end == next.start);
        let adjacent_slash_interp = matches!(
            (tokens.get(idx + 1), tokens.get(idx + 2)),
            (Some(slash), Some(interp))
                if slash.kind == SyntaxKind::Slash
                    && interp.kind == SyntaxKind::InterpStart
                    && token_end == slash.start
                    && slash.start + slash.len == interp.start
        );
        if adjacent_interp || adjacent_slash_interp {
            if let Some(resolved) = resolve_prefix_dir(dir, text, adjacent_slash_interp || text.ends_with('/')) {
                dynamic_dirs.push(DirRef {
                    dir: resolved,
                    module: module.to_path_buf(),
                    line: line_of(source, token.start),
                });
            }
            continue;
        }
        let Ok(resolved) = std::fs::canonicalize(dir.join(text)) else {
            continue;
        };
        if resolved.is_file() {
            let range = TextRange::new(token.start, token_end);
            refs.entry(resolved).or_default().push(PathRef {
                module: module.to_path_buf(),
                canon_module: canon_module.to_path_buf(),
                range,
                line: line_of(source, token.start),
            });
        } else if resolved.is_dir() {
            let is_dynamic = dyn_dir_ranges
                .map_or(true, |ranges| ranges.contains(&(token.start, token_end)));
            if is_dynamic {
                dynamic_dirs.push(DirRef {
                    dir: resolved,
                    module: module.to_path_buf(),
                    line: line_of(source, token.start),
                });
            }
        }
    }
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
    found.sort_by_key(|item| (item.range.start(), std::cmp::Reverse(item.range.end())));
    let mut non_overlapping: Vec<Found> = Vec::with_capacity(found.len());
    for item in found {
        if non_overlapping
            .last()
            .is_some_and(|prev| item.range.start() < prev.range.end())
        {
            continue;
        }
        non_overlapping.push(item);
    }
    non_overlapping
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

    fn call(&self) -> String {
        match self {
            Self::Tree {
                language,
                tree,
                names,
            } => {
                let set = format!("generators.{language}");
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

/// Where hoisted bindings go: before the `in` of the module's own top-level
/// `let`, or else in front of the module's body in a new `let`, flagged by
/// whether the spot is inside an existing `let`. Either way the bindings see
/// the module's arguments.
fn let_spot(tree: &SyntaxNode) -> Option<(u32, bool)> {
    let root = Root::cast(tree)?.expr()?;
    let Expr::Lambda(lambda) = unparen(root) else {
        return None;
    };
    let body = lambda.body()?;
    if let Expr::Let(let_expr) = unparen(body) {
        if let Some(token) = let_expr
            .syntax()
            .child_tokens()
            .find(|token| token.kind() == SyntaxKind::KwIn)
        {
            return Some((token.range().start(), true));
        }
    }
    Some((body.content_range().start(), false))
}

/// Where a split tree goes: beside the file it replaces, named after it so
/// the two are never confused.
fn split_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "source".to_owned());
    target.with_file_name(format!("{name}.nix"))
}

/// A path from the module to the file it names, as Nix writes it.
fn relative(module: &Path, path: &Path) -> String {
    let module = std::fs::canonicalize(module).unwrap_or_else(|_| module.to_path_buf());
    let from = module.parent().unwrap_or(Path::new(""));
    let mut up = 0;
    let mut base = from;
    loop {
        if let Ok(rest) = path.strip_prefix(base) {
            let mut out = String::new();
            for _ in 0..up {
                out.push_str("../");
            }
            out.push_str("./");
            out.push_str(&rest.display().to_string());
            return out;
        }
        match base.parent() {
            Some(parent) => {
                base = parent;
                up += 1;
            }
            None => return format!("./{}", path.display()),
        }
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
