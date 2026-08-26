use strictix_core::{
    config::LintConfig,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::advanced_rules::{
    BuiltinArity, DuplicateFunctionArgument, DynamicImport, ImportInList, SuspiciousImportArgument,
    SuspiciousRecursion, UnreachableBranch, UnsafeWithShadowing,
};
use strictix_syntax::parse;

fn codes(source: &str, rules: Vec<Box<dyn Rule>>) -> Vec<&'static str> {
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
    diags.into_iter().map(|d| d.code).collect()
}

fn one(rule: impl Rule + 'static) -> Vec<Box<dyn Rule>> {
    vec![Box::new(rule)]
}

#[test]
fn duplicate_function_argument_fires_only_for_adjacent_equal_args() {
    assert_eq!(
        codes("f x x", one(DuplicateFunctionArgument)),
        vec!["duplicate-function-argument"]
    );
    assert!(codes("f x y", one(DuplicateFunctionArgument)).is_empty());
}

#[test]
fn unreachable_branch_fires_for_literal_conditions() {
    assert_eq!(
        codes("if true then 1 else 2", one(UnreachableBranch)),
        vec!["unreachable-branch"]
    );
    assert_eq!(
        codes("if false then 1 else 2", one(UnreachableBranch)),
        vec!["unreachable-branch"]
    );
    assert!(codes("if condition then 1 else 2", one(UnreachableBranch)).is_empty());
}

#[test]
fn unsafe_with_shadowing_fires_only_on_with_fallbacks() {
    assert_eq!(
        codes("with pkgs; hello", one(UnsafeWithShadowing)),
        vec!["unsafe-with-shadowing"]
    );
    assert!(codes("let hello = 1; in hello", one(UnsafeWithShadowing)).is_empty());
}

#[test]
fn dynamic_import_fires_for_interpolated_paths() {
    assert_eq!(
        codes("import path", one(DynamicImport)),
        vec!["dynamic-import"]
    );
    assert!(codes("import ./module.nix", one(DynamicImport)).is_empty());
}

#[test]
fn import_in_list_fires_for_direct_import_items() {
    assert_eq!(
        codes("[ import ./a.nix ]", one(ImportInList)),
        vec!["import-in-list"]
    );
    assert!(codes("import ./a.nix", one(ImportInList)).is_empty());
}

#[test]
fn builtin_arity_checks_missing_and_extra_arguments() {
    assert_eq!(
        codes("builtins.length xs extra", one(BuiltinArity)),
        vec!["builtin-arity"]
    );
    assert_eq!(
        codes("builtins.elem x", one(BuiltinArity)),
        vec!["builtin-arity"]
    );
    assert!(codes("builtins.length xs", one(BuiltinArity)).is_empty());
    assert!(codes("builtins.elem", one(BuiltinArity)).is_empty());
}

#[test]
fn builtin_arity_skips_shadowed_and_with_provided_builtins() {
    assert!(codes(
        "let builtins = { length = x: x; }; in builtins.length",
        one(BuiltinArity)
    )
    .is_empty());
    assert!(codes(
        "with { builtins = { length = x: x; }; }; builtins.length",
        one(BuiltinArity)
    )
    .is_empty());
}

#[test]
fn suspicious_recursion_fires_for_direct_self_reference() {
    assert_eq!(
        codes("rec { value = value; }", one(SuspiciousRecursion)),
        vec!["suspicious-recursion"]
    );
    assert!(codes("rec { value = n: value; }", one(SuspiciousRecursion)).is_empty());
}

#[test]
fn suspicious_import_argument_fires_for_non_path_literals() {
    assert_eq!(
        codes("import 1", one(SuspiciousImportArgument)),
        vec!["suspicious-import-argument"]
    );
    assert!(codes("import ./module.nix", one(SuspiciousImportArgument)).is_empty());
}
