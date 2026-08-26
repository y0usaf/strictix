//! A node rule that flags if-expressions whose condition is statically
//! a non-boolean literal.
//!
//! Nix has no truthiness: an `if <cond>` requires the condition to be a
//! boolean, and any non-boolean literal there always fails at runtime
//! with "attempt to use ... as a boolean". Because the evaluator is
//! lazy, that error is only raised when the bad branch is actually
//! forced, so it can lurk undetected. This rule catches the literal
//! cases that are certain.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
};
use strictix_syntax::{AstNode, Expr, IfExpr, SyntaxKind as K, SyntaxNode, TextRange};

/// Flags `if`-expressions whose condition is statically a non-boolean
/// literal. No auto-fix: there is no safe rewrite for a condition that
/// can never be boolean.
pub struct NonBooleanCondition;

impl Rule for NonBooleanCondition {
    fn code(&self) -> &'static str {
        "non-boolean-condition"
    }

    fn name(&self) -> &'static str {
        "Non-boolean condition"
    }

    fn description(&self) -> &'static str {
        "Flags if-expressions whose condition is statically a non-boolean literal. Nix has no truthiness — such a condition always fails with 'attempt to use ... as a boolean', and because evaluation is lazy the error only surfaces when the bad branch is forced."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn node_kind(&self) -> Option<K> {
        Some(K::IfExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(if_expr) = IfExpr::cast(node) else {
            return;
        };
        let Some(mut cond) = if_expr.cond() else {
            return;
        };
        // Unwrap a single layer of parens fully: `if (0) ...` is still
        // an integer, and `if ((x)) ...` is still just the ident x.
        while let Expr::Paren(paren) = cond {
            let Some(inner) = paren.expr() else {
                return;
            };
            cond = inner;
        }
        // true/false/null and every variable are Idents — not provably
        // non-boolean at this node, so never fire on Ident. Everything
        // else in the non-boolean set is a literal that can never be a
        // boolean.
        if !is_non_boolean_literal(cond) {
            return;
        }
        let range = cond_range(cond);
        diags.push(Diagnostic::new(
            "non-boolean-condition",
            Severity::Error,
            format!(
                "if-condition has non-boolean type: `{}`",
                cond.content_text(source)
            ),
            range,
        ));
    }
}

/// Whether an expression is a literal that is statically never a
/// boolean. The atom-literal and node-literal variants are all certain;
/// Ident is deliberately omitted (variables, `true`, `false`, `null`
/// are all idents) as are all compound/non-literal expressions (which
/// may evaluate to a boolean).
fn is_non_boolean_literal(expr: Expr<'_>) -> bool {
    matches!(
        expr,
        Expr::Int(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::IndString(_)
            | Expr::List(_)
            | Expr::Path(_)
            | Expr::SearchPath(_)
            | Expr::Uri(_)
            | Expr::Attrset(_)
            | Expr::RecAttrset(_)
    )
}

/// The content byte range of a condition. Atom literals are single
/// tokens so their token range is exact; node literals flush leading
/// trivia into their range, so the trimmed `content_range` is used.
fn cond_range(cond: Expr<'_>) -> TextRange {
    match cond {
        Expr::Ident(t)
        | Expr::Int(t)
        | Expr::Float(t)
        | Expr::Path(t)
        | Expr::SearchPath(t)
        | Expr::Uri(t) => t.range(),
        Expr::String(e) => e.syntax().content_range(),
        Expr::IndString(e) => e.syntax().content_range(),
        Expr::List(e) => e.syntax().content_range(),
        Expr::Attrset(e) => e.syntax().content_range(),
        Expr::RecAttrset(e) => e.syntax().content_range(),
        _ => cond.range(),
    }
}
