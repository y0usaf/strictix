use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
};
use strictix_syntax::SyntaxKind as K;
use strictix_syntax::{
    AssertExpr, AstNode, BinExpr, Expr, IfExpr, SyntaxKind, SyntaxNode, TextRange,
};

pub struct ConstantIf;

pub struct ConstantIfBranches;

impl Rule for ConstantIfBranches {
    fn code(&self) -> &'static str {
        "constant-if-branches"
    }

    fn name(&self) -> &'static str {
        "Constant if branches"
    }

    fn description(&self) -> &'static str {
        "Flags if-expressions whose branches are the same literal constant. The branches produce one value, but the condition still runs, so this finding has no automatic fix."
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
        let (Some(then_branch), Some(else_branch)) = (if_expr.then_branch(), if_expr.else_branch())
        else {
            return;
        };
        if matches!(
            (then_branch, else_branch),
            (Expr::Ident(_), Expr::Ident(_)) | (Expr::Int(_), Expr::Int(_))
        ) {
            return;
        }
        if !same_literal(then_branch, else_branch, source) {
            return;
        }
        diags.push(Diagnostic::new(
            self.code(),
            self.severity(),
            "both if branches are the same constant",
            node.content_range(),
        ));
    }
}

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
        if let Some(semi) = node.child_tokens().find(|t| t.kind() == K::Semicolon) {
            let range = TextRange::new(node.range().start(), semi.range().end());
            let replacement = crate::safe_delete_replacement(source, range);
            let fix = Fix::new("remove assert true").edit(range, replacement);
            diag = diag.with_fix(fix);
        }
        diags.push(diag);
    }
}

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
        let same_kind = matches!(
            (lhs, rhs),
            (Expr::Ident(_), Expr::Ident(_)) | (Expr::Int(_), Expr::Int(_))
        );
        let same_text = atom_text(lhs, source)
            .zip(atom_text(rhs, source))
            .is_some_and(|(l, r)| l == r);
        if same_kind && same_text {
            let fix = match op {
                K::EqEq => Some(Fix::new("replace with true").edit(node.content_range(), "true")),
                K::Neq => Some(Fix::new("replace with false").edit(node.content_range(), "false")),
                _ => None,
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

fn atom_text<'a>(expr: Expr<'a>, source: &'a str) -> Option<&'a str> {
    match expr {
        Expr::Ident(t) | Expr::Int(t) => Some(t.text(source)),
        _ => None,
    }
}

fn same_literal(left: Expr<'_>, right: Expr<'_>, source: &str) -> bool {
    let same_kind = matches!(
        (left, right),
        (Expr::Float(_), Expr::Float(_))
            | (Expr::Path(_), Expr::Path(_))
            | (Expr::SearchPath(_), Expr::SearchPath(_))
            | (Expr::Uri(_), Expr::Uri(_))
            | (Expr::String(_), Expr::String(_))
            | (Expr::IndString(_), Expr::IndString(_))
    );
    same_kind && left.text(source) == right.text(source)
}
