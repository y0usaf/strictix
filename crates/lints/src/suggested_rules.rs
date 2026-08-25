//! Conservative correctness and generated-code lints.
//!
//! These rules intentionally fire only when syntax or the semantic model can
//! prove the finding. In particular, recursive attrset fields are externally
//! observable, so `unused-rec-binding` means that a field does not participate
//! in the recursive dependency graph, not that the field can be deleted.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::{BindingKind, SemanticModel},
};
use strictix_syntax::{
    AssertExpr, AstNode, BinExpr, Expr, ListExpr, RecAttrsetExpr, StringPart, SyntaxKind as K,
    SyntaxNode, TextRange,
};

/// Flags a recursive attrset that has no intra-set references.
pub struct UnnecessaryRec;

impl Rule for UnnecessaryRec {
    fn code(&self) -> &'static str {
        "unnecessary-rec"
    }
    fn name(&self) -> &'static str {
        "Unnecessary rec"
    }
    fn description(&self) -> &'static str {
        "Flags `rec` attrsets whose attributes never reference attributes from the same set. The `rec` keyword is unnecessary and can be removed."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model
            .root()
            .descendants()
            .filter(|n| n.kind() == K::RecAttrsetExpr)
        {
            let range = node.content_range();
            let uses_rec_scope = model.references().iter().any(|reference| {
                if !range.contains(reference.name.range().start()) {
                    return false;
                }
                model.resolve(reference.name).is_some_and(|binding| {
                    binding.kind == BindingKind::RecAttr
                        && range.contains(binding.name.range().start())
                })
            });
            if uses_rec_scope {
                continue;
            }
            let Some(rec_token) = node.child_tokens().find(|t| t.kind() == K::KwRec) else {
                continue;
            };
            let fix_end = RecAttrsetExpr::cast(node)
                .and_then(|rec| rec.attrset())
                .map_or(rec_token.range().end(), |attrset| {
                    attrset.syntax().content_range().start()
                });
            let fix_range = TextRange::new(rec_token.range().start(), fix_end);
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "recursive attrset has no recursive references",
                    rec_token.range(),
                )
                .with_fix(Fix::new("remove unnecessary rec").edit(fix_range, "")),
            );
        }
    }
}

/// Flags fields in a mixed recursive attrset that do not participate in its
/// intra-set dependency graph.
pub struct UnusedRecBinding;

impl Rule for UnusedRecBinding {
    fn code(&self) -> &'static str {
        "unused-rec-binding"
    }
    fn name(&self) -> &'static str {
        "Unused recursive binding"
    }
    fn description(&self) -> &'static str {
        "Flags attributes in a recursive attrset that do not participate in any intra-set reference, when another attribute does. The attribute remains externally observable and is not safe to delete."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model
            .root()
            .descendants()
            .filter(|n| n.kind() == K::RecAttrsetExpr)
        {
            let range = node.content_range();
            let bindings: Vec<_> = model
                .bindings()
                .iter()
                .filter(|binding| {
                    binding.kind == BindingKind::RecAttr
                        && range.contains(binding.name.range().start())
                })
                .collect();
            let participates = |binding: &&strictix_core::semantic::Binding<'_>| {
                let is_referenced = binding
                    .references
                    .iter()
                    .any(|reference| range.contains(reference.start()));
                let references_peer = model
                    .root()
                    .descendants()
                    .find(|candidate| {
                        candidate.kind() == K::Binding
                            && candidate.range().contains(binding.name.range().start())
                    })
                    .and_then(strictix_syntax::Binding::cast)
                    .and_then(|syntax_binding| syntax_binding.value())
                    .is_some_and(|value| {
                        model.references().iter().any(|reference| {
                            value.range().contains(reference.name.range().start())
                                && model.resolve(reference.name).is_some_and(|target| {
                                    target.kind == BindingKind::RecAttr
                                        && range.contains(target.name.range().start())
                                })
                        })
                    });
                is_referenced || references_peer
            };
            if !bindings.iter().any(participates) {
                continue;
            }
            for binding in bindings.iter().filter(|binding| !participates(binding)) {
                let name = binding.name.text(source);
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("attribute '{name}' does not participate in this recursive set"),
                    binding.name.range(),
                ));
            }
        }
    }
}

/// Flags an assertion whose condition is literally false.
pub struct AssertFalse;

impl Rule for AssertFalse {
    fn code(&self) -> &'static str {
        "assert-false"
    }
    fn name(&self) -> &'static str {
        "Assert always false"
    }
    fn description(&self) -> &'static str {
        "Flags `assert false;` expressions, which always abort when evaluated and can never produce their body."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn node_kind(&self) -> Option<K> {
        Some(K::AssertExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(assertion) = AssertExpr::cast(node) else {
            return;
        };
        let Some(condition) = assertion.cond() else {
            return;
        };
        if !is_bool_literal(condition, source, false) {
            return;
        }
        diags.push(Diagnostic::new(
            self.code(),
            self.severity(),
            "assertion condition is always false",
            condition.range(),
        ));
    }
}

/// Flags division by a literal numeric zero.
pub struct LiteralDivisionByZero;

impl Rule for LiteralDivisionByZero {
    fn code(&self) -> &'static str {
        "literal-division-by-zero"
    }
    fn name(&self) -> &'static str {
        "Literal division by zero"
    }
    fn description(&self) -> &'static str {
        "Flags division by a literal integer or floating-point zero, which always fails when evaluated."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn node_kind(&self) -> Option<K> {
        Some(K::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binary) = BinExpr::cast(node) else {
            return;
        };
        if binary.op() != Some(K::Slash) {
            return;
        }
        let Some(rhs) = binary.rhs() else { return };
        let rhs = unwrap_parens(rhs);
        let is_zero = match rhs {
            Expr::Int(token) | Expr::Float(token) => token
                .text(source)
                .parse::<f64>()
                .is_ok_and(|number| number == 0.0),
            _ => false,
        };
        if is_zero {
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                "division by literal zero",
                rhs.range(),
            ));
        }
    }
}

