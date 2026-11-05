//! repeated-dynamic-path: one option path with a computed segment, such as
//! `manzil.users.${config.user.name}`, spelled out in many modules.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    parse, AstNode, AttrItem, AttrName, AttrsetExpr, Expr, StringPart, SyntaxKind, SyntaxNode,
    TextRange,
};

use crate::config_file_rules::relative;
use crate::duplicate_definitions::merged_modules;

const CODE: &str = "repeated-dynamic-path";
const MIN_MODULES: usize = 4;

/// One module's first definition under a dynamic path.
struct Site {
    file: usize,
    module: Option<String>,
    range: TextRange,
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The interpolated attribute path of a segment, when it is one: `${a.b}`
/// or `"${a.b}"` alone. A bare `${name}` is usually a lambda parameter, so
/// two modules spelling it alike name nothing in common.
fn dynamic(name: AttrName<'_>, source: &str) -> Option<String> {
    let interp = match name {
        AttrName::Interp(interp) => interp,
        AttrName::Str(string) => {
            let mut parts = string.parts();
            let (Some(StringPart::Interp(interp)), None) = (parts.next(), parts.next()) else {
                return None;
            };
            interp
        }
        AttrName::Ident(_) => return None,
    };
    let Some(Expr::Select(select)) = interp.expr() else {
        return None;
    };
    Some(format!("${{{}}}", normalized(select.syntax().text(source))))
}

fn segment(name: AttrName<'_>, source: &str) -> String {
    match name {
        AttrName::Ident(token) => token.text(source).to_owned(),
        AttrName::Str(string) => string.syntax().text(source).to_owned(),
        AttrName::Interp(interp) => normalized(interp.syntax().text(source)),
    }
}

fn attrset_of(value: Expr<'_>) -> Option<AttrsetExpr<'_>> {
    match value {
        Expr::Paren(paren) => paren.expr().and_then(attrset_of),
        Expr::Attrset(set) => Some(set),
        Expr::RecAttrset(set) => set.attrset(),
        _ => None,
    }
}

/// Collect `path -> first range` for every binding path through `set`,
/// prefixed by `prefix`, that reaches a dynamic segment.
fn walk(
    set: AttrsetExpr<'_>,
    prefix: &[String],
    source: &str,
    nested: &mut HashSet<u32>,
    found: &mut Vec<(String, TextRange)>,
) {
    nested.insert(set.syntax().range().start());
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let Some(path) = binding.attrpath() else {
            continue;
        };
        let mut key = prefix.to_vec();
        let mut hit = None;
        for name in path.elements() {
            if let Some(text) = dynamic(name, source) {
                key.push(text);
                hit = Some(key.clone());
                break;
            }
            key.push(segment(name, source));
        }
        if let Some(hit) = hit {
            let hit = if hit.first().is_some_and(|s| s == "config") && hit.len() > 2 {
                &hit[1..]
            } else {
                &hit[..]
            };
            found.push((hit.join("."), path.syntax().content_range()));
            continue;
        }
        if let Some(inner) = binding.value().and_then(attrset_of) {
            walk(inner, &key, source, nested, found);
        }
    }
}

fn module_paths(root: &SyntaxNode, source: &str) -> Vec<(String, TextRange)> {
    let mut nested = HashSet::new();
    let mut found = Vec::new();
    for node in root.descendants() {
        if node.kind() != SyntaxKind::AttrsetExpr || nested.contains(&node.range().start()) {
            continue;
        }
        if let Some(set) = AttrsetExpr::cast(node) {
            walk(set, &[], source, &mut nested, &mut found);
        }
    }
    found
}

struct Findings(Vec<Vec<(TextRange, String)>>);

fn index(project: &ProjectContext, config: &LintConfig) -> Findings {
    let files = project.sources();
    let min = config
        .setting_usize(CODE, "min-modules")
        .unwrap_or(MIN_MODULES);
    let mut by_path: HashMap<String, Vec<Site>> = HashMap::new();
    let mut lines: Vec<Vec<usize>> = Vec::new();
    for (file, entry) in files.iter().enumerate() {
        let source = entry.source.as_str();
        lines.push(
            std::iter::once(0)
                .chain(source.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
        );
        if config.is_generated(&entry.path) {
            continue;
        }
        let tree = parse(source);
        if tree.error_nodes().next().is_some() {
            continue;
        }
        let modules: Vec<(Option<String>, &SyntaxNode)> = match merged_modules(&tree, source) {
            Some(bindings) => bindings
                .into_iter()
                .filter(|(_, _, path)| {
                    !path
                        .as_deref()
                        .is_some_and(|path| config.is_generated(Path::new(path)))
                })
                .map(|(name, node, _)| (Some(name), node))
                .collect(),
            None => vec![(None, &tree)],
        };
        for (name, node) in modules {
            let mut seen = HashSet::new();
            for (path, range) in module_paths(node, source) {
                if seen.insert(path.clone()) {
                    by_path.entry(path).or_default().push(Site {
                        file,
                        module: name.clone(),
                        range,
                    });
                }
            }
        }
    }
    let mut findings = vec![Vec::new(); files.len()];
    let place = |site: &Site, from: usize| {
        let line = lines[site.file].partition_point(|&start| start <= site.range.start() as usize);
        let at = if site.file == from {
            format!("line {line}")
        } else {
            format!(
                "{}:{line}",
                relative(&files[from].path, &files[site.file].path).display()
            )
        };
        match &site.module {
            Some(name) => format!("{name} ({at})"),
            None => at,
        }
    };
    for (path, sites) in &by_path {
        if sites.len() < min {
            continue;
        }
        for site in sites {
            let shown: Vec<String> = sites
                .iter()
                .filter(|other| !std::ptr::eq(*other, site))
                .take(3)
                .map(|other| place(other, site.file))
                .collect();
            let more = sites.len() - 1 - shown.len();
            let others = if more > 0 {
                format!("{} and {more} more", shown.join(", "))
            } else {
                shown.join(", ")
            };
            findings[site.file].push((
                site.range,
                format!(
                    "`{path}` is spelled out here and in {} other modules ({others}); declare one option with a static path and pass its definitions on once with `{path} = lib.mkAliasDefinitions options.<that option>;`",
                    sites.len() - 1
                ),
            ));
        }
    }
    for file in &mut findings {
        file.sort_by_key(|(range, _)| (range.start(), range.end()));
    }
    Findings(findings)
}

pub struct RepeatedDynamicPath;

impl Rule for RepeatedDynamicPath {
    fn code(&self) -> &'static str {
        CODE
    }

    fn name(&self) -> &'static str {
        "Repeated dynamic path"
    }

