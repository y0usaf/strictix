//! repeated-let-value: one computed value bound by a `let` in many modules.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, Expr, LetExpr, StringPart, SyntaxKind,
    SyntaxNode, TextRange,
};

use crate::config_file_rules::relative;
use crate::duplicate_definitions::merged_modules;

const CODE: &str = "repeated-let-value";
const MIN_MODULES: usize = 4;

struct Site {
    file: usize,
    module: Option<String>,
    range: TextRange,
    name: String,
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a value computes something: an alias (`config.user.name`), a
/// literal, a plain string and `import ./x.nix ARGS` only name or read a
/// value defined once already.
fn computed(value: Expr<'_>, source: &str) -> bool {
    match value {
        Expr::Ident(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Path(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_)
        | Expr::Select(_) => false,
        Expr::String(string) => string.parts().any(|p| matches!(p, StringPart::Interp(_))),
        Expr::IndString(string) => string.parts().any(|p| matches!(p, StringPart::Interp(_))),
        Expr::Paren(paren) => paren.expr().is_some_and(|e| computed(e, source)),
        Expr::Apply(apply) => !imports(apply, source),
        _ => true,
    }
}

/// Whether a call applies a file read with `import PATH`: the shared
/// definition is that file, and each module only reads it.
fn imports(apply: ApplyExpr<'_>, source: &str) -> bool {
    let mut call = apply;
    loop {
        match call.func() {
            Some(Expr::Apply(inner)) => call = inner,
            Some(Expr::Paren(paren)) => match paren.expr() {
                Some(Expr::Apply(inner)) => call = inner,
                _ => return false,
            },
            Some(func @ Expr::Ident(_)) => {
                return func.content_text(source) == "import"
                    && matches!(call.arg(), Some(Expr::Path(_)));
            }
            _ => return false,
        }
    }
}

fn let_values(root: &SyntaxNode, source: &str) -> Vec<(String, String, TextRange)> {
    let mut found = Vec::new();
    for node in root.descendants() {
        if node.kind() != SyntaxKind::LetExpr {
            continue;
        }
        let Some(bindings) = LetExpr::cast(node).and_then(|l| l.bindings()) else {
            continue;
        };
        for item in bindings.items() {
            let AttrItem::Binding(binding) = item else {
                continue;
            };
            let Some(path) = binding.attrpath() else {
                continue;
            };
            let mut elements = path.elements();
            let (Some(AttrName::Ident(name)), None) = (elements.next(), elements.next()) else {
                continue;
            };
            let Some(value) = binding.value() else {
                continue;
            };
            if computed(value, source) {
                found.push((
                    name.text(source).to_owned(),
                    normalized(value.content_text(source)),
                    binding.syntax().content_range(),
                ));
            }
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
    let mut by_value: HashMap<String, Vec<Site>> = HashMap::new();
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
        for (module, node) in modules {
            let mut seen = HashSet::new();
            for (name, value, range) in let_values(node, source) {
                if seen.insert(value.clone()) {
                    by_value.entry(value).or_default().push(Site {
                        file,
                        module: module.clone(),
                        range,
                        name,
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
    for (value, sites) in &by_value {
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
            let shown_value = if value.chars().count() > 60 {
                format!("{}...", value.chars().take(57).collect::<String>())
            } else {
                value.clone()
            };
            findings[site.file].push((
                site.range,
                format!(
                    "`{} = {shown_value}` is computed here and in {} other modules ({others}); define it once, as an option or an argument the modules read",
                    site.name,
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

pub struct RepeatedLetValue;

impl Rule for RepeatedLetValue {
    fn code(&self) -> &'static str {
        CODE
    }

    fn name(&self) -> &'static str {
        "Repeated let value"
    }

    fn description(&self) -> &'static str {
        "Flags a value that `let` bindings compute in at least `min-modules` (default 4) modules (files, or top-level bindings of a `strictix merge` file), compared by whitespace-normalized text whatever the binding's name: `runtimeDir = \"/run/user/${toString user.uid}\";` in four modules is one fact computed four times. Aliases (a name or attribute path such as `config.user.name`), literals, plain strings and `import ./x.nix` applied to any arguments (the file is the one definition each module reads) are not computations and are skipped. Every site is reported, naming the others; define the value once, as a read-only option or a module argument. Files and bindings under a `lint.generated` prefix are left out."
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
