use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::suggested_rules::{
    AssertFalse, DuplicateLiteralListItem, LiteralDivisionByZero, RedundantBooleanComparison,
    UnnecessaryRec, UnusedRecBinding,
};
use strictix_syntax::parse;

fn run(source: &str, rule: Box<dyn Rule>) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diagnostics = Vec::new();
    run_rules(
        &[rule],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diagnostics,
    );
    diagnostics
}

#[test]
fn unnecessary_rec_flags_independent_attributes_and_fixes_keyword() {
    let source = "rec { a = 1; b = 2; }";
    let diagnostics = run(source, Box::new(UnnecessaryRec {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "unnecessary-rec");
    let fix = diagnostics[0].fix.as_ref().expect("safe fix");
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok("{ a = 1; b = 2; }".to_owned())
    );
}

#[test]
fn unnecessary_rec_allows_intra_set_reference() {
    assert!(run("rec { a = 1; b = a; }", Box::new(UnnecessaryRec {})).is_empty());
}

#[test]
fn unused_rec_binding_flags_only_unreferenced_member_of_mixed_set() {
    let diagnostics = run(
        "rec { a = 1; b = a; c = 3; }",
        Box::new(UnusedRecBinding {}),
    );
    let messages: Vec<_> = diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        ["attribute 'c' does not participate in this recursive set"]
    );
    assert!(diagnostics.iter().all(|d| d.fix.is_none()));
}

#[test]
fn unused_rec_binding_skips_wholly_non_recursive_set() {
    assert!(run("rec { a = 1; b = 2; }", Box::new(UnusedRecBinding {})).is_empty());
}

#[test]
fn assert_false_is_an_error_without_fix() {
    let diagnostics = run("assert false; 1", Box::new(AssertFalse {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "assert-false");
    assert_eq!(diagnostics[0].severity_str(), "error");
    assert!(diagnostics[0].fix.is_none());
    assert!(run("assert condition; 1", Box::new(AssertFalse {})).is_empty());
}

#[test]
fn literal_division_by_zero_handles_int_float_and_parens() {
    for source in ["x / 0", "x / 0.0", "x / (0)"] {
        let diagnostics = run(source, Box::new(LiteralDivisionByZero {}));
        assert_eq!(diagnostics.len(), 1, "{source}");
        assert_eq!(diagnostics[0].severity_str(), "error");
    }
    assert!(run("x / divisor", Box::new(LiteralDivisionByZero {})).is_empty());
    assert!(run("x / 2", Box::new(LiteralDivisionByZero {})).is_empty());
}

#[test]
fn redundant_boolean_comparison_requires_known_boolean_expression() {
    let source = "(x == y) == true";
    let diagnostics = run(source, Box::new(RedundantBooleanComparison {}));
    assert_eq!(diagnostics.len(), 1);
    let fix = diagnostics[0].fix.as_ref().expect("safe simplification");
    assert_eq!(apply_fixes(source, &fix.edits), Ok("(x == y)".to_owned()));

    let source = "false != (x < y)";
    let diagnostics = run(source, Box::new(RedundantBooleanComparison {}));
    let fix = diagnostics[0].fix.as_ref().expect("safe simplification");
    assert_eq!(apply_fixes(source, &fix.edits), Ok(" (x < y)".to_owned()));

    assert!(run("x == true", Box::new(RedundantBooleanComparison {})).is_empty());
    assert!(run("true == false", Box::new(RedundantBooleanComparison {})).is_empty());
}

#[test]
fn redundant_boolean_comparison_negates_opposite_case() {
    let source = "(x == y) == false";
    let diagnostics = run(source, Box::new(RedundantBooleanComparison {}));
    let fix = diagnostics[0].fix.as_ref().expect("safe simplification");
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok("!((x == y))".to_owned())
    );
}

#[test]
fn duplicate_literal_list_item_flags_repeated_primitives() {
    let diagnostics = run(
        r#"[ "x" "x" true true null null ]"#,
        Box::new(DuplicateLiteralListItem {}),
    );
    assert_eq!(diagnostics.len(), 3);
    assert!(diagnostics.iter().all(|d| d.fix.is_none()));
}

#[test]
fn duplicate_literal_list_item_exempts_numeric_vectors() {
    // `position = [ 0 0 ]` and friends: numeric lists are coordinates
    // and dimensions, where repetition is the point.
    assert!(run(r#"[ 0 0 1.5 1.5 ]"#, Box::new(DuplicateLiteralListItem {}),).is_empty());
}

#[test]
fn duplicate_literal_list_item_skips_dynamic_and_nested_values() {
    assert!(run(
        r#"[ x x "${x}" "${x}" [ 1 ] [ 1 ] ]"#,
        Box::new(DuplicateLiteralListItem {}),
    )
    .is_empty());
}
