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
    // Require both arguments, and recognize the saturated inner call only once.
    if call.arg()?.range() == call.func()?.range()
        || condition.arg()?.range() == condition.func()?.range()
    {
        return None;
    }
    library_member(model, condition.func()?)?;
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

// Traverse operators in source order, so a && b || c && d has three runs,
// even though precedence makes the two && nodes siblings in the syntax tree.
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
    // A final else costs one even when its body is an atomic token.
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

#[cfg(test)]
mod tests {
    use super::*;
    use strictix_syntax::parse;

    fn scores(source: &str) -> Vec<usize> {
        let tree = parse(source);
        assert!(!tree
            .descendants()
            .any(|node| node.kind() == SyntaxKind::ErrorNode));
        let model = SemanticModel::new(source, &tree);
        tree.descendants()
            .filter_map(LambdaExpr::cast)
            .map(|lambda| cognitive_complexity(&model, lambda))
            .collect()
    }

    #[test]
    fn nesting_costs_more_than_flat_branches() {
        assert_eq!(
            scores("x: if a then if b then if c then 1 else 2 else 3 else 4"),
            [9]
        );
        assert_eq!(
            scores("x: if a then 1 else if b then 2 else if c then 3 else 4"),
            [4]
        );
        assert_eq!(
            scores("x: if a then (if b then 1 else if c then 2 else 3) else 4"),
            [6]
        );
    }

    #[test]
    fn logical_runs_follow_source_order_and_ignore_parentheses() {
        assert_eq!(scores("x: a && (b && c) && d"), [1]);
        assert_eq!(scores("x: a && b || c && d"), [3]);
        assert_eq!(scores("x: a -> b -> c"), [1]);
        assert_eq!(scores("x: a && !(b && c)"), [2]);
        assert_eq!(scores("x: [ (a && b) (c && d) ]"), [2]);
    }

    #[test]
    fn declarative_structure_is_free_but_children_are_visited() {
        assert_eq!(
            scores("x: let a = x ? foo; in with x; assert a; { nested.value = x.foo or 0; }"),
            [0]
        );
        assert_eq!(scores("x: { nested.value = if a then 1 else 2; }"), [2]);
        assert_eq!(scores("{ x ? if a then 1 else 2 }: x"), [2]);
        assert_eq!(scores("x: if (if a then b else c) then 1 else 2"), [5]);
    }

    #[test]
    fn nested_lambdas_are_independent() {
        assert_eq!(scores("x: y: if a then 1 else 2"), [0, 2]);
        assert_eq!(
            scores("x: if a then (y: if b then 1 else 2) else (z: z)"),
            [2, 2, 0]
        );
    }
    #[test]
    fn conditional_helpers_and_nested_bodies() {
        for helper in [
            "mkIf",
            "optional",
            "optionals",
            "optionalAttrs",
            "optionalString",
        ] {
            assert_eq!(scores(&format!("{{ lib }}: lib.{helper} a {{}}")), [1]);
        }
        assert_eq!(scores("{ lib }: lib.mkIf a { x = lib.optionalAttrs b { y = lib.optionalString c \"ok\"; }; }"), [6]);
        assert_eq!(
            scores("{ lib }: lib.mkIf (a && b) (if c then 1 else 2)"),
            [5]
        );
        assert_eq!(
            scores("{ lib }: if a then lib.optionalAttrs b {} else {}"),
            [4]
        );
        assert_eq!(
            scores("{ lib }: lib.mkIf a (x: lib.optionalAttrs b {})"),
            [1, 1]
        );
        assert_eq!(scores("{ lib }: lib.mkIf (lib.optionalAttrs a {}) {}"), [3]);
    }

    #[test]
    fn resolves_inherits_aliases_and_with_lib() {
        assert_eq!(scores("{ lib }: let inherit (lib) mkIf; in mkIf a {}"), [1]);
        assert_eq!(
            scores("{ lib }: let l = lib; when = l.optionalAttrs; alias = when; in alias a {}"),
            [1]
        );
        assert_eq!(
            scores("{ lib }: with lib; mkIf a (optionalAttrs b {})"),
            [3]
        );
        assert_eq!(
            scores("{ lib }: let l = lib; inherit (l) optionalString; in optionalString a \"x\""),
            [1]
        );
        assert_eq!(scores("x: lib.mkIf a {}"), [1]);
    }

    #[test]
    fn partial_and_extra_applications_are_not_double_counted() {
        assert_eq!(scores("{ lib }: lib.mkIf a"), [0]);
        assert_eq!(scores("{ lib }: ((lib.mkIf) a) {}"), [1]);
        assert_eq!(scores("{ lib }: lib.mkIf a {} extra"), [1]);
        assert_eq!(scores("{ lib }: [ (lib.mkIf a {}) (lib.mkIf b {}) ]"), [2]);
    }

