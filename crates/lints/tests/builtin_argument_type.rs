use strictix_core::{
    config::LintConfig,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::builtin_argument_type::BuiltinArgumentType;
use strictix_syntax::parse;

fn diagnostics(source: &str) -> Vec<strictix_core::diagnostic::Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut out = Vec::new();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(BuiltinArgumentType)];
    run_rules(
        &rules,
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut out,
    );
    out
}

fn count(source: &str) -> usize {
    diagnostics(source).len()
}

#[test]
fn checks_literal_builtin_contracts() {
    assert_eq!(count("builtins.attrNames []"), 1);
    assert_eq!(count("builtins.attrNames {}"), 0);
    assert_eq!(count("builtins.map 42 []"), 1);
    assert_eq!(count("builtins.map (x: x) 42"), 1);
    assert_eq!(count("builtins.elemAt [] 1"), 0);
    assert_eq!(count("builtins.elemAt 1 []"), 2);
    assert_eq!(count("builtins.substring 0 2 \"abc\""), 0);
    assert_eq!(count("builtins.substring \"0\" 2 3"), 2);
}

#[test]
fn unknown_values_and_partial_calls_are_conservative() {
    assert_eq!(count("builtins.attrNames value"), 0);
    assert_eq!(count("builtins.map (x: x)"), 0);
    assert_eq!(count("builtins.map 42"), 1);
    assert_eq!(count("builtins.map (x: builtins.attrNames x) [1]"), 0);
}

#[test]
fn respects_shadowing_and_callable_functors() {
    assert_eq!(count("let builtins = {}; in builtins.attrNames []"), 0);
    assert_eq!(count("let map = x: x; in map 42 []"), 0);
    assert_eq!(count("builtins.map { __functor = x: x; } []"), 0);
    assert_eq!(count("builtins.map { inherit __functor; } []"), 0);
    assert_eq!(count("builtins.map { \"__functor\" = x: x; } []"), 0);
    assert_eq!(count("builtins.map { ${name} = x: x; } []"), 0);
}

#[test]
fn nested_calls_and_builtin_specific_contracts() {
    assert_eq!(count("f (builtins.attrNames [])"), 1);
    assert_eq!(count("builtins.map (x: builtins.attrNames []) [1]"), 1);
    assert_eq!(count("builtins.functionArgs (x: x)"), 0);
    assert_eq!(count("builtins.listToAttrs []"), 0);
    assert_eq!(count("builtins.listToAttrs {}"), 1);
    assert_eq!(count("attrNames []"), 0);
    assert_eq!(
        count("builtins.map { __functor.__functor = self: x: x; } []"),
        0
    );
}

#[test]
fn forward_builtin_shadow_is_unknown() {
    assert_eq!(
        count("let x = builtins.attrNames []; builtins = {attrNames = x:x;}; in x"),
        0
    );
}
