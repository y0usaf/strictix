use crate::{Metadata, Report, Rule, Suggestion, utils};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Expr, IfElse},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `if cond then true else false` and `if cond then false else true`,
/// which are just the condition itself (or its negation).
///
/// ## Why is this bad?
/// Unnecessary code; booleans can be used directly.
///
/// ## Example
///
/// ```nix
/// if x then true else false
/// ```
///
/// Use the condition directly:
///
/// ```nix
/// x
/// ```
///
/// And:
///
/// ```nix
/// if x then false else true
/// ```
///
/// Negate it:
///
/// ```nix
/// !x
/// ```
#[lint(
    name = "redundant_if_bool",
    note = "Redundant `if` expression with boolean literals",
    code = 27,
    match_with = SyntaxKind::NODE_IF_ELSE
)]
struct RedundantIfBool;

impl Rule for RedundantIfBool {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let if_else = IfElse::cast(node.clone())?;
        let cond = if_else.condition()?;
        let then_body = if_else.body()?;
        let else_body = if_else.else_body()?;

        let then_bool = boolean_value(&then_body)?;
        let else_bool = boolean_value(&else_body)?;

        let at = node.text_range();
        let cond_node = cond.syntax();

        let (message, replacement) = match (then_bool, else_bool) {
            (true, false) => {
                // `if x then true else false` → `x`
                (
                    "Use the condition directly instead of `if cond then true else false`",
                    cond_node.clone(),
                )
            }
            (false, true) => {
                // `if x then false else true` → `!x`
                let negated = utils::unary_not(cond_node);
                (
                    "Use `!cond` instead of `if cond then false else true`",
                    negated,
                )
            }
            _ => return None,
        };

        Some(
            self.report()
                .suggest(at, message, Suggestion::with_replacement(at, replacement)),
        )
    }
}

fn boolean_value(expr: &Expr) -> Option<bool> {
    utils::bool_literal(expr)
}
