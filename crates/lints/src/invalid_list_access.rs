//! Flags provably failing applications of the list access builtins.
//!
//! This rule is deliberately syntax directed: only a literal list and a
//! literal integer index are enough to prove an `elemAt` failure.  Unknown
//! values, aliases, and arbitrary evaluation are left alone.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{ApplyExpr, AstNode, AttrName, Expr, ListExpr, SyntaxKind};

pub struct InvalidListAccess;

impl Rule for InvalidListAccess {
    fn code(&self) -> &'static str {
        "invalid-list-access"
    }

    fn name(&self) -> &'static str {
        "Invalid list access"
    }

    fn description(&self) -> &'static str {
        "Flags list access builtins that are provably applied outside a literal list's bounds."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            let Some((callee, args)) = call_parts(apply) else {
                continue;
            };
            let Some(name) = global_builtin(model, callee) else {
                continue;
            };

            match (name, args.as_slice()) {
                ("head" | "tail", [list]) => {
                    let Some(list) = literal_list(*list) else {
                        continue;
                    };
                    if list.items().next().is_some() {
                        continue;
                    }
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!("builtin '{name}' cannot be applied to an empty list"),
                        list.syntax().content_range(),
                    ));
                }
                ("elemAt", [list, index]) => {
                    let Some(list) = literal_list(*list) else {
                        continue;
                    };
                    let index_expr = *index;
                    let Some(index) = literal_integer(index_expr, source) else {
                        continue;
                    };
                    let length = list.items().count() as i128;
                    if index >= 0 && index < length {
                        continue;
                    }
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!("builtin 'elemAt' index {index} is out of bounds for list of length {length}"),
                        index_expr.content_range(),
                    ));
                }
                _ => {}
            }
        }
    }
}

fn literal_list(expr: Expr<'_>) -> Option<ListExpr<'_>> {
    match expr {
        Expr::List(list) => Some(list),
        Expr::Paren(paren) => paren.expr().and_then(literal_list),
        _ => None,
    }
}

fn literal_integer(expr: Expr<'_>, source: &str) -> Option<i128> {
    match expr {
        Expr::Int(token) => token.text(source).parse().ok(),
        Expr::Paren(paren) => paren.expr().and_then(|e| literal_integer(e, source)),
        Expr::Unary(unary)
            if unary
                .syntax()
                .child_tokens()
                .any(|t| t.kind() == SyntaxKind::Minus) =>
        {
            let operand = unary.operand()?;
            let Expr::Int(token) = operand else {
                return None;
            };
            token.text(source).parse::<i128>().ok()?.checked_neg()
        }
        _ => None,
    }
}

/// Flatten one complete application chain.  A bare builtin and a partial
/// application produce fewer arguments and therefore do not match callers.
fn call_parts<'a>(apply: ApplyExpr<'a>) -> Option<(Expr<'a>, Vec<Expr<'a>>)> {
    let mut args = Vec::new();
    let mut current = Expr::Apply(apply);
    loop {
        while let Expr::Paren(paren) = current {
            current = paren.expr()?;
        }
        let Expr::Apply(node) = current else {
            args.reverse();
            return Some((current, args));
        };
        args.push(node.arg()?);
        current = node.func()?;
    }
}

fn global_builtin<'a>(model: &SemanticModel<'a>, callee: Expr<'a>) -> Option<&'static str> {
    let source = model.source();
    match callee {
        // These functions are members of the `builtins` set. Bare names are
        // ordinary variables and may be supplied by a surrounding `with`.
        Expr::Ident(_) => None,
        Expr::Paren(paren) => paren.expr().and_then(|expr| global_builtin(model, expr)),
        Expr::Select(select) => {
            let Expr::Ident(base) = select.base()? else {
                return None;
            };
            if base.text(source) != "builtins"
                || !crate::static_binding::is_unshadowed(base, model)
                || select.default().is_some()
            {
                return None;
            }
            if model
                .references()
                .iter()
                .any(|r| r.name.range() == base.range() && r.via_with.is_some())
            {
                return None;
            }
            let mut elements = select.attrpath()?.elements();
            let AttrName::Ident(name) = elements.next()? else {
                return None;
            };
            if elements.next().is_some() {
                return None;
            }
            match name.text(source) {
                "head" => Some("head"),
                "tail" => Some("tail"),
                "elemAt" => Some("elemAt"),
                _ => None,
            }
        }
        _ => None,
    }
}
