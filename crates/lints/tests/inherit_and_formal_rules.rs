use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::duplicate_inherit::DuplicateInherit;
use strictix_lints::file_rules::{ShadowedFormal, UnusedInherit};
use strictix_syntax::parse;

fn run(source: &str, rules: Vec<Box<dyn Rule>>) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &rules,
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags.into_iter().map(render).collect()
}

fn render(diagnostic: Diagnostic) -> String {
    format!(
        "[{}] {} {}..{} {}",
        diagnostic.code,
        diagnostic.severity_str(),
        diagnostic.range.start(),
        diagnostic.range.end(),
        diagnostic.message
    )
}

#[test]
fn duplicate_inherit_flags_repeated_names_once() {
    assert_eq!(
        run(
            "{ inherit a; inherit a; }",
            vec![Box::new(DuplicateInherit)]
        ),
        ["[duplicate-inherit] error 21..22 attribute 'a' inherited more than once"]
    );
}

#[test]
fn duplicate_inherit_does_not_duplicate_attribute_diagnostic() {
    assert_eq!(
        run(
            "{ inherit a; inherit a; }",
            vec![
                Box::new(DuplicateInherit),
                Box::new(strictix_lints::duplicate_attr::DuplicateAttribute)
            ],
        ),
        ["[duplicate-inherit] error 21..22 attribute 'a' inherited more than once"]
    );
}

#[test]
fn unused_inherit_flags_only_unused_names() {
    assert_eq!(
        run(
            "rec { inherit (src) used unused; x = used; }",
            vec![Box::new(UnusedInherit)],
        ),
        ["[unused-inherit] warning 25..31 inherited name 'unused' is never used"]
    );
}

#[test]
fn shadowed_formal_has_its_own_diagnostic() {
    assert_eq!(
        run(
            "let a = 1; in { a }: a",
            vec![
                Box::new(ShadowedFormal),
                Box::new(strictix_lints::file_rules::ShadowedBinding)
            ],
        ),
        ["[shadowed-formal] warning 16..17 formal parameter 'a' shadows an outer binding"]
    );
}
