use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, TextRange,
    ast::{Attr, Entry, Expr, HasEntry as _, Inherit, LetIn},
};
use rowan::{Direction, ast::AstNode as _};

/// ## What it does
/// Checks for `let-in` expressions whose body is another `let-in`
/// expression.
///
/// ## Why is this bad?
/// Unnecessary code, the `let-in` expressions can be merged.
///
/// ## Example
///
/// ```nix
/// let
///   a = 2;
/// in
/// let
///   b = 3;
/// in
///   a + b
/// ```
///
/// Merge both `let-in` expressions:
///
/// ```nix
/// let
///   a = 2;
///   b = 3;
/// in
///   a + b
/// ```
#[lint(
    name = "collapsible_let_in",
    note = "These let-in expressions are collapsible",
    code = 6,
    match_with = SyntaxKind::NODE_LET_IN
)]
struct CollapsibleLetIn;

impl Rule for CollapsibleLetIn {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let let_in_expr = LetIn::cast(node.clone())?;
        let body = let_in_expr.body()?;

        let Expr::LetIn(ref inner_let) = body else {
            return None;
        };

        // Collapsing is only safe if neither let defines the same name as the other.
        // Duplicate bindings in the same `let` block are a parse/eval error.
        let outer_names = let_binding_names(&let_in_expr);
        let inner_names = let_binding_names(inner_let);
        if outer_names.iter().any(|n| inner_names.contains(n)) {
            return None;
        }

        let first_annotation = node.text_range();
        let first_message = "This `let in` expression contains a nested `let in` expression";

        let second_annotation = body.syntax().text_range();
        let second_message = "This `let in` expression is nested";

        let replacement_at = {
            let start = body
                .syntax()
                .siblings_with_tokens(Direction::Prev)
                .find(|elem| elem.kind() == SyntaxKind::TOKEN_IN)?
                .text_range()
                .start();
            let end = body
                .syntax()
                .descendants_with_tokens()
                .find(|elem| elem.kind() == SyntaxKind::TOKEN_LET)?
                .text_range()
                .end();
            TextRange::new(start, end)
        };

        Some(
            self.report()
                .diagnostic(first_annotation, first_message)
                .suggest(
                    second_annotation,
                    second_message,
                    Suggestion::with_empty(replacement_at),
                ),
        )
    }
}

/// Collect all top-level binding names introduced by a `let-in` expression.
fn let_binding_names(let_in: &LetIn) -> Vec<String> {
    let mut names = Vec::new();
    for entry in let_in.entries() {
        match entry {
            Entry::AttrpathValue(kv) => {
                let Some(attrpath) = kv.attrpath() else {
                    continue;
                };
                let Some(first) = attrpath.attrs().next() else {
                    continue;
                };
                if let Attr::Ident(ident) = first {
                    names.push(ident.to_string());
                }
            }
            Entry::Inherit(inherit) => {
                collect_inherit_names(&inherit, &mut names);
            }
        }
    }
    names
}

fn collect_inherit_names(inherit: &Inherit, names: &mut Vec<String>) {
    for attr in inherit.attrs() {
        if let Attr::Ident(ident) = attr {
            names.push(ident.to_string());
        }
    }
}
