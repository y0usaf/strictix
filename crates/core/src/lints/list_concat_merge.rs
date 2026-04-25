use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{BinOp, BinOpKind, Expr, List},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for adjacent list literals in list concatenations that can be merged.
///
/// ## Why is this bad?
/// Multiple `++` operations between unconditional lists make code harder to read.
/// Adjacent unconditional lists can be merged without changing element order.
///
/// ## Example
/// ```nix
/// base ++ [ a b ] ++ [ c d ] ++ tail
/// ```
///
/// Merge adjacent lists:
///
/// ```nix
/// base ++ [ a b c d ] ++ tail
/// ```
#[lint(
    name = "list_concat_merge",
    note = "Multiple list concatenations that can be merged",
    code = 36,
    match_with = SyntaxKind::NODE_BIN_OP
)]
struct ListConcatMerge;

impl Rule for ListConcatMerge {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let bin_expr = BinOp::cast(node.clone())?;
        if bin_expr.operator() != Some(BinOpKind::Concat) {
            return None;
        }

        let replacement = find_mergeable_concat(&bin_expr)?;

        Some(self.report().suggest(
            concat_operator_range(&bin_expr).unwrap_or_else(|| node.text_range()),
            "Multiple list concatenations that can be merged",
            Suggestion::with_replacement(node.text_range(), replacement),
        ))
    }
}

/// Find adjacent unconditional list literals in a concatenation tree.
///
/// This intentionally does not move list literals across dynamic expressions such as
/// `lib.optional ...`, because list element order is semantically meaningful.
fn find_mergeable_concat(bin_expr: &BinOp) -> Option<rnix::SyntaxNode> {
    let lhs = bin_expr.lhs()?;
    let rhs = bin_expr.rhs()?;

    if let (Expr::List(lhs_list), Expr::List(rhs_list)) = (&lhs, &rhs) {
        return parse_replacement(&merged_list_text(lhs_list, rhs_list)?);
    }

    if let (Expr::BinOp(lhs_bin), Expr::List(rhs_list)) = (&lhs, &rhs)
        && lhs_bin.operator() == Some(BinOpKind::Concat)
        && let Expr::List(lhs_rhs_list) = lhs_bin.rhs()?
    {
        let replacement = format!(
            "{} ++ {}",
            lhs_bin.lhs()?,
            merged_list_text(&lhs_rhs_list, rhs_list)?
        );
        return parse_replacement(&replacement);
    }

    if let (Expr::List(lhs_list), Expr::BinOp(rhs_bin)) = (&lhs, &rhs)
        && rhs_bin.operator() == Some(BinOpKind::Concat)
        && let Expr::List(rhs_lhs_list) = rhs_bin.lhs()?
    {
        let replacement = format!(
            "{} ++ {}",
            merged_list_text(lhs_list, &rhs_lhs_list)?,
            rhs_bin.rhs()?
        );
        return parse_replacement(&replacement);
    }

    None
}

fn concat_operator_range(bin_expr: &BinOp) -> Option<rnix::TextRange> {
    bin_expr
        .syntax()
        .children_with_tokens()
        .find(|elem| elem.kind() == SyntaxKind::TOKEN_CONCAT)
        .map(|elem| elem.text_range())
}

fn merged_list_text(lhs: &List, rhs: &List) -> Option<String> {
    let lhs_items = lhs.items().map(|item| item.to_string()).collect::<Vec<_>>();
    let rhs_items = rhs.items().map(|item| item.to_string()).collect::<Vec<_>>();

    if lhs_items.is_empty() || rhs_items.is_empty() {
        return None;
    }

    Some(format!(
        "[ {} ]",
        [lhs_items, rhs_items].concat().join(" ")
    ))
}

fn parse_replacement(text: &str) -> Option<rnix::SyntaxNode> {
    let parse = rnix::Root::parse(text).ok().ok()?;
    Some(Expr::cast(parse.syntax().clone())?.syntax().clone())
}
