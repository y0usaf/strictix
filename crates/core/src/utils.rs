use crate::make;
use rnix::{
    SyntaxKind, SyntaxNode, TextRange,
    ast::{Expr, Ident},
};
use rowan::ast::AstNode as _;

pub fn with_preceeding_whitespace(node: &SyntaxNode) -> TextRange {
    let start = node.prev_sibling_or_token().map_or_else(
        || node.text_range().start(),
        |t| {
            if t.kind() == SyntaxKind::TOKEN_WHITESPACE {
                t.text_range().start()
            } else {
                t.text_range().end()
            }
        },
    );
    let end = node.text_range().end();
    TextRange::new(start, end)
}

pub fn bool_literal(expr: &Expr) -> Option<bool> {
    let Expr::Ident(ident) = expr else {
        return None;
    };
    bool_literal_node(ident.syntax())
}

pub fn bool_literal_node(node: &SyntaxNode) -> Option<bool> {
    Ident::cast(node.clone()).and_then(|ident| match ident.to_string().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

pub fn unary_not(node: &SyntaxNode) -> SyntaxNode {
    if unary_not_needs_parens(node) {
        make::unary_not(make::parenthesize(node).syntax())
            .syntax()
            .clone()
    } else {
        make::unary_not(node).syntax().clone()
    }
}

fn unary_not_needs_parens(node: &SyntaxNode) -> bool {
    !matches!(
        node.kind(),
        SyntaxKind::NODE_APPLY
            | SyntaxKind::NODE_PAREN
            | SyntaxKind::NODE_IDENT
            | SyntaxKind::NODE_HAS_ATTR
    )
}
