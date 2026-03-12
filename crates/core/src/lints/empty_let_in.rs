use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{HasEntry as _, LetIn},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `let-in` expressions which create no new bindings.
///
/// ## Why is this bad?
/// `let-in` expressions that create no new bindings are useless.
/// These are probably remnants from debugging or editing expressions.
///
/// ## Example
///
/// ```nix
/// let in pkgs.statix
/// ```
///
/// Preserve only the body of the `let-in` expression:
///
/// ```nix
/// pkgs.statix
/// ```
#[lint(
    name = "empty_let_in",
    note = "Useless let-in expression",
    code = 2,
    match_with = SyntaxKind::NODE_LET_IN
)]
struct EmptyLetIn;

impl Rule for EmptyLetIn {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        if let NodeOrToken::Node(node) = node
            && let Some(let_in_expr) = LetIn::cast(node.clone())
            && let entries = let_in_expr.entries()
            && let inherits = let_in_expr.inherits()
            && entries.count() == 0
            && inherits.count() == 0
            && let Some(_body) = let_in_expr.body()
        {
            let at = node.text_range();
            let message = "This let-in expression has no entries";
            let replacement = empty_let_replacement(&let_in_expr)?;
            Some(
                self.report()
                    .suggest(at, message, Suggestion::with_text(at, replacement)),
            )
        } else {
            None
        }
    }
}

fn empty_let_replacement(let_in_expr: &LetIn) -> Option<String> {
    let let_node = let_in_expr.syntax();
    let let_text = let_node.to_string();
    let let_token = let_in_expr.let_token()?;
    let in_token = let_in_expr.in_token()?;
    let body = let_in_expr.body()?;

    let let_start = usize::from(let_node.text_range().start());
    let between_start = usize::from(let_token.text_range().end()) - let_start;
    let between_end = usize::from(in_token.text_range().start()) - let_start;
    let body_start = usize::from(body.syntax().text_range().start()) - let_start;
    let between = &let_text[between_start..between_end];
    let preserved_trivia = if between.contains('#') { between } else { "" };

    Some(format!("{}{}", preserved_trivia, &let_text[body_start..]))
}
