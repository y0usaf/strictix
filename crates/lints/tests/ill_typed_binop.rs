//! Integration tests for the `ill-typed-binop` node rule.
//!
//! The rule flags a binary operator applied to statically ill-typed
//! literal operands (certain eval errors hidden by laziness) and offers
//! exactly one auto-fix: `+` on two plain, interpolation-free string
//! literals collapses to their concatenation.
//!
//! Diagnostics are rendered in the contract one-line format
//! (`[code] severity start..end message`) and asserted exactly, byte
//! offsets recomputed from the source strings.

use strictix_core::{
    config::LintConfig, diagnostic::Diagnostic, fix::apply_fixes, rules::run_rules,
    semantic::SemanticModel,
};
use strictix_lints::ill_typed_binop::IllTypedBinop;
use strictix_syntax::parse;

/// Render diagnostics in the one-line contract format.
fn render(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .map(|d| {
            format!(
                "[{}] {} {}..{} {}",
                d.code,
                d.severity_str(),
                d.range.start(),
                d.range.end(),
                d.message
            )
        })
        .collect()
}

/// Run the rule over `source` and render findings.
fn run(source: &str) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(IllTypedBinop {})],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    render(&diags)
}

/// The raw diagnostics for `source` (to inspect the fix payload).
fn raw_diags(source: &str) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut out = Vec::new();
    run_rules(
        &[Box::new(IllTypedBinop {})],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut out,
    );
    out
}

#[test]
fn plus_strings_flag_and_fix() {
    let source = "\"a\" + \"b\"";
    assert_eq!(
        run(source),
        ["[ill-typed-binop] error 0..9 `+` operands are statically ill-typed: \"a\" and \"b\""]
    );
    let d = raw_diags(source);
    let fix = d[0].fix.as_ref().expect("string+string has a fix");
    assert_eq!(fix.label, "concatenate string literals");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "\"ab\"");
}

#[test]
fn plus_path_carve_out_is_clean() {
    assert_eq!(
        run("./src + \"/x\""),
        Vec::<String>::new(),
        "path+string is legal Nix; the path carve-out skips it"
    );
}

#[test]
fn plus_with_ident_is_clean() {
    assert_eq!(run("x + \"/suffix\""), Vec::<String>::new());
}

#[test]
fn plus_list_flags() {
    assert_eq!(
        run("[ 1 ] + 2"),
        ["[ill-typed-binop] error 0..9 `+` operands are statically ill-typed: [ 1 ] and 2"]
    );
}

#[test]
fn plus_int_string_flags() {
    assert_eq!(
        run("1 + \"a\""),
        ["[ill-typed-binop] error 0..7 `+` operands are statically ill-typed: 1 and \"a\""]
    );
}

#[test]
fn minus_string_flags() {
    assert_eq!(
        run("\"/\" - 1"),
        ["[ill-typed-binop] error 0..7 `-` operands are statically ill-typed: \"/\" and 1"]
    );
}

#[test]
fn minus_path_carve_out_is_clean() {
    assert_eq!(run("./a - 1"), Vec::<String>::new());
}

#[test]
fn star_list_flags_int_times_int_is_clean() {
    assert_eq!(
        run("[ 1 ] * 2"),
        ["[ill-typed-binop] error 0..9 `*` operands are statically ill-typed: [ 1 ] and 2"]
    );
    assert_eq!(run("2 * 3"), Vec::<String>::new());
}

#[test]
fn slash_string_flags() {
    assert_eq!(
        run("\"/\" / 2"),
        ["[ill-typed-binop] error 0..7 `/` operands are statically ill-typed: \"/\" and 2"]
    );
}

#[test]
fn concat_string_list_flags() {
    assert_eq!(
        run("\"x\" ++ [ 1 ]"),
        ["[ill-typed-binop] error 0..12 `++` operands are statically ill-typed: \"x\" and [ 1 ]"]
    );
}

#[test]
fn concat_list_list_is_clean() {
    assert_eq!(run("[ 1 ] ++ [ 2 ]"), Vec::<String>::new());
}

#[test]
fn concat_attrset_attrset_flags() {
    assert_eq!(
        run("{} ++ {}"),
        ["[ill-typed-binop] error 0..8 `++` operands are statically ill-typed: {} and {}"]
    );
}

#[test]
fn merge_attrsets_is_clean() {
    assert_eq!(run("{} // { a = 1; }"), Vec::<String>::new());
}

#[test]
fn merge_string_lhs_flags() {
    assert_eq!(
        run("\"s\" // x"),
        ["[ill-typed-binop] error 0..8 `//` operands are statically ill-typed: \"s\" and x"]
    );
}

#[test]
fn merge_int_lhs_flags_int_merge_ill_typed() {
    assert_eq!(
        run("1 // 2"),
        ["[ill-typed-binop] error 0..6 `//` operands are statically ill-typed: 1 and 2"]
    );
}

#[test]
fn bool_int_flags() {
    assert_eq!(
        run("1 && 2"),
        ["[ill-typed-binop] error 0..6 `&&` operands are statically ill-typed: 1 and 2"]
    );
}

#[test]
fn bool_or_nulls_flags() {
    assert_eq!(
        run("null || \"x\""),
        ["[ill-typed-binop] error 0..11 `||` operands are statically ill-typed: null and \"x\""]
    );
}

#[test]
fn bool_true_x_is_clean() {
    assert_eq!(run("true && x"), Vec::<String>::new());
}

#[test]
fn relational_ops_are_clean() {
    assert_eq!(run("1 == 1"), Vec::<String>::new());
    assert_eq!(run("\"a\" < \"b\""), Vec::<String>::new());
    assert_eq!(run("[ 1 ] != 2"), Vec::<String>::new());
}

#[test]
fn paren_unwrap_one_layer_fires_with_fix() {
    assert_eq!(
        run("(\"a\") + \"b\""),
        ["[ill-typed-binop] error 0..11 `+` operands are statically ill-typed: (\"a\") and \"b\""]
    );
    let src = "(\"a\") + \"b\"";
    let d = raw_diags(src);
    let fix = d[0]
        .fix
        .as_ref()
        .expect("paren-wrapped string+string fixes");
    let result = apply_fixes(src, &fix.edits).expect("fix applies");
    assert_eq!(result, "\"ab\"");
}

#[test]
fn double_paren_ident_clean() {
    assert_eq!(run("((x)) + 1"), Vec::<String>::new());
}

#[test]
fn plus_strings_fix_escapes_embedded_quotes() {
    let source = "\"a\\\"b\" + \"c\"";
    let d = raw_diags(source);
    assert_eq!(d.len(), 1);
    let fix = d[0].fix.as_ref().expect("plain strings have a fix");
    let result = apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "\"a\\\"bc\"");
}

#[test]
fn plus_interpolated_string_has_no_fix() {
    let source = "\"${x}\" + \"b\"";
    assert_eq!(run(source).len(), 1);
    assert!(raw_diags(source)[0].fix.is_none());
}

#[test]
fn plus_ind_string_has_no_fix() {
    let source = "''a'' + \"b\"";
    assert_eq!(run(source).len(), 1);
    assert!(raw_diags(source)[0].fix.is_none());
}
