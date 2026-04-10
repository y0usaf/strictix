use std::{fmt::Write, iter::IntoIterator};

use rnix::{
    Root, SyntaxNode,
    ast::{self, AstNode},
};
use rowan::ast::AstNode as _;

fn ast_from_text<N: AstNode>(text: &str) -> Option<N> {
    let parse = Root::parse(text).ok().ok()?;
    parse.syntax().descendants().find_map(N::cast)
}

pub fn parenthesize(node: &SyntaxNode) -> Option<ast::Paren> {
    ast_from_text(&format!("({node})"))
}

pub fn quote(node: &SyntaxNode) -> Option<ast::Str> {
    ast_from_text(&format!("\"{node}\""))
}

pub fn unary_not(node: &SyntaxNode) -> Option<ast::UnaryOp> {
    ast_from_text(&format!("!{node}"))
}

pub fn inherit_stmt<'a>(nodes: impl IntoIterator<Item = &'a ast::Ident>) -> Option<ast::Inherit> {
    let inherited_idents = nodes
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    ast_from_text(&format!("{{ inherit {inherited_idents}; }}"))
}

pub fn inherit_from_stmt_text<'a>(
    from: &str,
    nodes: impl IntoIterator<Item = &'a ast::Ident>,
) -> Option<ast::Inherit> {
    let inherited_idents = nodes
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    ast_from_text(&format!("{{ inherit ({from}) {inherited_idents}; }}"))
}

pub fn attrset(
    inherits: impl IntoIterator<Item = ast::Inherit>,
    entries: impl IntoIterator<Item = ast::Entry>,
    recursive: bool,
) -> Option<ast::AttrSet> {
    let mut buffer = String::new();

    writeln!(buffer, "{}{{", if recursive { "rec " } else { "" }).expect("write to String buffer");
    for inherit in inherits {
        writeln!(buffer, "  {inherit}").expect("write to String buffer");
    }
    for entry in entries {
        writeln!(buffer, "  {entry}").expect("write to String buffer");
    }
    write!(buffer, "}}").expect("write to String buffer");

    ast_from_text(&buffer)
}

pub fn select(set: &SyntaxNode, index: &SyntaxNode) -> Option<ast::Select> {
    ast_from_text(&format!("{set}.{index}"))
}

pub fn ident(text: &str) -> Option<ast::Ident> {
    ast_from_text(text)
}

pub fn binary(lhs: &SyntaxNode, op: &str, rhs: &SyntaxNode) -> Option<ast::BinOp> {
    ast_from_text(&format!("{lhs} {op} {rhs}"))
}

pub fn or_default(
    set: &SyntaxNode,
    index: &SyntaxNode,
    default: &SyntaxNode,
) -> Option<ast::Select> {
    ast_from_text(&format!("{set}.{index} or {default}"))
}
