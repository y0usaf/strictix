use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::unexpected_function_argument::UnexpectedFunctionArgument;
use strictix_syntax::parse;

fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut ds = Vec::<Diagnostic>::new();
    run_rules(
        &[Box::new(UnexpectedFunctionArgument) as Box<dyn Rule>],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut ds,
    );
    ds.into_iter()
        .map(|d| {
            format!(
                "{} {}..{} {}",
                d.code,
                d.range.start(),
                d.range.end(),
                d.message
            )
        })
        .collect()
}

#[test]
fn flags_extra_argument() {
    assert_eq!(
        run("({ a }: a) { a = 1; b = 2; }"),
        ["unexpected-function-argument 20..21 function does not accept argument 'b'"]
    );
}

#[test]
fn accepts_defaults_and_open_patterns() {
    assert!(run("({ a ? 1 }: a) { } ").is_empty());
    assert!(run("({ a, ... }: a) { b = 2; }").is_empty());
}

#[test]
fn resolves_local_binding_and_shadowing() {
    assert_eq!(
        run("let f = { a }: a; in f { b = 1; }"),
        ["unexpected-function-argument 25..26 function does not accept argument 'b'"]
    );
    assert!(run("let f = { a }: a; in let f = { a, b }: a; in f { b = 1; }").is_empty());
}

#[test]
fn skips_dynamic_callees_and_keys() {
    assert!(run("f { b = 1; }").is_empty());
    assert!(run("({ a }: a) { ${key} = 1; }").is_empty());
}

#[test]
fn skips_shadowed_forward_and_dotted_functions() {
    assert!(run("let f = {}: 1; in let x = f {a=1;}; f = x: x; in x").is_empty());
    assert!(run("let f.inner = {}: 1; in f {a=1;}").is_empty());
    assert!(run("let f = {}: 1; in (f: f {a=1;}) other").is_empty());
}
