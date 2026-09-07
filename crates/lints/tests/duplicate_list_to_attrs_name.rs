use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::run_rules;
use strictix_core::semantic::SemanticModel;
use strictix_lints::duplicate_list_to_attrs_name::DuplicateListToAttrsName;
use strictix_syntax::parse;

fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diagnostics = Vec::new();
    run_rules(
        &[Box::new(DuplicateListToAttrsName)],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diagnostics,
    );
    diagnostics.iter().map(render).collect()
}

fn render(diagnostic: &Diagnostic) -> String {
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
fn reports_repeated_literal_names_and_keeps_first_occurrence() {
    let source =
        "builtins.listToAttrs [ { name = \"x\"; value = 1; } { name = \"x\"; value = 2; } ]";
    assert_eq!(
        run(source),
        ["[duplicate-list-to-attrs-name] warning 59..62 duplicate listToAttrs name 'x'; first occurrence wins"]
    );
}

#[test]
fn supports_qualified_builtin_and_quoted_attribute_name() {
    let source =
        "builtins.listToAttrs [ { \"name\" = \"x\"; value = 1; } { name = \"x\"; value = 2; } ]";
    assert_eq!(
        run(source),
        ["[duplicate-list-to-attrs-name] warning 61..64 duplicate listToAttrs name 'x'; first occurrence wins"]
    );
}

#[test]
fn skips_shadowed_builtins_and_dynamic_entries() {
    assert!(run(
        "let builtins = {}; in builtins.listToAttrs [ { name = \"x\"; } { name = \"x\"; } ]"
    )
    .is_empty());
    // A forward let binding still shadows the builtin under Nix scoping.
    assert!(run(
        "let x = builtins.listToAttrs [ { name = \"x\"; } { name = \"x\"; } ]; builtins = {}; in x"
    )
    .is_empty());
    assert!(run("listToAttrs [ { name = \"x\"; } { name = \"x\"; } ]").is_empty());
    assert_eq!(
        run("builtins.listToAttrs [ { name = n; } { name = \"x\"; } { name = \"x\"; } ]"),
        ["[duplicate-list-to-attrs-name] warning 62..65 duplicate listToAttrs name 'x'; first occurrence wins"]
    );
}

#[test]
fn finds_qualified_call_inside_another_application() {
    let source = "foo (builtins.listToAttrs [ { name = \"x\"; } { name = \"x\"; } ])";
    assert_eq!(
        run(source),
        ["[duplicate-list-to-attrs-name] warning 53..56 duplicate listToAttrs name 'x'; first occurrence wins"]
    );
}

#[test]
fn string_escapes_follow_nix_semantics() {
    assert_eq!(
        run(r#"builtins.listToAttrs [{name="\q";} {name="q";}]"#).len(),
        1
    );
    assert!(run(r#"builtins.listToAttrs [{name="\q";} {name="\\q";}]"#).is_empty());
    assert_eq!(
        run(r#"builtins.listToAttrs [{name="\n";} {name="\n";}]"#).len(),
        1
    );
}
