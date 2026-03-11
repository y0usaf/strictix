use crate::{Metadata, Report, Rule};

use macros::lint;
use rnix::{NodeOrToken, SyntaxElement, SyntaxKind, ast::With};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `with` expressions.
///
/// ## Why is this bad?
/// `with` introduces implicit scope that makes code harder to reason about:
/// it breaks language server features (goto-definition, completions), can
/// cause subtle shadowing bugs, and makes refactoring fragile. Prefer
/// explicit attribute access or `inherit`.
///
/// ## Example
///
/// ```nix
/// with pkgs; [ git curl ]
/// ```
///
/// Use explicit attribute access instead:
///
/// ```nix
/// [ pkgs.git pkgs.curl ]
/// ```
#[lint(
    name = "with_expression",
    note = "Avoid `with` expressions",
    code = 24,
    match_with = SyntaxKind::NODE_WITH
)]
struct WithExpression;

impl Rule for WithExpression {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let with_expr = With::cast(node.clone())?;
        let namespace = with_expr.namespace()?;
        let at = node.text_range();

        Some(self.report().diagnostic(
            at,
            format!(
                "`with {};` introduces implicit scope; use explicit attribute access or `inherit` instead",
                namespace.syntax()
            ),
        ))
    }
}
