//! Rule trait, the dispatch loop, and the one registry mechanism.
//!
//! A rule is either a *node rule* (fires once per node of one
//! [`SyntaxKind`]) or a *file rule* (fires once per file with the lazy
//! [`SemanticModel`]). The registry is declared with the `rules!` macro
//! — one declaration mechanism, no hand-wired special cases.

use crate::config::LintConfig;
use crate::context::Context;
use crate::diagnostic::{Diagnostic, Severity};
use crate::fix::{FixError, TextEdit};
use crate::semantic::SemanticModel;
use strictix_syntax::{parse, SyntaxKind, SyntaxNode};

const SUPPRESSION_PREFIX: &str = "# strictix: disable-next-line=";

/// Remove diagnostics covered by an immediately preceding suppression comment.
///
/// The deliberately narrow syntax is a full-line comment of the form
/// `# strictix: disable-next-line=rule-code`; malformed directives and codes
/// that do not match a diagnostic are ignored. Matching is by source line,
/// never by byte proximity, so a suppression cannot affect its own line.
fn apply_suppressions(source: &str, diags: &mut Vec<Diagnostic>) {
    let mut suppressed_lines = std::collections::HashMap::<usize, Vec<&str>>::new();
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        let Some(code) = trimmed.strip_prefix(SUPPRESSION_PREFIX) else {
            continue;
        };
        if !code.is_empty() && !code.chars().any(char::is_whitespace) {
            suppressed_lines.entry(line_no + 1).or_default().push(code);
        }
    }
    let mut line_starts = vec![0usize];
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(offset + 1);
        }
    }
    diags.retain(|diag| {
        let line = line_starts
            .partition_point(|&start| start <= diag.range.start() as usize)
            .saturating_sub(1);
        !suppressed_lines
            .get(&line)
            .is_some_and(|codes| codes.contains(&diag.code))
    });
}

/// A single lint rule.
///
/// Node rules override [Self::node_kind] and [Self::check_node]; file
/// rules override [Self::check_file]. The default bodies do nothing, so
/// a rule implements only what it needs.
pub trait Rule: Send + Sync {
    /// Kebab-case unique id, e.g. `"unused-let-binding"`.
    fn code(&self) -> &'static str;

    /// Short human title.
    fn name(&self) -> &'static str;

    /// What the rule catches and why (1-3 sentences).
    fn description(&self) -> &'static str;

    /// Severity of the diagnostics this rule emits.
    fn severity(&self) -> Severity;

    /// Whether this rule runs without an explicit configuration opt-in.
    fn default_enabled(&self) -> bool {
        true
    }

    /// `Some(kind)` makes this a node rule: [Self::check_node] fires
    /// once per node of that kind. `None` makes it a file rule:
    /// [Self::check_file] fires once per file.
    fn node_kind(&self) -> Option<SyntaxKind> {
        None
    }

    /// Inspect one node of [Self::node_kind]. Node rules must not touch
    /// the semantic model (it is lazy and only file rules build it).
    fn check_node(&self, _node: &SyntaxNode, _source: &str, _diags: &mut Vec<Diagnostic>) {}

    fn check_file(
        &self,
        _model: &SemanticModel,
        _config: &LintConfig,
        _diags: &mut Vec<Diagnostic>,
    ) {
    }

    /// Inspect a file with optional project-wide static context. The default
    /// preserves the single-file rule API.
    fn check_file_project(
        &self,
        model: &SemanticModel,
        config: &LintConfig,
        _project: Option<&crate::project::ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        self.check_file(model, config, diags);
    }
}

/// Run every enabled rule over one file.
///
/// Node rules fire once per node of their kind, in source order (the
/// `descendants()` pre-order walk). File rules fire exactly once each,
/// receiving the semantic model — which node rules never touch, so the
/// model is only built when at least one file rule is enabled.
pub fn run_rules(
    rules: &[Box<dyn Rule>],
    tree: &SyntaxNode,
    model: &SemanticModel,
    config: &LintConfig,
    source: &str,
    diags: &mut Vec<Diagnostic>,
) {
    run_rules_project_inner(rules, tree, model, config, source, None, false, diags);
}

pub fn run_rules_project(
    rules: &[Box<dyn Rule>],
    tree: &SyntaxNode,
    model: &SemanticModel,
    config: &LintConfig,
    source: &str,
    project: Option<&crate::project::ProjectContext>,
    diags: &mut Vec<Diagnostic>,
) {
    run_rules_project_inner(rules, tree, model, config, source, project, true, diags);
}

#[allow(clippy::too_many_arguments)] // The dispatch seam keeps node/file/project context explicit.
fn run_rules_project_inner(
    rules: &[Box<dyn Rule>],
    tree: &SyntaxNode,
    model: &SemanticModel,
    config: &LintConfig,
    source: &str,
    project: Option<&crate::project::ProjectContext>,
    include_syntax: bool,
    diags: &mut Vec<Diagnostic>,
) {
    if include_syntax && syntax_diagnostics(tree, diags) {
        apply_suppressions(source, diags);
        return;
    }
    for rule in rules {
        if !config.is_enabled(rule.code())
            || (!rule.default_enabled() && !config.enabled.iter().any(|code| code == rule.code()))
        {
            continue;
        }
        match rule.node_kind() {
            Some(kind) => {
                for node in tree.descendants() {
                    if node.kind() == kind {
                        rule.check_node(node, source, diags);
                    }
                }
            }
            None => rule.check_file_project(model, config, project, diags),
        }
    }
    apply_suppressions(source, diags);
}

