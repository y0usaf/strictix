//! Integration tests for the duplicate-formal node rule.
//!
//! The rule flags a lambda formal set that declares the same name twice.
//! Only the declared *name tokens* are compared — a default value like
//! `{ a, b ? a }` is not a duplicate. Nix rejects duplicate formals at
//! parse time, so the finding is an Error with no auto-fix.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::duplicate_formal::DuplicateFormal;
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
fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(DuplicateFormal {})],
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

// --- positive cases --------------------------------------------------

#[test]
fn duplicate_formal_flags_repeated_name() {
    // "{ a, a }: a" — the second `a` spans byte 5..6.
    assert_eq!(
        run("{ a, a }: a"),
        ["[duplicate-formal] error 5..6 duplicate formal argument 'a'"]
    );
}

#[test]
fn duplicate_formal_flags_defaulted_duplicate() {
    // "{ pkgs, pkgs ? null }: pkgs" — the second `pkgs` spans 8..12.
    assert_eq!(
        run("{ pkgs, pkgs ? null }: pkgs"),
        ["[duplicate-formal] error 8..12 duplicate formal argument 'pkgs'"]
    );
}

#[test]
fn duplicate_formal_flags_each_repeat_after_first() {
    // Three `a`s: the second and third occurrences are each flagged on
    // their own name token.
    assert_eq!(
        run("{ a, a, a }: a"),
        [
            "[duplicate-formal] error 5..6 duplicate formal argument 'a'",
            "[duplicate-formal] error 8..9 duplicate formal argument 'a'",
        ]
    );
}

#[test]
fn duplicate_formal_carries_no_fix() {
    let source = "{ a, a }: a";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &one(Box::new(DuplicateFormal {})),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    assert_eq!(diags.len(), 1);
    // Which occurrence to delete is ambiguous, so there is no auto-fix.
    assert!(diags[0].fix.is_none());
}

// --- clean cases ------------------------------------------------------

#[test]
fn duplicate_formal_clean_distinct_names() {
    assert_eq!(run("{ a, b }: a"), Vec::<String>::new());
}

#[test]
fn duplicate_formal_clean_default_is_not_a_name() {
    // `a` appears only as the *default value* of `b`; the declared names
    // are a and b, so there is no duplicate.
    assert_eq!(run("{ a, b ? a }: a"), Vec::<String>::new());
}

#[test]
fn duplicate_formal_clean_ellipsis_adds_no_name() {
    // The ellipsis is an open interface, not a declared name.
    assert_eq!(run("{ a, ... }: a"), Vec::<String>::new());
}

#[test]
fn duplicate_formal_clean_single_param() {
    assert_eq!(run("{ a }: a"), Vec::<String>::new());
}