use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode,
    ast::{Ident, Lambda, Pattern},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for bound pattern parameters where the outer binding is never used.
///
/// ## Why is this bad?
/// In a function like `args @ { x, y }: x + y`, the `args` binding suggests the
/// full argument set matters, but only the destructured fields are used.
/// Dropping the unused binding makes the function intent clearer.
///
/// ## Example
///
/// ```nix
/// args @ { x, y }: x + y
/// ```
///
/// Remove the unused outer binding:
///
/// ```nix
/// { x, y }: x + y
/// ```
#[lint(
    name = "unused_pattern_bind",
    note = "This pattern binding is never used",
    code = 32,
    match_with = SyntaxKind::NODE_PATTERN
)]
struct UnusedPatternBind;

impl Rule for UnusedPatternBind {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let pattern = Pattern::cast(node.clone())?;
        let pat_bind = pattern.pat_bind()?;
        let ident = pat_bind.ident()?;

        // Keep the existing empty-pattern rules responsible for `{ ... } @ args`.
        if pattern.pat_entries().count() == 0 {
            return None;
        }

        let lambda = node.ancestors().find_map(Lambda::cast)?;
        let body = lambda.body()?;

        if mentions_ident(&ident, body.syntax()) {
            return None;
        }

        let pattern_text = pattern.syntax().to_string();
        let replacement = pattern_text
            .split_once('@')
            .and_then(|(left, right)| {
                [left.trim(), right.trim()]
                    .into_iter()
                    .find(|segment| segment.starts_with('{'))
            })?
            .to_owned();

        let at = pattern.syntax().text_range();
        Some(self.report().suggest(
            at,
            format!("`{ident}` is never used; remove the redundant pattern binding"),
            Suggestion::with_text(at, replacement),
        ))
    }
}

fn mentions_ident(ident: &Ident, node: &SyntaxNode) -> bool {
    if let Some(node_ident) = Ident::cast(node.clone()) {
        return node_ident.to_string() == ident.to_string();
    }

    node.children().any(|child| mentions_ident(ident, &child))
}
