//! A node rule that flags binary operators applied to statically
//! ill-typed literal operands.
//!
//! Nix has no operator overloading and no truthiness. A *literal*
//! operand of an incompatible type is therefore a certain evaluation
//! error — but because the evaluator is lazy, such an error hides until
//! the value is forced, so it can ship unnoticed. This rule catches the
//! literal cases that are certain.
//!
//! "Literal operand" is defined as statically an Int, Float, Str /
//! plain String, IndString, Path, Uri, List, Attset, or RecAttrset,
//! plus the constant idents `true`/`false` (Bool) and `null`. Any
//! other expression — an ordinary Ident, an application, a select, a
//! `let`, or a Paren beyond the one unwrap layer — is not provably
//! ill-typed, so the rule never fires on it. One paren layer is
//! unwrapped per operand before classifying.
//!
//! Relational operators (`==`, `!=`, `<`, `>`, `<=`, `>=`) and lambdas
//! (`->`) are skipped entirely (relational comparison is total, and a
//! lambda operand isn't a bug): FP risk outweighs the catch.
//!
//! Paths are a special case. Nix's `+` defines path+string / path+path
//! segment-append semantics and every arithmetic/string operator accepts
//! a path operand, so any headline with a Path literal is skipped (the
//! "path carve-out"). The `//` merge never fires when an attrset/rec
//! attrset is involved (that is its entire purpose), and `++` is clean
//! when both sides are lists.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
};
use strictix_syntax::{AstNode, BinExpr, Expr, SyntaxKind, SyntaxNode};

/// Flags a binary operator applied to statically ill-typed literal
/// operands. Severity is [Severity::Error]. One auto-fix, and only for
/// the truly-certain case: `+` with two plain, interpolation-free
/// `"..."` string literals, which can be spliced into one literal.
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
            return; // relational / lambda ops and everything else: skip
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
        // The one auto-fix: `+` on two plain (non-interpolated) strings
        // collapses to their concatenation.
        if op == Operator::Plus {
            if let Some(fix) = concat_fix(lhs, rhs, source, node) {
                diag = diag.with_fix(fix);
            }
        }
        diags.push(diag);
    }
}

/// The operators this rule knows how to classify; `None` for the
/// always-skipped relational/lambda operators.
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
        // EqEq, Neq, Lt, LtEq, Gt, GtEq, Arrow: skipped.
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
    /// The source symbol, for the diagnostic message.
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

/// The statically-known type class of a literal operand.
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
    /// The constant idents `true`/`false`.
    Bool,
    /// The constant ident `null`.
    Null,
}

/// Classify a literal operand into a [Lit], unwrapping exactly one paren
/// layer first. The constant idents `true`/`false`/`null` classify as
/// Bool/Null — rebinding them is its own lint (rebound-constant), so the
/// constant reading is the only sane one. Anything that is not a
/// statically-known literal is `None`, and `None` never fires.
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
            // An ordinary ident, application, select, let, and every
            // other compound: not provably ill-typed.
            _ => return None,
        },
        _ => return None,
    })
}

/// Whether the operator must fire given the two literal types. `None`
/// on either side never fires.
fn should_fire(op: Operator, l: Option<Lit>, r: Option<Lit>) -> bool {
    // `//` fires one-sided: a non-attrset literal can never merge, no
    // matter what the other operand turns out to be.
    if op == Operator::Merge {
        if l.is_some_and(is_attr_like) || r.is_some_and(is_attr_like) {
            return false; // `//` with an attrset is its whole purpose
        }
        return l.is_some_and(is_merge_bad) || r.is_some_and(is_merge_bad);
    }
    let (Some(l), Some(r)) = (l, r) else {
        return false;
    };
    match op {
        Operator::Plus => {
            // Path carve-out: path segment append is defined, so a
            // path on either side wins.
            if l == Lit::Path || r == Lit::Path {
                return false;
            }
            // A string, list, or attrset on a side is provably wrong
            // for `+` (the string+string case is still flagged; the
            // concatenation fix is the point of the rule).
            is_string(l) || is_string(r) || is_attr_like(l) || is_attr_like(r)
        }
        Operator::Minus | Operator::Star | Operator::Slash => {
            // `-`, `*`, `/`: a path operand is defined; skip.
            if l == Lit::Path || r == Lit::Path {
                return false;
            }
            is_string(l) || is_string(r) || is_attr_like(l) || is_attr_like(r)
        }
        Operator::Concat => {
            // `++` is list concat; both lists are fine. Any listed
            // literal (string/int/float/attr/rec-attr/path) on a side
            // is wrong, and so is a list against a non-list literal.
            if l == Lit::List && r == Lit::List {
                return false; // legal list concat
            }
            is_concat_bad(l) || is_concat_bad(r)
        }
        // Handled above (one-sided).
        Operator::Merge => unreachable!("merge is decided before the both-literal gate"),
        Operator::AndAnd | Operator::OrOr => {
            // `&&`/`||` need booleans; any non-boolean literal on a side
            // is a certain error (`true && x` is clean — an Ident may be
            // a boolean variable).
            is_boolean_bad(l) || is_boolean_bad(r)
        }
    }
}

fn is_string(l: Lit) -> bool {
    matches!(l, Lit::Str | Lit::IndStr)
}

/// Attrset-like: lists, attrsets, rec attrsets are all values `+`
/// rejects, and all values the `//` merge accepts.
fn is_attr_like(l: Lit) -> bool {
    matches!(l, Lit::List | Lit::Attrset | Lit::RecAttrset)
}

/// A literal `++` cannot concatenate: everything except a list.
fn is_concat_bad(l: Lit) -> bool {
    !matches!(l, Lit::List | Lit::Uri)
}

/// A literal `//` rejects when the other side is not an attrset:
/// everything except attrset-likes (and Uri, which unquoted-uri owns).
fn is_merge_bad(l: Lit) -> bool {
    !matches!(l, Lit::Attrset | Lit::RecAttrset | Lit::Uri)
}

/// A non-boolean literal `&&`/`||` reject exactly (everything except
/// Bool; an ordinary Ident never classifies at all).
fn is_boolean_bad(l: Lit) -> bool {
    l != Lit::Bool
}

/// Build the single auto-fix: `+` on two plain, interpolation-free
/// string literals collapses to their concatenation. The BinExpr node's
/// content range (through the closing quote of the RHS) is replaced with
/// one quoted literal. Any interpolation part, or a non-plain-string
/// side, means no fix.
fn concat_fix(lhs: Expr<'_>, rhs: Expr<'_>, source: &str, node: &SyntaxNode) -> Option<Fix> {
    let mut value = plain_string(lhs, source)?;
    value.push_str(&plain_string(rhs, source)?);
    Some(Fix::new("concatenate string literals").edit(node.content_range(), quote_string(&value)))
}

/// The runtime value of a plain `"..."` (non-interpolated) string
/// literal, or None for indentation strings, interpolated strings, and
/// non-strings. One paren layer is unwrapped so `("a") + "b"` still
/// fixes.
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
        return None; // interpolation → no fix
    }
    let mut out = String::new();
    for part in s.parts() {
        if let StringPart::Content(token) = part {
            out.push_str(&decode_escapes(token.text(source)));
        }
    }
    Some(out)
}

/// Decode Nix single-`"..."` string escapes from the raw content text.
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

/// Re-quote a runtime string value as a Nix `"..."` literal, escaping
/// the characters that would otherwise break the literal or turn into
/// an interpolation.
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
