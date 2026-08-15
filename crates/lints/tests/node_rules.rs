//! Integration tests for the node rules (constant-if, assert-true,
//! tautology).
//!
//! These are snapshot-style tests: every diagnostic is rendered with the
//! contract one-line format and compared against an exact string.

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::node_rules::{AssertTrue, ConstantIf, Tautology};
use strictix_syntax::parse;

/// Parse `source` and run all three node rules with an empty config.
///
/// Diagnostics are sorted by (start, end) so results are in source
/// order regardless of the rule order the dispatcher used, then
/// rendered as the contract one-line format.
fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let rules: Vec<Box<dyn Rule>> = vec![
        Box::new(ConstantIf {}),
        Box::new(AssertTrue {}),
        Box::new(Tautology {}),
    ];
    let mut diags = Vec::new();
    run_rules(
        &rules,
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags.sort_by_key(|d| (d.range.start(), d.range.end()));
    diags.iter().map(render).collect()
}

/// The deterministic one-line rendering shared by CLI and tests.
fn render(diag: &Diagnostic) -> String {
    format!(
        "[{}] {} {}..{} {}",
        diag.code,
        diag.severity_str(),
        diag.range.start(),
        diag.range.end(),
        diag.message
    )
}

/// Run the given rules over `source`, returning the raw diagnostics.
fn run_rules_only(source: &str, rules: Vec<Box<dyn Rule>>) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &rules,
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags
}

// --- constant-if ----------------------------------------------------

#[test]
fn constant_if_true() {
    assert_eq!(
        run("if true then 1 else 2"),
        vec!["[constant-if] warning 0..21 constant condition in if"],
    );
}

#[test]
fn constant_if_false() {
    assert_eq!(
        run("if false then 1 else 2"),
        vec!["[constant-if] warning 0..22 constant condition in if"],
    );
}

#[test]
fn constant_if_no_diag_for_variable_cond() {
    assert!(run("if x then 1 else 2").is_empty());
}

#[test]
fn constant_if_fix_true_replaces_with_then_branch() {
    let source = "if true then 1 else 2";
    let diags = run_rules_only(source, vec![Box::new(ConstantIf {})]);
    let fix = diags[0].fix.as_ref().expect("constant-if carries a fix");
    assert_eq!(fix.label, "replace with then branch");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "1");
}

#[test]
fn constant_if_fix_false_replaces_with_else_branch() {
    let source = "if false then 1 else 2";
    let diags = run_rules_only(source, vec![Box::new(ConstantIf {})]);
    let fix = diags[0].fix.as_ref().expect("constant-if carries a fix");
    assert_eq!(fix.label, "replace with else branch");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "2");
}

// --- assert-true ----------------------------------------------------

#[test]
fn assert_true_diag() {
    assert_eq!(
        run("assert true; 1"),
        vec!["[assert-true] warning 0..14 assert true is always satisfied"],
    );
}

#[test]
fn assert_no_diag_for_variable_cond() {
    assert!(run("assert x; 1").is_empty());
}

#[test]
fn assert_true_fix_removes_prefix() {
    // The fix removes `assert true;` (assert-start through the
    // semicolon), leaving the body preceded by the one space that
    // separated it from the semicolon — " 42". Deterministic and
    // accepted: the fix never destroys formatting around the edit.
    let source = "assert true; 42";
    let diags = run_rules_only(source, vec![Box::new(AssertTrue {})]);
    let fix = diags[0].fix.as_ref().expect("assert-true carries a fix");
    assert_eq!(fix.label, "remove assert true");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, " 42");
}

// --- tautology ------------------------------------------------------

#[test]
fn tautology_eq() {
    assert_eq!(
        run("x == x"),
        vec!["[tautology] warning 0..6 tautological comparison"],
    );
}

#[test]
fn tautology_neq() {
    assert_eq!(
        run("x != x"),
        vec!["[tautology] warning 0..6 tautological comparison"],
    );
}

#[test]
fn tautology_andand() {
    assert_eq!(
        run("x && x"),
        vec!["[tautology] warning 0..6 tautological comparison"],
    );
    // `&&`/`||` carry no fix: folding to the bare operand would change
    // the result type (an inline forces the operand to boolean, the
    // operand may not be boolean). Verify no fix is attached.
    let diags = run_rules_only("x && x", vec![Box::new(Tautology {})]);
    assert!(diags[0].fix.is_none(), "&& tautology has no fix");
}

#[test]
fn tautology_oror() {
    assert_eq!(
        run("x || x"),
        vec!["[tautology] warning 0..6 tautological comparison"],
    );
}

#[test]
fn tautology_no_diag_distinct_names() {
    assert!(run("a == b").is_empty());
}

#[test]
fn tautology_no_diag_int_vs_float() {
    // 1 == 1.0 compares different kinds (Int vs Float); not flagged.
    assert!(run("1 == 1.0").is_empty());
}

#[test]
fn tautology_int_vs_int_is_flagged() {
    assert_eq!(
        run("1 == 1"),
        vec!["[tautology] warning 0..6 tautological comparison"],
    );
}

#[test]
fn tautology_flagged_inside_let_body() {
    // Tautology compares source text, not semantics: the let-bound x
    // and the x in the body are different bindings, but the comparison
    // is still textually x == x, so it is flagged — intended.
    assert_eq!(
        run("let x = 1; in x == x"),
        vec!["[tautology] warning 14..20 tautological comparison"],
    );
}

// --- tautology fixes ------------------------------------------------

#[test]
fn tautology_eq_fix_folds_to_true() {
    let source = "x == x";
    let diags = run_rules_only(source, vec![Box::new(Tautology {})]);
    let fix = diags[0].fix.as_ref().expect("== tautology carries a fix");
    assert_eq!(fix.label, "replace with true");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "true");
}

#[test]
fn tautology_neq_fix_folds_to_false() {
    let source = "x != x";
    let diags = run_rules_only(source, vec![Box::new(Tautology {})]);
    let fix = diags[0].fix.as_ref().expect("!= tautology carries a fix");
    assert_eq!(fix.label, "replace with false");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "false");
}

#[test]
fn tautology_int_eq_fix_folds_to_true() {
    let source = "1 == 1";
    let diags = run_rules_only(source, vec![Box::new(Tautology {})]);
    let fix = diags[0].fix.as_ref().expect("int == carries a fix");
    assert_eq!(fix.label, "replace with true");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "true");
}

// --- multi-rule -----------------------------------------------------

#[test]
fn multi_rule_two_diagnostics_in_source_order() {
    // AssertExpr (0..34) wraps the whole file; the IfExpr (12..34) is
    // its body. Both fire, sorted into source order by the helper.
    assert_eq!(
        run("assert true; if true then a else b"),
        vec![
            "[assert-true] warning 0..34 assert true is always satisfied",
            "[constant-if] warning 13..34 constant condition in if",
        ],
    );
}
