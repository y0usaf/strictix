use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::call_simplification::{
    DeprecatedIsNull, ManualGetattr, ManualHasattr, ManualOptional, OptionalListArgument,
};
use strictix_syntax::parse;

fn run(source: &str, rule: Box<dyn Rule>) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diagnostics = Vec::new();
    run_rules(
        &[rule],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diagnostics,
    );
    diagnostics
}

/// Run one rule expecting exactly one diagnostic with a fix, apply the
/// fix, and return the fixed source.
fn fixed(source: &str, rule: Box<dyn Rule>) -> String {
    let diagnostics = run(source, rule);
    assert_eq!(diagnostics.len(), 1, "{source}");
    let fix = diagnostics[0].fix.as_ref().expect("fix offered");
    apply_fixes(source, &fix.edits).expect("fix applies")
}

// --- manual-hasattr ---------------------------------------------------

#[test]
fn manual_hasattr_fires_on_builtins_form_with_bare_key() {
    let diagnostics = run("builtins.hasAttr \"a\" x", Box::new(ManualHasattr {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "manual-hasattr");
    assert_eq!(
        fixed("builtins.hasAttr \"a\" x", Box::new(ManualHasattr {})),
        "x ? a"
    );
}

#[test]
fn manual_hasattr_fires_on_bare_global_form() {
    assert_eq!(
        fixed("hasAttr \"a\" x", Box::new(ManualHasattr {})),
        "x ? a"
    );
}

#[test]
fn manual_hasattr_keeps_non_identifier_and_keyword_keys_quoted() {
    assert_eq!(
        fixed("hasAttr \"a b\" x", Box::new(ManualHasattr {})),
        "x ? \"a b\""
    );
    assert_eq!(
        fixed("hasAttr \"if\" x", Box::new(ManualHasattr {})),
        "x ? \"if\""
    );
}

#[test]
fn manual_hasattr_renders_dashed_key_bare() {
    assert_eq!(
        fixed("hasAttr \"a-b'\" x", Box::new(ManualHasattr {})),
        "x ? a-b'"
    );
}

#[test]
fn manual_hasattr_keeps_parenthesized_set_verbatim() {
    assert_eq!(
        fixed("hasAttr \"a\" (f x)", Box::new(ManualHasattr {})),
        "(f x) ? a"
    );
}

#[test]
fn manual_hasattr_parenthesizes_replacement_under_tighter_parent() {
    assert_eq!(
        fixed("!hasAttr \"a\" x", Box::new(ManualHasattr {})),
        "!(x ? a)"
    );
}

#[test]
fn manual_hasattr_is_silent_when_shadowed_or_with_covered() {
    assert!(run(
        "let hasAttr = f: s: true; in hasAttr \"a\" x",
        Box::new(ManualHasattr {})
    )
    .is_empty());
    assert!(run("with lib; hasAttr \"a\" x", Box::new(ManualHasattr {})).is_empty());
}

#[test]
fn manual_hasattr_is_silent_on_interpolated_name_and_partial_application() {
    assert!(run("k: x: hasAttr \"${k}\" x", Box::new(ManualHasattr {})).is_empty());
    assert!(run("hasAttr \"a\"", Box::new(ManualHasattr {})).is_empty());
}

// --- manual-getattr ---------------------------------------------------

#[test]
fn manual_getattr_fires_on_both_callee_forms() {
    assert_eq!(
        fixed("builtins.getAttr \"a\" x", Box::new(ManualGetattr {})),
        "x.a"
    );
    assert_eq!(fixed("getAttr \"a\" x", Box::new(ManualGetattr {})), "x.a");
}

#[test]
fn manual_getattr_keeps_quoted_key_and_parenthesized_set() {
    assert_eq!(
        fixed("getAttr \"a b\" x", Box::new(ManualGetattr {})),
        "x.\"a b\""
    );
    assert_eq!(
        fixed("getAttr \"a\" (f x)", Box::new(ManualGetattr {})),
        "(f x).a"
    );
}

#[test]
fn manual_getattr_is_silent_on_shadow_interpolation_and_partial() {
    assert!(run(
        "let getAttr = a: b: a; in getAttr \"a\" x",
        Box::new(ManualGetattr {})
    )
    .is_empty());
    assert!(run("k: x: getAttr \"${k}\" x", Box::new(ManualGetattr {})).is_empty());
    assert!(run("getAttr \"a\"", Box::new(ManualGetattr {})).is_empty());
}

// --- deprecated-is-null -----------------------------------------------

#[test]
fn deprecated_is_null_fires_on_both_callee_forms() {
    assert_eq!(
        fixed("isNull x", Box::new(DeprecatedIsNull {})),
        "x == null"
    );
    assert_eq!(
        fixed("builtins.isNull (f x)", Box::new(DeprecatedIsNull {})),
        "(f x) == null"
    );
}

#[test]
fn deprecated_is_null_parenthesizes_replacement_under_binary_parent() {
    assert_eq!(
        fixed("isNull x && y", Box::new(DeprecatedIsNull {})),
        "(x == null) && y"
    );
}

#[test]
fn deprecated_is_null_is_silent_when_shadowed_or_with_covered() {
    assert!(run(
        "let isNull = v: false; in isNull x",
        Box::new(DeprecatedIsNull {})
    )
    .is_empty());
    assert!(run("with lib; isNull x", Box::new(DeprecatedIsNull {})).is_empty());
}

// --- manual-optional --------------------------------------------------

#[test]
fn manual_optional_rewrites_single_item_list() {
    assert_eq!(
        fixed(
            "lib: c: x: if c then [ x ] else [ ]",
            Box::new(ManualOptional {})
        ),
        "lib: c: x: lib.optional c x"
    );
}

#[test]
fn manual_optional_rewrites_multi_item_list_to_optionals() {
    assert_eq!(
        fixed(
            "let lib = 1; in if c then [ a b ] else [ ]",
            Box::new(ManualOptional {})
        ),
        "let lib = 1; in lib.optionals c [ a b ]"
    );
}

#[test]
fn manual_optional_rewrites_string_and_attrset_forms() {
    assert_eq!(
        fixed(
            "lib: c: s: if c then \"pre ${s}\" else \"\"",
            Box::new(ManualOptional {})
        ),
        "lib: c: s: lib.optionalString c \"pre ${s}\""
    );
    assert_eq!(
        fixed(
            "lib: c: if c then { a = 1; } else { }",
            Box::new(ManualOptional {})
        ),
        "lib: c: lib.optionalAttrs c { a = 1; }"
    );
}

#[test]
fn manual_optional_parenthesizes_apply_condition_and_unwraps_paren_branches() {
    assert_eq!(
        fixed(
            "lib: f: x: if f x then ([ 1 ]) else ([ ])",
            Box::new(ManualOptional {})
        ),
        "lib: f: x: lib.optional (f x) 1"
    );
}

#[test]
fn manual_optional_is_silent_without_lib_in_scope() {
    assert!(run(
        "c: x: if c then [ x ] else [ ]",
        Box::new(ManualOptional {})
    )
    .is_empty());
}

#[test]
fn manual_optional_leaves_literal_conditions_to_constant_if() {
    assert!(run(
        "lib: if true then [ 1 ] else [ ]",
        Box::new(ManualOptional {})
    )
    .is_empty());
    assert!(run(
        "lib: if false then [ 1 ] else [ ]",
        Box::new(ManualOptional {})
    )
    .is_empty());
}

#[test]
fn manual_optional_is_silent_on_non_matching_shapes() {
    // reversed polarity
    assert!(run(
        "lib: c: x: if c then [ ] else [ x ]",
        Box::new(ManualOptional {})
    )
    .is_empty());
    // non-empty else branch
    assert!(run(
        "lib: c: if c then [ 1 ] else [ 2 ]",
        Box::new(ManualOptional {})
    )
    .is_empty());
    // rec attrset then-branch
    assert!(run(
        "lib: c: if c then rec { a = 1; } else { }",
        Box::new(ManualOptional {})
    )
    .is_empty());
    // empty then-list: nothing to lift
    assert!(run(
        "lib: c: if c then [ ] else [ ]",
        Box::new(ManualOptional {})
    )
    .is_empty());
}

// --- optional-list-argument --------------------------------------------

#[test]
fn optional_list_argument_fires_on_lib_select_with_literal_list() {
    let source = "lib: c: lib.optional c [ 1 2 ]";
    let diagnostics = run(source, Box::new(OptionalListArgument {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "optional-list-argument");
    assert_eq!(
        fixed(source, Box::new(OptionalListArgument {})),
        "lib: c: lib.optionals c [ 1 2 ]"
    );
}

#[test]
fn optional_list_argument_fires_on_bare_form_under_with() {
    assert_eq!(
        fixed(
            "with lib; c: optional c [ 1 ]",
            Box::new(OptionalListArgument {})
        ),
        "with lib; c: optionals c [ 1 ]"
    );
}

#[test]
fn optional_list_argument_is_silent_for_lexically_bound_callees() {
    // a let-bound bare `optional` is the user's own function
    assert!(run(
        "let optional = c: v: v; in optional c [ 1 ]",
        Box::new(OptionalListArgument {})
    )
    .is_empty());
    // a let-bound `lib` is not provably nixpkgs lib
    assert!(run(
        "let lib = x; in lib.optional c [ 1 ]",
        Box::new(OptionalListArgument {})
    )
    .is_empty());
    // bare `optional` with neither binding nor with is undefined-variable's finding
    assert!(run("optional c [ 1 ]", Box::new(OptionalListArgument {})).is_empty());
}

#[test]
fn optional_list_argument_is_silent_for_non_literal_second_argument() {
    assert!(run(
        "lib: c: xs: lib.optional c xs",
        Box::new(OptionalListArgument {})
    )
    .is_empty());
}
