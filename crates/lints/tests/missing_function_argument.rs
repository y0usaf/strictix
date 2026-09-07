use strictix_core::{
    config::LintConfig,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::missing_function_argument::MissingFunctionArgument;
use strictix_syntax::parse;

fn codes(source: &str) -> Vec<&'static str> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diagnostics = Vec::new();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(MissingFunctionArgument)];
    run_rules(
        &rules,
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diagnostics,
    );
    diagnostics.into_iter().map(|d| d.code).collect()
}

#[test]
fn reports_missing_required_formal_on_literal_attrset() {
    assert_eq!(
        codes("({ name, version ? \"1\" }: name) { }"),
        vec!["missing-function-argument"]
    );
    assert_eq!(
        codes("({ name, version ? \"1\" }: name) { name = \"app\"; }"),
        Vec::<&str>::new()
    );
}

#[test]
fn accepts_ellipsis_and_defaults() {
    assert!(codes("({ name, ... }: name) { name = 1; extra = 2; }").is_empty());
    assert!(codes("({ name ? \"default\" }: name) { }").is_empty());
}

#[test]
fn resolves_a_local_lambda_alias() {
    assert_eq!(
        codes("let f = { name }: name; in f { }"),
        vec!["missing-function-argument"]
    );
    assert!(codes("let f = { name }: name; in f { name = \"app\"; }").is_empty());
}

#[test]
fn skips_dynamic_or_unknown_attribute_sets_and_callees() {
    assert!(codes("({ name }: name) { inherit name; }").is_empty());
    assert!(codes("({ name }: name) { \"name\" = \"app\"; }").is_empty());
    assert!(codes("f { }").is_empty());
    assert!(codes("{ name }: name value").is_empty());
}

#[test]
fn ellipsis_still_requires_named_formals() {
    assert_eq!(
        codes("({ name, ... }: name) {}"),
        ["missing-function-argument"]
    );
}

#[test]
fn skips_shadowed_forward_and_dotted_functions() {
    assert!(codes("let f = {name}: name; in let x = f {}; f = x: x; in x").is_empty());
    assert!(codes("let f.inner = {name}: name; in f {}").is_empty());
    assert!(codes("let f = {name}: name; in (f: f {}) other").is_empty());
}
