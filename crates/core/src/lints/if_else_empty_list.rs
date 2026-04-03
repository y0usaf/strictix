use crate::{Metadata, Report, Rule, Suggestion, utils};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Expr, IfElse},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `if cond then [...] else []` patterns.
///
/// ## Why is this bad?
/// This is a verbose way to conditionally include list items.
/// `lib.optionals` expresses the same intent more concisely.
///
/// ## Example
///
/// ```nix
/// if config.foo.enable then [ foo bar ] else []
/// ```
///
/// Use `lib.optionals` instead:
///
/// ```nix
/// lib.optionals config.foo.enable [ foo bar ]
/// ```
#[lint(
    name = "if_else_empty_list",
    note = "Prefer `lib.optionals` over `if cond then [...] else []`",
    code = 33,
    match_with = SyntaxKind::NODE_IF_ELSE
)]
struct IfElseEmptyList;

impl Rule for IfElseEmptyList {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let if_else = IfElse::cast(node.clone())?;
        let cond = if_else.condition()?;
        let then_body = if_else.body()?;
        let else_body = if_else.else_body()?;

        // `else` must be an empty list
        if !utils::is_empty_list(&else_body) {
            return None;
        }

        // `then` must be a non-empty list
        let Expr::List(then_list) = &then_body else {
            return None;
        };
        if then_list.items().count() == 0 {
            return None;
        }

        let at = node.text_range();
        Some(self.report().suggest(
            at,
            "Use `lib.optionals` to express conditional list inclusion",
            Suggestion::with_text(
                at,
                format!(
                    "lib.optionals {} {}",
                    utils::fmt_as_fn_arg(cond.syntax()),
                    then_list.syntax()
                ),
            ),
        ))
    }
}
