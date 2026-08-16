//! Integration tests for the style rules.
//!
//! These are snapshot-style tests like the node-rule suite: every
//! diagnostic is rendered with the contract one-line format and compared
//! against an exact string. `run` runs all fifteen style rules and
//! returns the rendered findings in source order.

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::style_rules::{
    CollapsibleLetIn, DeprecatedToPath, EmptyInherit, EmptyLetIn, EmptyListConcat, EmptyPattern,
    EtaReduction, ManualInherit, ManualInheritFrom, RedundantPatternBind, RepeatedKeys,
    UnquotedUri, UselessHasAttr, UselessParens,
};
use strictix_syntax::parse;

/// All fifteen style rules.
fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(EmptyLetIn {}),
        Box::new(ManualInherit {}),
        Box::new(ManualInheritFrom {}),
        Box::new(CollapsibleLetIn {}),
        Box::new(EtaReduction {}),
        Box::new(EmptyPattern {}),
        Box::new(RedundantPatternBind {}),
        Box::new(EmptyInherit {}),
        Box::new(DeprecatedToPath {}),
        Box::new(UselessHasAttr {}),
        Box::new(EmptyListConcat {}),
        Box::new(UselessParens {}),
        Box::new(RepeatedKeys {}),
        Box::new(UnquotedUri {}),
    ]
}

/// Parse `source` and run all fifteen style rules with an empty config.
///
/// Diagnostics are sorted by (start, end) so results are in source
/// order, then rendered as the contract one-line format.
fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &all_rules(),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags.sort_by_key(|d| (d.range.start(), d.range.end()));
    diags.iter().map(render).collect()
}

/// The deterministic one-line rendering shared by CLI and tests.
fn render(diag: &Diagnostic) -> String {
    format!(
        "[{}] {} {}..{} {}",
        diag.code,
        diag.severity_str(),
        diag.range.start(),
        diag.range.end(),
        diag.message
    )
}

/// Run all rules over `source`, returning the raw diagnostics.
fn run_rules_only(source: &str) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &all_rules(),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags
}

/// Apply the fix of the single diagnostic with `code`, returning the
/// rewrite.
fn fix_result(source: &str, code: &str) -> String {
    let diag = run_rules_only(source)
        .into_iter()
        .find(|d| d.code == code)
        .unwrap_or_else(|| panic!("no {code} diagnostic"));
    let fix = diag.fix.expect("diagnostic carries a fix");
    apply_fixes(source, &fix.edits).expect("fix applies")
}

// --- empty-let-in ----------------------------------------------------

#[test]
fn empty_let_in_diag() {
    assert_eq!(
        run("let in 1"),
        vec!["[empty-let-in] warning 0..8 useless let-in expression"],
    );
}

#[test]
fn empty_let_in_no_diag_for_bindings() {
    assert!(run("let x = 1; in x").is_empty());
}

#[test]
fn empty_let_in_fix_removes_let() {
    assert_eq!(fix_result("let in 1", "empty-let-in"), "1");
}

// --- manual-inherit --------------------------------------------------

#[test]
fn manual_inherit_diag() {
    assert_eq!(
        run("{ a = a; }"),
        vec!["[manual-inherit] warning 2..8 assignment instead of inherit"],
    );
}

#[test]
fn manual_inherit_no_diag_for_distinct_names() {
    assert!(run("{ a = b; }").is_empty());
}

#[test]
fn manual_inherit_fix_uses_inherit() {
    assert_eq!(fix_result("{ a = a; }", "manual-inherit"), "{ inherit a; }");
}

// --- manual-inherit-from ---------------------------------------------

#[test]
fn manual_inherit_from_diag() {
    assert_eq!(
        run("{ a = some.a; }"),
        vec!["[manual-inherit-from] warning 2..13 assignment instead of inherit from"],
    );
}

#[test]
fn manual_inherit_from_fix_uses_inherit_from() {
    assert_eq!(
        fix_result("{ a = some.a; }", "manual-inherit-from"),
        "{ inherit (some) a; }",
    );
}

#[test]
fn manual_inherit_from_static_multi_segment_diag() {
    assert_eq!(
        run("{ dev = cfg.devices.dev; }"),
        vec!["[manual-inherit-from] warning 2..24 assignment instead of inherit from"],
    );
}

#[test]
fn manual_inherit_from_static_multi_segment_fix() {
    assert_eq!(
        fix_result("{ dev = cfg.devices.dev; }", "manual-inherit-from"),
        "{ inherit (cfg.devices) dev; }",
    );
}

#[test]
fn manual_inherit_from_dynamic_select_no_diag() {
    // `cfg.devices.${name}` is a dynamic select; the selected attribute
    // path is not a static ident equal to the binding key, so the rule
    // must NOT fire.
    assert!(run("{ dev = cfg.devices.${name}; }").is_empty());
}

#[test]
fn manual_inherit_from_or_default_no_diag() {
    // `x = foo.bar or default` is a SelectExpr with an `or` fallback;
    // rewriting it to `inherit (foo) bar;` would silently drop the
    // default, so the rule must NOT fire.
    assert!(run("{ x = foo.bar or null; }").is_empty());
}

#[test]
fn manual_inherit_from_multiline_fix_preserves_newline() {
    // The binding's node range includes the leading newline; the fix must
    // not eat it (that would merge `let` + `inherit` into `letinherit`).
    assert_eq!(
        fix_result(
            "let\n  system = pkgs.stdenv.hostPlatform.system;\nin null",
            "manual-inherit-from"
        ),
        "let\n  inherit (pkgs.stdenv.hostPlatform) system;\nin null",
    );
}

