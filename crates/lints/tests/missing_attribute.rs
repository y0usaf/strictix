//! Tests for the conservative, literal-only missing-attribute rule.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::missing_attribute::MissingAttribute;
use strictix_syntax::parse;

fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut ds = Vec::<Diagnostic>::new();
    run_rules(
        &[Box::new(MissingAttribute) as Box<dyn Rule>],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut ds,
    );
    ds.into_iter()
        .map(|d| format!("[{}] {}", d.code, d.message))
        .collect()
}

#[test]
fn reports_missing_key_on_local_literal() {
    assert_eq!(
        run("let cfg = { enable = true; }; in cfg.enabel"),
        ["[missing-attribute] attribute 'enabel' is missing"]
    );
}

#[test]
fn accepts_present_and_nested_keys() {
    assert!(run("let cfg = { nested = { enable = true; }; }; in cfg.nested.enable").is_empty());
    assert!(run("let cfg = { nested.enable = true; }; in cfg.nested.enable").is_empty());
}

#[test]
fn or_default_and_unknown_values_are_ignored() {
    assert!(run("let cfg = { enable = true; }; in cfg.enabel or false").is_empty());
    assert!(run("let cfg = import ./cfg.nix; in cfg.enabel").is_empty());
    assert!(run("let cfg = { ${name} = true; }; in cfg.enabel").is_empty());
}

#[test]
fn nested_shadowing_does_not_use_outer_literal() {
    assert!(run(
        "let cfg = { enable = true; }; in (x: let cfg = { enabel = true; }; in cfg.enabel) 1"
    )
    .is_empty());
}

#[test]
fn dotted_paths_compare_every_component() {
    assert_eq!(run("{ a.b = 1; }.a.c").len(), 1);
    assert_eq!(run("{ a.b.c = 1; }.a.d").len(), 1);
    assert!(run("{ a.b.c = 1; }.a.b").is_empty());
}

#[test]
fn skips_forward_shadow_and_dotted_binding_values() {
    assert!(run("let cfg = {}; in let x = cfg.good; cfg = {good=1;}; in x").is_empty());
    assert!(run("let cfg.nested = {}; in cfg.nested").is_empty());
    assert!(run(r#"{ "a\n" = 1; }."a
""#)
    .is_empty());
}
