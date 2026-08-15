//! Node rules: syntax-level lints that fire on one node kind each.
//!
//! These are the structural rules — they look only at the lossless tree,
//! never at meaning. Each rule declares its node kind via
//! [Rule::node_kind] and inspects exactly those nodes in
//! [Rule::check_node]. They never touch the semantic model, so a run
//! with only node rules enabled never builds it.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
};
use strictix_syntax::SyntaxKind as K;
use strictix_syntax::{
    AssertExpr, AstNode, BinExpr, Expr, IfExpr, SyntaxKind, SyntaxNode, TextRange,
};

/// Flags `if`-expressions whose condition is the literal `true` or
/// `false`.
///
/// A constant condition makes one branch dead by construction. This is
/// classic AI-generated slop — `if true then a else b` is just `a` —
/// so the rule offers a fix that inlines the live branch.
pub struct ConstantIf;

impl Rule for ConstantIf {
    fn code(&self) -> &'static str {
        "constant-if"
    }

    fn name(&self) -> &'static str {
        "Constant condition"
    }

    fn description(&self) -> &'static str {
        "Flags if-expressions whose condition is the literal true or false. One branch is dead by construction; the live branch can be inlined (the fix does exactly that)."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(K::IfExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(if_expr) = IfExpr::cast(node) else {
            return;
        };
        let Some(cond) = if_expr.cond() else { return };
        // In Nix, true/false lex as Ident tokens. A *binding* named true
        // is legal ({ true = 1; }), but in expression position these
        // texts are always the literals — match on the exact text, which
        // is the only way to tell the two apart.
        let is_true = matches!(cond, Expr::Ident(t) if t.text(source) == "true");
        let is_false = matches!(cond, Expr::Ident(t) if t.text(source) == "false");
        if !is_true && !is_false {
            return;
        }
        let branch = if is_true {
            if_expr.then_branch()
        } else {
            if_expr.else_branch()
        };
        let mut diag = Diagnostic::new(
            "constant-if",
            Severity::Warning,
            "constant condition in if",
            node.content_range(),
        );
        if let Some(branch_expr) = branch {
            let label = if is_true {
                "replace with then branch"
            } else {
                "replace with else branch"
            };
            let fix = Fix::new(label).edit(node.content_range(), branch_expr.text(source));
            diag = diag.with_fix(fix);
        }
        diags.push(diag);
    }
}

/// Flags `assert true;` — a precondition that can never fail.
///
/// An assertion of a statically-true condition adds noise without
/// checking anything. The fix drops the `assert true;` prefix, keeping
/// the body.
pub struct AssertTrue;

impl Rule for AssertTrue {
    fn code(&self) -> &'static str {
        "assert-true"
    }

    fn name(&self) -> &'static str {
        "Assert always true"
    }

    fn description(&self) -> &'static str {
        "Flags assert-expressions whose condition is the literal true. The assertion never fails, so it guards nothing; the fix removes the dead prefix."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(K::AssertExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(assert_expr) = AssertExpr::cast(node) else {
            return;
        };
        let Some(cond) = assert_expr.cond() else {
            return;
        };
        // Same exact-text check as constant-if: true is an Ident token,
        // and in expression position that text is always the literal.
        let is_true = matches!(cond, Expr::Ident(t) if t.text(source) == "true");
        if !is_true {
            return;
        }
        let mut diag = Diagnostic::new(
            "assert-true",
            Severity::Warning,
            "assert true is always satisfied",
            node.content_range(),
        );
        // The AssertExpr node's range runs through the body, so the fix
        // targets assert-start .. the semicolon that ends the condition.
        // Only direct-child semicolons qualify — a semicolon inside the
        // body belongs to a child node and is never the terminator here.
        if let Some(semi) = node.child_tokens().find(|t| t.kind() == K::Semicolon) {
            let range = TextRange::new(node.range().start(), semi.range().end());
            let fix = Fix::new("remove assert true").edit(range, "");
            diag = diag.with_fix(fix);
        }
        diags.push(diag);
    }
}

/// Flags binary comparisons of an atom with itself.
///
/// `x == x`, `x != x`, `x && x`, and `x || x` have a fixed truth value
/// for any `x`. Only same-kind atoms with identical source text are
/// compared: `1 == 1` is flagged, `1 == 1.0` is not (the texts differ
/// in kind, and floats and ints are not equal in Nix).
pub struct Tautology;

impl Rule for Tautology {
    fn code(&self) -> &'static str {
        "tautology"
    }

    fn name(&self) -> &'static str {
        "Tautological comparison"
    }

    fn description(&self) -> &'static str {
        "Flags binary expressions comparing an atom with itself (x == x, x != x, x && x, x || x), which have a fixed truth value. AI-generated Nix often repeats an operand to pad a condition; the comparison is dead code."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(K::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(bin_expr) = BinExpr::cast(node) else {
            return;
        };
        let Some(op) = bin_expr.op() else { return };
        if !matches!(op, K::EqEq | K::Neq | K::AndAnd | K::OrOr) {
            return;
        }
        let (Some(lhs), Some(rhs)) = (bin_expr.lhs(), bin_expr.rhs()) else {
            return;
        };
        // Same kind AND identical text: Ident vs Ident, Int vs Int. A
        // float is a different kind, so 1 == 1.0 never matches.
        let same_kind = matches!(
            (lhs, rhs),
            (Expr::Ident(_), Expr::Ident(_)) | (Expr::Int(_), Expr::Int(_))
        );
        let same_text = atom_text(lhs, source)
            .zip(atom_text(rhs, source))
            .is_some_and(|(l, r)| l == r);
        if same_kind && same_text {
            // Nix equality is pure, total, and reflexive, so a same-
            // operand comparison folds to a constant exactly. (`&&`/`||`
            // are NOT folded: they force the operand to a boolean, so
            // for a non-boolean operand an inline adds a forced-eval
            // error that "replace with the operand" would silently
            // remove — too risky for an automated fix.)
            let fix = match op {
                K::EqEq => Some(
                    Fix::new("replace with true").edit(node.content_range(), "true"),
                ),
                K::Neq => Some(
                    Fix::new("replace with false").edit(node.content_range(), "false"),
                ),
                _ => None, // && / || carry no fix
            };
            let mut diag = Diagnostic::new(
                "tautology",
                Severity::Warning,
                "tautological comparison",
                node.content_range(),
            );
            if let Some(fix) = fix {
                diag = diag.with_fix(fix);
            }
            diags.push(diag);
        }
    }
}

/// The source text of an ident or int atom; `None` for anything else.
///
/// Tautology compares atoms only — the same-kind guard then keeps the
/// two sides' kinds aligned (Ident/Ident, Int/Int).
fn atom_text<'a>(expr: Expr<'a>, source: &'a str) -> Option<&'a str> {
    match expr {
        Expr::Ident(t) | Expr::Int(t) => Some(t.text(source)),
        _ => None,
    }
}
