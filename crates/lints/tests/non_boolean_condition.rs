//! Integration tests for the `non-boolean-condition` node rule.
//!
//! Diagnostics are rendered in the contract one-line format
//! (`[code] severity start..end message`) and asserted exactly.

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::non_boolean_condition::NonBooleanCondition;
use strictix_syntax::parse;

/// Render diagnostics in the one-line contract format.
fn render(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .map(|d| {
            format!(
                "[{}] {} {}..{} {}",
                d.code,
                d.severity_str(),
                d.range.start(),
                d.range.end(),
                d.message
            )
        })
        .collect()
}

/// Run the rule over `source` with a fresh semantic model per source.
fn run(source: &str, rule: Box<dyn Rule>) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &one(rule),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    render(&diags)
}

fn one(rule: Box<dyn Rule>) -> Vec<Box<dyn Rule>> {
    vec![rule]
}

fn rule() -> Box<dyn Rule> {
    Box::new(NonBooleanCondition {})
}

// --- trigger cases --------------------------------------------------

#[test]
fn flags_int_condition() {
    assert_eq!(
        run("if 1 then a else b", rule()),
        ["[non-boolean-condition] error 3..4 if-condition has non-boolean type: `1`"]
    );
}

#[test]
fn flags_string_condition() {
    assert_eq!(
        run("if \"x\" then a else b", rule()),
        ["[non-boolean-condition] error 3..6 if-condition has non-boolean type: `\"x\"`"]
    );
}

#[test]
fn flags_empty_list_condition() {
    assert_eq!(
        run("if [ ] then a else b", rule()),
        ["[non-boolean-condition] error 3..6 if-condition has non-boolean type: `[ ]`"]
    );
}

#[test]
fn flags_path_condition() {
    // ./foo lexes as a Path token, distinct from a division.
    assert_eq!(
        run("if ./foo then a else b", rule()),
        ["[non-boolean-condition] error 3..8 if-condition has non-boolean type: `./foo`"]
    );
}

#[test]
fn flags_attrset_condition() {
    assert_eq!(
        run("if { a = 1; } then x else y", rule()),
        ["[non-boolean-condition] error 3..13 if-condition has non-boolean type: `{ a = 1; }`"]
    );
}

#[test]
fn flags_paren_unwrapped_int_condition() {
    // The paren is unwrapped, so the condition is the inner `0`.
    assert_eq!(
        run("if (0) then a else b", rule()),
        ["[non-boolean-condition] error 4..5 if-condition has non-boolean type: `0`"]
    );
}

// --- clean near-misses ---------------------------------------------

#[test]
fn clean_ident_condition() {
    // An ident may be a boolean variable; not provable, so no diag.
    assert_eq!(run("if x then a else b", rule()), Vec::<String>::new());
}

#[test]
fn clean_comparison_condition() {
    // A comparison evaluates to a boolean; not a literal.
    assert_eq!(run("if x == 1 then a else b", rule()), Vec::<String>::new());
}

#[test]
fn clean_true_condition() {
    // true is an Ident and owned by constant-if; never fire here.
    assert_eq!(run("if true then a else b", rule()), Vec::<String>::new());
}

#[test]
fn clean_paren_ident_condition() {
    // (x) unwraps to an ident variable; not provable.
    assert_eq!(run("if (x) then a else b", rule()), Vec::<String>::new());
}