// --- collapsible-let-in ----------------------------------------------

#[test]
fn collapsible_let_in_diag() {
    assert_eq!(
        run("let x = 1; in let y = 2; in 3"),
        vec!["[collapsible-let-in] warning 0..29 these let-in expressions are collapsible"],
    );
}

#[test]
fn collapsible_let_in_fix_merges_bindings() {
    assert_eq!(
        fix_result("let x = 1; in let y = 2; in 3", "collapsible-let-in"),
        "let x = 1;  y = 2; in 3",
    );
}

// --- eta-reduction ---------------------------------------------------

#[test]
fn eta_reduction_diag() {
    assert_eq!(
        run("x: f x"),
        vec!["[eta-reduction] warning 0..6 this function is eta-reducible"],
    );
}

#[test]
fn eta_reduction_no_diag_when_param_used_as_func() {
    assert!(run("f: f x").is_empty());
}

#[test]
fn eta_reduction_fix() {
    assert_eq!(fix_result("x: f x", "eta-reduction"), "f");
}

// --- empty-pattern ---------------------------------------------------

#[test]
fn empty_pattern_diag() {
    assert_eq!(
        run("{ ... }: 1"),
        vec!["[empty-pattern] warning 0..10 empty pattern in function argument"],
    );
}

#[test]
fn empty_pattern_no_diag_for_bound_formals() {
    assert!(run("{ a, ... }: a").is_empty());
}

#[test]
fn empty_pattern_fix_uses_underscore() {
    assert_eq!(fix_result("{ ... }: 1", "empty-pattern"), "_: 1");
}

// --- redundant-pattern-bind ------------------------------------------

#[test]
fn redundant_pattern_bind_diag() {
    assert_eq!(
        run("args @ { ... }: 1"),
        vec!["[redundant-pattern-bind] warning 0..17 redundant pattern bind in function argument"],
    );
}

#[test]
fn redundant_pattern_bind_fix_drops_formals() {
    assert_eq!(
        fix_result("args @ { ... }: 1", "redundant-pattern-bind"),
        "args: 1",
    );
}

// --- empty-inherit ---------------------------------------------------

#[test]
fn empty_inherit_diag() {
    assert_eq!(
        run("{ inherit; }"),
        vec!["[empty-inherit] warning 2..10 empty inherit statement"],
    );
}

#[test]
fn empty_inherit_no_diag_for_names() {
    assert!(run("{ inherit pkgs; }").is_empty());
}

#[test]
fn empty_inherit_fix_removes() {
    assert_eq!(fix_result("{ inherit; }", "empty-inherit"), "{ }");
}

// --- deprecated-to-path ----------------------------------------------

#[test]
fn deprecated_to_path_diag() {
    assert_eq!(
        run("toPath \"abc\""),
        vec![
            "[deprecated-to-path] warning 0..12 `toPath` is deprecated; use `/. +` or `./. +`"
        ],
    );
}

#[test]
fn deprecated_to_path_no_fix() {
    let diag = run_rules_only("toPath \"abc\"")
        .into_iter()
        .find(|d| d.code == "deprecated-to-path")
        .expect("deprecated-to-path fires");
    assert!(diag.fix.is_none());
}

// --- useless-has-attr ------------------------------------------------

#[test]
fn useless_has_attr_diag() {
    assert_eq!(
        run("if x ? a then x.a else 0"),
        vec![
            "[useless-has-attr] warning 0..24 this if-expression can be simplified with `or`"
        ],
    );
}

#[test]
fn useless_has_attr_fix_uses_or() {
    assert_eq!(
        fix_result("if x ? a then x.a else 0", "useless-has-attr"),
        "x.a or 0",
    );
}

// --- empty-list-concat -----------------------------------------------

#[test]
fn empty_list_concat_diag() {
    assert_eq!(
        run("[] ++ x"),
        vec!["[empty-list-concat] warning 0..7 concatenation with the empty list is a no-op"],
    );
}

#[test]
fn empty_list_concat_fix_drops_empty_list() {
    assert_eq!(fix_result("[] ++ x", "empty-list-concat"), "x");
}

// --- useless-parens --------------------------------------------------

#[test]
fn useless_parens_body_of_let_diag() {
    assert_eq!(
        run("let x = 1; in (2)"),
        vec!["[useless-parens] warning 14..17 useless parentheses around body of let"],
    );
}

#[test]
fn useless_parens_fix_removes_parens() {
    assert_eq!(
        fix_result("let x = 1; in (2)", "useless-parens"),
        "let x = 1; in 2",
    );
}

#[test]
fn useless_parens_general_case_preserves_space() {
    assert_eq!(
        fix_result("f (x)", "useless-parens"),
        "f x",
    );
}

// --- repeated-keys ---------------------------------------------------

#[test]
fn repeated_keys_diag() {
    assert_eq!(
        run("{ a.b = 1; a.c = 2; a.d = 3; }"),
        vec!["[repeated-keys] warning 2..10 key `a` is repeated 3 times; consider nesting"],
    );
}

#[test]
fn repeated_keys_no_diag_for_two() {
    assert!(run("{ a.b = 1; a.c = 2; }").is_empty());
}

// --- unquoted-uri ----------------------------------------------------

#[test]
fn unquoted_uri_diag() {
    assert_eq!(
        run("fetchurl https://example.com/x"),
        vec!["[unquoted-uri] warning 9..30 unquoted URI expression"],
    );
}

#[test]
fn unquoted_uri_fix_quotes() {
    assert_eq!(
        fix_result("fetchurl https://example.com/x", "unquoted-uri"),
        "fetchurl \"https://example.com/x\"",
    );
}
