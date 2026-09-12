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
    assert!(codes("import inputs.nixpkgs", one(DynamicImport)).is_empty());
    assert!(codes("import (toString inputs.nixpkgs)", one(DynamicImport)).is_empty());
    assert!(codes("import (toString flakeInputs.deno2nix)", one(DynamicImport)).is_empty());
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
fn builtin_arity_checks_extra_arguments_and_accepts_partial_applications() {
    assert_eq!(
        codes("builtins.length xs extra", one(BuiltinArity)),
        vec!["builtin-arity"]
    );
    assert!(codes("builtins.elem x", one(BuiltinArity)).is_empty());
    assert!(codes("builtins.map (x: x)", one(BuiltinArity)).is_empty());
    assert!(codes("builtins.replaceStrings [\"a\"] [\"b\"]", one(BuiltinArity)).is_empty());
    assert!(codes("builtins.length xs", one(BuiltinArity)).is_empty());
    assert!(codes("builtins.elem", one(BuiltinArity)).is_empty());

    // A complete application contains partial applications in its syntax tree.
    // Only the outer application represents the call being checked.
    assert!(codes(
        r#"builtins.concatStringsSep " " ["a" "b"]"#,
        one(BuiltinArity)
    )
    .is_empty());
    assert!(codes(
        r#"builtins.replaceStrings ["a"] ["b"] "a""#,
        one(BuiltinArity)
    )
    .is_empty());
    assert!(codes("builtins.filter (x: x > 1) [1 2]", one(BuiltinArity)).is_empty());
    assert!(codes("builtins.map (x: x + 1) [1 2]", one(BuiltinArity)).is_empty());
}

#[test]
fn builtin_arity_accepts_applying_callable_results() {
    for source in [
        "builtins.head [(x: x)] 1",
        "builtins.elemAt [(x: x)] 0 1",
        "builtins.getAttr \"f\" { f = x: x; } 1",
        "builtins.removeAttrs { __functor = self: x: x; } [] 1",
        "builtins.intersectAttrs { __functor = null; } { __functor = self: x: x; } 1",
    ] {
        assert!(codes(source, one(BuiltinArity)).is_empty(), "{source}");
    }
}

#[test]
fn builtin_arity_checks_nested_calls_and_parenthesized_chains() {
    for source in [
        "f (builtins.length [] 1)",
        "(builtins.length []) 1",
        "((builtins.length) []) 1",
    ] {
        assert_eq!(
            codes(source, one(BuiltinArity)),
            ["builtin-arity"],
            "{source}"
        );
    }
    assert!(codes("(builtins.map (x: x)) [1]", one(BuiltinArity)).is_empty());
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
    for source in [
        "let value = builtins.length [] 1; builtins = { length = a: b: b; }; in value",
        "builtins.length.custom [] 1",
        "(builtins.length or (x: y: y)) [] 1",
        "(rec { \"builtins\" = { length = _: _: 1; }; result = builtins.length [] 1; }).result",
        "(builtins@{ x ? builtins.length [] 1, ... }: x) { length = _: _: 1; }",
        "({ x ? builtins.length [] 1, ... }@builtins: x) { length = _: _: 1; }",
    ] {
        assert!(codes(source, one(BuiltinArity)).is_empty(), "{source}");
    }
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
    assert!(codes("import inputs.nixpkgs", one(SuspiciousImportArgument)).is_empty());
    assert!(codes(
        "import (toString inputs.nixpkgs)",
        one(SuspiciousImportArgument)
    )
    .is_empty());
}
