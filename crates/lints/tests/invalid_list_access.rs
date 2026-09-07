//! Integration tests for `invalid-list-access`.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::invalid_list_access::InvalidListAccess;
use strictix_syntax::parse;

fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::<Diagnostic>::new();
    run_rules(
        &[Box::new(InvalidListAccess) as Box<dyn Rule>],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
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

#[test]
fn flags_empty_head_and_tail() {
    assert_eq!(
        run("builtins.head []"),
        ["[invalid-list-access] error 14..16 builtin 'head' cannot be applied to an empty list"]
    );
    assert_eq!(
        run("builtins.tail []"),
        ["[invalid-list-access] error 14..16 builtin 'tail' cannot be applied to an empty list"]
    );
}

#[test]
fn flags_out_of_bounds_and_negative_indices() {
    assert_eq!(
        run("builtins.elemAt [\"a\"] 1"),
        ["[invalid-list-access] error 22..23 builtin 'elemAt' index 1 is out of bounds for list of length 1"]
    );
    assert_eq!(
        run("builtins.elemAt [\"a\"] (-1)"),
        ["[invalid-list-access] error 22..26 builtin 'elemAt' index -1 is out of bounds for list of length 1"]
    );
}

#[test]
fn accepts_boundaries_and_skips_unknowns_and_partial_calls() {
    for source in [
        "builtins.head [1]",
        "builtins.tail [1]",
        "builtins.elemAt [\"a\"] 0",
        "builtins.elemAt [1 2] 1",
        "builtins.elemAt xs 1",
        "builtins.elemAt [1] i",
        "builtins.elemAt [1]",
        "builtins.head",
    ] {
        assert!(run(source).is_empty(), "unexpected finding for {source}");
    }
}

#[test]
fn respects_shadowing_and_with() {
    for source in [
        "let head = x: []; in head []",
        "let elemAt = x: y: 0; in elemAt [] 1",
        "let builtins = { head = x: 1; }; in builtins.head []",
        "with { head = x: 1; }; head []",
        "with { builtins = { head = x: 1; }; }; builtins.head []",
    ] {
        assert!(run(source).is_empty(), "unexpected finding for {source}");
    }
}

#[test]
fn nested_calls_parentheses_and_forward_shadowing() {
    assert_eq!(run("f (builtins.head [])").len(), 1);
    assert_eq!(run("(builtins.head) []").len(), 1);
    assert_eq!(run("(builtins.elemAt []) 0").len(), 1);
    assert_eq!(run("builtins.head [] 1").len(), 1);
    assert!(run("let x = builtins.head []; builtins = {head=x:x;}; in x").is_empty());
    assert!(run("head []").is_empty());
}
