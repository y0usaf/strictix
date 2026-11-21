//! A Nix adaptation of cognitive complexity, measured per lambda.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    ApplyExpr, AstNode, BinExpr, Expr, IfExpr, LambdaExpr, NodeOrToken, SyntaxKind, SyntaxNode,
};

use crate::lib_helpers::{library_member, unparen};

pub const MAX_COGNITIVE_COMPLEXITY: usize = 5;

fn helper_call<'a>(
    model: &SemanticModel<'_>,
    node: &'a SyntaxNode,
) -> Option<(ApplyExpr<'a>, ApplyExpr<'a>)> {
    let call = ApplyExpr::cast(node)?;
    let Expr::Apply(condition) = unparen(call.func()?)? else {
        return None;
    };
    if call.arg()?.range() == call.func()?.range()
        || condition.arg()?.range() == condition.func()?.range()
    {
        return None;
    }
    if !matches!(
        library_member(model, condition.func()?)?,
        "mkIf" | "optional" | "optionals" | "optionalAttrs" | "optionalString"
    ) {
        return None;
    }
    Some((call, condition))
}

fn argument_score(model: &SemanticModel<'_>, call: ApplyExpr<'_>, nesting: usize) -> usize {
    let range = call.arg().map(|arg| arg.range());
    call.syntax()
        .child_nodes()
        .find(|node| Some(node.range()) == range)
        .map_or(0, |node| walk(model, node, nesting))
}

/// Score one lambda independently of nested lambdas (including curried arguments).
///
/// Start at zero. An `if` costs 1 + control-flow nesting; each `else` costs
/// one. An `else if` costs one without another nesting penalty. Each run of
/// `&&`, `||`, or `->` costs one; changing operators starts another run.
/// Recognized `lib.mkIf` and `lib.optional*` calls cost 1 + nesting, and
/// increase nesting within both arguments. Library aliases use the semantic model.
/// Parentheses do not break runs. Other expressions only contribute the
/// complexity of their children: declarations, assertions, attribute tests,
/// and `or` defaults have no intrinsic cost. Recursion is not scored.
#[must_use]
pub fn cognitive_complexity(model: &SemanticModel<'_>, lambda: LambdaExpr<'_>) -> usize {
    lambda
        .syntax()
        .child_nodes()
        .map(|node| walk(model, node, 0))
        .sum()
}

fn logical_op(node: &SyntaxNode) -> Option<SyntaxKind> {
    BinExpr::cast(node).and_then(|expr| expr.op()).filter(|op| {
        matches!(
            op,
            SyntaxKind::AndAnd | SyntaxKind::OrOr | SyntaxKind::Arrow
        )
    })
}

fn logical_sequence(
    model: &SemanticModel<'_>,
    node: &SyntaxNode,
    nesting: usize,
    previous: &mut Option<SyntaxKind>,
) -> usize {
    if node.kind() == SyntaxKind::ParenExpr {
        return node
            .child_nodes()
            .map(|child| logical_sequence(model, child, nesting, previous))
            .sum();
    }
    if let Some(op) = logical_op(node) {
        let mut score = 0;
        for child in node.children() {
            match child {
                NodeOrToken::Node(child) => {
                    score += logical_sequence(model, child, nesting, previous)
                }
                NodeOrToken::Token(token) if token.kind() == op => {
                    score += usize::from(*previous != Some(op));
                    *previous = Some(op);
                }
                _ => {}
            }
        }
        score
    } else {
        walk(model, node, nesting)
    }
}

fn conditional(
    model: &SemanticModel<'_>,
    expr: IfExpr<'_>,
    nesting: usize,
    else_if: bool,
) -> usize {
    let mut score = 1 + if else_if { 0 } else { nesting };
    let else_range = expr.else_branch().map(|branch| branch.range());
    let chained = expr
        .syntax()
        .child_nodes()
        .find(|child| Some(child.range()) == else_range && child.kind() == SyntaxKind::IfExpr);
    if else_range.is_some() && chained.is_none() {
        score += 1;
    }
    for child in expr.syntax().child_nodes() {
        score += if Some(child.range()) == chained.map(SyntaxNode::range) {
            conditional(model, IfExpr::cast(child).unwrap(), nesting, true)
        } else {
            walk(model, child, nesting + 1)
        };
    }
    score
}

fn walk(model: &SemanticModel<'_>, node: &SyntaxNode, nesting: usize) -> usize {
    if node.kind() == SyntaxKind::LambdaExpr {
        return 0;
    }
    if let Some(expr) = IfExpr::cast(node) {
        return conditional(model, expr, nesting, false);
    }
    if let Some((call, condition)) = helper_call(model, node) {
        return 1
            + nesting
            + argument_score(model, condition, nesting + 1)
            + argument_score(model, call, nesting + 1);
    }
    if logical_op(node).is_some() {
        return logical_sequence(model, node, nesting, &mut None);
    }
    node.child_nodes()
        .map(|child| walk(model, child, nesting))
        .sum()
}

pub struct CognitiveComplexity;

impl Rule for CognitiveComplexity {
    fn code(&self) -> &'static str {
        "cognitive-complexity"
    }
    fn name(&self) -> &'static str {
        "Cognitive complexity"
    }
    fn description(&self) -> &'static str {
        "Flags lambdas with cognitive complexity above 5. Starts at zero; each if adds 1 plus control-flow nesting, each else adds 1, and else-if adds 1 without extra nesting. Recognized lib.mkIf, lib.optional, lib.optionals, lib.optionalAttrs, and lib.optionalString calls add 1 plus nesting and increase nesting in their arguments. Direct aliases, inherit (lib), and with lib are recognized; locally shadowed lookalikes are ignored. Each sequence of &&, ||, or -> adds 1, with another point when the operator changes. Parentheses do not break sequences. Nested lambdas (including curried arguments) are measured separately. Attribute sets, let, with, assert, attribute tests, or defaults, and recursion add no intrinsic cost. No automatic fix."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for lambda in model.root().descendants().filter_map(LambdaExpr::cast) {
            let score = cognitive_complexity(model, lambda);
            if score > MAX_COGNITIVE_COMPLEXITY {
                diags.push(Diagnostic::new(
                    self.code(), self.severity(),
                    format!("lambda has cognitive complexity {score}; maximum is {MAX_COGNITIVE_COMPLEXITY}"),
                    lambda.syntax().content_range(),
                ));
            }
        }
    }
}