fn syntax_diagnostics(tree: &SyntaxNode, diags: &mut Vec<Diagnostic>) -> bool {
    let mut fatal = false;
    for node in tree.error_nodes() {
        fatal = true;
        diags.push(Diagnostic::new(
            "syntax-error",
            Severity::Error,
            "syntax error: malformed syntax",
            node.content_range(),
        ));
    }
    fatal
}

/// Cap on fix passes. A rule whose fix re-triggers itself (a rule bug)
/// would otherwise loop forever; ten passes is far beyond any real
/// composition chain.
pub const MAX_FIX_PASSES: usize = 10;

/// Result of linting one file through the engine.
pub struct LintRun {
    /// Findings from the first pass — what the user sees.
    pub diagnostics: Vec<Diagnostic>,
    /// Final text, `Some` only when it differs from the input.
    pub fixed: Option<String>,
    /// Number of fix passes committed (zero in check mode).
    pub passes: usize,
    /// Set when a fix pass's edits overlap or go out of bounds. The
    /// context is rolled back to the input, so `fixed` is `None`.
    pub error: Option<FixError>,
}

/// The lint engine: the single entry point for one file.
///
/// Every path — `check` and `fix` — routes through here. The source
/// text is the host-owned context ([`Context`]); rules read it through
/// derived views (tree + model) and commit effects (fixes = text
/// edits). `fix = false` is a single read-only pass. `fix = true` is
/// the reactive loop: each pass collects fixes, commits them as one
/// mutation, and re-runs rules on the changed text to a fixpoint, so a
/// fix that reveals another finding is caught. Stops when a pass
/// produces no fixes, or after [`MAX_FIX_PASSES`].
///
/// # Errors
///
/// Returns [`FixError`] when a pass's edits overlap or go out of bounds
/// — the same validation as the one-shot [`crate::fix::apply_fixes`].
pub fn lint(
    rules: &[Box<dyn Rule>],
    source: &str,
    path: Option<&std::path::Path>,
    config: &LintConfig,
    fix: bool,
) -> LintRun {
    lint_project(rules, source, path, config, fix, None)
}

pub fn lint_project(
    rules: &[Box<dyn Rule>],
    source: &str,
    path: Option<&std::path::Path>,
    config: &LintConfig,
    fix: bool,
    project: Option<&crate::project::ProjectContext>,
) -> LintRun {
    let mut context = Context::new(source.to_string());
    let mut diagnostics = Vec::new();
    let mut passes = 0usize;
    let mut error = None;

    loop {
        let tree = parse(context.source());
        let mut diags = Vec::new();
        let model = SemanticModel::new(context.source(), &tree).with_path(path);
        run_rules_project(
            rules,
            &tree,
            &model,
            config,
            context.source(),
            project,
            &mut diags,
        );

        let edits: Vec<TextEdit> = if fix {
            diags
                .iter()
                .filter_map(|d| d.fix.as_ref())
                .flat_map(|f| f.edits.iter().cloned())
                .collect()
        } else {
            Vec::new()
        };

        if passes == 0 {
            diags.sort_by_key(|d| d.range.start());
            diagnostics = diags;
        }

        if !fix || edits.is_empty() {
            break;
        }
        match context.commit(&edits) {
            Ok(()) => {}
            Err(err) => {
                // Atomic: undo any prior passes, leave the input intact.
                context.rollback_all();
                error = Some(err);
                break;
            }
        }
        passes += 1;
        if passes >= MAX_FIX_PASSES {
            break;
        }
    }

    let final_text = context.source().to_string();
    let fixed = (final_text != source).then_some(final_text);

    LintRun {
        diagnostics,
        fixed,
        passes,
        error,
    }
}

/// Declare the whole rule registry in one place.
///
/// Takes a comma-separated list of unit-struct rule type names (plain
/// identifiers of structs implementing [Rule], declared in scope) and
/// expands to a `Vec<Box<dyn Rule>>`, one boxed instance per type:
///
/// ```text
/// rules! { UnusedLetBinding, ConstantIf }
/// ```
///
/// expands to
///
/// ```text
/// vec![Box::new(UnusedLetBinding {}), Box::new(ConstantIf {})]
/// ```
///
/// A trailing comma is accepted. Because the macro is `#[macro_export]`ed
/// it lives at the crate root: `strictix_core::rules!`.
#[macro_export]
macro_rules! rules {
    ($($rule:ident),* $(,)?) => {
        vec![$(Box::new($rule {}) as Box<dyn $crate::rules::Rule>),*]
    };
}
