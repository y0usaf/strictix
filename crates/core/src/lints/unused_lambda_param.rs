use crate::{Metadata, Report, Rule, Suggestion, make, utils};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Lambda, Param},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for lambda parameters that are never referenced in the body.
///
/// ## Why is this bad?
/// An unused named parameter suggests the argument matters when it does not.
/// Replacing it with `_` makes the intent explicit while keeping the same
/// function shape.
///
/// ## Example
///
/// ```nix
/// x: 42
/// ```
///
/// Use `_` instead:
///
/// ```nix
/// _: 42
/// ```
#[lint(
    name = "unused_lambda_param",
    note = "This lambda parameter is never used",
    code = 31,
    match_with = SyntaxKind::NODE_LAMBDA
)]
struct UnusedLambdaParam;

impl Rule for UnusedLambdaParam {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let lambda_expr = Lambda::cast(node.clone())?;
        let Some(Param::IdentParam(ident_param)) = lambda_expr.param() else {
            return None;
        };

        let ident = ident_param.ident()?;

        if ident.to_string() == "_" {
            return None;
        }

        let body = lambda_expr.body()?;
        if utils::mentions_ident(&ident.to_string(), body.syntax()) {
            return None;
        }

        let at = ident_param.syntax().text_range();
        Some(self.report().suggest(
            at,
            format!("`{ident}` is never used; replace it with `_`"),
            Suggestion::with_replacement(at, make::ident("_").syntax().clone()),
        ))
    }
}
