use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{BinOp, BinOpKind, Expr, HasEntry},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for attribute set merges with an empty set: `{} // x` or `x // {}`.
///
/// ## Why is this bad?
/// Merging with an empty attrset is a no-op, just like `[] ++ x`.
///
/// ## Example
/// ```nix
/// {} // { a = 1; }
/// ```
///
/// Remove the pointless merge:
///
/// ```nix
/// { a = 1; }
/// ```
#[lint(
    name = "empty_attrset_merge",
    note = "Unnecessary merge with empty attrset",
    code = 26,
    match_with = SyntaxKind::NODE_BIN_OP
)]
struct EmptyAttrsetMerge;

impl Rule for EmptyAttrsetMerge {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let bin_expr = BinOp::cast(node.clone())?;
        let lhs = bin_expr.lhs()?;
        let rhs = bin_expr.rhs()?;
        let Some(BinOpKind::Update) = bin_expr.operator() else {
            return None;
        };

        let at = node.text_range();
        let message = "Merging with an empty attrset `{}` is a no-op";

        let non_empty = if is_empty_attrset(&lhs) {
            rhs
        } else if is_empty_attrset(&rhs) {
            lhs
        } else {
            return None;
        };

        Some(self.report().suggest(
            at,
            message,
            Suggestion::with_replacement(at, non_empty.syntax().clone()),
        ))
    }
}

fn is_empty_attrset(expr: &Expr) -> bool {
    let Expr::AttrSet(set) = expr else {
        return false;
    };
    // Must not be recursive and must have no entries
    set.rec_token().is_none() && set.entries().count() == 0
}
