//! Flags conditions that are certain to fail the Nix boolean check.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{AssertExpr, AstNode, Expr, IfExpr, SyntaxKind as K, TextRange};

/// Flags statically non-boolean `if` and `assert` conditions.
pub struct NonBooleanCondition;

impl Rule for NonBooleanCondition {
    fn code(&self) -> &'static str {
        "non-boolean-condition"
    }

    fn name(&self) -> &'static str {
        "Non-boolean condition"
    }

    fn description(&self) -> &'static str {
        "Flags if and assert conditions that are statically non-boolean, including the global null value. Shadowed names remain unknown."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            let condition = match node.kind() {
                K::IfExpr => IfExpr::cast(&node).and_then(|expr| expr.cond()),
                K::AssertExpr => AssertExpr::cast(&node).and_then(|expr| expr.cond()),
                _ => None,
            };
            let Some(mut condition) = condition else {
                continue;
            };
            while let Expr::Paren(paren) = condition {
                let Some(inner) = paren.expr() else { break };
                condition = inner;
            }
            let is_global_null = matches!(condition, Expr::Ident(token)
            if token.text(model.source()) == "null"
                && model.references().iter().any(|reference| {
                    reference.name.range() == token.range()
                        && reference.resolved.is_none()
                        && reference.via_with.is_none()
                }));
            if !is_global_null && !is_non_boolean_literal(condition) {
                continue;
            }
            let range = cond_range(condition);
            let kind = if node.kind() == K::AssertExpr {
                "assert"
            } else {
                "if"
            };
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!(
                    "{kind}-condition has non-boolean type: `{}`",
                    condition.content_text(model.source())
                ),
                range,
            ));
        }
    }
}

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
