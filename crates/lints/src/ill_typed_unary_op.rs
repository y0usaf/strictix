//! Flags unary operators applied to literals of incompatible types.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
};
use strictix_syntax::{AstNode, Expr, SyntaxKind, SyntaxNode, UnaryExpr};

pub struct IllTypedUnaryOp;

impl Rule for IllTypedUnaryOp {
    fn code(&self) -> &'static str {
        "ill-typed-unary-op"
    }
    fn name(&self) -> &'static str {
        "Ill-typed unary operator"
    }
    fn description(&self) -> &'static str {
        "Flags unary operators applied to statically incompatible literal values."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::UnaryExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(unary) = UnaryExpr::cast(node) else {
            return;
        };
        let Some(operand) = unary.operand() else {
            return;
        };
        // The parser currently represents `!` as UnaryExpr. Future prefix
        // operators can use the same rule once represented by the AST.
        let Some(kind) = node.child_tokens().find_map(|token| match token.kind() {
            SyntaxKind::Bang => Some(UnaryOp::Not),
            SyntaxKind::Minus => Some(UnaryOp::Neg),
            _ => None,
        }) else {
            return;
        };
        let Some(lit) = literal(operand, source) else {
            return;
        };
        let invalid = match kind {
            UnaryOp::Not => lit != Literal::Bool,
            UnaryOp::Neg => !matches!(lit, Literal::Int | Literal::Float),
        };
        if !invalid {
            return;
        }
        diags.push(Diagnostic::new(
            self.code(),
            self.severity(),
            format!(
                "`{}` operand is statically ill-typed: {}",
                kind.symbol(),
                operand.content_text(source)
            ),
            unary.syntax().content_range(),
        ));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnaryOp {
    Not,
    Neg,
}
impl UnaryOp {
    fn symbol(self) -> &'static str {
        match self {
            Self::Not => "!",
            Self::Neg => "-",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Literal {
    Int,
    Float,
    Str,
    Bool,
    Other,
}

fn literal(expr: Expr<'_>, source: &str) -> Option<Literal> {
    let inner = match expr {
        Expr::Paren(paren) => paren.expr().unwrap_or(expr),
        other => other,
    };
    Some(match inner {
        Expr::Int(_) => Literal::Int,
        Expr::Float(_) => Literal::Float,
        Expr::String(_) | Expr::IndString(_) => Literal::Str,
        Expr::Ident(token) => match token.text(source) {
            "true" | "false" => Literal::Bool,
            _ => return None,
        },
        Expr::Path(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_)
        | Expr::List(_)
        | Expr::Attrset(_)
        | Expr::RecAttrset(_) => Literal::Other,
        _ => return None,
    })
}
