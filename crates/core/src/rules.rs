//! Rule trait, the dispatch loop, and the one registry mechanism.
//!
//! A rule is either a *node rule* (fires once per node of one
//! [SyntaxKind]) or a *file rule* (fires once per file with the lazy
//! [SemanticModel]). The registry is declared with the `rules!` macro
//! — one declaration mechanism, no hand-wired special cases.

use crate::config::LintConfig;
use crate::context::Context;
use crate::diagnostic::{Diagnostic, Severity};
use crate::fix::{FixError, TextEdit};
use crate::semantic::SemanticModel;
use strictix_syntax::{parse, SyntaxKind, SyntaxNode};

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

    /// `Some(kind)` makes this a node rule: [Self::check_node] fires
    /// once per node of that kind. `None` makes it a file rule:
    /// [Self::check_file] fires once per file.
    fn node_kind(&self) -> Option<SyntaxKind> {
        None
    }

    /// Inspect one node of [Self::node_kind]. Node rules must not touch
    /// the semantic model (it is lazy and only file rules build it).
    fn check_node(&self, _node: &SyntaxNode, _source: &str, _diags: &mut Vec<Diagnostic>) {}

    /// Inspect the whole file through the (lazy) semantic model.
    fn check_file(
        &self,
        _model: &SemanticModel,
        _config: &LintConfig,
        _diags: &mut Vec<Diagnostic>,
    ) {
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
    for rule in rules {
        if !config.is_enabled(rule.code()) {
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
            None => rule.check_file(model, config, diags),
        }
    }
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
pub fn lint(rules: &[Box<dyn Rule>], source: &str, config: &LintConfig, fix: bool) -> LintRun {
    let mut context = Context::new(source.to_string());
    let mut diagnostics = Vec::new();
    let mut passes = 0usize;
    let mut error = None;

    loop {
        let tree = parse(context.source());
        let model = SemanticModel::new(context.source(), &tree);
        let mut diags = Vec::new();
        run_rules(rules, &tree, &model, config, context.source(), &mut diags);

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
