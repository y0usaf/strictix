//! Flags applications whose function is a literal that cannot be called.
//!
//! Unknown and compound expressions are deliberately skipped. Attrsets are
//! callable when they define the conventional `__functor` attribute.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
};
use strictix_syntax::{ApplyExpr, AstNode, AttrItem, Expr, SyntaxKind, SyntaxNode};

pub struct NonCallableApplication;

impl Rule for NonCallableApplication {
    fn code(&self) -> &'static str {
        "non-callable-application"
    }
    fn name(&self) -> &'static str {
        "Non-callable application"
    }
    fn description(&self) -> &'static str {
        "Flags applying a statically known non-callable literal value."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::ApplyExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(apply) = ApplyExpr::cast(node) else {
            return;
        };
        let Some(func) = apply.func() else { return };
        if !is_definitely_non_callable(func, source) {
            return;
        }
        diags.push(Diagnostic::new(
            self.code(),
            self.severity(),
            format!(
                "cannot call non-callable value `{}`",
                func.content_text(source)
            ),
            apply.syntax().content_range(),
        ));
    }
}

fn is_definitely_non_callable(expr: Expr<'_>, source: &str) -> bool {
    let inner = match expr {
        Expr::Paren(paren) => paren.expr().unwrap_or(expr),
        other => other,
    };
    match inner {
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::String(_)
        | Expr::IndString(_)
        | Expr::Path(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_)
        | Expr::List(_) => true,
        Expr::Ident(token) => matches!(token.text(source), "true" | "false" | "null"),
        Expr::Attrset(attrset) => !has_functor(attrset, source),
        Expr::RecAttrset(rec) => rec.attrset().is_some_and(|a| !has_functor(a, source)),
        _ => false,
    }
}

fn has_functor(attrset: strictix_syntax::AttrsetExpr<'_>, source: &str) -> bool {
    attrset.items().any(|item| match item {
        AttrItem::Binding(binding) => binding
            .attrpath()
            .and_then(|path| path.elements().next())
            .is_some_and(|name| match name {
                strictix_syntax::AttrName::Ident(token) => token.text(source) == "__functor",
                _ => false,
            }),
        AttrItem::Inherit(_) => false,
    })
}
