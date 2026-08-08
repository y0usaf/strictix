//! Rule trait, the dispatch loop, and the one registry mechanism.
//!
//! A rule is either a *node rule* (fires once per node of one
//! [SyntaxKind]) or a *file rule* (fires once per file with the lazy
//! [SemanticModel]). The registry is declared with the `rules!` macro
//! — one declaration mechanism, no hand-wired special cases.

use crate::config::LintConfig;
use crate::diagnostic::{Diagnostic, Severity};
use crate::semantic::SemanticModel;
use strictix_syntax::{SyntaxKind, SyntaxNode};

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