    fn description(&self) -> &'static str {
        "Flags an option path with a computed segment, such as `manzil.users.${config.user.name}` or `finix.persistence.allowlist.users.${config.user.name}`, that at least `min-modules` (default 4) modules spell out: different files, or different top-level bindings of a `strictix merge` file. The path runs up to and including the first segment that is a whole interpolated attribute path (`${a.b}`; a bare `${name}`, usually a lambda parameter, does not count), with a leading `config.` dropped, and each module is counted once. Every site is reported, naming the others: declare one option with a static path (a `deferredModule`, or `attrsOf deferredModule`, keeps each definition as written) and set the dynamic path once to `lib.mkAliasDefinitions options.<that option>`, which passes every definition on in module order, so the target sees the same definitions in the same order and each module writes a short static path. Files and bindings under a `lint.generated` prefix are left out."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_file_project(
        &self,
        model: &SemanticModel,
        config: &LintConfig,
        project: Option<&ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let Some(project) = project else {
            return;
        };
        let files = project.sources();
        let Some(current) = files
            .iter()
            .position(|f| model.path() == Some(f.path.as_path()))
            .or_else(|| files.iter().position(|f| f.source == model.source()))
        else {
            return;
        };
        let findings = project.memo(|project| index(project, config));
        for (range, message) in &findings.0[current] {
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                message.clone(),
                *range,
            ));
        }
    }
}
