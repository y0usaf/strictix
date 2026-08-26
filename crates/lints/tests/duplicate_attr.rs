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

// --- identical static path within one container ----------------------

#[test]
fn duplicate_attribute_identical_path() {
    // `{ a = 1; a = 2; }` — second `a` at byte 9..10.
    assert_eq!(
        run("{ a = 1; a = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 9..10 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_identical_dotted_path() {
    // `{ a.b = 1; a.b = 2; }` — offending second binding's first element
    // `a` at byte 11..12.
    assert_eq!(
        run("{ a.b = 1; a.b = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 11..12 attribute 'a.b' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_let_bindings() {
    // `let a = 1; a = 2; in a` — second `a` at byte 11..12.
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
    // `rec { a = 1; a = 2; }` — second `a` at byte 13..14.
    assert_eq!(
        run("rec { a = 1; a = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 13..14 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_detect_nested_container() {
    // The inner attrset has its own duplicate; the outer `a` is a single
    // binding and must not produce one.
    assert_eq!(
        run(
            "{ a = { x = 1; x = 2; }; }",
            &dup_rules(),
            LintConfig::default()
        ),
        ["[duplicate-attribute] error 15..16 attribute 'x' defined more than once"]
    );
}

// --- inherit collisions ----------------------------------------------

#[test]
fn duplicate_attribute_inherit_collides_with_binding() {
    // `{ a = 1; inherit a; }` — confirmed with `nix eval --expr '{ a =
    // 1; inherit a; }'` which errors "attribute 'a' already defined".
    // The offending inherit name `a` sits at byte 17..18.
    assert_eq!(
        run("{ a = 1; inherit a; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 17..18 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_inherit_from_collides() {
    // `inherit (b) a;` re-binds `a`; confirmed error on the inherit name.
    assert_eq!(
        run(
            "{ a = 1; inherit (b) a; }",
            &dup_rules(),
            LintConfig::default()
        ),
        ["[duplicate-attribute] error 21..22 attribute 'a' defined more than once"]
    );
}

// --- quoted string normalizes to plain ident -------------------------

#[test]
fn duplicate_attribute_quoted_ident_same_as_ident() {
    // `{ a = 1; "a" = 2; }` — Nix errors "attribute 'a' already
    // defined": a quoted `"a"` is the same attribute as the ident `a`.
    // The offending string node spans bytes 9..12.
    assert_eq!(
        run("{ a = 1; \"a\" = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 9..12 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_quoted_dotted_string_is_single_attr() {
    // `{ "a.b" = 1; a.b = 2; }` is LEGAL in Nix (gives `{ a = { b = 2;
    // }; "a.b" = 1; }`): a quoted dotted string is one attribute named
    // `a.b`, not a two-element path, so it cannot collide with `a.b`.
    assert_eq!(
        run(
            "{ \"a.b\" = 1; a.b = 2; }",
            &dup_rules(),
            LintConfig::default()
        ),
        Vec::<String>::new()
    );
}

// --- dynamic keys are excluded --------------------------------------

#[test]
fn duplicate_attribute_dynamic_key_untracked() {
    // `${k}` may be anything, so it cannot be proven to collide with the
    // static `k` below. Confirmed: Nix never raises an "already defined"
    // error for this (only, here, an unrelated undefined-variable one).
    assert_eq!(
        run("{ ${k} = 1; k = 2; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

// --- scope: siblings only -------------------------------------------

#[test]
fn duplicate_attribute_clean_across_scope() {
    // The outer let `a` and the attrset `a` are different containers.
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
    // Siblings `a.b` and `a.c` share a prefix but diverge: clean.
    assert_eq!(
        run("{ a.b = 1; a.c = 2; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

// --- prefix collision (case (b)) ------------------------------------

#[test]
fn duplicate_attribute_prefix_scalar_leaf() {
    // `{ a = 1; a.b = 2; }` errors in Nix ("attribute 'a' already
    // defined"): a scalar leaf `a` cannot have dotted children. The
    // offending `a.b`'s first element sits at byte 9..10.
    assert_eq!(
        run("{ a = 1; a.b = 2; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 9..10 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_prefix_scalar_leaf_first() {
    // Reversed order `{ a.c = 2; a = 3; }` still collides; Nix reports
    // on the later scalar leaf `a` at byte 11..12.
    assert_eq!(
        run("{ a.c = 2; a = 3; }", &dup_rules(), LintConfig::default()),
        ["[duplicate-attribute] error 11..12 attribute 'a' defined more than once"]
    );
}

#[test]
fn duplicate_attribute_prefix_attrset_leaf_merges() {
    // `{ a = { }; a.c = 1; }` is LEGAL in Nix — an attrset literal value
    // merges with its dotted children (`{ a = { c = 1; }; }`), so zero
    // diagnostics.
    assert_eq!(
        run("{ a = { }; a.c = 1; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_prefix_attrset_leaf_reversed_merges() {
    // Same merge rule with the attrset literal leaf coming last.
    assert_eq!(
        run("{ a.c = 1; a = { }; }", &dup_rules(), LintConfig::default()),
        Vec::<String>::new()
    );
}

#[test]
fn duplicate_attribute_no_fix_offered() {
    // This rule is a pure error with no auto-fix: deleting the wrong
    // duplicate would change intent.
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