    #[test]
    fn ignores_shadowed_and_unknown_helpers() {
        assert_eq!(scores("{ other }: other.mkIf a {}"), [0]);
        assert_eq!(scores("{ mkIf }: mkIf a {}"), [0]);
        assert_eq!(
            scores("{ lib }: let mkIf = a: b: b; in mkIf a {}"),
            [0, 0, 0]
        );
        assert_eq!(
            scores("{ lib }: let lib = { mkIf = a: b: b; }; in lib.mkIf a {}"),
            [0, 0, 0]
        );
        assert_eq!(
            scores("{ other }: let inherit (other) mkIf; in mkIf a {}"),
            [0]
        );
        assert_eq!(scores("{ other }: with other; mkIf a {}"), [0]);
        assert_eq!(
            scores("{ lib, other }: with lib; with other; mkIf a {}"),
            [0]
        );
        assert_eq!(scores("{ lib }: let f = f; in f a {}"), [0]);
        assert_eq!(scores("{ lib }: (lib.mkIf or fallback) a {}"), [0]);
        assert_eq!(scores("{ lib }: lib.mkMerge [ {} {} ]"), [0]);
    }

    #[test]
    fn forward_local_declarations_do_not_look_like_library_helpers() {
        assert_eq!(
            scores("{ lib }: let result = lib.mkIf a {}; lib = {}; in result"),
            [0]
        );
        assert_eq!(
            scores("{ lib }: with lib; let result = mkIf a {}; mkIf = a: b: b; in result"),
            [0, 0, 0]
        );
    }

    #[test]
    fn finix_rule_file_conditions_are_counted() {
        assert_eq!(
            scores(
                r#"{ lib }: name: rule: {
            value.text = let
                frontmatter = lib.optionalAttrs (rule.condition != null) {}
                    // lib.optionalAttrs (rule.minOutputLength != null && rule.condition == null) {}
                    // lib.optionalAttrs (rule.astCondition != null) {}
                    // lib.optionalAttrs (rule.scope != null) {}
                    // lib.optionalAttrs (rule.globs != []) {}
                    // lib.optionalAttrs (rule.interruptMode != null) {};
            in "${lib.optionalString (frontmatter != {}) "header"}${rule.content}";
        }"#
            ),
            [0, 0, 8]
        );
    }

    #[test]
    fn default_boundary_registry_and_suppression() {
        use strictix_core::rules::run_rules;
        let rules: Vec<_> = crate::all_rules()
            .into_iter()
            .filter(|rule| rule.code() == "cognitive-complexity")
            .collect();
        assert_eq!(rules.len(), 1);
        let check = |source: &str, config: LintConfig| {
            let tree = parse(source);
            let model = SemanticModel::new(source, &tree);
            let mut diags = Vec::new();
            run_rules(&rules, &tree, &model, &config, source, &mut diags);
            diags
        };
        let helpers = format!(
            "{{ lib }}: [ {} ]",
            "(lib.optionalAttrs a {}) ".repeat(MAX_COGNITIVE_COMPLEXITY + 1)
        );
        let helper_diags = check(&helpers, LintConfig::default());
        assert_eq!(helper_diags.len(), 1);
        assert_eq!(
            helper_diags[0].message,
            format!(
                "lambda has cognitive complexity {}; maximum is {MAX_COGNITIVE_COMPLEXITY}",
                MAX_COGNITIVE_COMPLEXITY + 1
            )
        );
        let branches = "(if a then b else c) ".repeat(MAX_COGNITIVE_COMPLEXITY / 2);
        let boolean = if MAX_COGNITIVE_COMPLEXITY % 2 == 1 {
            "(a && b)"
        } else {
            ""
        };
        let at_limit = format!("x: [ {branches}{boolean} ]");
        assert_eq!(scores(&at_limit), [MAX_COGNITIVE_COMPLEXITY]);
        assert!(check(&at_limit, LintConfig::default()).is_empty());
        let over_limit = format!("outer: inner: [ {branches}{boolean} (c || d) ]");
        let diags = check(&over_limit, LintConfig::default());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].range.start(), 7);
        assert_eq!(diags[0].range.end() as usize, over_limit.len());
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(
            diags[0].message,
            format!(
                "lambda has cognitive complexity {}; maximum is {MAX_COGNITIVE_COMPLEXITY}",
                MAX_COGNITIVE_COMPLEXITY + 1
            )
        );
        assert!(diags[0].fix.is_none());
        let mut config = LintConfig::default();
        config.disabled.push("cognitive-complexity".into());
        assert!(check(&over_limit, config).is_empty());
        assert!(check(
            &format!("# strictix: disable-next-line=cognitive-complexity\n{over_limit}"),
            LintConfig::default()
        )
        .is_empty());
    }
}
