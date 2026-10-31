//! Integration tests for the duplicate-attribute rule.
//!
//! Diagnostics are rendered with the one-line contract format
//! (`[code] severity start..end message`) and asserted exactly,
//! including byte offsets recomputed from the source strings.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::duplicate_attr::DuplicateAttribute;
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

/// Run `rules` over `source` with a fresh semantic model per source.
fn run(source: &str, rules: &[Box<dyn Rule>], config: LintConfig) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(rules, &tree, &model, &config, source, &mut diags);
    render(&diags)
}

fn one(rule: Box<dyn Rule>) -> Vec<Box<dyn Rule>> {
    vec![rule]
}

fn dup_rules() -> Vec<Box<dyn Rule>> {
    one(Box::new(DuplicateAttribute {}))
}

#[test]
fn duplicate_attribute_identical_path() {
    assert_eq!(
        run("{ a = 1; a = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 9..10 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_identical_dotted_path() {
    assert_eq!(
        run("{ a.b = 1; a.b = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 11..12 attribute 'a.b' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_let_bindings() {
    assert_eq!(
        run(
            "let a = 1; a = 2; in a",
            &dup_rules(),
            LintConfig::default()
        ),
        ["[duplicate-attribute] error 11..12 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_rec_attrset() {
    assert_eq!(
        run("rec { a = 1; a = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 13..14 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_detect_nested_container() {
    assert_eq!(
        run(
            "{ a = { x = 1; x = 2; }; }",
            &dup_rules(),
            LintConfig::default()
        ),
        ["[duplicate-attribute] error 15..16 attribute 'x' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_inherit_collides_with_binding() {
    assert_eq!(
        run("{ a = 1; inherit a; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 17..18 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_inherit_from_collides() {
    assert_eq!(
        run(
            "{ a = 1; inherit (b) a; }",
            &dup_rules(),
            LintConfig::default()
        ),
        ["[duplicate-attribute] error 21..22 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_quoted_ident_same_as_ident() {
    assert_eq!(
        run("{ a = 1; \"a\" = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 9..12 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_quoted_dotted_string_is_single_attr() {
    assert_eq!(
        run(
            "{ \"a.b\" = 1; a.b = 2; }",
            &dup_rules(),
            LintConfig::default()
        ),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_dynamic_key_untracked() {
    assert_eq!(
        run("{ ${k} = 1; k = 2; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_clean_across_scope() {
    assert_eq!(
        run(
            "let a = 1; in { a = 2; }",
            &dup_rules(),
            LintConfig::default()
        ),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_sibling_paths_are_legal() {
    assert_eq!(
        run("{ a.b = 1; a.c = 2; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_prefix_scalar_leaf() {
    assert_eq!(
        run("{ a = 1; a.b = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 9..10 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_prefix_scalar_leaf_first() {
    assert_eq!(
        run("{ a.c = 2; a = 3; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 11..12 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_prefix_attrset_leaf_merges() {
    assert_eq!(
        run("{ a = { }; a.c = 1; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_prefix_attrset_leaf_reversed_merges() {
    assert_eq!(
        run("{ a.c = 1; a = { }; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_no_fix_offered() {
    let source = "{ a = 1; a = 2; }";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &dup_rules(),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0].fix.is_none());
}
