use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
};
use strictix_syntax::{AstNode, BinExpr, Expr, SyntaxKind, SyntaxNode};

pub struct IllTypedBinop;

impl Rule for IllTypedBinop {
    fn code(&self) -> &'static str {
        "ill-typed-binop"
    }

    fn name(&self) -> &'static str {
        "Ill-typed binary operator"
    }

    fn description(&self) -> &'static str {
        "Flags a binary operator applied to statically ill-typed literal operands. Nix has no overloading or truthiness, so such an operation always fails — and because evaluation is lazy the error hides until forced."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(bin_expr) = BinExpr::cast(node) else {
            return;
        };
        let Some(op_kind) = bin_expr.op() else {
            return;
        };
        let Some(op) = operator(op_kind) else {
            return;
        };
        let (Some(lhs), Some(rhs)) = (bin_expr.lhs(), bin_expr.rhs()) else {
            return;
        };
        let l = operand_type(lhs, source);
        let r = operand_type(rhs, source);
        if !should_fire(op, l, r) {
            return;
        }
        let mut diag = Diagnostic::new(
            self.code(),
            self.severity(),
            format!(
                "`{}` operands are statically ill-typed: {} and {}",
                op.symbol(),
                lhs.content_text(source),
                rhs.content_text(source)
            ),
            bin_expr.syntax().content_range(),
        );
        if op == Operator::Plus {
            if let Some(fix) = concat_fix(lhs, rhs, source, node) {
                diag = diag.with_fix(fix);
            }
        }
        diags.push(diag);
    }
}

fn operator(kind: SyntaxKind) -> Option<Operator> {
    use strictix_syntax::SyntaxKind::*;
    Some(match kind {
        Plus => Operator::Plus,
        Minus => Operator::Minus,
        Star => Operator::Star,
        Slash => Operator::Slash,
        PlusPlus => Operator::Concat,
        SlashSlash => Operator::Merge,
        AndAnd => Operator::AndAnd,
        OrOr => Operator::OrOr,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operator {
    Plus,
    Minus,
    Star,
    Slash,
    Concat,
    Merge,
    AndAnd,
    OrOr,
}

impl Operator {
        fn symbol(self) -> &'static str {
        match self {
            Operator::Plus => "+",
            Operator::Minus => "-",
            Operator::Star => "*",
            Operator::Slash => "/",
            Operator::Concat => "++",
            Operator::Merge => "//",
            Operator::AndAnd => "&&",
            Operator::OrOr => "||",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lit {
    Int,
    Float,
    Str,
    IndStr,
    Path,
    Uri,
    List,
    Attrset,
    RecAttrset,
        Bool,
        Null,
}

fn operand_type(expr: Expr<'_>, source: &str) -> Option<Lit> {
    let inner = match expr {
        Expr::Paren(paren) => paren.expr().unwrap_or(expr),
        other => other,
    };
    Some(match inner {
        Expr::Int(_) => Lit::Int,
        Expr::Float(_) => Lit::Float,
        Expr::String(_) => Lit::Str,
        Expr::IndString(_) => Lit::IndStr,
        Expr::Path(_) => Lit::Path,
        Expr::Uri(_) => Lit::Uri,
        Expr::List(_) => Lit::List,
        Expr::Attrset(_) => Lit::Attrset,
        Expr::RecAttrset(_) => Lit::RecAttrset,
        Expr::Ident(t) => match t.text(source) {
            "true" | "false" => Lit::Bool,
            "null" => Lit::Null,
            _ => return None,
        },
        _ => return None,
    })
}

fn should_fire(op: Operator, l: Option<Lit>, r: Option<Lit>) -> bool {
    if op == Operator::Merge {
        if l.is_some_and(is_attr_like) || r.is_some_and(is_attr_like) {
            return false;
        }
        return l.is_some_and(is_merge_bad) || r.is_some_and(is_merge_bad);
    }
    let (Some(l), Some(r)) = (l, r) else {
        return false;
    };
    match op {
        Operator::Plus => {
            if l == Lit::Path || r == Lit::Path {
                return false;
            }
            is_string(l) || is_string(r) || is_attr_like(l) || is_attr_like(r)
        }
        Operator::Minus | Operator::Star | Operator::Slash => {
            if l == Lit::Path || r == Lit::Path {
                return false;
            }
            is_string(l) || is_string(r) || is_attr_like(l) || is_attr_like(r)
        }
        Operator::Concat => {
            if l == Lit::List && r == Lit::List {
                return false;
            }
            is_concat_bad(l) || is_concat_bad(r)
        }
        Operator::Merge => unreachable!("merge is decided before the both-literal gate"),
        Operator::AndAnd | Operator::OrOr => is_boolean_bad(l) || is_boolean_bad(r),
    }
}

fn is_string(l: Lit) -> bool {
    matches!(l, Lit::Str | Lit::IndStr)
}

fn is_attr_like(l: Lit) -> bool {
    matches!(l, Lit::List | Lit::Attrset | Lit::RecAttrset)
}

fn is_concat_bad(l: Lit) -> bool {
    !matches!(l, Lit::List | Lit::Uri)
}

fn is_merge_bad(l: Lit) -> bool {
    !matches!(l, Lit::Attrset | Lit::RecAttrset | Lit::Uri)
}

fn is_boolean_bad(l: Lit) -> bool {
    l != Lit::Bool
}

fn concat_fix(lhs: Expr<'_>, rhs: Expr<'_>, source: &str, node: &SyntaxNode) -> Option<Fix> {
    let mut value = plain_string(lhs, source)?;
    value.push_str(&plain_string(rhs, source)?);
    Some(Fix::new("concatenate string literals").edit(node.content_range(), quote_string(&value)))
}

fn plain_string(expr: Expr<'_>, source: &str) -> Option<String> {
    use strictix_syntax::StringPart;
    let inner = match expr {
        Expr::Paren(paren) => paren.expr().unwrap_or(expr),
        other => other,
    };
    let Expr::String(s) = inner else {
        return None;
    };
    if s.parts().any(|part| matches!(part, StringPart::Interp(_))) {
        return None;
    }
    let mut out = String::new();
    for part in s.parts() {
        if let StringPart::Content(token) = part {
            out.push_str(&decode_escapes(token.text(source)));
        }
    }
    Some(out)
}

fn decode_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('$') => out.push('$'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn quote_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
