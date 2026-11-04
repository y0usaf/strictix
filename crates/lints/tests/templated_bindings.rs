use std::path::Path;

use strictix_core::{
    config::LintConfig, diagnostic::Diagnostic, fix::apply_fixes, rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::templated_bindings::TemplatedBindings;
use strictix_syntax::parse;

fn check(source: &str, config: &LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree).with_path(Some(Path::new("m.nix")));
    let mut diags = Vec::new();
    TemplatedBindings.check_file(&model, config, &mut diags);
    diags
}

fn fixed(source: &str) -> String {
    let edits: Vec<_> = check(source, &LintConfig::default())
        .into_iter()
        .filter_map(|d| d.fix)
        .flat_map(|f| f.edits)
        .collect();
    apply_fixes(source, &edits).unwrap()
}

#[test]
fn bindings_that_differ_by_their_name_become_one_gen_attrs() {
    let source =
        "{lib}: {\n  a = v: {a = v;};\n\n  b = v: {b = v;};\n\n  c = v: {c = v;};\n\n  d = 1;\n}\n";
    let diags = check(source, &LintConfig::default());
    assert_eq!(diags.len(), 1);
    assert_eq!(
        fixed(source),
        "{lib}: {\n  inherit (lib.genAttrs [\"a\" \"b\" \"c\"] (name: v: {${name} = v;})) a b c;\n\n  d = 1;\n}\n"
    );
}

#[test]
fn a_string_of_the_name_is_the_name_and_a_let_inherits() {
    let source = "{lib}: let\n  x = \"x\";\n  y = \"y\";\n  z = \"z\";\nin x\n";
    assert_eq!(
        fixed(source),
        "{lib}: let\n  inherit (lib.genAttrs [\"x\" \"y\" \"z\"] lib.id) x y z;\nin x\n"
    );
}

#[test]
fn a_bound_name_is_not_the_parameter() {
    let source = "{lib}: let\n  name = 1;\nin {\n  a = v: {a = v;};\n  b = v: {b = v;};\n  c = v: {c = v;};\n}\n";
    assert!(fixed(source).contains("(key: v: {${key} = v;})"));
}

#[test]
fn groups_below_min_and_without_lib_are_not_fixed() {
    let two = "{lib}: {\n  a = v: {a = v;};\n  b = v: {b = v;};\n}\n";
    assert!(check(two, &LintConfig::default()).is_empty());
    let config = LintConfig::default().with_setting("templated-bindings", "min", "2");
    assert_eq!(check(two, &config).len(), 1);
    let bare = "{\n  a = v: {a = v;};\n  b = v: {b = v;};\n  c = v: {c = v;};\n}\n";
    let diags = check(bare, &LintConfig::default());
    assert_eq!(diags.len(), 1);
    assert!(diags[0].fix.is_none());
}
