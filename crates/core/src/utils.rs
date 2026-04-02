use crate::make;
use rnix::{
    SyntaxKind, SyntaxNode, TextRange,
    ast::{Expr, Ident},
};
use rowan::ast::AstNode as _;
use std::collections::{HashMap, HashSet};

pub fn with_preceding_whitespace(node: &SyntaxNode) -> TextRange {
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

pub fn mentions_ident(ident: &str, node: &SyntaxNode) -> bool {
    if let Some(node_ident) = Ident::cast(node.clone()) {
        return node_ident.to_string() == ident;
    }
    node.children().any(|child| mentions_ident(ident, &child))
}

/// Format a syntax node as a function argument, adding parentheses if needed
/// to avoid ambiguity (e.g. `a && b` → `(a && b)`, `cond` → `cond`).
pub fn fmt_as_fn_arg(node: &SyntaxNode) -> String {
    let text = node.to_string();
    let text = text.trim();
    let needs_parens = match Expr::cast(node.clone()) {
        Some(Expr::Ident(_) | Expr::Paren(_) | Expr::List(_) | Expr::Str(_) | Expr::AttrSet(_)) => {
            false
        }
        // `foo.bar or default` needs parens to avoid `or` being parsed as an argument
        Some(Expr::Select(select)) => select.or_token().is_some(),
        _ => true,
    };
    if needs_parens {
        format!("({text})")
    } else {
        text.to_owned()
    }
}

pub fn unary_not(node: &SyntaxNode) -> Option<SyntaxNode> {
    if unary_not_needs_parens(node) {
        Some(
            make::unary_not(make::parenthesize(node)?.syntax())?
                .syntax()
                .clone(),
        )
    } else {
        Some(make::unary_not(node)?.syntax().clone())
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

/// Returns every select-expression prefix (≥2 dot-separated components) that
/// appears in 2 or more `NODE_SELECT` nodes anywhere within `scope`.
///
/// Used by `single_use_let` to suppress inlining advice when a binding's
/// value participates in a repeated expression — i.e. when `repeated_expression`
/// would fire on the same let-in.
pub fn repeated_select_prefixes(scope: &SyntaxNode) -> HashSet<String> {
    let mut selects: Vec<String> = Vec::new();
    collect_select_texts(scope, &mut selects);

    let mut prefix_counts: HashMap<String, usize> = HashMap::new();
    for raw in &selects {
        let text = normalize_select(raw);
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() < 2 {
            continue;
        }
        let mut prefix = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                prefix.push('.');
            }
            prefix.push_str(part);
            if i >= 1 {
                *prefix_counts.entry(prefix.clone()).or_insert(0) += 1;
            }
        }
    }

    prefix_counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(p, _)| p)
        .collect()
}

/// Returns `true` if `value_text` (the normalized text of a binding's RHS) is
/// a repeated select expression or an extension of one within `scope`.
pub fn value_is_repeated_select(value_text: &str, repeated: &HashSet<String>) -> bool {
    repeated.iter().any(|p| {
        value_text == p
            || (value_text.starts_with(p.as_str())
                && value_text.as_bytes().get(p.len()) == Some(&b'.'))
    })
}

/// Normalize whitespace in a select expression text so that expressions
/// written with varying trivia still compare equal.
pub fn normalize_select(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_select_texts(node: &SyntaxNode, result: &mut Vec<String>) {
    if node.kind() == SyntaxKind::NODE_SELECT {
        result.push(node.to_string());
    }
    for child in node.children() {
        collect_select_texts(&child, result);
    }
}