/// Flags comparisons to a boolean literal when the other operand is
/// syntactically guaranteed to produce a boolean.
pub struct RedundantBooleanComparison;

impl Rule for RedundantBooleanComparison {
    fn code(&self) -> &'static str {
        "redundant-boolean-comparison"
    }
    fn name(&self) -> &'static str {
        "Redundant boolean comparison"
    }
    fn description(&self) -> &'static str {
        "Flags `boolean == true` and equivalent comparisons when the non-literal operand is syntactically guaranteed to be boolean."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn node_kind(&self) -> Option<K> {
        Some(K::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binary) = BinExpr::cast(node) else {
            return;
        };
        let Some(op) = binary.op() else { return };
        if !matches!(op, K::EqEq | K::Neq) {
            return;
        }
        let (Some(lhs), Some(rhs)) = (binary.lhs(), binary.rhs()) else {
            return;
        };
        let pair = bool_literal(lhs, source)
            .map(|value| (rhs, value))
            .or_else(|| bool_literal(rhs, source).map(|value| (lhs, value)));
        let Some((other, literal)) = pair else { return };
        if bool_literal(other, source).is_some() || !is_definitely_boolean(other, source) {
            return;
        }
        let keep = (op == K::EqEq && literal) || (op == K::Neq && !literal);
        let replacement = if keep {
            other.text(source).to_owned()
        } else {
            format!("!({})", other.text(source))
        };
        diags.push(
            Diagnostic::new(
                self.code(),
                self.severity(),
                "comparison to a boolean literal is redundant",
                node.content_range(),
            )
            .with_fix(
                Fix::new("simplify boolean comparison").edit(node.content_range(), replacement),
            ),
        );
    }
}

/// Flags a repeated string/boolean/null literal in one list.
///
/// Numeric literals are deliberately exempt: lists of numbers are
/// almost always coordinate or dimension vectors (`position = [ 0 0 ]`,
/// margins, RGBA channels) where repetition is the point, not a
/// copy-paste slip.
pub struct DuplicateLiteralListItem;

impl Rule for DuplicateLiteralListItem {
    fn code(&self) -> &'static str {
        "duplicate-literal-list-item"
    }
    fn name(&self) -> &'static str {
        "Duplicate literal list item"
    }
    fn description(&self) -> &'static str {
        "Flags a repeated string, boolean, or null literal in one list — usually a copy-paste slip in an imports or package list. Numeric literals are exempt (coordinate and dimension vectors repeat them legitimately), and duplicates may still be intentional, so the rule is warning-only and offers no fix."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn node_kind(&self) -> Option<K> {
        Some(K::ListExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(list) = ListExpr::cast(node) else {
            return;
        };
        let mut seen: Vec<(LiteralKey, TextRange)> = Vec::new();
        for item in list.items() {
            let Some(key) = literal_key(item, source) else {
                continue;
            };
            if seen.iter().any(|(prior, _)| prior == &key) {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "literal `{}` appears more than once in this list",
                        item.text(source)
                    ),
                    item.range(),
                ));
            } else {
                seen.push((key, item.range()));
            }
        }
    }
}

fn unwrap_parens(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        let Some(inner) = paren.expr() else { break };
        expr = inner;
    }
    expr
}

fn bool_literal(expr: Expr<'_>, source: &str) -> Option<bool> {
    match unwrap_parens(expr) {
        Expr::Ident(token) if token.text(source) == "true" => Some(true),
        Expr::Ident(token) if token.text(source) == "false" => Some(false),
        _ => None,
    }
}

fn is_bool_literal(expr: Expr<'_>, source: &str, expected: bool) -> bool {
    bool_literal(expr, source) == Some(expected)
}

fn is_definitely_boolean(expr: Expr<'_>, source: &str) -> bool {
    match unwrap_parens(expr) {
        Expr::Unary(_) | Expr::HasAttr(_) => true,
        Expr::Bin(binary) => matches!(
            binary.op(),
            Some(
                K::EqEq
                    | K::Neq
                    | K::Lt
                    | K::LtEq
                    | K::Gt
                    | K::GtEq
                    | K::AndAnd
                    | K::OrOr
                    | K::Arrow
            )
        ),
        other => bool_literal(other, source).is_some(),
    }
}

#[derive(PartialEq, Eq)]
enum LiteralKey {
    Bool(bool),
    Null,
    String(String),
}

/// The comparable identity of a duplicable list literal. Numbers return
/// `None` on purpose — see [DuplicateLiteralListItem].
fn literal_key(expr: Expr<'_>, source: &str) -> Option<LiteralKey> {
    match unwrap_parens(expr) {
        Expr::Ident(token) if token.text(source) == "true" => Some(LiteralKey::Bool(true)),
        Expr::Ident(token) if token.text(source) == "false" => Some(LiteralKey::Bool(false)),
        Expr::Ident(token) if token.text(source) == "null" => Some(LiteralKey::Null),
        Expr::String(string)
            if string
                .parts()
                .all(|part| matches!(part, StringPart::Content(_))) =>
        {
            Some(LiteralKey::String(string.syntax().text(source).to_owned()))
        }
        _ => None,
    }
}
